//! egui interface: menu bar, dockable panels, status bar and dialogs.
//!
//! The interface holds no document state. It reads application state and returns the
//! actions and commands the user triggered; the application executes them.

use egui::{Color32, RichText, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use egui_phosphor::regular as icons;
use fluent_bundle::FluentArgs;
use tracing::Level;

use patine_core::channel::Channel;
use patine_core::command::Command;
use patine_core::project::Project;
use patine_render::settings::DisplayMode;

use crate::actions::{ActionContext, ActionId, ActionRegistry};
use crate::display_panel::{self, EnvironmentView, PickedHdri};
use crate::document::EnvironmentChange;
use crate::history_panel::{self, HistoryView};
use crate::i18n::I18n;
use crate::layers_panel::{self, LayersState};
use crate::logging::LogBuffer;
use crate::new_project::{CreatedProject, DialogOutcome, NewProjectDialog};
use crate::properties_panel;
use crate::texture_sets_panel::{self, TextureSetsState};
use crate::view_2d::{self, View2dState};
use crate::viewport_panel::{self, ViewState};

/// A dockable panel of the main window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Real-time PBR render of the mesh.
    Viewport3D,
    /// Texture of the current texture set with the UV wireframe.
    View2D,
    /// Texture sets, resolution, channels and baked maps.
    TextureSets,
    /// Layer stack.
    Layers,
    /// Properties of the active tool or selected layer.
    Properties,
    /// Asset library.
    Library,
    /// Environment, tone mapping and display mode.
    Display,
    /// Command history.
    History,
    /// Logs and profiler.
    Console,
}

impl Panel {
    /// Every panel.
    pub const ALL: [Panel; 9] = [
        Panel::Viewport3D,
        Panel::View2D,
        Panel::TextureSets,
        Panel::Layers,
        Panel::Properties,
        Panel::Library,
        Panel::Display,
        Panel::History,
        Panel::Console,
    ];

    /// Translation key of the panel title.
    pub fn title_key(self) -> &'static str {
        match self {
            Panel::Viewport3D => "panel-viewport-3d",
            Panel::View2D => "panel-view-2d",
            Panel::TextureSets => "panel-texture-sets",
            Panel::Layers => "panel-layers",
            Panel::Properties => "panel-properties",
            Panel::Library => "panel-library",
            Panel::Display => "panel-display",
            Panel::History => "panel-history",
            Panel::Console => "panel-console",
        }
    }

    /// Phosphor icon of the panel.
    pub fn icon(self) -> &'static str {
        match self {
            Panel::Viewport3D => icons::CUBE,
            Panel::View2D => icons::GRID_FOUR,
            Panel::TextureSets => icons::SQUARES_FOUR,
            Panel::Layers => icons::STACK,
            Panel::Properties => icons::SLIDERS_HORIZONTAL,
            Panel::Library => icons::BOOKS,
            Panel::Display => icons::SUN,
            Panel::History => icons::CLOCK_COUNTER_CLOCKWISE,
            Panel::Console => icons::TERMINAL_WINDOW,
        }
    }
}

/// Default layout: viewports in the center, stack and properties on the right,
/// library and console at the bottom.
pub fn default_layout() -> DockState<Panel> {
    let mut dock = DockState::new(vec![Panel::Viewport3D, Panel::View2D]);
    let tree = dock.main_surface_mut();
    let [center, right] = tree.split_right(
        NodeIndex::root(),
        0.74,
        vec![Panel::Layers, Panel::TextureSets],
    );
    tree.split_below(
        right,
        0.45,
        vec![Panel::Properties, Panel::Display, Panel::History],
    );
    tree.split_below(center, 0.72, vec![Panel::Library, Panel::Console]);
    dock
}

