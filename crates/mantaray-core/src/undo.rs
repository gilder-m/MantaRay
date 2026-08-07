//! Undo and redo for destructive edits.
//!
//! MAESTRO has no undo: a mistaken Clear, Smooth or Strip loses the data. Every
//! command in mantaray that changes or discards spectrum data snapshots it first,
//! so it can be taken back.
//!
//! The stack stores whole snapshots rather than diffs. A spectrum is a few tens
//! of kilobytes, so a depth of [`DEFAULT_CAPACITY`] costs little and keeps the
//! implementation obviously correct.

use serde::{Deserialize, Serialize};

/// How many steps are remembered by default.
pub const DEFAULT_CAPACITY: usize = 32;

/// One remembered state and what the command that replaced it was called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot<T> {
    /// What the command was called, for the menu ("Undo Clear").
    pub label: String,
    /// The state before the command ran.
    pub state: T,
}

/// A bounded undo/redo history of whole states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoStack<T> {
    undo: Vec<Snapshot<T>>,
    redo: Vec<Snapshot<T>>,
    capacity: usize,
}

impl<T> Default for UndoStack<T> {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl<T> UndoStack<T> {
    /// An empty history that remembers `capacity` steps (at least one).
    pub fn new(capacity: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Records the state as it was before `label` ran.
    ///
    /// Doing something new makes a previously undone command unrepeatable, so
    /// the redo history is dropped.
    pub fn push(&mut self, label: impl Into<String>, state: T) {
        self.redo.clear();
        self.undo.push(Snapshot {
            label: label.into(),
            state,
        });
        while self.undo.len() > self.capacity {
            self.undo.remove(0);
        }
    }

    /// Takes back the last command.
    ///
    /// `current` is the state to return to with [`UndoStack::redo`]. Returns the
    /// label of the undone command and the state to restore.
    pub fn undo(&mut self, current: T) -> Option<(String, T)> {
        let snapshot = self.undo.pop()?;
        self.redo.push(Snapshot {
            label: snapshot.label.clone(),
            state: current,
        });
        Some((snapshot.label, snapshot.state))
    }

    /// Redoes the last undone command, given the state to be able to undo again.
    pub fn redo(&mut self, current: T) -> Option<(String, T)> {
        let snapshot = self.redo.pop()?;
        self.undo.push(Snapshot {
            label: snapshot.label.clone(),
            state: current,
        });
        Some((snapshot.label, snapshot.state))
    }

    /// Whether anything can be undone.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether anything can be redone.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Name of the command that would be undone.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|snapshot| snapshot.label.as_str())
    }

    /// Name of the command that would be redone.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|snapshot| snapshot.label.as_str())
    }

    /// Steps that can be undone.
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// Forgets the whole history.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// How many steps are remembered.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
