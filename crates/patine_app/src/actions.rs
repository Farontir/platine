//! Action registry: every user command (menus, shortcuts, later the command palette) is
//! declared here once, with its label, shortcut and enabling condition.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::Language;

/// Identifier of an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionId {
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
}

/// Application state that actions read to decide whether they are enabled or checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionContext {
    /// Whether the profiler is recording.
    pub profiler_enabled: bool,
    /// Current interface language.
    pub language: Language,
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
                    id: ActionId::Quit,
                    name: "app.quit",
                    label_key: "action-app-quit",
                    shortcut: Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Q)),
                    enabled: always,
                    checked: None,
                },
                Action {
                    id: ActionId::ResetLayout,
                    name: "view.reset_layout",
                    label_key: "action-view-reset-layout",
                    shortcut: None,
                    enabled: always,
                    checked: None,
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
                },
                Action {
                    id: ActionId::LanguageEnglish,
                    name: "language.english",
                    label_key: "action-language-english",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.language == Language::English),
                },
                Action {
                    id: ActionId::LanguageFrench,
                    name: "language.french",
                    label_key: "action-language-french",
                    shortcut: None,
                    enabled: always,
                    checked: Some(|ctx| ctx.language == Language::French),
                },
                Action {
                    id: ActionId::About,
                    name: "help.about",
                    label_key: "action-help-about",
                    shortcut: None,
                    enabled: always,
                    checked: None,
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
    pub fn triggered_by_shortcuts(
        &self,
        input: &mut egui::InputState,
        context: &ActionContext,
    ) -> Vec<ActionId> {
        self.actions
            .iter()
            .filter(|action| {
                action
                    .shortcut
                    .is_some_and(|shortcut| input.consume_shortcut(&shortcut))
                    && (action.enabled)(context)
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
            ActionId::Quit,
            ActionId::ResetLayout,
            ActionId::ToggleProfiler,
            ActionId::LanguageEnglish,
            ActionId::LanguageFrench,
            ActionId::About,
        ] {
            assert_eq!(registry.get(id).id, id);
        }
    }

    #[test]
    fn shortcut_triggers_its_action() {
        let registry = ActionRegistry::default();
        let context = ActionContext {
            profiler_enabled: false,
            language: Language::English,
        };
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: Key::Q,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        let mut triggered = Vec::new();
        ctx.run_ui(input, |ui| {
            triggered = ui
                .ctx()
                .input_mut(|input| registry.triggered_by_shortcuts(input, &context));
        })
        .drop_without_applying_deltas();
        assert_eq!(triggered, vec![ActionId::Quit]);
    }
}
