//! Action registry: every user command (menus, shortcuts, later the command palette) is
//! declared here once, with its label, shortcut and enabling condition.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::Language;

/// Identifier of an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionId {
    /// Open the New Project dialog.
    NewProject,
    /// Quit the application.
    Quit,
    /// Restore the default panel layout.
    ResetLayout,
    /// Start or stop the puffin profiler.
    ToggleProfiler,
    /// Switch the interface to English.
    LanguageEnglish,
    /// Switch the interface to French.
    LanguageFrench,
    /// Show the About dialog.
    About,
    /// Frame the mesh in the viewport.
    FrameMesh,
    /// Switch between perspective and orthographic projection.
    ToggleProjection,
    /// Show or hide the wireframe overlay.
    ToggleWireframe,
    /// Enable or disable shadows.
    ToggleShadows,
    /// Undo the last command.
    Undo,
    /// Redo the last undone command.
    Redo,
}

/// Application state that actions read to decide whether they are enabled or checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionContext {
    /// Whether the profiler is recording.
    pub profiler_enabled: bool,
    /// Current interface language.
    pub language: Language,
    /// Whether a project is open.
    pub has_project: bool,
    /// Whether the viewport uses an orthographic projection.
    pub orthographic: bool,
    /// Whether the wireframe overlay is shown.
    pub wireframe: bool,
    /// Whether shadows are enabled.
    pub shadows: bool,
    /// Whether the history has a command to undo.
    pub can_undo: bool,
    /// Whether the history has a command to redo.
    pub can_redo: bool,
}

/// Declaration of an action.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    /// Identifier used by the code.
    pub id: ActionId,
    /// Stable textual identifier, for preferences and the command palette.
    pub name: &'static str,
    /// Translation key of the label.
    pub label_key: &'static str,
    /// Default keyboard shortcut.
    pub shortcut: Option<KeyboardShortcut>,
    /// Whether the action can run in the given state.
    pub enabled: fn(&ActionContext) -> bool,
    /// For toggles: whether the action is currently on.
    pub checked: Option<fn(&ActionContext) -> bool>,
    /// Whether a focused text field keeps the shortcut even with a modifier (Ctrl+Z
    /// undoes the typing, not the document).
    pub yields_to_text: bool,
}

fn always(_: &ActionContext) -> bool {
    true
}

/// The single registry of actions.
#[derive(Clone, Debug)]
pub struct ActionRegistry {
    actions: Vec<Action>,
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self {
            actions: vec![
                Action {
                    id: ActionId::NewProject,
                    name: "file.new_project",
                    label_key: "action-file-new-project",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::N)),
                    enabled: always,
                    checked: None,
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::Quit,
                    name: "app.quit",
                    label_key: "action-app-quit",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Q)),
                    enabled: always,
                    checked: None,
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::ResetLayout,
                    name: "view.reset_layout",
                    label_key: "action-view-reset-layout",
                    shortcut: None,
                    enabled: always,
                    checked: None,
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::ToggleProfiler,
                    name: "debug.toggle_profiler",
                    label_key: "action-debug-toggle-profiler",
                    shortcut: Some(KeyboardShortcut::new(
                        Modifiers::COMMAND.plus(Modifiers::SHIFT),
                        Key::P,
                    )),
                    enabled: always,
                    checked: Some(|ctx| ctx.profiler_enabled),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::LanguageEnglish,
                    name: "language.english",
                    label_key: "action-language-english",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.language == Language::English),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::LanguageFrench,
                    name: "language.french",
                    label_key: "action-language-french",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.language == Language::French),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::About,
                    name: "help.about",
                    label_key: "action-help-about",
                    shortcut: None,
                    enabled: always,
                    checked: None,
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::FrameMesh,
                    name: "view.frame",
                    label_key: "action-view-frame",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::NONE, Key::F)),
                    enabled: |ctx| ctx.has_project,
                    checked: None,
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::ToggleProjection,
                    name: "view.toggle_projection",
                    label_key: "action-view-orthographic",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.orthographic),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::ToggleWireframe,
                    name: "view.toggle_wireframe",
                    label_key: "action-view-wireframe",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.wireframe),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::ToggleShadows,
                    name: "view.toggle_shadows",
                    label_key: "action-view-shadows",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.shadows),
                    yields_to_text: false,
                },
                Action {
                    id: ActionId::Undo,
                    name: "edit.undo",
                    label_key: "action-edit-undo",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)),
                    enabled: |ctx| ctx.can_undo,
                    checked: None,
                    yields_to_text: true,
                },
                Action {
                    id: ActionId::Redo,
                    name: "edit.redo",
                    label_key: "action-edit-redo",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)),
                    enabled: |ctx| ctx.can_redo,
                    checked: None,
                    yields_to_text: true,
                },
            ],
        }
    }
}

