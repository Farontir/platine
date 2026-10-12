//! Descriptions of the procedurals: their parameters, the layout of those parameters in
//! the shader's uniform buffer, and the values a project's [`ProceduralRef`] resolves to.
//!
//! Every shader declares its `Params` structure by hand; a test checks with naga that the
//! structure of each shader has exactly the layout computed here from the definition.

use patine_core::channel::Channel;
use patine_core::layer::ProceduralRef;
use patine_core::mask::{ParamValue, Params};
use serde::{Deserialize, Serialize};

/// Type, domain and default value of a parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamKind {
    /// Real number in `min..=max`.
    Float {
        /// Smallest value.
        min: f32,
        /// Largest value.
        max: f32,
        /// Default value.
        default: f32,
    },
    /// Integer in `min..=max`.
    Int {
        /// Smallest value.
        min: i32,
        /// Largest value.
        max: i32,
        /// Default value.
        default: i32,
    },
    /// Random seed (a non-negative integer; 0 by default).
    Seed,
    /// Switch.
    Bool {
        /// Default value.
        default: bool,
    },
    /// Linear RGBA color.
    Color {
        /// Default value.
        default: [f32; 4],
    },
    /// One option among names.
    Enum {
        /// Option names (identifiers, translated in the interface).
        options: Vec<String>,
        /// Index of the default option.
        default: usize,
    },
    /// Tone curve (filters, M15).
    Curve,
    /// Color gradient (M15).
    Gradient,
    /// Image resource (library, M16).
    Texture,
}

/// One parameter of a procedural.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    /// Identifier, also the member name in the shader's `Params` structure.
    pub name: String,
    /// Translation key of its label.
    pub label_key: String,
    /// Type, domain and default.
    pub kind: ParamKind,
}

impl ParamDef {
    /// A parameter named `name`, labeled by `label_key`.
    pub fn new(name: &str, label_key: &str, kind: ParamKind) -> Self {
        Self {
            name: name.to_owned(),
            label_key: label_key.to_owned(),
            kind,
        }
    }

    /// The default value.
    pub fn default_value(&self) -> ParamValue {
        match &self.kind {
            ParamKind::Float { default, .. } => ParamValue::Float(*default),
            ParamKind::Int { default, .. } => ParamValue::Int(*default),
            ParamKind::Seed => ParamValue::Int(0),
            ParamKind::Bool { default } => ParamValue::Bool(*default),
            ParamKind::Color { default } => ParamValue::Color(*default),
            ParamKind::Enum { options, default } => {
                ParamValue::Enum(options.get(*default).cloned().unwrap_or_default())
            }
            ParamKind::Curve => ParamValue::Curve(vec![[0.0, 0.0], [1.0, 1.0]]),
            ParamKind::Gradient => ParamValue::Gradient(Vec::new()),
            ParamKind::Texture => ParamValue::Texture(None),
        }
    }

    /// `value` brought into the parameter's domain, or `None` when it has another type.
    pub fn accept(&self, value: &ParamValue) -> Option<ParamValue> {
        Some(match (&self.kind, value) {
            (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) if v.is_finite() => {
                ParamValue::Float(v.clamp(*min, *max))
            }
            (ParamKind::Int { min, max, .. }, ParamValue::Int(v)) => {
                ParamValue::Int((*v).clamp(*min, *max))
            }
            (ParamKind::Seed, ParamValue::Int(v)) => ParamValue::Int((*v).max(0)),
            (ParamKind::Bool { .. }, ParamValue::Bool(v)) => ParamValue::Bool(*v),
            (ParamKind::Color { .. }, ParamValue::Color(c)) if c.iter().all(|v| v.is_finite()) => {
                ParamValue::Color(*c)
            }
            (ParamKind::Enum { options, .. }, ParamValue::Enum(name)) if options.contains(name) => {
                ParamValue::Enum(name.clone())
            }
            (ParamKind::Curve, ParamValue::Curve(points)) => ParamValue::Curve(points.clone()),
            (ParamKind::Gradient, ParamValue::Gradient(stops)) => {
                ParamValue::Gradient(stops.clone())
            }
            (ParamKind::Texture, ParamValue::Texture(texture)) => ParamValue::Texture(*texture),
            _ => return None,
        })
    }
}

/// Family of a procedural, for the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    /// A material producing several channels.
    Material,
    /// A grayscale noise.
    Noise,
    /// A grayscale pattern.
    Pattern,
}

/// What a procedural produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Output {
    /// A channel of a texture set (materials).
    Channel(Channel),
    /// One gray value (noises and patterns, for masks).
    Gray,
}

