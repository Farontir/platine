//! A document using every kind of node, value and reference, for format tests.

use std::sync::Arc;

use patine_core::bake::BakedMapKind;
use patine_core::channel::{Channel, ChannelDef};
use patine_core::environment::EnvironmentSource;
use patine_core::export::{
    Axis, ColorSpace, Component, ExportOutput, ExportPreset, ImageFormat, OutputChannel,
};
use patine_core::ids::{AnchorId, EffectId, LayerId};
use patine_core::layer::{
    Anchor, BlendMode, ChannelBlend, FillSource, Layer, LayerKind, ProceduralRef, Projection,
    ProjectionKind,
};
use patine_core::mask::{
    Effect, EffectEntry, FaceSelection, FilterKind, GeneratorKind, GradientStop, LevelsParams,
    MaskEffect, MaskEffectEntry, MaskInitial, MaskStack, ParamValue, Params,
};
use patine_core::project::{NormalConvention, Project, Resolution, TextureSetId};
use patine_core::resource::{Resource, ResourceHash, ResourceKind};
use patine_core::stroke::{
    BrushSnapshot, Curve, Dab, PressureCurves, SizeSpace, Stroke, StrokeCamera, StrokeLog,
    StrokeSpace, Symmetry, Tool,
};

use super::{folder, project};

fn strokes(seed: u64, alpha: ResourceHash) -> Arc<StrokeLog> {
    Arc::new(StrokeLog {
        strokes: vec![Arc::new(Stroke {
            brush: BrushSnapshot {
                size: 32.5,
                size_space: SizeSpace::Screen,
                flow: 0.7,
                opacity: 1.0,
                hardness: 0.3,
                spacing: 0.1,
                angle: std::f32::consts::FRAC_PI_3,
                size_jitter: 0.0,
                angle_jitter: 0.2,
                flow_jitter: 0.0,
                scatter: 0.05,
                follow_path: true,
                alpha: Some(alpha),
                material: FillSource::Uniform([(Channel::Roughness, [0.9, 0.0, 0.0, 0.0])].into()),
                // What a version 1 stroke becomes.
                pressure: PressureCurves::none(),
                tilt_angle: false,
                projection: BrushSnapshot::default_projection(),
            },
            tool: Tool::Eraser,
            camera: StrokeCamera {
                view_proj: [
                    [1.5, 0.0, 0.0, 0.0],
                    [0.0, 2.0, 0.0, 0.0],
                    [0.0, 0.0, -1.0, -1.0],
                    [0.0, 0.0, 0.1, 0.0],
                ],
                viewport: [1920, 1080],
            },
            space: StrokeSpace::View,
            symmetry: Symmetry {
                mirror: [true, false, false],
                radial: 0,
            },
            seed,
            dabs: (0..50)
                .map(|i| Dab {
                    position: [i as f32 * 1.7, 100.0 + (i as f32).sin()],
                    pressure: 0.5 + 0.01 * i as f32,
                    tilt: [0.1, -0.2],
                    time: i as f32 / 120.0,
                })
                .collect(),
        })],
    })
}

