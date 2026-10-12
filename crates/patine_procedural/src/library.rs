//! The procedurals shipped with Patine: materials, noises and patterns. Each definition
//! names its shader; the order of its parameters is the order of the shader's `Params`
//! structure (checked by the layout test).
//!
//! Names and parameter labels are translation keys: `procedural-<id with dashes>` for
//! names, `param-<name>` for parameters, `option-<option>` for enum options.

use std::sync::LazyLock;

use patine_core::channel::Channel;

use crate::def::{Category, Output, ParamDef, ParamKind, ProceduralDef};

fn float(name: &str, min: f32, max: f32, default: f32) -> ParamDef {
    ParamDef::new(
        name,
        &format!("param-{name}"),
        ParamKind::Float { min, max, default },
    )
}

fn int(name: &str, min: i32, max: i32, default: i32) -> ParamDef {
    ParamDef::new(
        name,
        &format!("param-{name}"),
        ParamKind::Int { min, max, default },
    )
}

fn seed() -> ParamDef {
    ParamDef::new("seed", "param-seed", ParamKind::Seed)
}

fn choice(name: &str, options: &[&str], default: usize) -> ParamDef {
    ParamDef::new(
        name,
        &format!("param-{name}"),
        ParamKind::Enum {
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            default,
        },
    )
}

fn color(name: &str, default: [f32; 4]) -> ParamDef {
    ParamDef::new(name, &format!("param-{name}"), ParamKind::Color { default })
}

/// Channels every material produces.
pub const MATERIAL_CHANNELS: [Channel; 5] = [
    Channel::BaseColor,
    Channel::Metallic,
    Channel::Roughness,
    Channel::Normal,
    Channel::Height,
];

/// A material whose shader is `shaders/procedural/materials/<name>` (id `material.<name>`).
fn material(name: &str, params: Vec<ParamDef>) -> ProceduralDef {
    ProceduralDef {
        id: format!("material.{name}"),
        version: 1,
        name_key: format!("procedural-material-{name}"),
        category: Category::Material,
        params,
        shader: format!("procedural/materials/{name}"),
        outputs: MATERIAL_CHANNELS.map(Output::Channel).to_vec(),
    }
}

fn boolean(name: &str, default: bool) -> ParamDef {
    ParamDef::new(name, &format!("param-{name}"), ParamKind::Bool { default })
}

/// A gray procedural of `category` whose shader is `shaders/procedural/<folder>/<file>`.
fn gray(id: &str, category: Category, params: Vec<ParamDef>) -> ProceduralDef {
    let (family, file) = id.split_once('.').expect("ids are family.name");
    let folder = match family {
        "noise" => "noises",
        _ => "patterns",
    };
    ProceduralDef {
        id: id.to_owned(),
        version: 1,
        name_key: format!("procedural-{}", id.replace('.', "-")),
        category,
        params,
        shader: format!("procedural/{folder}/{file}"),
        outputs: vec![Output::Gray],
    }
}

fn fractal(id: &str) -> ProceduralDef {
    gray(
        id,
        Category::Noise,
        vec![
            int("scale", 1, 64, 4),
            int("octaves", 1, 8, 5),
            float("gain", 0.0, 1.0, 0.5),
            seed(),
        ],
    )
}

fn cells(id: &str, count: i32) -> ProceduralDef {
    gray(
        id,
        Category::Pattern,
        vec![
            int("count", 1, 32, count),
            float("gap", 0.0, 0.5, 0.06),
            float("bevel", 0.0, 0.5, 0.04),
            float("variation", 0.0, 1.0, 0.3),
            seed(),
        ],
    )
}