/// A procedural material, noise or pattern.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProceduralDef {
    /// Stable identifier, stored in projects (`material.copper`, `noise.perlin`...).
    pub id: String,
    /// Version of the parameters, to migrate older projects.
    pub version: u32,
    /// Translation key of its name.
    pub name_key: String,
    /// Family.
    pub category: Category,
    /// Parameters, in the order of the shader's `Params` structure.
    pub params: Vec<ParamDef>,
    /// Shader, by its path under `shaders/` without extension.
    pub shader: String,
    /// What it produces.
    pub outputs: Vec<Output>,
}

/// WGSL type of a parameter in the uniform buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WgslType {
    /// `f32`.
    F32,
    /// `i32`.
    I32,
    /// `u32` (seeds, booleans, enum indices).
    U32,
    /// `vec4<f32>` (colors).
    Vec4F32,
}

impl WgslType {
    /// Size and alignment in a uniform buffer.
    fn size_align(self) -> (u32, u32) {
        match self {
            WgslType::F32 | WgslType::I32 | WgslType::U32 => (4, 4),
            WgslType::Vec4F32 => (16, 16),
        }
    }
}

/// Place of one parameter in the uniform buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Parameter (member) name.
    pub name: String,
    /// Byte offset.
    pub offset: u32,
    /// Type.
    pub ty: WgslType,
}

/// Layout of the parameters of a procedural.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// One field per parameter, in order.
    pub fields: Vec<Field>,
    /// Size of the structure, a multiple of 16 as uniform buffers require.
    pub size: u32,
}

/// Errors of the procedural definitions.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DefError {
    /// The parameter's type has no shader representation yet.
    #[error("parameter {0} has a type that shaders cannot read yet")]
    UnsupportedParam(String),
    /// The definition has no parameter: WGSL structures need a member.
    #[error("procedural {0} has no parameter")]
    NoParam(String),
}

fn round_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align) * align
}