/// Interface state that does not belong to the document.
pub struct UiState {
    /// Panel layout.
    pub dock: DockState<Panel>,
    /// Whether the About dialog is open.
    pub show_about: bool,
    /// Most verbose level shown in the Console.
    pub console_level: Level,
    /// The New Project dialog, when open.
    pub new_project: Option<NewProjectDialog>,
    /// Camera and display settings of the 3D viewport.
    pub view: ViewState,
    /// Texture Sets panel: current texture set, renaming.
    pub texture_sets: TextureSetsState,
    /// The 2D view.
    pub view_2d: View2dState,
    /// Layers panel: selection, renaming, current channel, closed folders.
    pub layers: LayersState,
    /// Name of the document whose unsaved changes the user must decide about.
    pub unsaved_prompt: Option<String>,
    /// Error shown in a dialog until dismissed.
    pub error_message: Option<String>,
    /// Project left by a crash, offered for recovery: name and minutes since autosave.
    pub recovery_prompt: Option<(String, u64)>,
}

/// Answer to the unsaved changes dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChoice {
    /// Save, then continue.
    Save,
    /// Continue without saving.
    Discard,
    /// Do nothing.
    Cancel,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            dock: default_layout(),
            show_about: false,
            console_level: Level::INFO,
            new_project: None,
            view: ViewState::default(),
            texture_sets: TextureSetsState::default(),
            view_2d: View2dState::default(),
            layers: LayersState::default(),
            unsaved_prompt: None,
            error_message: None,
            recovery_prompt: None,
        }
    }
}

/// Read-only data the interface displays this frame.
pub struct FrameInfo<'a> {
    /// Translations of the current language.
    pub i18n: &'a I18n,
    /// Action registry.
    pub actions: &'a ActionRegistry,
    /// State used by actions.
    pub action_context: ActionContext,
    /// Console log lines.
    pub logs: &'a LogBuffer,
    /// Description of the GPU adapter, for the status bar.
    pub gpu_name: &'a str,
    /// Backend of the GPU adapter, for the status bar.
    pub gpu_backend: &'a str,
    /// The open project, if any.
    pub project: Option<&'a Project>,
    /// Triangle of the project mesh under the cursor.
    pub hovered: Option<u32>,
    /// Environment of the document, or of the draft without a project.
    pub environment: EnvironmentView<'a>,
    /// Translation key of the file operation in progress, for the status bar.
    pub file_status: Option<&'static str>,
    /// History of the open document.
    pub history: Option<HistoryView>,
    /// Memory of the channel textures of the document, in bytes.
    pub gpu_memory: Option<u64>,
}

/// What the interface asks the application to do after a frame.
#[derive(Default)]
pub struct UiOutput {
    /// Actions triggered from menus.
    pub actions: Vec<ActionId>,
    /// A project created by the New Project dialog, to replace the document.
    pub created_project: Option<CreatedProject>,
    /// Commands to execute on the document, in order.
    pub commands: Vec<Box<dyn Command>>,
    /// Environment change (a command with a project, a direct change without).
    pub environment: Option<EnvironmentChange>,
    /// An HDRI the user picked, already decoded, that `environment` imports.
    pub picked_hdri: Option<PickedHdri>,
    /// A gesture ended: the next command starts a new history entry.
    pub seal: bool,
    /// Answer to the unsaved changes dialog.
    pub unsaved_choice: Option<UnsavedChoice>,
    /// Answer to the recovery dialog: `true` to recover, `false` to delete the copy.
    pub recovery_choice: Option<bool>,
    /// History state to go to: number of entries to apply.
    pub history_go_to: Option<usize>,
}

/// Applies the Patine theme and adds the icon font. Call once on the egui context.
pub fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        let accent = Color32::from_rgb(0xC0, 0x7A, 0x3A);
        style.visuals.selection.bg_fill = accent;
        style.visuals.hyperlink_color = accent;
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
    });
}

