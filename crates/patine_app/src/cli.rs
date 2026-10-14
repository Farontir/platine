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
  --display MODE                lit, checker, mask or channel:<id> (base-color, roughness,
                                user-0...)
  --texture-set N               current texture set (Texture Sets panel, 2D view)
  --edit EDIT                   apply a command to the opened project (repeatable):
                                rotation=DEG, exposure=EV, rename=SET:NAME, undo=N,
                                fill=R,G,B,ROUGHNESS, mode=MODE,OPACITY, group=,
                                mask=black|white, mask-fill=VALUE,MODE,OPACITY,
                                mask-levels=IN_LOW,IN_HIGH,GAMMA,OUT_LOW,OUT_HIGH,
                                mask-invert=, material=ID, mask-procedural=ID,
                                param=NAME:VALUE, projection=KIND,SCALE, paint-layer=,
                                mask-paint=, height-force=V
  --tool brush|eraser           painting tool
  --brush KEY=VALUE             brush setting (repeatable): size=PX, flow=V, opacity=V,
                                hardness=V, spacing=V, angle=DEG, smoothing=PX,
                                alpha=ID|round, color=R,G,B, roughness=V, metallic=V,
                                height=V, mask=V, off=CHANNEL, size-jitter=V,
                                angle-jitter=DEG, flow-jitter=V, scatter=V,
                                follow-path=on|off, tilt-angle=on|off,
                                pressure-size|pressure-flow|pressure-opacity=linear|off,
                                material=ID|values, mask-pattern=ID|value
  --preset NAME                 brush preset: standard, airbrush, hard-felt, chalk,
                                splatter, scratches, or a saved preset's name
  --symmetry x,y,z,radial=N     symmetry of the strokes (any of the items)
  --stroke [PREFIX:]X,Y[,P];... paint a stroke through these pixels, as with the mouse,
                                at pen pressure P (default 1) (repeatable). Prefixes,
                                combinable: eraser: (with the eraser), uv: (in the 2D
                                view), line: (Shift + click: a straight line from the
                                end of the previous stroke)

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
    /// Add a fill layer above the selection (base color and roughness) and select it.
    Fill([f32; 3], f32),
    /// Set the base color blend of the selected layer: mode name (as in the translation
    /// keys, without `blend-`) and opacity.
    Mode(String, f32),
    /// Put the selected layer into a new folder.
    Group,
    /// Give the selected layer a mask: white (`true`) or black.
    Mask(bool),
    /// Add a fill to the selected layer's mask: value, mode name, opacity.
    MaskFill(f32, String, f32),
    /// Add levels to the selected layer's mask: in_low, in_high, gamma, out_low, out_high.
    MaskLevels([f32; 5]),
    /// Add an inversion to the selected layer's mask.
    MaskInvert,
    /// Add a fill of the procedural material `id` above the selection and select it.
    Material(String),
    /// Add a fill of the noise or pattern `id` to the selected layer's mask.
    MaskProcedural(String),
    /// Set a parameter of the selected procedural (layer or mask effect): name and value
    /// (a number, `true`/`false`, `R,G,B` or an option name).
    Param(String, String),
    /// Set the projection of the selected procedural fill: kind and uniform scale.
    Projection(String, f32),
    /// Add a paint layer above the selection and select it.
    PaintLayer,
    /// Add a paint effect to the selected layer's mask and select it.
    MaskPaint,
    /// Set the height force of the current texture set.
    HeightForce(f32),
}

/// A brush setting given on the command line.
#[derive(Clone, Debug, PartialEq)]
pub enum BrushOption {
    /// Diameter in pixels.
    Size(f32),
    /// Flow.
    Flow(f32),
    /// Opacity.
    Opacity(f32),
    /// Hardness.
    Hardness(f32),
    /// Spacing, as a fraction of the size.
    Spacing(f32),
    /// Rotation, degrees.
    Angle(f32),
    /// Lazy mouse string, pixels.
    Smoothing(f32),
    /// Shipped alpha, or `None` for round dabs.
    Alpha(Option<String>),
    /// Paint this base color.
    Color([f32; 3]),
    /// Paint this value on a scalar channel.
    Scalar(Channel, f32),
    /// Do not paint this channel.
    Off(Channel),
    /// Gray painted into masks.
    MaskValue(f32),
    /// Random size variation.
    SizeJitter(f32),
    /// Random rotation, degrees.
    AngleJitter(f32),
    /// Random flow variation.
    FlowJitter(f32),
    /// Random offset across the stroke.
    Scatter(f32),
    /// Dabs turn along the stroke.
    FollowPath(bool),
    /// The pen tilt turns the dabs.
    TiltAngle(bool),
    /// A pressure curve (0 size, 1 flow, 2 opacity): linear (`true`) or off.
    Pressure(usize, bool),
    /// Procedural material painted on layers, or `None` for the values.
    Material(Option<String>),
    /// Procedural pattern painted into masks, or `None` for the gray.
    MaskPattern(Option<String>),
}

