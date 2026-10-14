//! Autosave and recovery after a crash.
//!
//! Each running instance owns a session folder in the autosave directory, holding an
//! exclusive lock on its `session.lock` file for as long as it runs. The document is
//! periodically saved there (`project.patine`, with `info.ron`). A clean exit deletes the
//! folder; a folder whose lock is free at startup was left by a crash, and its project
//! can be recovered. Two instances running together never see each other's session as
//! abandoned, since the lock of a live session is held.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::document::Document;

const LOCK: &str = "session.lock";
const PROJECT: &str = "project.patine";
const INFO: &str = "info.ron";

/// Default time between autosaves.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// What the recovery dialog shows about an autosaved project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutosaveInfo {
    /// Document name.
    pub name: String,
    /// File the document was opened from or saved to, if any.
    pub original_path: Option<PathBuf>,
}

/// The autosave directory: `PATINE_AUTOSAVE_DIR`, or the user's application data.
pub fn default_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PATINE_AUTOSAVE_DIR") {
        return Some(PathBuf::from(dir));
    }
    data_dir().map(|dir| dir.join("autosave"))
}

/// The user's application data folder of Patine (`%APPDATA%\Patine` on Windows,
/// `$XDG_DATA_HOME/patine` elsewhere).
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Patine"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|dir| dir.join("patine"))
    }
}

/// The session folder of this instance, locked while it lives.
#[derive(Debug)]
pub struct Session {
    dir: PathBuf,
    /// Held open: the lock lasts as long as the handle.
    lock: File,
}

impl Session {
    /// Creates and locks a new session folder under `root`.
    pub fn start(root: &Path) -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let dir = root.join(format!("{stamp}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::Error(error) => error,
            TryLockError::WouldBlock => std::io::Error::other("session folder already locked"),
        })?;
        Ok(Self { dir, lock })
    }

    /// Where the document is autosaved.
    pub fn project_path(&self) -> PathBuf {
        self.dir.join(PROJECT)
    }

    /// Records what the autosaved document is.
    pub fn write_info(&self, info: &AutosaveInfo) -> std::io::Result<()> {
        let text = ron::ser::to_string_pretty(info, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        std::fs::write(self.dir.join(INFO), text)
    }

    /// Deletes the autosaved document (it was saved, or replaced).
    pub fn clear(&self) {
        for name in [PROJECT, INFO] {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
    }

    /// Ends the session cleanly: the folder is deleted.
    pub fn finish(self) {
        let Self { dir, lock } = self;
        drop(lock);
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            tracing::warn!(dir = %dir.display(), %error, "cannot delete the autosave session");
        }
    }
}

/// A project left by a session that did not end cleanly.
#[derive(Clone, Debug, PartialEq)]
pub struct Orphan {
    /// Session folder.
    pub dir: PathBuf,
    /// Autosaved project.
    pub project: PathBuf,
    /// Its description.
    pub info: AutosaveInfo,
    /// Time since the autosave.
    pub age: Duration,
}

impl Orphan {
    /// Deletes the session folder.
    pub fn discard(&self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir) {
            tracing::warn!(dir = %self.dir.display(), %error, "cannot delete the autosave");
        }
    }
}

/// Whether the session folder `dir` is abandoned: its lock can be taken.
fn abandoned(dir: &Path) -> bool {
    match File::options().write(true).open(dir.join(LOCK)) {
        // The lock is released when the handle is dropped.
        Ok(file) => file.try_lock().is_ok(),
        // A folder without lock file is not a session (or is broken): leave it alone.
        Err(_) => false,
    }
}