fn build() -> Vec<ProceduralDef> {
    let roughness = |default| float("roughness", 0.02, 1.0, default);
    let relief = || float("relief", 0.0, 2.0, 0.5);
    vec![
        material(
            "steel_brushed",
            vec![
                color("color", [0.56, 0.57, 0.58, 1.0]),
                roughness(0.35),
                int("scale", 1, 64, 8),
                float("brushing", 0.0, 1.0, 0.6),
                relief(),
                seed(),
            ],
        ),
        material(
            "copper",
            vec![
                color("color", [0.95, 0.64, 0.54, 1.0]),
                roughness(0.3),
                int("scale", 1, 64, 4),
                float("variation", 0.0, 1.0, 0.4),
                float("oxidation", 0.0, 1.0, 0.0),
                relief(),
                seed(),
            ],
        ),
        material(
            "plastic_grained",
            vec![
                color("color", [0.7, 0.08, 0.06, 1.0]),
                roughness(0.45),
                int("scale", 1, 128, 32),
                float("grain", 0.0, 1.0, 0.6),
                relief(),
                seed(),
            ],
        ),
        material(
            "wood",
            vec![
                color("light", [0.62, 0.43, 0.25, 1.0]),
                color("dark", [0.36, 0.2, 0.1, 1.0]),
                roughness(0.55),
                int("rings", 1, 64, 10),
                float("distortion", 0.0, 2.0, 0.5),
                relief(),
                seed(),
            ],
        ),
        material(
            "concrete",
            vec![
                color("color", [0.55, 0.54, 0.52, 1.0]),
                roughness(0.85),
                int("scale", 1, 64, 6),
                float("pores", 0.0, 1.0, 0.5),
                float("stains", 0.0, 1.0, 0.3),
                relief(),
                seed(),
            ],
        ),
        gray(
            "noise.value",
            Category::Noise,
            vec![int("scale", 1, 64, 8), seed()],
        ),
        gray(
            "noise.perlin",
            Category::Noise,
            vec![int("scale", 1, 64, 8), seed()],
        ),
        gray(
            "noise.simplex",
            Category::Noise,
            vec![int("scale", 1, 64, 4), seed()],
        ),
        gray(
            "noise.worley",
            Category::Noise,
            vec![
                int("scale", 1, 64, 8),
                choice("mode", &["f1", "f2", "f2-f1"], 0),
                seed(),
            ],
        ),
        fractal("noise.fbm"),
        fractal("noise.ridged"),
        fractal("noise.turbulence"),
        gray(
            "noise.warp",
            Category::Noise,
            vec![
                int("scale", 1, 64, 4),
                int("octaves", 1, 8, 4),
                float("gain", 0.0, 1.0, 0.5),
                float("strength", 0.0, 2.0, 0.6),
                seed(),
            ],
        ),
        gray(
            "pattern.gradient",
            Category::Pattern,
            vec![choice(
                "mode",
                &["linear-x", "linear-y", "triangle", "radial"],
                0,
            )],
        ),
        gray(
            "pattern.checker",
            Category::Pattern,
            vec![boolean("invert", false)],
        ),
        cells("pattern.tiles", 4),
        cells("pattern.bricks", 3),
        gray(
            "pattern.stripes",
            Category::Pattern,
            vec![
                int("count", 1, 64, 8),
                float("width", 0.0, 1.0, 0.5),
                float("softness", 0.0, 1.0, 0.05),
                choice("axis", &["x", "y"], 0),
            ],
        ),
        gray(
            "pattern.circles",
            Category::Pattern,
            vec![
                int("count", 1, 64, 6),
                float("radius", 0.0, 1.4, 0.6),
                float("softness", 0.0, 1.0, 0.05),
            ],
        ),
        gray(
            "pattern.scratches",
            Category::Pattern,
            vec![
                int("count", 1, 64, 8),
                float("length", 0.0, 1.0, 0.8),
                float("width", 0.001, 0.2, 0.03),
                seed(),
            ],
        ),
        gray(
            "pattern.fibers",
            Category::Pattern,
            vec![int("count", 1, 32, 4), int("stretch", 1, 16, 6), seed()],
        ),
    ]
}

static LIBRARY: LazyLock<Vec<ProceduralDef>> = LazyLock::new(build);

/// Every procedural, materials first.
pub fn all() -> &'static [ProceduralDef] {
    &LIBRARY
}

/// The procedural `id`.
pub fn find(id: &str) -> Option<&'static ProceduralDef> {
    LIBRARY.iter().find(|def| def.id == id)
}

/// Every translation key the library uses (names, parameters, options), for tests.
pub fn translation_keys() -> Vec<String> {
    let mut keys = Vec::new();
    for def in all() {
        keys.push(def.name_key.clone());
        for param in &def.params {
            keys.push(param.label_key.clone());
            if let ParamKind::Enum { options, .. } = &param.kind {
                keys.extend(options.iter().map(|o| format!("option-{o}")));
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_layouts_valid() {
        let mut ids: Vec<&str> = all().iter().map(|def| def.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all().len());
        for def in all() {
            def.layout().expect("every shipped procedural has a layout");
            assert_eq!(find(&def.id), Some(def));
        }
        assert!(find("noise.unknown").is_none());
        assert!(translation_keys().contains(&"option-f2-f1".to_owned()));
    }
}
