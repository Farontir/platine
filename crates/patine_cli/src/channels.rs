//! `patine_cli channels`: writes the final channels of a project's texture sets as PNG
//! files, at the working resolution or at another one (M10, ahead of the M17 exporter).
//!
//! One 16-bit PNG per channel, named `<texture set>_<channel>.png`: the base color and
//! the emissive in sRGB, the normal mapped from [-1, 1] to [0, 1], the height from
//! [-1, 1] to [0, 1], the other channels as they are (linear, clamped).

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, bail};
use image::{ImageBuffer, Luma, Rgb};
use patine_compositor::compositor::Compositor;
use patine_compositor::offline::{ChannelImage, evaluate_set};
use patine_compositor::reference::ChannelKind;
use patine_core::channel::Channel;
use patine_core::file;
use patine_core::project::Resolution;
use patine_gpu::{AdapterChoice, GpuContext};

/// Options of `patine_cli channels`.
#[derive(Debug, PartialEq)]
pub struct ChannelsCommand {
    /// Project file.
    pub project: PathBuf,
    /// Output folder.
    pub out: PathBuf,
    /// Resolution of the files; the working one of each texture set by default.
    pub resolution: Option<Resolution>,
    /// Only this texture set, by name.
    pub set: Option<String>,
    /// The software adapter instead of the GPU (tests, machines without a GPU).
    pub software: bool,
}

impl ChannelsCommand {
    /// Parses `<project> --out DIR [--resolution N] [--set NAME] [--software]`.
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut project = None;
        let mut out = None;
        let mut resolution = None;
        let mut set = None;
        let mut software = false;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--out" => out = Some(PathBuf::from(value()?)),
                "--resolution" => {
                    let pixels: u32 = value()?.parse()?;
                    resolution = Some(
                        Resolution::new(pixels).context("resolution 512 to 8192, a power of 2")?,
                    );
                }
                "--set" => set = Some(value()?.clone()),
                "--software" => software = true,
                other if other.starts_with("--") => bail!("unknown option {other}"),
                _ => project = Some(PathBuf::from(arg)),
            }
        }
        Ok(Self {
            project: project.context("missing project file")?,
            out: out.context("missing --out folder")?,
            resolution,
            set,
            software,
        })
    }

    /// Evaluates and writes the channels, printing each file.
    pub fn run(&self) -> anyhow::Result<()> {
        let project = file::load(&self.project)
            .with_context(|| format!("cannot open {}", self.project.display()))?;
        let sets: Vec<_> = project
            .texture_sets
            .iter()
            .filter(|set| self.set.as_ref().is_none_or(|name| *name == set.name))
            .collect();
        if sets.is_empty() {
            bail!(
                "no texture set named {}",
                self.set.as_deref().unwrap_or("?")
            );
        }
        let adapter = if self.software {
            AdapterChoice::Software
        } else {
            AdapterChoice::Hardware
        };
        let ctx = GpuContext::new_headless(adapter)?;
        let compositor = Compositor::new(&ctx)?;
        std::fs::create_dir_all(&self.out)
            .with_context(|| format!("cannot create {}", self.out.display()))?;
        for set in sets {
            let resolution = self.resolution.unwrap_or(set.resolution);
            let start = Instant::now();
            let images = evaluate_set(&ctx, &compositor, &project, set.id, resolution)?;
            let evaluated = start.elapsed();
            for image in &images {
                let path = self.out.join(file_name(&set.name, image.def.channel));
                write_png(image, &path)?;
                println!("{}", path.display());
            }
            println!(
                "  {}: {} channels at {}² evaluated in {} ms, written in {} ms",
                set.name,
                images.len(),
                resolution.pixels(),
                evaluated.as_millis(),
                (start.elapsed() - evaluated).as_millis()
            );
        }
        Ok(())
    }
}

/// `<set>_<channel>.png`, with the characters a file name cannot hold replaced by `_`.
pub fn file_name(set: &str, channel: Channel) -> String {
    let set: String = set
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{set}_{}.png", channel.id())
}

/// sRGB encoding of a linear value.
fn srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// A texel of `channel` as stored in the PNG, 0 to 1 per component.
pub fn encode(channel: Channel, texel: [f32; 4]) -> [f32; 3] {
    let rgb = [texel[0], texel[1], texel[2]];
    let mapped = match channel {
        Channel::BaseColor | Channel::Emissive => rgb.map(srgb),
        Channel::Normal => rgb.map(|v| v * 0.5 + 0.5),
        Channel::Height => [texel[0] * 0.5 + 0.5; 3],
        _ => rgb,
    };
    mapped.map(|v| v.clamp(0.0, 1.0))
}

fn quantize(value: f32) -> u16 {
    (value * f32::from(u16::MAX)).round() as u16
}

/// Writes `image` as a 16-bit PNG (gray for scalar channels).
fn write_png(image: &ChannelImage, path: &Path) -> anyhow::Result<()> {
    let channel = image.def.channel;
    let size = image.size;
    let result = match ChannelKind::of(channel) {
        ChannelKind::Scalar => {
            let pixels: Vec<u16> = image
                .texels
                .iter()
                .map(|t| quantize(encode(channel, *t)[0]))
                .collect();
            ImageBuffer::<Luma<u16>, _>::from_raw(size, size, pixels)
                .context("as many texels as pixels")?
                .save(path)
        }
        _ => {
            let pixels: Vec<u16> = image
                .texels
                .iter()
                .flat_map(|t| encode(channel, *t).map(quantize))
                .collect();
            ImageBuffer::<Rgb<u16>, _>::from_raw(size, size, pixels)
                .context("as many texels as pixels")?
                .save(path)
        }
    };
    result.with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_parse() {
        let command = ChannelsCommand::parse(&args(&[
            "p.patine",
            "--out",
            "out",
            "--resolution",
            "2048",
            "--set",
            "Cuir",
            "--software",
        ]))
        .unwrap();
        assert_eq!(
            command,
            ChannelsCommand {
                project: "p.patine".into(),
                out: "out".into(),
                resolution: Resolution::new(2048),
                set: Some("Cuir".into()),
                software: true,
            }
        );
        assert!(ChannelsCommand::parse(&args(&["p.patine"])).is_err());
        assert!(ChannelsCommand::parse(&args(&["--out", "o"])).is_err());
        assert!(
            ChannelsCommand::parse(&args(&["p", "--out", "o", "--resolution", "1000"])).is_err()
        );
        assert!(ChannelsCommand::parse(&args(&["p", "--out", "o", "--size", "3"])).is_err());
    }

    #[test]
    fn channels_are_encoded_for_viewing() {
        assert_eq!(encode(Channel::Roughness, [0.25; 4]), [0.25; 3]);
        assert_eq!(
            encode(Channel::Normal, [0.0, 0.0, 1.0, 0.0]),
            [0.5, 0.5, 1.0]
        );
        assert_eq!(encode(Channel::Height, [-1.0; 4]), [0.0; 3]);
        let color = encode(Channel::BaseColor, [0.5, 0.0, 1.0, 1.0]);
        assert!((color[0] - 0.735_357).abs() < 1e-5);
        // f32 rounding: sRGB of 1 is 1 - 6e-8.
        assert!(color[1] == 0.0 && (color[2] - 1.0).abs() < 1e-6);
        assert_eq!(encode(Channel::Metallic, [1.7; 4]), [1.0; 3]);
        assert_eq!(
            file_name("Cuir / bras", Channel::BaseColor),
            "Cuir___bras_base-color.png"
        );
    }
}
