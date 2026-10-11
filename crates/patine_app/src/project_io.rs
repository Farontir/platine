//! Opening and saving project files on a worker thread.
//!
//! Saving works on a snapshot of the project (cloning it is cheap: heavy data is shared),
//! so the user can keep working while the file is written. One job runs at a time.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use fluent_bundle::FluentArgs;
use image::RgbaImage;
use patine_core::file::{self, FileError};
use patine_core::project::Project;

use crate::i18n::I18n;

/// Width of the thumbnail stored in project files.
pub const THUMBNAIL_WIDTH: u32 = 256;

/// What a finished job produced.
#[derive(Debug)]
pub enum JobResult {
    /// The project was written to `path`; `state` is the history state it contains.
    Saved {
        /// File written.
        path: PathBuf,
        /// History state of the snapshot.
        state: u64,
    },
    /// The project was read from `path`.
    Opened {
        /// File read.
        path: PathBuf,
        /// The project.
        project: Box<Project>,
    },
    /// The job failed.
    Failed {
        /// Whether it was a save (otherwise an open).
        saving: bool,
        /// File concerned.
        path: PathBuf,
        /// The error.
        error: FileError,
    },
    /// The user closed the file dialog.
    Cancelled,
}

/// The running job, if any.
#[derive(Default)]
pub struct FileJobs {
    task: Option<(bool, Receiver<JobResult>)>,
}

impl std::fmt::Debug for FileJobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileJobs")
            .field("saving", &self.saving())
            .field("opening", &self.opening())
            .finish()
    }
}

/// Encodes a thumbnail of `image` as PNG.
fn thumbnail_png(image: &RgbaImage) -> Option<Vec<u8>> {
    let height = (image.height() * THUMBNAIL_WIDTH / image.width().max(1)).max(1);
    let small = image::imageops::thumbnail(image, THUMBNAIL_WIDTH, height);
    let mut png = Cursor::new(Vec::new());
    small.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// Asks for a file name to save to, with the project extension.
fn pick_save_path(filter: &str, suggested: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(filter, &[file::EXTENSION])
        .set_file_name(format!("{suggested}.{}", file::EXTENSION))
        .save_file()
        .map(file::with_extension)
}

impl FileJobs {
    /// Whether a job is running.
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Whether a save is running.
    pub fn saving(&self) -> bool {
        matches!(self.task, Some((true, _)))
    }

    /// Whether an open is running.
    pub fn opening(&self) -> bool {
        matches!(self.task, Some((false, _)))
    }

    /// Saves `project` (a snapshot at history state `state`) to `path`, or to a file
    /// picked in a dialog when `path` is `None`. `filter` and `suggested` name the
    /// dialog's file type and default file name.
    #[allow(clippy::too_many_arguments)]
    pub fn save(
        &mut self,
        ctx: &egui::Context,
        project: Project,
        state: u64,
        path: Option<PathBuf>,
        thumbnail: Option<RgbaImage>,
        filter: String,
        suggested: String,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = match path.or_else(|| pick_save_path(&filter, &suggested)) {
                None => JobResult::Cancelled,
                Some(path) => {
                    let start = std::time::Instant::now();
                    let png = thumbnail.as_ref().and_then(thumbnail_png);
                    match file::save(&project, &path, png.as_deref()) {
                        Ok(()) => {
                            tracing::info!(
                                path = %path.display(),
                                ms = start.elapsed().as_millis() as u64,
                                "project saved"
                            );
                            JobResult::Saved { path, state }
                        }
                        Err(error) => JobResult::Failed {
                            saving: true,
                            path,
                            error,
                        },
                    }
                }
            };
            // The application may be closing; nobody listens then.
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.task = Some((true, receiver));
    }

    /// Opens `path`, or a file picked in a dialog when `path` is `None`.
    pub fn open(&mut self, ctx: &egui::Context, path: Option<PathBuf>, filter: String) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = path.or_else(|| {
                rfd::FileDialog::new()
                    .add_filter(filter, &[file::EXTENSION])
                    .pick_file()
            });
            let result = match path {
                None => JobResult::Cancelled,
                Some(path) => {
                    let start = std::time::Instant::now();
                    match file::load(&path) {
                        Ok(project) => {
                            tracing::info!(
                                path = %path.display(),
                                ms = start.elapsed().as_millis() as u64,
                                "project read"
                            );
                            JobResult::Opened {
                                path,
                                project: Box::new(project),
                            }
                        }
                        Err(error) => JobResult::Failed {
                            saving: false,
                            path,
                            error,
                        },
                    }
                }
            };
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.task = Some((false, receiver));
    }

    /// The result of the running job, once it is finished.
    pub fn poll(&mut self) -> Option<JobResult> {
        let (_, task) = self.task.as_ref()?;
        let result = match task.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => JobResult::Cancelled,
        };
        self.task = None;
        Some(result)
    }

    /// Waits for the running job (before quitting), and returns its result.
    pub fn wait(&mut self) -> Option<JobResult> {
        let (_, task) = self.task.take()?;
        Some(task.recv().unwrap_or(JobResult::Cancelled))
    }
}

