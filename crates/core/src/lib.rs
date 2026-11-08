//! UI-independent typing session primitives.

#![forbid(unsafe_code)]

mod localization;
mod session;

pub use localization::{Catalog, FormatArgs, Locale, LocalizationDiagnostic, LocalizationError};
pub use session::{Keystroke, Session, SessionSnapshot, SessionState, SessionStats, StopReason};