impl ProceduralDef {
    /// Layout of the parameters in the uniform buffer, following WGSL's rules: each
    /// member at the next multiple of its alignment, the structure rounded up to 16.
    pub fn layout(&self) -> Result<Layout, DefError> {
        if self.params.is_empty() {
            return Err(DefError::NoParam(self.id.clone()));
        }
        let mut offset = 0;
        let mut fields = Vec::with_capacity(self.params.len());
        for param in &self.params {
            let ty = match param.kind {
                ParamKind::Float { .. } => WgslType::F32,
                ParamKind::Int { .. } => WgslType::I32,
                ParamKind::Seed | ParamKind::Bool { .. } | ParamKind::Enum { .. } => WgslType::U32,
                ParamKind::Color { .. } => WgslType::Vec4F32,
                ParamKind::Curve | ParamKind::Gradient | ParamKind::Texture => {
                    return Err(DefError::UnsupportedParam(param.name.clone()));
                }
            };
            let (size, align) = ty.size_align();
            offset = round_up(offset, align);
            fields.push(Field {
                name: param.name.clone(),
                offset,
                ty,
            });
            offset += size;
        }
        Ok(Layout {
            fields,
            size: round_up(offset, 16),
        })
    }

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&ParamDef> {
        self.params.iter().find(|param| param.name == name)
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Params {
        self.params
            .iter()
            .map(|param| (param.name.clone(), param.default_value()))
            .collect()
    }

    /// A new reference to this procedural, with default parameters.
    pub fn instance(&self) -> ProceduralRef {
        ProceduralRef {
            id: self.id.clone(),
            version: self.version,
            params: self.defaults(),
        }
    }

    /// The values the parameters of `instance` take: its own when they fit their
    /// definition (brought into their domain), defaults for the missing ones; unknown
    /// parameters are dropped. Older versions keep their values (no migration needed
    /// before a definition changes).
    pub fn resolve(&self, instance: &ProceduralRef) -> Params {
        self.params
            .iter()
            .map(|param| {
                let value = instance
                    .params
                    .get(&param.name)
                    .and_then(|value| param.accept(value))
                    .unwrap_or_else(|| param.default_value());
                (param.name.clone(), value)
            })
            .collect()
    }

    /// The uniform buffer bytes of `params` (resolved values, see [`Self::resolve`]).
    pub fn pack(&self, params: &Params) -> Result<Vec<u8>, DefError> {
        let layout = self.layout()?;
        let mut bytes = vec![0u8; layout.size as usize];
        for (param, field) in self.params.iter().zip(&layout.fields) {
            let value = params
                .get(&param.name)
                .and_then(|value| param.accept(value))
                .unwrap_or_else(|| param.default_value());
            let at = field.offset as usize;
            let mut put = |data: &[u8]| bytes[at..at + data.len()].copy_from_slice(data);
            match (&param.kind, value) {
                (_, ParamValue::Float(v)) => put(&v.to_le_bytes()),
                (ParamKind::Int { .. }, ParamValue::Int(v)) => put(&v.to_le_bytes()),
                (_, ParamValue::Int(v)) => put(&(v.max(0) as u32).to_le_bytes()),
                (_, ParamValue::Bool(v)) => put(&u32::from(v).to_le_bytes()),
                (_, ParamValue::Color(c)) => {
                    let data: Vec<u8> = c.iter().flat_map(|v| v.to_le_bytes()).collect();
                    put(&data);
                }
                (ParamKind::Enum { options, .. }, ParamValue::Enum(name)) => {
                    let index = options.iter().position(|o| *o == name).unwrap_or(0);
                    put(&(index as u32).to_le_bytes());
                }
                _ => return Err(DefError::UnsupportedParam(param.name.clone())),
            }
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> ProceduralDef {
        ProceduralDef {
            id: "test.probe".into(),
            version: 2,
            name_key: "probe".into(),
            category: Category::Noise,
            params: vec![
                ParamDef::new(
                    "scale",
                    "p-scale",
                    ParamKind::Float {
                        min: 1.0,
                        max: 64.0,
                        default: 4.0,
                    },
                ),
                ParamDef::new("seed", "p-seed", ParamKind::Seed),
                ParamDef::new(
                    "tint",
                    "p-tint",
                    ParamKind::Color {
                        default: [1.0, 0.5, 0.25, 1.0],
                    },
                ),
                ParamDef::new("invert", "p-invert", ParamKind::Bool { default: false }),
                ParamDef::new(
                    "mode",
                    "p-mode",
                    ParamKind::Enum {
                        options: vec!["f1".into(), "f2".into()],
                        default: 1,
                    },
                ),
                ParamDef::new(
                    "octaves",
                    "p-octaves",
                    ParamKind::Int {
                        min: 1,
                        max: 8,
                        default: 4,
                    },
                ),
            ],
            shader: "tests/params_probe".into(),
            outputs: vec![Output::Gray],
        }
    }

    #[test]
    fn layouts_follow_the_wgsl_alignment_rules() {
        let layout = def().layout().unwrap();
        let offsets: Vec<u32> = layout.fields.iter().map(|f| f.offset).collect();
        // f32 at 0, u32 at 4, vec4 aligned to 16, then u32, u32, i32 after it.
        assert_eq!(offsets, [0, 4, 16, 32, 36, 40]);
        assert_eq!(layout.size, 48);
        let mut curve = def();
        curve
            .params
            .push(ParamDef::new("curve", "p-curve", ParamKind::Curve));
        assert_eq!(
            curve.layout(),
            Err(DefError::UnsupportedParam("curve".into()))
        );
    }

    #[test]
    fn instances_resolve_to_valid_values() {
        let def = def();
        let mut instance = def.instance();
        assert_eq!(instance.version, 2);
        assert_eq!(
            instance.params.get("mode"),
            Some(&ParamValue::Enum("f2".into()))
        );
        // Out of range, wrong type, unknown option, unknown parameter, missing parameter.
        instance
            .params
            .insert("scale".into(), ParamValue::Float(500.0));
        instance
            .params
            .insert("seed".into(), ParamValue::Bool(true));
        instance
            .params
            .insert("mode".into(), ParamValue::Enum("f9".into()));
        instance
            .params
            .insert("gone".into(), ParamValue::Float(1.0));
        instance.params.remove("octaves");
        let resolved = def.resolve(&instance);
        assert_eq!(resolved.get("scale"), Some(&ParamValue::Float(64.0)));
        assert_eq!(resolved.get("seed"), Some(&ParamValue::Int(0)));
        assert_eq!(resolved.get("mode"), Some(&ParamValue::Enum("f2".into())));
        assert_eq!(resolved.get("octaves"), Some(&ParamValue::Int(4)));
        assert!(!resolved.contains_key("gone"));
    }

    #[test]
    fn packing_writes_each_field_at_its_offset() {
        let def = def();
        let mut params = def.defaults();
        params.insert("seed".into(), ParamValue::Int(7));
        params.insert("invert".into(), ParamValue::Bool(true));
        let bytes = def.pack(&params).unwrap();
        assert_eq!(bytes.len(), 48);
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(f32::from_bits(word(0)), 4.0);
        assert_eq!(word(4), 7);
        assert_eq!(f32::from_bits(word(16)), 1.0);
        assert_eq!(f32::from_bits(word(28)), 1.0);
        assert_eq!(word(32), 1);
        assert_eq!(word(36), 1, "option f2");
        assert_eq!(word(40) as i32, 4);
    }
}
