//! Undo history with merging of continuous gestures and a memory budget.

use std::collections::VecDeque;

use crate::command::{Command, CommandError, Label};
use crate::project::Project;

/// Default memory budget of the history: 2 GiB.
pub const DEFAULT_BUDGET: usize = 2 << 30;

/// Fixed cost counted for each entry, on top of the command's own heap size.
const ENTRY_OVERHEAD: usize = 64;

#[derive(Debug)]
struct Entry {
    /// Identifier of the document state right after this entry.
    state: u64,
    command: Box<dyn Command>,
    size: usize,
}

impl Entry {
    fn new(state: u64, command: Box<dyn Command>) -> Self {
        let size = command.heap_size() + ENTRY_OVERHEAD;
        Self {
            state,
            command,
            size,
        }
    }
}

/// The commands applied to a document, for undo and redo.
///
/// Each document state reachable through the history has a number; the document is
/// modified when the current state is not the one last saved.
#[derive(Debug)]
pub struct History {
    /// Applied commands, oldest first.
    undo: VecDeque<Entry>,
    /// Undone commands, most recently undone last.
    redo: Vec<Entry>,
    /// State before the oldest entry of `undo`.
    base_state: u64,
    /// Next state number.
    next_state: u64,
    /// State written by the last save, if still known.
    saved_state: Option<u64>,
    /// When set, the next command starts a new entry instead of merging.
    sealed: bool,
    /// Memory budget in bytes.
    budget: usize,
    /// Memory counted for all entries.
    used: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }
}

impl History {
    /// An empty history with a memory budget in bytes. The initial document counts as
    /// saved.
    pub fn with_budget(budget: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            base_state: 0,
            next_state: 1,
            saved_state: Some(0),
            sealed: true,
            budget,
            used: 0,
        }
    }

    /// Applies `command` to `project` and records it. On error the document and the
    /// history are unchanged.
    pub fn push(
        &mut self,
        project: &mut Project,
        mut command: Box<dyn Command>,
    ) -> Result<(), CommandError> {
        command.apply(project)?;
        self.drop_redo();
        let state = self.new_state();
        let merged = match self.undo.back_mut() {
            Some(top) if !self.sealed => top.command.merge(command.as_ref()),
            _ => false,
        };
        if merged {
            let top = self.undo.back_mut().expect("merged into the top entry");
            // The merged entry leads to a new state: a save made mid-gesture no longer
            // matches it.
            top.state = state;
            self.used -= top.size;
            top.size = top.command.heap_size() + ENTRY_OVERHEAD;
            self.used += top.size;
        } else {
            let entry = Entry::new(state, command);
            self.used += entry.size;
            self.undo.push_back(entry);
        }
        self.sealed = false;
        self.evict();
        Ok(())
    }

    /// Ends the current gesture: the next command makes a new entry.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// Undoes the last command. Returns `Ok(false)` when there is nothing to undo.
    ///
    /// If the command fails to revert, the document is unchanged but the history can no
    /// longer be trusted: it is cleared, and the error returned.
    pub fn undo(&mut self, project: &mut Project) -> Result<bool, CommandError> {
        self.sealed = true;
        let Some(mut entry) = self.undo.pop_back() else {
            return Ok(false);
        };
        if let Err(error) = entry.command.revert(project) {
            // The document is still in the state after the entry.
            self.clear(entry.state);
            return Err(error);
        }
        self.redo.push(entry);
        Ok(true)
    }

    /// Redoes the last undone command. Returns `Ok(false)` when there is nothing to redo.
    /// On failure the history is cleared, as for [`History::undo`].
    pub fn redo(&mut self, project: &mut Project) -> Result<bool, CommandError> {
        self.sealed = true;
        let Some(mut entry) = self.redo.pop() else {
            return Ok(false);
        };
        if let Err(error) = entry.command.apply(project) {
            self.clear(self.current_state());
            return Err(error);
        }
        self.undo.push_back(entry);
        Ok(true)
    }

    /// Undoes or redoes until `applied` entries are applied (0 = state before the oldest
    /// entry), as when clicking a line of the history panel.
    pub fn go_to(&mut self, project: &mut Project, applied: usize) -> Result<(), CommandError> {
        while self.undo.len() > applied {
            self.undo(project)?;
        }
        while self.undo.len() < applied && self.redo(project)? {}
        Ok(())
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Number of applied entries.
    pub fn applied(&self) -> usize {
        self.undo.len()
    }

    /// Labels of every entry, oldest first: the applied ones, then the undone ones.
    pub fn labels(&self) -> Vec<Label> {
        self.undo
            .iter()
            .chain(self.redo.iter().rev())
            .map(|entry| entry.command.label())
            .collect()
    }

    /// Whether the oldest states were evicted: the first entry no longer starts from the
    /// document as opened.
    pub fn truncated(&self) -> bool {
        self.base_state != 0
    }

    /// Records that the current document was saved.
    pub fn mark_saved(&mut self) {
        self.saved_state = Some(self.current_state());
        self.sealed = true;
    }

    /// Starts a save in the background: ends the current gesture, so that the saved state
    /// stays reachable, and returns its number for [`History::mark_saved_as`].
    pub fn begin_save(&mut self) -> u64 {
        self.sealed = true;
        self.current_state()
    }

    /// Records that state `state` was saved, when the save finishes after more edits.
    pub fn mark_saved_as(&mut self, state: u64) {
        self.saved_state = Some(state);
    }

    /// Records that the document matches no saved file (a recovered autosave).
    pub fn mark_unsaved(&mut self) {
        self.saved_state = None;
    }

    /// Whether the document differs from the last save.
    pub fn is_dirty(&self) -> bool {
        self.saved_state != Some(self.current_state())
    }

    /// Memory counted for the entries, in bytes.
    pub fn memory(&self) -> usize {
        self.used
    }

    /// Number identifying the current document state.
    pub fn current_state(&self) -> u64 {
        self.undo
            .back()
            .map_or(self.base_state, |entry| entry.state)
    }

    fn new_state(&mut self) -> u64 {
        let state = self.next_state;
        self.next_state += 1;
        state
    }

    fn drop_redo(&mut self) {
        for entry in self.redo.drain(..) {
            self.used -= entry.size;
        }
    }

    /// Forgets the oldest entries while over budget, always keeping the last one.
    fn evict(&mut self) {
        while self.used > self.budget && self.undo.len() > 1 {
            let entry = self.undo.pop_front().expect("more than one entry");
            self.used -= entry.size;
            self.base_state = entry.state;
        }
    }

    /// Forgets every entry; the document is in state `state`.
    fn clear(&mut self, state: u64) {
        self.base_state = state;
        self.undo.clear();
        self.redo.clear();
        self.used = 0;
    }
}
