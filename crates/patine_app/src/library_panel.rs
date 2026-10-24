//! Library panel: the procedural materials, and the noises and patterns for masks, with a
//! thumbnail each. Double-clicking a material adds it as a fill layer above the selection;
//! double-clicking a noise or a pattern adds it to the mask of the selected layer.
//!
//! Thumbnails are rendered under the loading page ([`render_thumbnail`]); one missing
//! afterwards is rendered by the application (`App::render_thumbnails`), one per frame
//! while the panel is visible, the panel showing a placeholder until then.

use std::collections::HashMap;

use egui::{RichText, Sense, Ui};
use patine_compositor::compositor::Compositor;
use patine_core::command::Command;
use patine_core::ids::IdSequence;
use patine_core::layer::{BlendMode, Layer};
use patine_core::project::{Project, TextureSetId};
use patine_gpu::{GpuContext, wgpu};
use patine_procedural::def::{Category, ProceduralDef};
use patine_procedural::library::{self, MATERIAL_CHANNELS};

use crate::i18n::I18n;
use crate::layers_panel::{LayersOutput, LayersState, insertion};
use crate::masks::{LayerPart, insert_effect, procedural_fill};

/// Side of a thumbnail, in texels.
pub const THUMBNAIL_SIZE: u32 = 96;

/// Interface state of the panel.
#[derive(Default)]
pub struct LibraryState {
    /// Rendered thumbnails, by procedural id.
    pub thumbnails: HashMap<String, egui::TextureHandle>,
    /// Set when the panel was drawn this frame: thumbnails are rendered only then.
    pub visible: bool,
}

impl std::fmt::Debug for LibraryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryState")
            .field("thumbnails", &self.thumbnails.len())
            .finish()
    }
}

/// A fill layer of the material `def` above the selection of `state`, in texture set
/// `set`; it affects the material's channels that the set has.
pub fn new_material_layer(
    i18n: &I18n,
    project: &Project,
    set: TextureSetId,
    selected: Option<patine_core::ids::LayerId>,
    def: &ProceduralDef,
) -> Option<patine_core::commands::InsertLayer> {
    let set = project.texture_set(set)?;
    let channels: Vec<_> = MATERIAL_CHANNELS
        .into_iter()
        .filter(|channel| set.channels.iter().any(|d| d.channel == *channel))
        .collect();
    let mut ids = IdSequence::starting_at(project.next_id);
    let layer = Layer::fill_procedural(
        ids.layer(),
        i18n.tr(&def.name_key),
        def.instance(),
        &channels,
    );
    Some(patine_core::commands::InsertLayer::new(
        set.id,
        insertion(set, selected),
        layer,
    ))
}

/// The noise or pattern `def` as a fill of the mask of the selected layer, above the
/// selected effect; `None` without a masked selection.
pub fn new_mask_procedural(
    project: &Project,
    state: &LayersState,
    def: &ProceduralDef,
) -> Option<patine_core::commands::InsertMaskEffect> {
    let layer = state.selected.and_then(|id| {
        project
            .texture_sets
            .iter()
            .find_map(|set| set.stack.find(id))
    })?;
    let above = match state.part {
        LayerPart::MaskEffect(effect) => Some(effect),
        _ => None,
    };
    insert_effect(
        project,
        layer,
        procedural_fill(def),
        BlendMode::Normal,
        1.0,
        above,
    )
}

fn item(ui: &mut Ui, i18n: &I18n, state: &LibraryState, def: &ProceduralDef) -> egui::Response {
    ui.vertical(|ui| {
        ui.set_width(84.0);
        let size = egui::vec2(76.0, 76.0);
        let response = match state.thumbnails.get(&def.id) {
            Some(texture) => ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(size)
                    .sense(Sense::click()),
            ),
            None => {
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
                response
            }
        };
        ui.label(RichText::new(i18n.tr(&def.name_key)).small());
        response
    })
    .inner
}

/// Draws the panel.
pub fn show(
    ui: &mut Ui,
    i18n: &I18n,
    project: Option<&Project>,
    set: TextureSetId,
    layers: &mut LayersState,
    state: &mut LibraryState,
) -> LayersOutput {
    state.visible = true;
    let mut output = LayersOutput::default();
    let sections = [
        ("library-materials", vec![Category::Material]),
        ("library-masks", vec![Category::Noise, Category::Pattern]),
    ];
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (title, categories) in sections {
                egui::CollapsingHeader::new(i18n.tr(title))
                    .default_open(true)
                    .show(ui, |ui| {
                        if title == "library-masks" {
                            ui.label(RichText::new(i18n.tr("library-masks-hint")).weak().small());
                        } else {
                            ui.label(
                                RichText::new(i18n.tr("library-materials-hint"))
                                    .weak()
                                    .small(),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            for def in library::all()
                                .iter()
                                .filter(|def| categories.contains(&def.category))
                            {
                                let response = item(ui, i18n, state, def);
                                if !response.double_clicked() {
                                    continue;
                                }
                                let Some(project) = project else {
                                    continue;
                                };
                                let command: Option<Box<dyn Command>> = if def.category
                                    == Category::Material
                                {
                                    new_material_layer(i18n, project, set, layers.selected, def)
                                        .map(|insert| {
                                            layers.selected =
                                                Some(patine_core::ids::LayerId(project.next_id));
                                            layers.part = LayerPart::Content;
                                            Box::new(insert) as Box<dyn Command>
                                        })
                                } else {
                                    new_mask_procedural(project, layers, def).map(|insert| {
                                        layers.part = LayerPart::MaskEffect(
                                            patine_core::ids::EffectId(project.next_id),
                                        );
                                        Box::new(insert) as Box<dyn Command>
                                    })
                                };
                                if let Some(command) = command {
                                    output.commands.push(command);
                                    output.seal = true;
                                }
                            }
                        });
                    });
            }
        });
    output
}