/// The rich document: two texture sets, every layer kind, mask effect, parameter value,
/// projection and reference, two resources and an export preset.
pub fn rich_project() -> Project {
    let mut project = project();
    let (sky_hash, sky) = Resource::new(
        "sky.exr",
        ResourceKind::Environment,
        Arc::from(&b"v/1 fake exr"[..]),
    );
    let (image_hash, image) = Resource::new(
        "scratches.png",
        ResourceKind::Image,
        Arc::from(&[0x89, b'P', b'N', b'G', 0, 1, 2, 255][..]),
    );
    project.resources.insert(sky_hash, sky);
    project.resources.insert(image_hash, image);
    project.environment.source = EnvironmentSource::Resource(sky_hash);
    project.environment.rotation = 0.1;
    project.environment.exposure_ev = -1.0 / 3.0;
    project.normal_convention = NormalConvention::DirectX;

    let params: Params = [
        (
            "scale".to_owned(),
            ParamValue::Float(f32::MIN_POSITIVE / 4.0),
        ),
        ("octaves".to_owned(), ParamValue::Int(-3)),
        ("invert".to_owned(), ParamValue::Bool(true)),
        ("tint".to_owned(), ParamValue::Color([0.1, 0.2, 0.3, 1.0])),
        ("mode".to_owned(), ParamValue::Enum("worley".into())),
        (
            "curve".to_owned(),
            ParamValue::Curve(vec![[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]]),
        ),
        (
            "ramp".to_owned(),
            ParamValue::Gradient(vec![GradientStop {
                position: 0.25,
                color: [1.0, 0.5, 0.0, 1.0],
            }]),
        ),
        ("mask".to_owned(), ParamValue::Texture(Some(image_hash))),
        ("none".to_owned(), ParamValue::Texture(None)),
    ]
    .into();

    // Texture set 0: a folder with a hidden painted layer, an image fill carrying an
    // anchor, and a procedural fill with a mask using every effect and two effects.
    let mut painted = Layer::new(
        LayerId(2),
        "paint",
        LayerKind::Paint {
            strokes: strokes(42, image_hash),
        },
        &[Channel::BaseColor, Channel::User(3)],
    );
    painted.visible = false;
    let mut decal = Layer::new(
        LayerId(3),
        "decal",
        LayerKind::Fill {
            source: FillSource::Image {
                resource: image_hash,
            },
            projection: Projection {
                kind: ProjectionKind::Triplanar { blend: 0.3 },
                scale: [2.0, 2.0],
                rotation: 0.25,
                offset: [0.5, -0.5],
            },
        },
        &[Channel::BaseColor],
    );
    decal.anchor = Some(Anchor {
        id: AnchorId(4),
        name: "decal height".into(),
        channels: vec![Channel::Height],
    });
    let mut rust = Layer::new(
        LayerId(5),
        "rust",
        LayerKind::Fill {
            source: FillSource::Procedural(ProceduralRef {
                id: "iron_rust".into(),
                version: 2,
                params: params.clone(),
            }),
            projection: Projection::default(),
        },
        &[
            Channel::BaseColor,
            Channel::Metallic,
            Channel::Roughness,
            Channel::Normal,
        ],
    );
    rust.channels.insert(
        Channel::Roughness,
        ChannelBlend {
            mode: BlendMode::Overlay,
            opacity: 0.35,
        },
    );
    let entry = |id: u64, blend, effect| MaskEffectEntry {
        id: EffectId(id),
        visible: id.is_multiple_of(2),
        blend,
        opacity: 0.8,
        effect,
    };
    rust.mask = Some(MaskStack {
        initial: MaskInitial::White,
        effects: vec![
            entry(
                6,
                BlendMode::Normal,
                MaskEffect::Paint {
                    strokes: strokes(7, image_hash),
                },
            ),
            entry(
                7,
                BlendMode::Multiply,
                MaskEffect::Fill {
                    source: FillSource::Uniform([(Channel::User(0), [0.5; 4])].into()),
                },
            ),
            entry(
                8,
                BlendMode::Screen,
                MaskEffect::Generator {
                    kind: GeneratorKind::EdgeWear,
                    params: params.clone(),
                },
            ),
            entry(
                9,
                BlendMode::Subtract,
                MaskEffect::Filter {
                    kind: FilterKind::GaussianBlur,
                    params: Params::default(),
                },
            ),
            entry(
                10,
                BlendMode::Normal,
                MaskEffect::Levels(LevelsParams::default()),
            ),
            entry(
                11,
                BlendMode::Lighten,
                MaskEffect::AnchorRef {
                    anchor: AnchorId(4),
                    channel: Channel::Height,
                },
            ),
            entry(
                12,
                BlendMode::Replace,
                MaskEffect::PolygonFill {
                    selection: FaceSelection {
                        triangles: vec![0, 1],
                    },
                },
            ),
        ],
    });
    rust.effects = vec![
        EffectEntry {
            id: EffectId(13),
            visible: true,
            effect: Effect::Filter {
                kind: FilterKind::Hsl,
                params,
            },
        },
        EffectEntry {
            id: EffectId(14),
            visible: false,
            effect: Effect::Levels(LevelsParams {
                gamma: 2.2,
                ..LevelsParams::default()
            }),
        },
    ];
    let mut group = folder(LayerId(1), "weathering");
    if let LayerKind::Folder { children } = &mut group.kind {
        children.push(painted);
    }
    let set = &mut project.texture_sets[0];
    set.stack.layers = vec![group, decal, rust];
    set.baked_maps.maps.insert(
        BakedMapKind::AmbientOcclusion,
        Resolution::new(4096).expect("supported resolution"),
    );

    // Texture set 1: an instance of the rust layer, at another resolution.
    let set = project
        .texture_set_mut(TextureSetId(1))
        .expect("the mesh has two materials");
    set.resolution = Resolution::new(512).expect("supported resolution");
    set.channels.push(ChannelDef::standard(Channel::Emissive));
    let mut instance = Layer::new(
        LayerId(15),
        "rust instance",
        LayerKind::Instance { source: LayerId(5) },
        &[Channel::BaseColor],
    );
    instance.channels.insert(
        Channel::BaseColor,
        ChannelBlend {
            mode: BlendMode::Passthrough,
            opacity: 1.0,
        },
    );
    set.stack.layers = vec![instance];

    project.export_presets = vec![ExportPreset {
        name: "Engine".into(),
        normal_convention: NormalConvention::DirectX,
        outputs: vec![
            ExportOutput {
                file: "$mesh_$textureSet_ORM".into(),
                format: ImageFormat::Png,
                bit_depth: 8,
                color_space: ColorSpace::Linear,
                r: OutputChannel::Baked(BakedMapKind::AmbientOcclusion, Component::Gray),
                g: OutputChannel::Channel(Channel::Roughness, Component::Gray),
                b: OutputChannel::Inverted(Channel::Metallic, Component::R),
                a: OutputChannel::Constant(1.0),
            },
            ExportOutput {
                file: "$mesh_$textureSet_Normal".into(),
                format: ImageFormat::Exr,
                bit_depth: 32,
                color_space: ColorSpace::Srgb,
                r: OutputChannel::Normal(Axis::X),
                g: OutputChannel::Normal(Axis::Y),
                b: OutputChannel::NormalizedHeight,
                a: OutputChannel::None,
            },
        ],
    }];
    project.next_id = 16;
    project.validate().expect("the rich project is valid");
    project
}

