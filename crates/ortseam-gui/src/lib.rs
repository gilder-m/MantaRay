//! ortseam's desktop application, as a library.
//!
//! The binary in `main.rs` is a thin wrapper: everything it does lives here, so
//! the whole application - opening files, acquiring, marking regions, undoing,
//! running jobs - can be driven from tests without a window on screen.
//!
//! [`app::App::headless`] builds the application without touching egui;
//! [`app::App::apply_action`] performs exactly what a menu item, toolbar button
//! or job command performs.

#![forbid(unsafe_code)]

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