/// Draws the whole interface and returns what the application must do.
pub fn show(ui: &mut Ui, state: &mut UiState, info: &FrameInfo<'_>) -> UiOutput {
    let mut output = UiOutput::default();
    let mut triggered = Vec::new();

    egui::Panel::top("menu_bar").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| menu_bar(ui, info, &mut triggered));
    });

    egui::Panel::bottom("status_bar").show(ui, |ui| {
        let mut args = FluentArgs::new();
        args.set("name", info.gpu_name);
        args.set("backend", info.gpu_backend);
        ui.horizontal(|ui| {
            if let Some(key) = info.file_status {
                ui.spinner();
                ui.label(RichText::new(info.i18n.tr(key)).small());
                ui.separator();
            }
            ui.label(RichText::new(info.i18n.tr_args("status-gpu", &args)).small());
            if let Some(project) = info.project {
                ui.separator();
                let mut args = FluentArgs::new();
                args.set("name", project.mesh.name.clone());
                ui.label(RichText::new(info.i18n.tr_args("status-project", &args)).small());
                if let Some(triangle) = info.hovered {
                    let mesh = &project.mesh;
                    if let (Some(polygon), Some(object)) = (
                        mesh.triangle_polygon.get(triangle as usize),
                        mesh.triangle_object
                            .get(triangle as usize)
                            .and_then(|o| mesh.objects.get(*o as usize)),
                    ) {
                        ui.separator();
                        let mut args = FluentArgs::new();
                        args.set("triangle", triangle);
                        args.set("polygon", *polygon);
                        args.set("object", object.clone());
                        ui.label(RichText::new(info.i18n.tr_args("status-hover", &args)).small());
                    }
                }
            }
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let style = Style::from_egui(ui.style());
        let mut viewer = PanelViewer {
            info,
            console_level: &mut state.console_level,
            view: &mut state.view,
            texture_sets: &mut state.texture_sets,
            view_2d: &mut state.view_2d,
            layers: &mut state.layers,
            output: &mut output,
        };
        DockArea::new(&mut state.dock)
            .style(style)
            // Panels are permanent: nothing can be closed, only moved or undocked.
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_add_buttons(false)
            .show_inside(ui, &mut viewer);
    });

    if state.show_about && about_dialog(ui.ctx(), info.i18n) {
        state.show_about = false;
    }

    if let Some(dialog) = &mut state.new_project {
        match dialog.show(ui.ctx(), info.i18n) {
            DialogOutcome::Open => {}
            DialogOutcome::Cancelled => state.new_project = None,
            DialogOutcome::Created(created) => {
                state.new_project = None;
                output.created_project = Some(*created);
            }
        }
    }

    if let Some(name) = &state.unsaved_prompt {
        output.unsaved_choice = unsaved_dialog(ui.ctx(), info.i18n, name);
    }
    if let Some((name, minutes)) = &state.recovery_prompt {
        output.recovery_choice = recovery_dialog(ui.ctx(), info.i18n, name, *minutes);
    }
    if let Some(message) = &state.error_message
        && message_dialog(ui.ctx(), info.i18n, message)
    {
        state.error_message = None;
    }

    output.actions = triggered;
    output
}

/// Asks what to do with unsaved changes; `None` while the user has not answered.
fn unsaved_dialog(ctx: &egui::Context, i18n: &I18n, name: &str) -> Option<UnsavedChoice> {
    let mut choice = None;
    let response = egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
        ui.heading(i18n.tr("unsaved-title"));
        let mut args = FluentArgs::new();
        args.set("name", name.to_owned());
        ui.label(i18n.tr_args("unsaved-message", &args));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(i18n.tr("unsaved-save")).clicked() {
                choice = Some(UnsavedChoice::Save);
            }
            if ui.button(i18n.tr("unsaved-discard")).clicked() {
                choice = Some(UnsavedChoice::Discard);
            }
            if ui.button(i18n.tr("unsaved-cancel")).clicked() {
                choice = Some(UnsavedChoice::Cancel);
            }
        });
    });
    if choice.is_none() && response.should_close() {
        choice = Some(UnsavedChoice::Cancel);
    }
    choice
}

/// Offers to recover a project left by a crash; `None` while the user has not answered.
fn recovery_dialog(ctx: &egui::Context, i18n: &I18n, name: &str, minutes: u64) -> Option<bool> {
    let mut choice = None;
    egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.heading(i18n.tr("recovery-title"));
        let mut args = FluentArgs::new();
        args.set("name", name.to_owned());
        args.set("minutes", minutes);
        ui.label(i18n.tr_args("recovery-message", &args));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(i18n.tr("recovery-recover")).clicked() {
                choice = Some(true);
            }
            if ui.button(i18n.tr("recovery-discard")).clicked() {
                choice = Some(false);
            }
        });
    });
    choice
}

