//! History panel: the commands of the document, oldest first. The current state is
//! highlighted, undone commands are dimmed, and a click goes back or forward to a state.

use egui::{RichText, Ui};
use patine_core::command::Label;

use crate::document::translate;
use crate::i18n::I18n;

/// What the panel shows of the history.
pub struct HistoryView {
    /// Labels of every entry, oldest first.
    pub labels: Vec<Label>,
    /// Number of applied entries.
    pub applied: usize,
    /// Whether older entries were dropped by the memory budget.
    pub truncated: bool,
}

/// How a row is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// Applied, before the current state.
    Done,
    /// The current state.
    Current,
    /// Undone: a click redoes up to it.
    Undone,
}

/// State of row `row` (0 = the document as opened) when `applied` entries are applied.
pub fn row_state(row: usize, applied: usize) -> RowState {
    match row.cmp(&applied) {
        std::cmp::Ordering::Less => RowState::Done,
        std::cmp::Ordering::Equal => RowState::Current,
        std::cmp::Ordering::Greater => RowState::Undone,
    }
}

/// Draws the panel; returns the number of entries to apply when the user clicks a row.
pub fn show(ui: &mut Ui, i18n: &I18n, history: Option<&HistoryView>) -> Option<usize> {
    let Some(history) = history else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(i18n.tr("panel-empty")).weak());
        });
        return None;
    };
    let mut target = None;
    let rows = history.labels.len() + 1;
    let row_height = ui.text_style_height(&egui::TextStyle::Button) + 4.0;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show_rows(ui, row_height, rows, |ui, range| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for row in range {
                    let text = if row == 0 {
                        i18n.tr(if history.truncated {
                            "history-truncated"
                        } else {
                            "history-initial"
                        })
                        .into_owned()
                    } else {
                        translate(i18n, &history.labels[row - 1])
                    };
                    let state = row_state(row, history.applied);
                    let text = match state {
                        RowState::Undone => RichText::new(text).weak(),
                        _ => RichText::new(text),
                    };
                    let button = egui::Button::selectable(state == RowState::Current, text)
                        .frame_when_inactive(false)
                        .min_size(egui::vec2(0.0, row_height - 4.0));
                    if ui.add(button).clicked() && state != RowState::Current {
                        target = Some(row);
                    }
                }
            });
        });
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_before_the_current_state_are_done() {
        assert_eq!(row_state(0, 0), RowState::Current);
        assert_eq!(row_state(0, 2), RowState::Done);
        assert_eq!(row_state(2, 2), RowState::Current);
        assert_eq!(row_state(3, 2), RowState::Undone);
    }
}