/// Translation keys of the panel, for tests.
pub const TRANSLATION_KEYS: [&str; 4] = [
    "library-materials",
    "library-masks",
    "library-materials-hint",
    "library-masks-hint",
];

/// RGBA8 thumbnail of a material from its base color and normal at
/// [`THUMBNAIL_SIZE`]², lit by a fixed light; or of a gray procedural from its value.
pub fn thumbnail_pixels(base: &[[f32; 4]], normal: Option<&[[f32; 4]]>) -> Vec<u8> {
    let light = [0.4f32, -0.5, 0.77];
    let encode = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
    base.iter()
        .enumerate()
        .flat_map(|(i, c)| match normal {
            Some(normal) => {
                let n = normal[i];
                let lambert =
                    (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0) * 0.8 + 0.25;
                [
                    encode(c[0] * lambert),
                    encode(c[1] * lambert),
                    encode(c[2] * lambert),
                    255,
                ]
            }
            None => {
                let v = (c[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                [v, v, v, 255]
            }
        })
        .collect()
}

/// Renders the RGBA8 thumbnail of `def` at [`THUMBNAIL_SIZE`]²: every output of the
/// procedural in one pass, read back. A failed thumbnail is gray, so that it is not
/// rendered again.
///
/// The first thumbnail of a procedural compiles its shader, which can take seconds:
/// the loading page renders them all before the interface shows.
pub fn render_thumbnail(gpu: &GpuContext, compositor: &Compositor, def: &ProceduralDef) -> Vec<u8> {
    use patine_procedural::evaluate::{EvaluationUniforms, SurfaceInput};

    let size = THUMBNAIL_SIZE;
    let render = || -> Option<Vec<Vec<[f32; 4]>>> {
        let formats = patine_procedural::evaluate::output_formats(def);
        let textures: Vec<wgpu::Texture> = formats
            .iter()
            .map(|format| {
                gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("thumbnail"),
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: *format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            })
            .collect();
        let views: Vec<wgpu::TextureView> = textures
            .iter()
            .map(|texture| texture.create_view(&Default::default()))
            .collect();
        let targets: Vec<_> = views.iter().zip(formats).collect();
        let uniforms = EvaluationUniforms::new(
            &patine_core::layer::Projection::default(),
            size,
            [0.0; 3],
            1.0,
            true,
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let result = compositor.procedurals().evaluate(
            gpu,
            &mut encoder,
            def,
            &def.defaults(),
            &uniforms,
            SurfaceInput::Flat,
            &targets,
        );
        if let Err(error) = result {
            tracing::error!(id = %def.id, %error, "thumbnail failed");
            return None;
        }
        gpu.queue.submit([encoder.finish()]);
        textures
            .iter()
            .map(|texture| match texture.format() {
                wgpu::TextureFormat::Rgba16Float => {
                    patine_gpu::readback::read_rgba16f(gpu, texture).ok()
                }
                _ => patine_gpu::readback::read_r16f_level(gpu, texture, 0, 0)
                    .ok()
                    .map(|values| values.into_iter().map(|v| [v, 0.0, 0.0, 0.0]).collect()),
            })
            .collect()
    };
    let pixels = render().and_then(|outputs| {
        if def.category == Category::Material {
            // Outputs in the order of their index: base color 0, normal 3.
            Some(thumbnail_pixels(outputs.first()?, Some(outputs.get(3)?)))
        } else {
            Some(thumbnail_pixels(outputs.first()?, None))
        }
    });
    pixels.unwrap_or_else(|| vec![64; (size * size * 4) as usize])
}

/// Adds the thumbnail `pixels` of the procedural `id` to the panel.
pub fn insert_thumbnail(state: &mut LibraryState, ctx: &egui::Context, id: &str, pixels: &[u8]) {
    let side = THUMBNAIL_SIZE as usize;
    let image = egui::ColorImage::from_rgba_unmultiplied([side; 2], pixels);
    let handle = ctx.load_texture(
        format!("thumbnail {id}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    state.thumbnails.insert(id.to_owned(), handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn every_text_of_the_library_is_translated() {
        let mut keys: Vec<String> = library::translation_keys();
        keys.extend(TRANSLATION_KEYS.iter().map(|k| (*k).to_owned()));
        keys.extend(
            crate::procedural_ui::TRANSLATION_KEYS
                .iter()
                .map(|k| (*k).to_owned()),
        );
        keys.push("properties-projection".to_owned());
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for key in &keys {
                assert!(i18n.has(key), "{language:?}: {key}");
            }
        }
    }

    #[test]
    fn thumbnails_light_materials_and_show_gray_values() {
        let base = vec![[0.5, 0.25, 0.125, 1.0]; 4];
        let flat = vec![[0.0, 0.0, 1.0, 0.0]; 4];
        let lit = thumbnail_pixels(&base, Some(&flat));
        assert_eq!(lit.len(), 16);
        assert!(lit[0] > lit[1] && lit[1] > lit[2] && lit[3] == 255);
        let gray = thumbnail_pixels(&[[0.5, 0.0, 0.0, 1.0]], None);
        assert_eq!(gray, [128, 128, 128, 255]);
    }
}
