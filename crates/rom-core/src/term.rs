//! Minimal terminal control via raw ANSI escape sequences.
//!
//! Replaces the tiny slice of crossterm we actually used. Unix-style
//! terminals (including modern Windows Terminal) honor these; anything
//! older just prints the escapes, which is harmless.
use std::io::{self, Write};

/// ANSI colors. `Default` resets to the terminal's default foreground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
  Default,
  DarkRed,
  DarkGreen,
  DarkYellow,
  DarkBlue,
  DarkMagenta,
  DarkCyan,
  DarkGrey,
  Red,
  Green,
  Yellow,
  Blue,
  Magenta,
  Cyan,
  White,
}

impl Color {
  const fn code(self) -> &'static str {
    match self {
      Self::Default => "39",
      Self::DarkRed => "31",
      Self::DarkGreen => "32",
      Self::DarkYellow => "33",
      Self::DarkBlue => "34",
      Self::DarkMagenta => "35",
      Self::DarkCyan => "36",
      Self::DarkGrey => "90",
      Self::Red => "91",
      Self::Green => "92",
      Self::Yellow => "93",
      Self::Blue => "94",
      Self::Magenta => "95",
      Self::Cyan => "96",
      Self::White => "97",
    }
  }
}

/// Wrap `text` in an SGR foreground color with reset at the end.
#[must_use]
pub fn paint(text: &str, color: Color) -> String {
  format!("\x1b[{}m{}\x1b[0m", color.code(), text)
}

/// Wrap `text` in SGR foreground color + bold with full reset at the end.
#[must_use]
pub fn paint_bold(text: &str, color: Color) -> String {
  format!("\x1b[{};1m{}\x1b[0m", color.code(), text)
}

/// Clear `n` previous lines and position at column 1 for the next render.
///
/// Writes one escape sequence: move up N, move to column 1, clear from
/// cursor to end of screen. Avoids N round-trips.
pub fn clear_lines<W: Write>(w: &mut W, n: usize) -> io::Result<()> {
  if n == 0 {
    return Ok(());
  }
  // \x1b[{n}A  — cursor up n
  // \x1b[G     — cursor to column 1
  // \x1b[J     — clear from cursor to end of display
  write!(w, "\x1b[{n}A\x1b[G\x1b[J")
}

/// Begin a synchronized update (DEC private mode 2026). Modern terminals
/// buffer output until the end-sequence; older ones ignore it.
pub fn begin_sync<W: Write>(w: &mut W) -> io::Result<()> {
  w.write_all(b"\x1b[?2026h")
}

/// End a synchronized update — terminal flushes the buffered frame.
pub fn end_sync<W: Write>(w: &mut W) -> io::Result<()> {
  w.write_all(b"\x1b[?2026l")
}

/// Hide the cursor while rendering.
pub fn hide_cursor<W: Write>(w: &mut W) -> io::Result<()> {
  w.write_all(b"\x1b[?25l")
}

/// Show the cursor (always call before normal exit).
pub fn show_cursor<W: Write>(w: &mut W) -> io::Result<()> {
  w.write_all(b"\x1b[?25h")
}

/// RAII guard that hides the terminal cursor on construction and restores
/// it on drop — including panic-unwind and every early-return in the
/// caller. Pair with `install_signal_handlers()` to also cover
/// Ctrl+C / SIGTERM, which bypass Drop.
///
/// The guard writes to stderr via a direct fd so it works even if the
/// caller holds a `&mut Stderr` elsewhere.
pub struct CursorGuard {
  active: bool,
}

impl CursorGuard {
  /// Hide the cursor. Returns a guard that restores it on drop.
  #[must_use]
  pub fn hide() -> Self {
    let _ = std::io::stderr().write_all(b"\x1b[?25l");
    Self { active: true }
  }

  /// Dummy guard that does nothing on drop — for the silent path.
  #[must_use]
  pub const fn noop() -> Self {
    Self { active: false }
  }
}

impl Drop for CursorGuard {
  fn drop(&mut self) {
    if self.active {
      let _ = std::io::stderr().write_all(b"\x1b[?25h");
    }
  }
}

/// Query the controlling terminal's size. Returns `(cols, rows)` or
/// `(80, 24)` if the terminal can't be queried (pipe, cron, etc.).
#[must_use]
pub fn size() -> (usize, usize) {
  #[cfg(unix)]
  unsafe {
    let mut ws: libc::winsize = std::mem::zeroed();
    // Try stderr, then stdout — whichever is actually a tty.
    for fd in [libc::STDERR_FILENO, libc::STDOUT_FILENO] {
      if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0
        && ws.ws_col > 0
        && ws.ws_row > 0
      {
        return (ws.ws_col as usize, ws.ws_row as usize);
      }
    }
  }
  (80, 24)
}

/// Install async-signal-safe handlers for SIGINT/SIGTERM/SIGHUP/SIGQUIT
/// that restore the cursor before re-raising the signal with its default
/// disposition. Idempotent — first call wins.
///
/// Without this, hitting Ctrl+C mid-render would leave the user's terminal
/// with a hidden cursor until they typed `tput cnorm` or restarted.
pub fn install_signal_handlers() {
  #[cfg(unix)]
  {
    use std::sync::Once;
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
      for sig in
        [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT]
      {
        // SAFETY: setting a signal handler is safe; the handler itself is
        // async-signal-safe (only calls libc::write, libc::signal, libc::raise).
        unsafe {
          libc::signal(
            sig,
            signal_handler as *const () as libc::sighandler_t,
          );
        }
      }
    });
  }
}

#[cfg(unix)]
extern "C" fn signal_handler(sig: libc::c_int) {
  const SHOW: &[u8] = b"\x1b[?25h\x1b[?2026l";
  // SAFETY: write, signal, raise are all async-signal-safe per POSIX.
  unsafe {
    libc::write(libc::STDERR_FILENO, SHOW.as_ptr().cast(), SHOW.len());
    libc::signal(sig, libc::SIG_DFL);
    libc::raise(sig);
  }
}
