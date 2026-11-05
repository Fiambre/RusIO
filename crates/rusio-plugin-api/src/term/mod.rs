//! Shared terminal widget: a vt100 emulator drawn with egui, plus a ready-made
//! [`Session`](crate::Session) that plugins wrap around their byte-stream backend.
pub mod emulator;
pub mod session;

pub use emulator::{SharedTerminal, TerminalEmulator};
pub use session::{TerminalBackend, TerminalSession};