impl BrushOption {
    fn parse(text: &str) -> Option<Self> {
        let (key, value) = text.split_once('=')?;
        let number = || value.parse::<f32>().ok();
        let switch = || match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        let curve = || match value {
            "linear" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        Some(match key {
            "size-jitter" => BrushOption::SizeJitter(number()?),
            "angle-jitter" => BrushOption::AngleJitter(number()?),
            "flow-jitter" => BrushOption::FlowJitter(number()?),
            "scatter" => BrushOption::Scatter(number()?),
            "follow-path" => BrushOption::FollowPath(switch()?),
            "tilt-angle" => BrushOption::TiltAngle(switch()?),
            "pressure-size" => BrushOption::Pressure(0, curve()?),
            "pressure-flow" => BrushOption::Pressure(1, curve()?),
            "pressure-opacity" => BrushOption::Pressure(2, curve()?),
            "material" if value == "values" => BrushOption::Material(None),
            "material" => BrushOption::Material(Some(value.to_owned())),
            "mask-pattern" if value == "value" => BrushOption::MaskPattern(None),
            "mask-pattern" => BrushOption::MaskPattern(Some(value.to_owned())),
            "size" => BrushOption::Size(number()?),
            "flow" => BrushOption::Flow(number()?),
            "opacity" => BrushOption::Opacity(number()?),
            "hardness" => BrushOption::Hardness(number()?),
            "spacing" => BrushOption::Spacing(number()?),
            "angle" => BrushOption::Angle(number()?),
            "smoothing" => BrushOption::Smoothing(number()?),
            "alpha" if value == "round" => BrushOption::Alpha(None),
            "alpha" => BrushOption::Alpha(Some(value.to_owned())),
            "color" => {
                let rgb: Vec<f32> = value
                    .split(',')
                    .map(|v| v.parse().ok())
                    .collect::<Option<_>>()?;
                BrushOption::Color(rgb.try_into().ok()?)
            }
            "roughness" => BrushOption::Scalar(Channel::Roughness, number()?),
            "metallic" => BrushOption::Scalar(Channel::Metallic, number()?),
            "height" => BrushOption::Scalar(Channel::Height, number()?),
            "mask" => BrushOption::MaskValue(number()?),
            "off" => BrushOption::Off(Channel::from_id(value)?),
            _ => return None,
        })
    }
}

/// A stroke given on the command line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartupStroke {
    /// Painted with the eraser instead of the tool chosen with `--tool`.
    pub eraser: bool,
    /// Painted in the 2D view (pixels of the 2D view).
    pub uv: bool,
    /// Starts as a Shift + click: a straight line from the end of the previous stroke.
    pub line: bool,
    /// Pixels the pointer goes through, with the pen pressure.
    pub points: Vec<[f32; 3]>,
}