impl ActionRegistry {
    /// Every action, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &Action> {
        self.actions.iter()
    }

    /// The action `id`.
    pub fn get(&self, id: ActionId) -> &Action {
        self.actions
            .iter()
            .find(|action| action.id == id)
            .expect("every ActionId is declared in the registry")
    }

    /// Consumes the shortcuts pressed this frame and returns the enabled actions they trigger.
    ///
    /// While a text field has the keyboard (`text_focus`), shortcuts without a modifier are
    /// left to the text field (typing an "f" must not frame the mesh), and so are the
    /// actions that yield to text (Ctrl+Z undoes the typing).
    pub fn triggered_by_shortcuts(
        &self,
        input: &mut egui::InputState,
        context: &ActionContext,
        text_focus: bool,
    ) -> Vec<ActionId> {
        self.actions
            .iter()
            .filter(|action| {
                action.shortcut.is_some_and(|shortcut| {
                    !(text_focus && (shortcut.modifiers.is_none() || action.yields_to_text))
                        && input.consume_shortcut(&shortcut)
                }) && (action.enabled)(context)
            })
            .map(|action| action.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::i18n::{I18n, Language};

    #[test]
    fn every_action_label_is_translated() {
        let registry = ActionRegistry::default();
        for language in Language::ALL {
            let i18n = I18n::new(language);
            for action in registry.iter() {
                assert!(
                    i18n.has(action.label_key),
                    "{language:?}: {}",
                    action.label_key
                );
            }
        }
    }

    #[test]
    fn ids_names_and_shortcuts_are_unique() {
        let registry = ActionRegistry::default();
        let ids: HashSet<_> = registry.iter().map(|a| a.id).collect();
        let names: HashSet<_> = registry.iter().map(|a| a.name).collect();
        let shortcuts: Vec<_> = registry.iter().filter_map(|a| a.shortcut).collect();
        let unique_shortcuts: HashSet<_> = shortcuts.iter().collect();
        assert_eq!(ids.len(), registry.iter().count());
        assert_eq!(names.len(), registry.iter().count());
        assert_eq!(unique_shortcuts.len(), shortcuts.len());
    }

    #[test]
    fn every_id_is_registered() {
        let registry = ActionRegistry::default();
        for id in [
            ActionId::NewProject,
            ActionId::Quit,
            ActionId::ResetLayout,
            ActionId::ToggleProfiler,
            ActionId::LanguageEnglish,
            ActionId::LanguageFrench,
            ActionId::About,
            ActionId::FrameMesh,
            ActionId::ToggleProjection,
            ActionId::ToggleWireframe,
            ActionId::ToggleShadows,
            ActionId::Undo,
            ActionId::Redo,
        ] {
            assert_eq!(registry.get(id).id, id);
        }
    }

    /// Presses `key` with `modifiers` and returns the triggered actions.
    fn press(
        key: Key,
        modifiers: Modifiers,
        context: &ActionContext,
        text_focus: bool,
    ) -> Vec<ActionId> {
        let registry = ActionRegistry::default();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        let mut triggered = Vec::new();
        ctx.run_ui(input, |ui| {
            triggered = ui
                .ctx()
                .input_mut(|input| registry.triggered_by_shortcuts(input, context, text_focus));
        })
        .drop_without_applying_deltas();
        triggered
    }

    #[test]
    fn shortcut_triggers_its_action() {
        let context = ActionContext::default();
        assert_eq!(
            press(Key::Q, Modifiers::COMMAND, &context, false),
            [ActionId::Quit]
        );
        // Modified shortcuts still work while typing.
        assert_eq!(
            press(Key::Q, Modifiers::COMMAND, &context, true),
            [ActionId::Quit]
        );
    }

    #[test]
    fn plain_keys_are_left_to_text_fields() {
        let context = ActionContext {
            has_project: true,
            ..ActionContext::default()
        };
        assert_eq!(
            press(Key::F, Modifiers::NONE, &context, false),
            [ActionId::FrameMesh]
        );
        assert!(press(Key::F, Modifiers::NONE, &context, true).is_empty());
    }

    #[test]
    fn disabled_actions_do_not_trigger() {
        // Framing needs a project.
        assert!(press(Key::F, Modifiers::NONE, &ActionContext::default(), false).is_empty());
        // Undo needs something to undo.
        assert!(press(Key::Z, Modifiers::COMMAND, &ActionContext::default(), false).is_empty());
    }

    #[test]
    fn undo_is_left_to_a_focused_text_field() {
        let context = ActionContext {
            can_undo: true,
            can_redo: true,
            ..ActionContext::default()
        };
        assert_eq!(
            press(Key::Z, Modifiers::COMMAND, &context, false),
            [ActionId::Undo]
        );
        assert_eq!(
            press(Key::Y, Modifiers::COMMAND, &context, false),
            [ActionId::Redo]
        );
        assert!(press(Key::Z, Modifiers::COMMAND, &context, true).is_empty());
        assert!(press(Key::Y, Modifiers::COMMAND, &context, true).is_empty());
    }
}