/// Shows an error; returns `true` when dismissed.
fn message_dialog(ctx: &egui::Context, i18n: &I18n, message: &str) -> bool {
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("error_message")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.heading(i18n.tr("error-title"));
        ui.label(message);
        ui.add_space(8.0);
        if ui.button(i18n.tr("error-ok")).clicked() {
            close = true;
        }
    });
    close || response.should_close()
}

fn menu_bar(ui: &mut Ui, info: &FrameInfo<'_>, triggered: &mut Vec<ActionId>) {
    let i18n = info.i18n;
    ui.menu_button(i18n.tr("menu-file"), |ui| {
        action_item(ui, info, ActionId::NewProject, triggered);
        action_item(ui, info, ActionId::Open, triggered);
        ui.separator();
        action_item(ui, info, ActionId::Save, triggered);
        action_item(ui, info, ActionId::SaveAs, triggered);
        ui.separator();
        action_item(ui, info, ActionId::Quit, triggered);
    });
    ui.menu_button(i18n.tr("menu-edit"), |ui| {
        action_item(ui, info, ActionId::Undo, triggered);
        action_item(ui, info, ActionId::Redo, triggered);
        ui.separator();
        action_item(ui, info, ActionId::DeleteLayer, triggered);
        action_item(ui, info, ActionId::GroupLayer, triggered);
    });
    ui.menu_button(i18n.tr("menu-view"), |ui| {
        action_item(ui, info, ActionId::FrameMesh, triggered);
        action_item(ui, info, ActionId::ToggleProjection, triggered);
        action_item(ui, info, ActionId::ToggleWireframe, triggered);
        action_item(ui, info, ActionId::ToggleShadows, triggered);
        ui.separator();
        action_item(ui, info, ActionId::ToggleChannelView, triggered);
        action_item(ui, info, ActionId::NextChannel, triggered);
        action_item(ui, info, ActionId::ToggleMaskView, triggered);
        ui.separator();
        action_item(ui, info, ActionId::ResetLayout, triggered);
        action_item(ui, info, ActionId::ToggleProfiler, triggered);
    });
    ui.menu_button(i18n.tr("menu-language"), |ui| {
        action_item(ui, info, ActionId::LanguageEnglish, triggered);
        action_item(ui, info, ActionId::LanguageFrench, triggered);
    });
    ui.menu_button(i18n.tr("menu-help"), |ui| {
        action_item(ui, info, ActionId::About, triggered);
    });
}

/// Menu entry built from the action registry.
fn action_item(ui: &mut Ui, info: &FrameInfo<'_>, id: ActionId, triggered: &mut Vec<ActionId>) {
    let action = info.actions.get(id);
    let checked = action
        .checked
        .is_some_and(|checked| checked(&info.action_context));
    let mut button = egui::Button::new(info.i18n.tr(action.label_key)).selected(checked);
    if let Some(shortcut) = &action.shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
    }
    let enabled = (action.enabled)(&info.action_context);
    if ui.add_enabled(enabled, button).clicked() {
        triggered.push(id);
        ui.close();
    }
}

/// Returns `true` when the dialog must close.
fn about_dialog(ctx: &egui::Context, i18n: &I18n) -> bool {
    let mut close = false;
    let response = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
        ui.heading(i18n.tr("app-title"));
        ui.label(i18n.tr("about-description"));
        let mut args = FluentArgs::new();
        args.set("version", env!("CARGO_PKG_VERSION"));
        ui.label(i18n.tr_args("about-version", &args));
        ui.add_space(8.0);
        if ui.button(i18n.tr("about-close")).clicked() {
            close = true;
        }
    });
    close || response.should_close()
}

struct PanelViewer<'a, 'b> {
    info: &'a FrameInfo<'b>,
    console_level: &'a mut Level,
    view: &'a mut ViewState,
    texture_sets: &'a mut TextureSetsState,
    view_2d: &'a mut View2dState,
    layers: &'a mut LayersState,
    output: &'a mut UiOutput,
}

