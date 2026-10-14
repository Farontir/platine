//! Brush presets: the shipped ones (`patine_paint::presets`) and the user's, RON files
//! in the `presets` folder of the application data.
//!
//! A shipped preset sets how the brush behaves (tip, dynamics, pressure) and keeps the
//! painted material; a user preset also records the material.

use std::path::{Path, PathBuf};

pub use patine_paint::presets::{
    BrushPreset, BrushSettings, PresetMaterial, SHIPPED_KEYS, find_shipped, shipped,
};

/// The folder of the user's presets: `PATINE_PRESETS_DIR`, or `presets` in the
/// application data.
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PATINE_PRESETS_DIR") {
        return Some(PathBuf::from(dir));
    }
    crate::autosave::data_dir().map(|dir| dir.join("presets"))
}

/// Errors of the user's presets.
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    /// No folder for the presets on this system.
    #[error("no folder for the brush presets")]
    NoFolder,
    /// The name is empty once cleaned up.
    #[error("the preset name is empty")]
    EmptyName,
    /// Reading or writing failed.
    #[error("{path}: {source}")]
    Io {
        /// File or folder.
        path: PathBuf,
        /// Cause.
        source: std::io::Error,
    },
    /// The preset could not be serialized.
    #[error("cannot write the preset: {0}")]
    Serialize(#[from] ron::Error),
}

/// The file name of a preset called `name`: letters, digits, spaces, dashes and
/// underscores kept, other characters replaced by `_`; `None` when nothing is left.
pub fn file_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    (!cleaned.trim().is_empty()).then(|| format!("{}.ron", cleaned.trim()))
}

/// The shipped presets and the user's, read from a folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetLibrary {
    /// Folder of the user's presets, if any.
    pub dir: Option<PathBuf>,
    /// The user's presets, by name.
    pub user: Vec<BrushPreset>,
}

impl PresetLibrary {
    /// Reads the user's presets in `dir` (none when it does not exist). Unreadable files
    /// are skipped with a warning.
    pub fn load(dir: Option<PathBuf>) -> Self {
        let mut user = Vec::new();
        if let Some(entries) = dir.as_deref().and_then(|dir| std::fs::read_dir(dir).ok()) {
            for path in entries.filter_map(|entry| entry.ok().map(|e| e.path())) {
                if path.extension().is_none_or(|ext| ext != "ron") {
                    continue;
                }
                match read(&path) {
                    Ok(preset) => user.push(preset),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "brush preset skipped")
                    }
                }
            }
        }
        user.sort_by_key(|preset| preset.name.to_lowercase());
        Self { dir, user }
    }

    /// Saves `preset` in the folder, replacing a preset of the same name.
    pub fn save(&mut self, preset: BrushPreset) -> Result<(), PresetError> {
        let dir = self.dir.as_deref().ok_or(PresetError::NoFolder)?;
        let name = file_name(&preset.name).ok_or(PresetError::EmptyName)?;
        std::fs::create_dir_all(dir).map_err(|source| PresetError::Io {
            path: dir.to_owned(),
            source,
        })?;
        let text = ron::ser::to_string_pretty(&preset, ron::ser::PrettyConfig::default())?;
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|source| PresetError::Io { path, source })?;
        self.user.retain(|p| p.name != preset.name);
        self.user.push(preset);
        self.user.sort_by_key(|preset| preset.name.to_lowercase());
        Ok(())
    }

    /// Deletes the user's preset `name`.
    pub fn delete(&mut self, name: &str) -> Result<(), PresetError> {
        let dir = self.dir.as_deref().ok_or(PresetError::NoFolder)?;
        let path = dir.join(file_name(name).ok_or(PresetError::EmptyName)?);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(PresetError::Io { path, source }),
        }
        self.user.retain(|p| p.name != name);
        Ok(())
    }
}

/// Reads one preset file.
fn read(path: &Path) -> Result<BrushPreset, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    ron::from_str(&text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use patine_core::channel::Channel;
    use patine_core::layer::Projection;

    use super::*;
    use crate::i18n::{I18n, Language};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("patine-presets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn shipped_presets_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in SHIPPED_KEYS {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_name("Ma brosse").as_deref(), Some("Ma brosse.ron"));
        assert_eq!(file_name(" a/b\\c:d ").as_deref(), Some("a_b_c_d.ron"));
        assert_eq!(file_name("Éclat-1_x").as_deref(), Some("Éclat-1_x.ron"));
        assert_eq!(file_name("   "), None);
    }

    #[test]
    fn user_presets_are_saved_read_and_deleted() {
        let dir = scratch("roundtrip");
        let mut library = PresetLibrary::load(Some(dir.clone()));
        assert!(library.user.is_empty());
        let mut chalk = shipped().remove(3);
        chalk.name = "Craie rouge".to_owned();
        chalk.material = Some(PresetMaterial {
            channels: vec![(Channel::BaseColor, true, [0.8, 0.1, 0.1, 1.0])],
            mask_value: 0.5,
            procedural: None,
            mask_procedural: None,
            projection: Projection::default(),
        });
        library.save(chalk.clone()).expect("saved");
        let mut other = shipped().remove(0);
        other.name = "autre".to_owned();
        library.save(other.clone()).expect("saved");
        // A broken file is skipped.
        std::fs::write(dir.join("broken.ron"), "(name: ").expect("written");

        let reloaded = PresetLibrary::load(Some(dir.clone()));
        assert_eq!(reloaded.user, [other.clone(), chalk.clone()]);

        // Saving under the same name replaces.
        chalk.brush.size = 99.0;
        library.save(chalk.clone()).expect("saved");
        assert_eq!(
            PresetLibrary::load(Some(dir.clone())).user[1].brush.size,
            99.0
        );

        library.delete("Craie rouge").expect("deleted");
        assert_eq!(library.user, [other.clone()]);
        assert_eq!(PresetLibrary::load(Some(dir.clone())).user, [other]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_needs_a_folder_and_a_name() {
        let mut library = PresetLibrary::default();
        assert!(matches!(
            library.save(shipped().remove(0)),
            Err(PresetError::NoFolder)
        ));
        let mut library = PresetLibrary::load(Some(scratch("empty")));
        let mut preset = shipped().remove(0);
        preset.name = "///".to_owned();
        // Only separators: replaced, still a name.
        assert!(library.save(preset.clone()).is_ok());
        preset.name = " ".to_owned();
        assert!(matches!(library.save(preset), Err(PresetError::EmptyName)));
        if let Some(dir) = &library.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
