//! Dead-simple debug logging. Set `ROM_DEBUG=1` to enable.
//!
//! Replaces `tracing` + `tracing-subscriber`. Writes to stderr; one
//! env-var check is memoized across the process.
use std::sync::OnceLock;

static ENABLED: OnceLock<bool> = OnceLock::new();

/// Returns true if `ROM_DEBUG` is set to a truthy value.
#[must_use]
pub fn enabled() -> bool {
  *ENABLED.get_or_init(|| {
    std::env::var_os("ROM_DEBUG")
      .is_some_and(|v| matches!(v.to_str(), Some("1" | "true" | "yes")))
  })
}

/// Log a formatted message to stderr when `ROM_DEBUG` is enabled.
#[macro_export]
macro_rules! debug {
  ($($arg:tt)*) => {{
    if $crate::debug_log::enabled() {
      eprintln!("[rom] {}", format_args!($($arg)*));
    }
  }};
}

/// Trace-level log. Same destination as `debug!`; kept distinct only so
/// existing call sites compile unchanged.
#[macro_export]
macro_rules! trace {
  ($($arg:tt)*) => {{ $crate::debug!($($arg)*); }};
}
