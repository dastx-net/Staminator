//! UI-independent typing session primitives.

#![forbid(unsafe_code)]

mod session;

pub use session::{Keystroke, Session, SessionSnapshot, SessionState, SessionStats};
