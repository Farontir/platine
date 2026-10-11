//! Command line of the application: an optional mesh to open, and developer options that
//! set the interface up and capture the window to a PNG (tutorial screenshots, visual
//! checks). Every option only changes view state or opens what a user could open by hand.

use std::path::PathBuf;

use patine_core::channel::Channel;
use patine_render::camera::Projection;
use patine_render::settings::{Background, DisplayMode, ToneMapper};
use thiserror::Error;

use crate::i18n::Language;
use crate::ui::Panel;

/// Usage text of the command line.
pub const USAGE: &str = "usage: patine_app [mesh] [options]
  --language fr|en              interface language
  --window WxH                  window size in pixels
  --tab PANEL                   bring a panel to front: viewport-3d, view-2d, texture-sets,
                                layers, properties, library, display, history, console
  --hdri PATH|studio            environment
  --rotation DEG                environment rotation
  --exposure EV                 exposure
  --tonemap agx|aces|neutral|linear
  --background env:BLUR|solid:R,G,B
  --material R,G,B,METALLIC,ROUGHNESS
  --wireframe                   show polygon edges
  --no-shadows                  disable shadows
  --ortho                       orthographic projection
  --fov DEG                     field of view
  --orbit YAW,PITCH             camera angles in degrees (after framing)
  --zoom FACTOR                 camera distance factor (after framing)
  --hover X,Y                   pretend the cursor is at this viewport pixel
  --dialog new-project|about    open a dialog
  --dialog-path PATH            mesh path typed in the New Project dialog
  --dialog-create               press Create in the New Project dialog
  --capture PNG                 save the window to PNG after a few frames, then quit
  --capture-frames N            frames to wait before capturing (default 30)
  --autosave-interval SECS      time between autosaves (default 300)
  --display MODE                lit, checker or channel:<id> (base-color, roughness, user-0...)
  --texture-set N               current texture set (Texture Sets panel, 2D view)
  --edit EDIT                   apply a command to the opened project (repeatable):
                                rotation=DEG, exposure=EV, rename=SET:NAME, undo=N

  A project file (.patine) can be given instead of a mesh.";

/// A command applied to the project opened at startup, as if made by the user.
#[derive(Clone, Debug, PartialEq)]
pub enum StartupEdit {
    /// Rotate the environment, degrees.
    Rotation(f32),
    /// Set the exposure, EV.
    Exposure(f32),
    /// Rename a texture set (by index).
    RenameTextureSet(u32, String),
    /// Undo this many commands.
    Undo(u32),
}

impl StartupEdit {
    fn parse(text: &str) -> Option<Self> {
        let (kind, value) = text.split_once('=')?;
        Some(match kind {
            "rotation" => StartupEdit::Rotation(value.parse().ok()?),
            "exposure" => StartupEdit::Exposure(value.parse().ok()?),
            "rename" => {
                let (set, name) = value.split_once(':')?;
                StartupEdit::RenameTextureSet(set.parse().ok()?, name.to_owned())
            }
            "undo" => StartupEdit::Undo(value.parse().ok()?),
            _ => return None,
        })
    }
}

/// A dialog to open at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupDialog {
    /// New Project.
    NewProject,
    /// About.
    About,
}

/// Parsed command line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartupOptions {
    /// Mesh to open as a project.
    pub mesh: Option<PathBuf>,
    /// Interface language.
    pub language: Option<Language>,
    /// Window size.
    pub window: Option<[u32; 2]>,
    /// Panels to bring to front, in order.
    pub tabs: Vec<Panel>,
    /// Environment file; `Some(None)` for the procedural studio sky.
    pub hdri: Option<Option<PathBuf>>,
    /// Environment rotation, degrees.
    pub rotation: Option<f32>,
    /// Exposure, EV.
    pub exposure: Option<f32>,
    /// Display operator.
    pub tone_mapper: Option<ToneMapper>,
    /// Background.
    pub background: Option<Background>,
    /// Preview material: base color, metallic, roughness.
    pub material: Option<([f32; 3], f32, f32)>,
    /// Wireframe overlay.
    pub wireframe: bool,
    /// Shadows disabled.
    pub no_shadows: bool,
    /// Projection.
    pub projection: Option<Projection>,
    /// Field of view, degrees.
    pub fov: Option<f32>,
    /// Camera yaw and pitch, degrees.
    pub orbit: Option<[f32; 2]>,
    /// Camera distance factor.
    pub zoom: Option<f32>,
    /// Forced cursor position in the viewport image.
    pub hover: Option<[u32; 2]>,
    /// Dialog to open.
    pub dialog: Option<StartupDialog>,
    /// Path typed in the New Project dialog.
    pub dialog_path: Option<PathBuf>,
    /// Press Create in the New Project dialog.
    pub dialog_create: bool,
    /// PNG file to capture the window to.
    pub capture: Option<PathBuf>,
    /// Frames before the capture.
    pub capture_frames: u32,
    /// Seconds between autosaves.
    pub autosave_interval: Option<u64>,
    /// Commands applied to the startup project, in order.
    pub edits: Vec<StartupEdit>,
    /// Display mode, with the inspected channel for the channel mode.
    pub display: Option<(DisplayMode, Option<Channel>)>,
    /// Current texture set.
    pub texture_set: Option<u32>,
}