/// A `--stroke` value: `[eraser:][uv:][line:]X,Y[,P];X,Y[,P];...`.
fn stroke_points(value: &str) -> Option<StartupStroke> {
    let mut stroke = StartupStroke::default();
    let mut rest = value;
    loop {
        if let Some(r) = rest.strip_prefix("eraser:") {
            stroke.eraser = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("uv:") {
            stroke.uv = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("line:") {
            stroke.line = true;
            rest = r;
        } else {
            break;
        }
    }
    stroke.points = rest
        .split(';')
        .map(|point| {
            let numbers: Vec<f32> = point
                .split(',')
                .map(|v| v.trim().parse().ok())
                .collect::<Option<_>>()?;
            match numbers.as_slice() {
                [x, y] => Some([*x, *y, 1.0]),
                [x, y, p] => Some([*x, *y, *p]),
                _ => None,
            }
        })
        .collect::<Option<_>>()?;
    (!stroke.points.is_empty()).then_some(stroke)
}

/// A `--symmetry` value: `x`, `y`, `z` and `radial=N`, separated by commas.
fn symmetry(value: &str) -> Option<patine_core::stroke::Symmetry> {
    let mut symmetry = patine_core::stroke::Symmetry::default();
    for item in value.split(',').map(str::trim) {
        match item {
            "x" => symmetry.mirror[0] = true,
            "y" => symmetry.mirror[1] = true,
            "z" => symmetry.mirror[2] = true,
            _ => {
                let n: u32 = item.strip_prefix("radial=")?.parse().ok()?;
                if !(1..=16).contains(&n) {
                    return None;
                }
                symmetry.radial = n;
            }
        }
    }
    Some(symmetry)
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
            "fill" => {
                let numbers: Vec<f32> = value
                    .split(',')
                    .map(|v| v.parse().ok())
                    .collect::<Option<_>>()?;
                let [r, g, b, roughness] = numbers.as_slice() else {
                    return None;
                };
                StartupEdit::Fill([*r, *g, *b], *roughness)
            }
            "mode" => {
                let (mode, opacity) = value.split_once(',')?;
                StartupEdit::Mode(mode.to_owned(), opacity.parse().ok()?)
            }
            "group" => StartupEdit::Group,
            "mask" => match value {
                "black" => StartupEdit::Mask(false),
                "white" => StartupEdit::Mask(true),
                _ => return None,
            },
            "mask-fill" => {
                let mut parts = value.split(',');
                let (Some(v), Some(mode), Some(opacity), None) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    return None;
                };
                StartupEdit::MaskFill(v.parse().ok()?, mode.to_owned(), opacity.parse().ok()?)
            }
            "mask-levels" => {
                let numbers: Vec<f32> = value
                    .split(',')
                    .map(|v| v.parse().ok())
                    .collect::<Option<_>>()?;
                StartupEdit::MaskLevels(numbers.try_into().ok()?)
            }
            "mask-invert" => StartupEdit::MaskInvert,
            "material" => StartupEdit::Material(value.to_owned()),
            "mask-procedural" => StartupEdit::MaskProcedural(value.to_owned()),
            "param" => {
                let (name, value) = value.split_once(':')?;
                StartupEdit::Param(name.to_owned(), value.to_owned())
            }
            "projection" => {
                let (kind, scale) = value.split_once(',')?;
                StartupEdit::Projection(kind.to_owned(), scale.parse().ok()?)
            }
            "paint-layer" => StartupEdit::PaintLayer,
            "mask-paint" => StartupEdit::MaskPaint,
            "height-force" => StartupEdit::HeightForce(value.parse().ok()?),
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
    /// Painting tool: `true` for the eraser.
    pub eraser: Option<bool>,
    /// Brush settings, in order.
    pub brush: Vec<BrushOption>,
    /// Brush preset applied before the `--brush` settings.
    pub preset: Option<String>,
    /// Symmetry of the strokes.
    pub symmetry: Option<patine_core::stroke::Symmetry>,
    /// Strokes to paint once their view has settled.
    pub strokes: Vec<StartupStroke>,
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
                        "mask" => (DisplayMode::Mask, None),
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
                "--tool" => {
                    let v = value()?;
                    options.eraser = Some(match v.as_str() {
                        "brush" => false,
                        "eraser" => true,
                        _ => return Err(invalid(&v)),
                    });
                }
                "--brush" => {
                    let v = value()?;
                    options
                        .brush
                        .push(BrushOption::parse(&v).ok_or_else(|| invalid(&v))?);
                }
                "--stroke" => {
                    let v = value()?;
                    options
                        .strokes
                        .push(stroke_points(&v).ok_or_else(|| invalid(&v))?);
                }
                "--preset" => options.preset = Some(value()?),
                "--symmetry" => {
                    let v = value()?;
                    options.symmetry = Some(symmetry(&v).ok_or_else(|| invalid(&v))?);
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
        let options = parse(&[
            "--edit",
            "fill=1,0,0,0.3",
            "--edit",
            "mode=multiply,0.5",
            "--edit",
            "group=",
        ])
        .unwrap();
        assert_eq!(
            options.edits,
            [
                StartupEdit::Fill([1.0, 0.0, 0.0], 0.3),
                StartupEdit::Mode("multiply".into(), 0.5),
                StartupEdit::Group,
            ]
        );
        assert!(parse(&["--edit", "fill=1,0"]).is_err());
        let options = parse(&[
            "--edit",
            "mask=black",
            "--edit",
            "mask-fill=0.5,multiply,0.8",
            "--edit",
            "mask-levels=0,1,2,0.1,0.9",
            "--edit",
            "mask-invert=",
            "--display",
            "mask",
        ])
        .unwrap();
        assert_eq!(
            options.edits,
            [
                StartupEdit::Mask(false),
                StartupEdit::MaskFill(0.5, "multiply".into(), 0.8),
                StartupEdit::MaskLevels([0.0, 1.0, 2.0, 0.1, 0.9]),
                StartupEdit::MaskInvert,
            ]
        );
        assert_eq!(options.display, Some((DisplayMode::Mask, None)));
        assert!(parse(&["--edit", "mask=grey"]).is_err());
        let options = parse(&[
            "--edit",
            "material=material.copper",
            "--edit",
            "param=oxidation:0.4",
            "--edit",
            "projection=triplanar,2",
            "--edit",
            "mask-procedural=noise.perlin",
        ])
        .unwrap();
        assert_eq!(
            options.edits,
            [
                StartupEdit::Material("material.copper".into()),
                StartupEdit::Param("oxidation".into(), "0.4".into()),
                StartupEdit::Projection("triplanar".into(), 2.0),
                StartupEdit::MaskProcedural("noise.perlin".into()),
            ]
        );
        assert!(parse(&["--edit", "param=oxidation"]).is_err());
        assert!(parse(&["--edit", "mask-levels=0,1,2"]).is_err());
        assert!(parse(&["--edit", "mask-fill=0.5,normal"]).is_err());
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
    fn painting_options() {
        let options = parse(&[
            "--edit",
            "paint-layer=",
            "--edit",
            "mask-paint=",
            "--tool",
            "eraser",
            "--brush",
            "size=60",
            "--brush",
            "color=0.8,0.1,0.1",
            "--brush",
            "alpha=alpha.splatter",
            "--brush",
            "off=roughness",
            "--stroke",
            "10,20;30.5,40",
            "--stroke",
            "eraser:1,2",
        ])
        .unwrap();
        assert_eq!(
            options.edits,
            [StartupEdit::PaintLayer, StartupEdit::MaskPaint]
        );
        assert_eq!(options.eraser, Some(true));
        assert_eq!(
            options.brush,
            [
                BrushOption::Size(60.0),
                BrushOption::Color([0.8, 0.1, 0.1]),
                BrushOption::Alpha(Some("alpha.splatter".into())),
                BrushOption::Off(Channel::Roughness),
            ]
        );
        assert_eq!(
            options.strokes,
            [
                StartupStroke {
                    points: vec![[10.0, 20.0, 1.0], [30.5, 40.0, 1.0]],
                    ..StartupStroke::default()
                },
                StartupStroke {
                    eraser: true,
                    points: vec![[1.0, 2.0, 1.0]],
                    ..StartupStroke::default()
                },
            ]
        );
        let options = parse(&[
            "--preset",
            "chalk",
            "--symmetry",
            "x,z,radial=4",
            "--brush",
            "size-jitter=0.4",
            "--brush",
            "angle-jitter=90",
            "--brush",
            "follow-path=on",
            "--brush",
            "pressure-opacity=linear",
            "--brush",
            "material=material.copper",
            "--brush",
            "mask-pattern=value",
            "--stroke",
            "uv:line:eraser:1,2,0.5;3,4",
            "--edit",
            "height-force=2.5",
        ])
        .unwrap();
        assert_eq!(options.preset.as_deref(), Some("chalk"));
        assert_eq!(
            options.symmetry,
            Some(patine_core::stroke::Symmetry {
                mirror: [true, false, true],
                radial: 4
            })
        );
        assert_eq!(
            options.brush,
            [
                BrushOption::SizeJitter(0.4),
                BrushOption::AngleJitter(90.0),
                BrushOption::FollowPath(true),
                BrushOption::Pressure(2, true),
                BrushOption::Material(Some("material.copper".into())),
                BrushOption::MaskPattern(None),
            ]
        );
        assert_eq!(
            options.strokes,
            [StartupStroke {
                eraser: true,
                uv: true,
                line: true,
                points: vec![[1.0, 2.0, 0.5], [3.0, 4.0, 1.0]],
            }]
        );
        assert_eq!(options.edits, [StartupEdit::HeightForce(2.5)]);
        assert!(parse(&["--symmetry", "w"]).is_err());
        assert!(parse(&["--symmetry", "radial=17"]).is_err());
        assert!(parse(&["--brush", "follow-path=yes"]).is_err());
        assert!(parse(&["--stroke", "1,2,3,4"]).is_err());
        assert!(parse(&["--brush", "color=1,0"]).is_err());
        assert!(parse(&["--brush", "girth=3"]).is_err());
        assert!(parse(&["--stroke", "10;20"]).is_err());
        assert!(parse(&["--tool", "pencil"]).is_err());
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