impl PanelViewer<'_, '_> {
    /// Channels of the current texture set.
    fn current_channels(&self) -> Vec<Channel> {
        self.info
            .project
            .and_then(|project| project.texture_set(self.texture_sets.selected))
            .map(|set| set.channels.iter().map(|def| def.channel).collect())
            .unwrap_or_default()
    }
}

impl TabViewer for PanelViewer<'_, '_> {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        format!("{} {}", tab.icon(), self.info.i18n.tr(tab.title_key())).into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Panel) {
        match tab {
            Panel::Viewport3D => {
                let overlay = match self.view.settings.display {
                    DisplayMode::Lit => None,
                    DisplayMode::Mask => Some(crate::masks::mask_view_label(
                        self.info.i18n,
                        self.info.project,
                        self.layers.selected,
                    )),
                    display => Some(display_panel::display_mode_label(
                        self.info.i18n,
                        display,
                        self.view.inspect_channel,
                    )),
                };
                let viewport = viewport_panel::show(ui, self.view, overlay.as_deref());
                if let Some(rotation) = viewport.environment_rotation {
                    self.output.environment = Some(EnvironmentChange::Rotation(rotation));
                }
                self.output.seal |= viewport.seal;
            }
            Panel::Console => console(ui, self.info, self.console_level),
            Panel::History => {
                let target = history_panel::show(ui, self.info.i18n, self.info.history.as_ref());
                if target.is_some() {
                    self.output.history_go_to = target;
                }
            }
            Panel::TextureSets => {
                let panel = texture_sets_panel::show(
                    ui,
                    self.info.i18n,
                    self.info.project,
                    self.texture_sets,
                    self.info.gpu_memory,
                );
                self.output.commands.extend(panel.command);
                self.output.seal |= panel.seal;
            }
            Panel::Layers => {
                let panel = layers_panel::show(
                    ui,
                    self.info.i18n,
                    self.info.project,
                    self.texture_sets.selected,
                    self.layers,
                );
                self.output.commands.extend(panel.commands);
                self.output.seal |= panel.seal;
            }
            Panel::Properties => {
                let panel = properties_panel::show(
                    ui,
                    self.info.i18n,
                    self.info.project,
                    self.texture_sets.selected,
                    self.layers.selected,
                    self.layers.part,
                );
                self.output.commands.extend(panel.commands);
                self.output.seal |= panel.seal;
            }
            Panel::View2D => {
                view_2d::show(
                    ui,
                    self.view_2d,
                    self.info.i18n,
                    self.info.project.is_some(),
                );
            }
            Panel::Display => {
                let channels = self.current_channels();
                let display = display_panel::show(
                    ui,
                    self.view,
                    self.info.i18n,
                    &self.info.environment,
                    &channels,
                );
                if display.change.is_some() {
                    self.output.environment = display.change;
                }
                if display.picked.is_some() {
                    self.output.picked_hdri = display.picked;
                }
                self.output.seal |= display.seal;
            }
            _ => {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(self.info.i18n.tr("panel-empty")).weak());
                });
            }
        }
    }

    fn is_closeable(&self, _tab: &Panel) -> bool {
        false
    }

    fn scroll_bars(&self, tab: &Panel) -> [bool; 2] {
        // The viewport fills its panel exactly.
        if *tab == Panel::Viewport3D {
            [false, false]
        } else {
            [true, true]
        }
    }
}

const LEVELS: [(Level, &str); 5] = [
    (Level::ERROR, "console-level-error"),
    (Level::WARN, "console-level-warn"),
    (Level::INFO, "console-level-info"),
    (Level::DEBUG, "console-level-debug"),
    (Level::TRACE, "console-level-trace"),
];

fn level_color(level: Level, ui: &Ui) -> Color32 {
    match level {
        Level::ERROR => ui.visuals().error_fg_color,
        Level::WARN => ui.visuals().warn_fg_color,
        Level::INFO => ui.visuals().text_color(),
        _ => ui.visuals().weak_text_color(),
    }
}

fn level_key(level: Level) -> &'static str {
    LEVELS
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, key)| *key)
        .expect("LEVELS lists every tracing level")
}