/// Invalid command line.
#[derive(Debug, Error, PartialEq)]
pub enum CliError {
    /// An option needs a value.
    #[error("option {0} needs a value")]
    MissingValue(String),
    /// A value could not be parsed.
    #[error("invalid value `{value}` for {option}")]
    InvalidValue {
        /// Option name.
        option: String,
        /// Value given.
        value: String,
    },
    /// Unknown option.
    #[error("unknown option {0}")]
    UnknownOption(String),
}

fn numbers<const N: usize>(option: &str, value: &str) -> Result<[f32; N], CliError> {
    let invalid = || CliError::InvalidValue {
        option: option.to_owned(),
        value: value.to_owned(),
    };
    let parts: Vec<f32> = value
        .split(',')
        .map(|part| part.trim().parse::<f32>().map_err(|_| invalid()))
        .collect::<Result<_, _>>()?;
    parts.try_into().map_err(|_| invalid())
}

fn panel(value: &str) -> Option<Panel> {
    Some(match value {
        "viewport-3d" => Panel::Viewport3D,
        "view-2d" => Panel::View2D,
        "texture-sets" => Panel::TextureSets,
        "layers" => Panel::Layers,
        "properties" => Panel::Properties,
        "library" => Panel::Library,
        "display" => Panel::Display,
        "history" => Panel::History,
        "console" => Panel::Console,
        _ => return None,
    })
}