/// Translated message for a failed open or save.
pub fn error_message(i18n: &I18n, saving: bool, path: &Path, error: &FileError) -> String {
    let mut args = FluentArgs::new();
    args.set("path", path.display().to_string());
    let reason = match error {
        FileError::NotAProject | FileError::Archive(_) => {
            i18n.tr("file-error-not-a-project").into_owned()
        }
        FileError::NewerVersion { found, .. } => {
            let mut version = FluentArgs::new();
            version.set("version", *found);
            i18n.tr_args("file-error-newer-version", &version)
        }
        FileError::Io(io) => io.to_string(),
        other => {
            let mut detail = FluentArgs::new();
            detail.set("message", other.to_string());
            i18n.tr_args("file-error-damaged", &detail)
        }
    };
    args.set("message", reason);
    i18n.tr_args(
        if saving {
            "file-error-save"
        } else {
            "file-error-open"
        },
        &args,
    )
}

/// Every translation key of this module, for tests.
pub const TRANSLATION_KEYS: [&str; 7] = [
    "file-error-not-a-project",
    "file-error-newer-version",
    "file-error-damaged",
    "file-error-save",
    "file-error-open",
    "file-filter",
    "file-busy-saving",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn errors_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in TRANSLATION_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
            let message = error_message(
                &i18n,
                false,
                Path::new("a.patine"),
                &FileError::NewerVersion {
                    found: 9,
                    supported: 1,
                },
            );
            assert!(
                message.contains("a.patine") && message.contains('9'),
                "{message}"
            );
        }
    }

    #[test]
    fn thumbnails_keep_the_aspect_ratio() {
        let image = RgbaImage::from_pixel(1600, 900, image::Rgba([10, 20, 30, 255]));
        let png = thumbnail_png(&image).expect("encodes");
        let decoded = image::load_from_memory(&png).expect("decodes").to_rgba8();
        assert_eq!(decoded.dimensions(), (256, 144));
    }

    #[test]
    fn save_then_open_round_trips() {
        let dir = std::env::temp_dir().join(format!("patine-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("cube.patine");
        let project = Project::new(
            patine_mesh::primitives::cube(),
            "cube.glb".into(),
            Default::default(),
        );
        let ctx = egui::Context::default();
        let mut jobs = FileJobs::default();
        jobs.save(
            &ctx,
            project.clone(),
            7,
            Some(path.clone()),
            Some(RgbaImage::new(64, 32)),
            String::new(),
            String::new(),
        );
        assert!(jobs.saving());
        match jobs.wait() {
            Some(JobResult::Saved { path: saved, state }) => {
                assert_eq!((saved, state), (path.clone(), 7));
            }
            other => panic!("{other:?}"),
        }
        assert!(file::load_thumbnail(&path).expect("readable").is_some());
        jobs.open(&ctx, Some(path), String::new());
        match jobs.wait() {
            Some(JobResult::Opened { project: read, .. }) => assert_eq!(*read, project),
            other => panic!("{other:?}"),
        }
        assert!(!jobs.busy());
    }
}
