//! Undo and redo of destructive edits.

use mantaray_core::{Roi, Spectrum, UndoStack, smooth};

#[test]
fn a_new_history_has_nothing_to_undo() {
    let stack: UndoStack<u32> = UndoStack::default();
    assert!(!stack.can_undo());
    assert!(!stack.can_redo());
    assert_eq!(stack.undo_label(), None);
    assert_eq!(stack.depth(), 0);
}

#[test]
fn undo_returns_the_state_from_before_the_command() {
    let mut stack: UndoStack<u32> = UndoStack::default();
    stack.push("Clear", 10);
    assert!(stack.can_undo());
    assert_eq!(stack.undo_label(), Some("Clear"));

    let (label, restored) = stack.undo(0).expect("something to undo");
    assert_eq!(label, "Clear");
    assert_eq!(restored, 10);
    assert!(!stack.can_undo());
    assert!(stack.can_redo());
    assert_eq!(stack.redo_label(), Some("Clear"));
}

#[test]
fn redo_puts_the_command_back() {
    let mut stack: UndoStack<u32> = UndoStack::default();
    stack.push("Smooth", 1);
    let (_, restored) = stack.undo(2).unwrap();
    assert_eq!(restored, 1);
    let (label, redone) = stack.redo(restored).expect("something to redo");
    assert_eq!(label, "Smooth");
    assert_eq!(redone, 2, "redo returns the state the undo replaced");
    assert!(stack.can_undo());
    assert!(!stack.can_redo());
}

#[test]
fn several_steps_unwind_in_order() {
    let mut stack: UndoStack<&str> = UndoStack::default();
    stack.push("first", "a");
    stack.push("second", "b");
    stack.push("third", "c");
    assert_eq!(stack.depth(), 3);
    assert_eq!(stack.undo("d").unwrap(), ("third".into(), "c"));
    assert_eq!(stack.undo("c").unwrap(), ("second".into(), "b"));
    assert_eq!(stack.undo("b").unwrap(), ("first".into(), "a"));
    assert!(stack.undo("a").is_none());
}

#[test]
fn doing_something_new_drops_the_redo_history() {
    let mut stack: UndoStack<u32> = UndoStack::default();
    stack.push("Clear", 1);
    stack.undo(2).unwrap();
    assert!(stack.can_redo());
    stack.push("Strip", 3);
    assert!(!stack.can_redo(), "a new command cannot be redone past");
    assert_eq!(stack.undo_label(), Some("Strip"));
}

#[test]
fn the_history_is_bounded_and_forgets_the_oldest() {
    let mut stack: UndoStack<u32> = UndoStack::new(3);
    for value in 0..5u32 {
        stack.push(format!("step {value}"), value);
    }
    assert_eq!(stack.depth(), 3);
    assert_eq!(stack.capacity(), 3);
    // Only the last three survive: 2, 3 and 4.
    assert_eq!(stack.undo(9).unwrap().1, 4);
    assert_eq!(stack.undo(4).unwrap().1, 3);
    assert_eq!(stack.undo(3).unwrap().1, 2);
    assert!(!stack.can_undo());
}

#[test]
fn a_capacity_of_zero_still_remembers_one_step() {
    let mut stack: UndoStack<u32> = UndoStack::new(0);
    stack.push("Clear", 7);
    assert_eq!(stack.undo(0).unwrap().1, 7);
}

#[test]
fn clearing_forgets_everything() {
    let mut stack: UndoStack<u32> = UndoStack::default();
    stack.push("Clear", 1);
    stack.undo(2).unwrap();
    stack.clear();
    assert!(!stack.can_undo());
    assert!(!stack.can_redo());
}

#[test]
fn an_accidentally_cleared_spectrum_comes_back_intact() {
    let mut spectrum = Spectrum::from_counts((0..512u64).map(|c| c * 3 + 1).collect());
    spectrum.live_time = 600.0;
    spectrum.real_time = 610.0;
    spectrum.rois.mark(Roi::new(100, 130));
    let mut history: UndoStack<Spectrum> = UndoStack::default();

    // Acquire/Clear, the mistake this feature exists for.
    history.push("Clear", spectrum.clone());
    spectrum.clear();
    assert_eq!(spectrum.total_counts(), 0);
    assert_eq!(spectrum.live_time, 0.0);

    let (label, restored) = history.undo(spectrum.clone()).unwrap();
    assert_eq!(label, "Clear");
    spectrum = restored;
    assert_eq!(
        spectrum.total_counts(),
        (0..512u64).map(|c| c * 3 + 1).sum()
    );
    assert_eq!(spectrum.live_time, 600.0);
    assert_eq!(spectrum.real_time, 610.0);
    assert_eq!(spectrum.rois.len(), 1);
}

#[test]
fn smoothing_can_be_taken_back() {
    let original = Spectrum::from_counts((0..64u64).map(|c| (c * 7) % 23).collect());
    let mut spectrum = original.clone();
    let mut history: UndoStack<Spectrum> = UndoStack::default();

    history.push("Smooth", spectrum.clone());
    smooth(&mut spectrum);
    assert_ne!(spectrum.channels, original.channels);

    let (_, restored) = history.undo(spectrum.clone()).unwrap();
    assert_eq!(restored.channels, original.channels);

    // And redone, because a mistaken undo is also a mistake.
    let (_, redone) = history.redo(restored).unwrap();
    assert_eq!(redone.channels, spectrum.channels);
}