impl StartupOptions {
    /// Parses the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, CliError> {
        let mut options = Self {
            capture_frames: 30,
            ..Self::default()
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| CliError::MissingValue(arg.clone()))
            };
            let invalid = |value: &str| CliError::InvalidValue {
                option: arg.clone(),
                value: value.to_owned(),
            };
            match arg.as_str() {
                "--language" => {
                    let v = value()?;
                    options.language = Some(match v.as_str() {
                        "fr" => Language::French,
                        "en" => Language::English,
                        _ => return Err(invalid(&v)),
                    });
                }
                "--window" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or_else(|| invalid(&v))?;
                    let parse = |s: &str| s.parse::<u32>().map_err(|_| invalid(&v));
                    options.window = Some([parse(w)?, parse(h)?]);
                }
                "--tab" => {
                    let v = value()?;
                    options.tabs.push(panel(&v).ok_or_else(|| invalid(&v))?);
                }
                "--hdri" => {
                    let v = value()?;
                    options.hdri = Some((v != "studio").then(|| PathBuf::from(v)));
                }
                "--rotation" => options.rotation = Some(numbers::<1>(&arg, &value()?)?[0]),
                "--exposure" => options.exposure = Some(numbers::<1>(&arg, &value()?)?[0]),
                "--fov" => options.fov = Some(numbers::<1>(&arg, &value()?)?[0]),
                "--zoom" => options.zoom = Some(numbers::<1>(&arg, &value()?)?[0]),
                "--orbit" => options.orbit = Some(numbers::<2>(&arg, &value()?)?),
                "--tonemap" => {
                    let v = value()?;
                    options.tone_mapper = Some(match v.as_str() {
                        "agx" => ToneMapper::AgX,
                        "aces" => ToneMapper::Aces,
                        "neutral" => ToneMapper::PbrNeutral,
                        "linear" => ToneMapper::Linear,
                        _ => return Err(invalid(&v)),
                    });
                }
                "--background" => {
                    let v = value()?;
                    options.background = Some(if let Some(blur) = v.strip_prefix("env:") {
                        Background::Environment {
                            blur: numbers::<1>(&arg, blur)?[0],
                        }
                    } else if let Some(color) = v.strip_prefix("solid:") {
                        Background::Solid(numbers::<3>(&arg, color)?)
                    } else {
                        return Err(invalid(&v));
                    });
                }
                "--material" => {
                    let [r, g, b, metallic, roughness] = numbers::<5>(&arg, &value()?)?;
                    options.material = Some(([r, g, b], metallic, roughness));
                }
                "--hover" => {
                    let [x, y] = numbers::<2>(&arg, &value()?)?;
                    options.hover = Some([x as u32, y as u32]);
                }
                "--wireframe" => options.wireframe = true,
                "--no-shadows" => options.no_shadows = true,
                "--ortho" => options.projection = Some(Projection::Orthographic),
                "--dialog" => {
                    let v = value()?;
                    options.dialog = Some(match v.as_str() {
                        "new-project" => StartupDialog::NewProject,
                        "about" => StartupDialog::About,
                        _ => return Err(invalid(&v)),
                    });
                }
                "--dialog-path" => options.dialog_path = Some(PathBuf::from(value()?)),
                "--dialog-create" => options.dialog_create = true,
                "--display" => {
                    let v = value()?;
                    options.display = Some(match v.as_str() {
                        "lit" => (DisplayMode::Lit, None),
                        "checker" => (DisplayMode::UvChecker, None),
                        other => {
                            let channel = other
                                .strip_prefix("channel:")
                                .and_then(Channel::from_id)
                                .ok_or_else(|| invalid(&v))?;
                            (DisplayMode::Channel, Some(channel))
                        }
                    });
                }
                "--texture-set" => {
                    let v = value()?;
                    options.texture_set = Some(v.parse().map_err(|_| invalid(&v))?);
                }
                "--edit" => {
                    let v = value()?;
                    options
                        .edits
                        .push(StartupEdit::parse(&v).ok_or_else(|| invalid(&v))?);
                }
                "--capture" => options.capture = Some(PathBuf::from(value()?)),
                "--capture-frames" => {
                    let v = value()?;
                    options.capture_frames = v.parse().map_err(|_| invalid(&v))?;
                }
                "--autosave-interval" => {
                    let v = value()?;
                    let seconds: u64 = v.parse().map_err(|_| invalid(&v))?;
                    if seconds == 0 {
                        return Err(invalid(&v));
                    }
                    options.autosave_interval = Some(seconds);
                }
                other if other.starts_with("--") => {
                    return Err(CliError::UnknownOption(other.to_owned()));
                }
                _ => options.mesh = Some(PathBuf::from(arg)),
            }
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<StartupOptions, CliError> {
        StartupOptions::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn mesh_and_view_options() {
        let options = parse(&[
            "model.glb",
            "--language",
            "fr",
            "--tab",
            "display",
            "--hdri",
            "studio",
            "--tonemap",
            "aces",
            "--background",
            "solid:0.1,0.2,0.3",
            "--material",
            "1,0.5,0.2,1,0.3",
            "--orbit",
            "30,15",
            "--wireframe",
            "--capture",
            "out.png",
        ])
        .unwrap();
        assert_eq!(options.mesh, Some(PathBuf::from("model.glb")));
        assert_eq!(options.language, Some(Language::French));
        assert_eq!(options.tabs, [Panel::Display]);
        assert_eq!(options.hdri, Some(None));
        assert_eq!(options.tone_mapper, Some(ToneMapper::Aces));
        assert_eq!(options.background, Some(Background::Solid([0.1, 0.2, 0.3])));
        assert_eq!(options.material, Some(([1.0, 0.5, 0.2], 1.0, 0.3)));
        assert_eq!(options.orbit, Some([30.0, 15.0]));
        assert!(options.wireframe);
        assert_eq!(options.capture, Some(PathBuf::from("out.png")));
        assert_eq!(options.capture_frames, 30);
        assert_eq!(options.autosave_interval, None);
        let options = parse(&["work.patine", "--autosave-interval", "10"]).unwrap();
        assert_eq!(options.mesh, Some(PathBuf::from("work.patine")));
        assert_eq!(options.autosave_interval, Some(10));
        assert!(parse(&["--autosave-interval", "0"]).is_err());
        let options = parse(&[
            "--edit",
            "rotation=90",
            "--edit",
            "rename=1:Verre",
            "--edit",
            "undo=1",
        ])
        .unwrap();
        assert_eq!(
            options.edits,
            [
                StartupEdit::Rotation(90.0),
                StartupEdit::RenameTextureSet(1, "Verre".into()),
                StartupEdit::Undo(1),
            ]
        );
        assert!(parse(&["--edit", "scale=2"]).is_err());
        let options = parse(&["--display", "channel:roughness", "--texture-set", "2"]).unwrap();
        assert_eq!(
            options.display,
            Some((DisplayMode::Channel, Some(Channel::Roughness)))
        );
        assert_eq!(options.texture_set, Some(2));
        assert_eq!(
            parse(&["--display", "checker"]).unwrap().display,
            Some((DisplayMode::UvChecker, None))
        );
        assert!(parse(&["--display", "channel:albedo"]).is_err());
        assert!(parse(&["--edit", "rename=x"]).is_err());
    }

    #[test]
    fn errors_name_the_option() {
        assert_eq!(
            parse(&["--tab"]),
            Err(CliError::MissingValue("--tab".into()))
        );
        assert_eq!(
            parse(&["--tab", "nowhere"]),
            Err(CliError::InvalidValue {
                option: "--tab".into(),
                value: "nowhere".into()
            })
        );
        assert_eq!(
            parse(&["--frobnicate"]),
            Err(CliError::UnknownOption("--frobnicate".into()))
        );
        assert!(parse(&["--orbit", "1"]).is_err());
        assert!(parse(&["--window", "800"]).is_err());
    }

    #[test]
    fn every_panel_has_a_name() {
        for name in [
            "viewport-3d",
            "view-2d",
            "texture-sets",
            "layers",
            "properties",
            "library",
            "display",
            "history",
            "console",
        ] {
            assert!(panel(name).is_some(), "{name}");
        }
    }
}