/// [`rich_project`] with what version 2 of the format added: a texture set's height
/// force, and a stroke painted in UV space with pressure curves, tilt and a procedural
/// material.
pub fn rich_project_v2() -> Project {
    let mut project = rich_project();
    project.texture_sets[0].height_force = 0.37;
    let mut found = false;
    project.texture_sets[0].stack.visit_mut(&mut |layer| {
        if let LayerKind::Paint { strokes } = &mut layer.kind {
            let mut stroke = (*strokes.strokes[0]).clone();
            stroke.space = StrokeSpace::Uv;
            stroke.seed = 1234;
            stroke.brush.pressure = PressureCurves {
                size: Curve(vec![[0.0, 0.2], [0.6, 0.9], [1.0, 1.0]]),
                flow: Curve::identity(),
                opacity: Curve::constant(0.75),
            };
            stroke.brush.tilt_angle = true;
            stroke.brush.material = FillSource::Procedural(ProceduralRef {
                id: "material.copper".into(),
                version: 1,
                params: [("oxidation".to_owned(), ParamValue::Float(0.4))].into(),
            });
            stroke.brush.projection = Projection {
                kind: ProjectionKind::Spherical,
                scale: [3.0, 1.5],
                rotation: 0.2,
                offset: [0.1, 0.0],
            };
            Arc::make_mut(strokes).strokes.push(Arc::new(stroke));
            found = true;
        }
    });
    assert!(found, "the rich project has a paint layer");
    project
}
