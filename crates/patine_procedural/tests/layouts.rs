//! The `Params` structure of every procedural shader has exactly the layout computed from
//! its definition: same members, in the same order, with the same types and offsets.

use patine_procedural::def::{
    Category, Layout, Output, ParamDef, ParamKind, ProceduralDef, WgslType,
};

/// Compares the `Params` structure of `def.shader` with `def.layout()`.
fn check(def: &ProceduralDef) -> Result<(), String> {
    let layout: Layout = def.layout().map_err(|e| e.to_string())?;
    let source = patine_gpu::shaders::load(&def.shader).map_err(|e| e.to_string())?;
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| e.emit_to_string(&source))?;
    let (members, span) = module
        .types
        .iter()
        .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
            (Some(name), naga::TypeInner::Struct { members, span }) if name == "Params" => {
                Some((members, *span))
            }
            _ => None,
        })
        .ok_or("no Params structure")?;
    if members.len() != layout.fields.len() {
        return Err(format!(
            "{} members in the shader, {} parameters",
            members.len(),
            layout.fields.len()
        ));
    }
    for (member, field) in members.iter().zip(&layout.fields) {
        let ty = match &module.types[member.ty].inner {
            naga::TypeInner::Scalar(scalar) => match scalar.kind {
                naga::ScalarKind::Float => WgslType::F32,
                naga::ScalarKind::Sint => WgslType::I32,
                naga::ScalarKind::Uint => WgslType::U32,
                other => return Err(format!("{:?}: unexpected {other:?}", member.name)),
            },
            naga::TypeInner::Vector {
                size: naga::VectorSize::Quad,
                scalar,
            } if scalar.kind == naga::ScalarKind::Float => WgslType::Vec4F32,
            other => return Err(format!("{:?}: unexpected {other:?}", member.name)),
        };
        let found = (member.name.as_deref(), member.offset, ty);
        let expected = (Some(field.name.as_str()), field.offset, field.ty);
        if found != expected {
            return Err(format!("shader {found:?}, definition {expected:?}"));
        }
    }
    // The shader may round its size up further; it must hold the packed bytes.
    if span < layout.size.min(span.next_multiple_of(16)) {
        return Err(format!("shader size {span}, definition {}", layout.size));
    }
    Ok(())
}

fn probe() -> ProceduralDef {
    let float = ParamKind::Float {
        min: 1.0,
        max: 64.0,
        default: 4.0,
    };
    ProceduralDef {
        id: "test.probe".into(),
        version: 1,
        name_key: "probe".into(),
        category: Category::Noise,
        params: vec![
            ParamDef::new("scale", "p", float),
            ParamDef::new("seed", "p", ParamKind::Seed),
            ParamDef::new("tint", "p", ParamKind::Color { default: [1.0; 4] }),
            ParamDef::new("invert", "p", ParamKind::Bool { default: false }),
            ParamDef::new(
                "mode",
                "p",
                ParamKind::Enum {
                    options: vec!["a".into()],
                    default: 0,
                },
            ),
            ParamDef::new(
                "octaves",
                "p",
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
fn the_probe_shader_matches_its_definition() {
    check(&probe()).unwrap();
    // A definition that disagrees with the shader is caught.
    let mut swapped = probe();
    swapped.params.swap(3, 4);
    assert!(check(&swapped).is_err());
    let mut missing = probe();
    missing.params.pop();
    assert!(check(&missing).is_err());
}
