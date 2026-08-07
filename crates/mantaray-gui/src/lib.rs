//! mantaray's desktop application, as a library.
//!
//! The binary in `main.rs` is a thin wrapper: everything it does lives here, so
//! the whole application - opening files, acquiring, marking regions, undoing,
//! running jobs - can be driven from tests without a window on screen.
//!
//! [`app::App::headless`] builds the application without touching egui;
//! [`app::App::apply_action`] performs exactly what a menu item, toolbar button
//! or job command performs.

#![forbid(unsafe_code)]

/// What this program is called, wherever a person reads it.
///
/// Kept apart from [`APPLICATION_ID`] because the two answer to different
/// things: this one to the people using it, that one to the operating system
/// and to files written by earlier versions. Changing this is a matter of
/// taste; changing that one moves everybody's settings.
pub const DISPLAY_NAME: &str = "MantaRay";

/// What the operating system knows this program as.
///
/// The directory settings and the crash snapshot live in, and the key they are
/// stored under. Lower case and without spaces, because it becomes a path.
pub const APPLICATION_ID: &str = "mantaray";

pub mod app;
pub mod assoc;
pub mod crash;
pub mod dialogs;
pub mod jobs;
pub mod session;
pub mod snapshot;
pub mod stability;
pub mod theme;
pub mod view;
pub mod viewmodel;