/// Projects left by crashed sessions under `root`, most recent first. Abandoned folders
/// without a project (a crash before the first autosave) are deleted.
pub fn orphans(root: &Path) -> Vec<Orphan> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let dir = entry.path();
        if !dir.is_dir() || !abandoned(&dir) {
            continue;
        }
        let project = dir.join(PROJECT);
        let info = std::fs::read_to_string(dir.join(INFO))
            .ok()
            .and_then(|text| ron::from_str::<AutosaveInfo>(&text).ok());
        let modified = std::fs::metadata(&project).and_then(|meta| meta.modified());
        match (info, modified) {
            (Some(info), Ok(modified)) => found.push(Orphan {
                dir,
                project,
                info,
                age: modified.elapsed().unwrap_or_default(),
            }),
            _ => {
                tracing::info!(dir = %dir.display(), "deleting an empty abandoned session");
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
    found.sort_by_key(|orphan| orphan.age);
    found
}

/// Result of an autosave thread: document generation, history state and outcome.
type AutosaveResult = (u64, u64, Result<(), String>);

/// Saves the open document into the session folder every `interval`, while it has
/// changes that are neither saved by the user nor autosaved yet.
#[derive(Debug)]
pub struct Autosaver {
    session: Session,
    interval: Duration,
    /// When the last autosave started (or the session began).
    last: Instant,
    /// Document generation and history state written in the session folder.
    saved: Option<(u64, u64)>,
    /// Whether the session folder holds an autosaved project.
    has_file: bool,
    /// Running autosave: generation, state and result.
    task: Option<Receiver<AutosaveResult>>,
}

impl Autosaver {
    /// Autosaves into `session` every `interval`.
    pub fn new(session: Session, interval: Duration) -> Self {
        Self {
            session,
            interval,
            last: Instant::now(),
            saved: None,
            has_file: false,
            task: None,
        }
    }

    /// Whether `document` (of generation `generation`) has changes to autosave.
    fn pending(&self, document: &Document, generation: u64) -> bool {
        document.history.is_dirty()
            && self.saved != Some((generation, document.history.current_state()))
    }

    /// When the next autosave is due, if one is.
    pub fn deadline(&self, document: Option<&Document>, generation: u64) -> Option<Instant> {
        let document = document?;
        (self.task.is_none() && self.pending(document, generation))
            .then(|| self.last + self.interval)
    }

    /// Makes the next autosave due now (a recovered document must be protected again).
    pub fn due_now(&mut self) {
        self.last = Instant::now()
            .checked_sub(self.interval)
            .unwrap_or_else(Instant::now);
    }

    /// Collects a finished autosave, deletes the autosave once the document is saved or
    /// closed, and starts an autosave when one is due. `generation` identifies the open
    /// document: it changes whenever the document is replaced.
    pub fn tick(&mut self, ctx: &egui::Context, document: Option<&Document>, generation: u64) {
        if let Some(task) = &self.task {
            match task.try_recv() {
                Ok((done_generation, state, result)) => {
                    self.task = None;
                    match result {
                        Ok(()) => {
                            self.saved = Some((done_generation, state));
                            self.has_file = true;
                        }
                        Err(message) => tracing::warn!(%message, "autosave failed"),
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => self.task = None,
            }
        }
        let Some(document) = document.filter(|document| document.history.is_dirty()) else {
            if self.has_file {
                self.session.clear();
                self.has_file = false;
                self.saved = None;
            }
            return;
        };
        if !self.pending(document, generation) || self.last.elapsed() < self.interval {
            return;
        }
        self.last = Instant::now();
        let info = AutosaveInfo {
            name: document.name(),
            original_path: document.path.clone(),
        };
        if let Err(error) = self.session.write_info(&info) {
            tracing::warn!(%error, "cannot write the autosave description");
            return;
        }
        let snapshot = document.project.clone();
        let state = document.history.current_state();
        let path = self.session.project_path();
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result =
                patine_core::file::save(&snapshot, &path, None).map_err(|error| error.to_string());
            if result.is_ok() {
                tracing::info!(ms = start.elapsed().as_millis() as u64, "autosaved");
            }
            let _ = sender.send((generation, state, result));
            ctx.request_repaint();
        });
        self.task = Some(receiver);
    }

    /// Ends the session cleanly, after a running autosave.
    pub fn finish(self) {
        if let Some(task) = &self.task {
            let _ = task.recv();
        }
        self.session.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("patine-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_live_session_is_not_an_orphan() {
        let root = root("live");
        let session = Session::start(&root).unwrap();
        std::fs::write(session.project_path(), b"project").unwrap();
        session
            .write_info(&AutosaveInfo {
                name: "helmet".into(),
                original_path: None,
            })
            .unwrap();
        assert!(orphans(&root).is_empty());
        session.finish();
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "finish deletes the folder"
        );
    }

    #[test]
    fn a_crashed_session_leaves_an_orphan() {
        let root = root("crash");
        let session = Session::start(&root).unwrap();
        std::fs::write(session.project_path(), b"project").unwrap();
        let info = AutosaveInfo {
            name: "suitcase".into(),
            original_path: Some(PathBuf::from("C:/work/suitcase.patine")),
        };
        session.write_info(&info).unwrap();
        // A crash: the lock is released without the folder being deleted.
        drop(session);
        let found = orphans(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info, info);
        assert!(found[0].project.exists());
        found[0].discard();
        assert!(orphans(&root).is_empty());
    }

    #[test]
    fn a_crash_before_any_autosave_is_cleaned_up() {
        let root = root("empty");
        drop(Session::start(&root).unwrap());
        assert!(orphans(&root).is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    fn document() -> Document {
        Document::new(patine_core::project::Project::new(
            patine_mesh::primitives::cube(),
            "cube.glb".into(),
            Default::default(),
        ))
    }

    /// Ticks until the running autosave is collected.
    fn settle(autosaver: &mut Autosaver, document: Option<&Document>, generation: u64) {
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(20);
        autosaver.tick(&ctx, document, generation);
        while autosaver.task.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            autosaver.tick(&ctx, document, generation);
        }
    }

    #[test]
    fn autosaves_only_unsaved_changes() {
        let root = root("autosaver");
        let mut autosaver = Autosaver::new(Session::start(&root).unwrap(), Duration::from_secs(60));
        let mut document = document();
        assert_eq!(
            autosaver.deadline(Some(&document), 1),
            None,
            "nothing to save"
        );

        let command = crate::document::environment_command(
            &document.project.environment,
            crate::document::EnvironmentChange::Exposure(1.0),
        );
        document.execute(command);
        assert!(autosaver.deadline(Some(&document), 1).is_some());
        settle(&mut autosaver, Some(&document), 1);
        assert!(!autosaver.has_file, "not due before the interval");

        autosaver.due_now();
        settle(&mut autosaver, Some(&document), 1);
        assert!(autosaver.has_file);
        assert_eq!(
            autosaver.deadline(Some(&document), 1),
            None,
            "already autosaved"
        );
        let saved = patine_core::file::load(&autosaver.session.project_path()).unwrap();
        assert_eq!(saved, document.project);

        // Another document with the same history state is not considered saved.
        assert!(autosaver.deadline(Some(&document), 2).is_some());

        // Saving the document deletes the autosave.
        document.history.mark_saved();
        settle(&mut autosaver, Some(&document), 1);
        assert!(!autosaver.has_file);
        assert!(!autosaver.session.project_path().exists());
        autosaver.finish();
    }

    #[test]
    fn cleared_sessions_have_nothing_to_recover() {
        let root = root("clear");
        let session = Session::start(&root).unwrap();
        std::fs::write(session.project_path(), b"project").unwrap();
        session
            .write_info(&AutosaveInfo {
                name: "lantern".into(),
                original_path: None,
            })
            .unwrap();
        session.clear();
        drop(session);
        assert!(orphans(&root).is_empty());
    }
}