fn console(ui: &mut Ui, info: &FrameInfo<'_>, min_level: &mut Level) {
    let i18n = info.i18n;
    ui.horizontal(|ui| {
        ui.label(i18n.tr("console-level"));
        egui::ComboBox::from_id_salt("console_level")
            .selected_text(i18n.tr(level_key(*min_level)))
            .show_ui(ui, |ui| {
                for (level, key) in LEVELS {
                    ui.selectable_value(min_level, level, i18n.tr(key));
                }
            });
        if ui.button(i18n.tr("console-clear")).clicked() {
            info.logs.clear();
        }
        if info.action_context.profiler_enabled {
            let mut args = FluentArgs::new();
            args.set("port", puffin_http::DEFAULT_PORT);
            ui.label(RichText::new(i18n.tr_args("console-profiler-on", &args)).small());
        }
    });
    ui.separator();

    let lines = info.logs.lock();
    let visible: Vec<_> = lines
        .iter()
        .filter(|line| line.level <= *min_level)
        .collect();
    if visible.is_empty() {
        ui.label(RichText::new(i18n.tr("console-empty")).weak());
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show_rows(ui, row_height, visible.len(), |ui, rows| {
            for line in &visible[rows] {
                let text = format!("{:5} {}: {}", line.level, line.target, line.message);
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .color(level_color(line.level, ui)),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::i18n::Language;

    #[test]
    fn default_layout_has_every_panel_once() {
        let dock = default_layout();
        let tabs: Vec<Panel> = dock.iter_all_tabs().map(|(_, tab)| *tab).collect();
        let unique: HashSet<_> = tabs.iter().copied().collect();
        assert_eq!(tabs.len(), Panel::ALL.len());
        assert_eq!(unique, Panel::ALL.into_iter().collect());
    }

    #[test]
    fn panel_titles_and_levels_are_translated() {
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for panel in Panel::ALL {
                assert!(i18n.has(panel.title_key()), "{language:?} {panel:?}");
            }
            for (_, key) in LEVELS {
                assert!(i18n.has(key), "{language:?} {key}");
            }
        }
    }

    /// Runs a few headless frames of the full interface in both languages.
    #[test]
    fn interface_renders_headless() {
        let logs = LogBuffer::default();
        logs.push(crate::logging::LogLine {
            level: Level::WARN,
            target: "test".into(),
            message: "hello".into(),
        });
        let project = Project::new(
            patine_mesh::primitives::cube(),
            "cube.glb".into(),
            Default::default(),
        );
        for language in Language::ALL {
            let ctx = egui::Context::default();
            setup_style(&ctx);
            let i18n = I18n::new(language);
            let actions = ActionRegistry::default();
            let mut state = UiState {
                show_about: true,
                new_project: Some(NewProjectDialog::default()),
                unsaved_prompt: Some("helmet".into()),
                error_message: Some("cannot open".into()),
                recovery_prompt: Some(("lantern".into(), 3)),
                ..UiState::default()
            };
            let info = FrameInfo {
                i18n: &i18n,
                actions: &actions,
                action_context: ActionContext {
                    profiler_enabled: true,
                    language,
                    has_project: true,
                    ..ActionContext::default()
                },
                logs: &logs,
                gpu_name: "test adapter",
                gpu_backend: "none",
                project: Some(&project),
                hovered: Some(3),
                environment: EnvironmentView {
                    settings: &project.environment,
                    name: "Studio".into(),
                    loading: true,
                    error: Some("test error"),
                },
                file_status: Some("file-busy-saving"),
                history: Some(HistoryView {
                    labels: vec![patine_core::command::Label::new(
                        "command-environment-exposure",
                    )],
                    applied: 0,
                    truncated: false,
                }),
                gpu_memory: Some(512 << 20),
            };
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600.0, 900.0),
                    )),
                    ..Default::default()
                };
                ctx.run_ui(input, |ui| {
                    let output = show(ui, &mut state, &info);
                    assert!(output.actions.is_empty() && output.created_project.is_none());
                })
                .drop_without_applying_deltas();
            }
        }
    }
}
