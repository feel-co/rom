//! Icon sets for ROM display output.
//!
//! Two sets are available: Unicode (standard, widely supported) and Nerd Fonts
//! (requires a patched font). Detection is opt-in via `NERD_FONTS=1`.

/// A complete set of display icons.
pub struct Icons {
  pub running:  &'static str,
  pub done:     &'static str,
  pub planned:  &'static str,
  pub failed:   &'static str,
  pub download: &'static str,
  pub upload:   &'static str,
  pub clock:    &'static str,
  pub estimate: &'static str,
  pub summary:  &'static str,
}

/// Standard Unicode icons are always available, no special font required.
pub static UNICODE: Icons = Icons {
  running:  "⏵",
  done:     "✔",
  planned:  "⏸",
  failed:   "✗",
  download: "↓",
  upload:   "↑",
  clock:    "⏱",
  estimate: "∅",
  summary:  "∑",
};

/// Nerd Fonts icons.
///
/// Requires a Nerd Font–patched terminal font. Opt in with `NERD_FONTS=1`.
pub static NERD: Icons = Icons {
  running:  "\u{f04b}",  // 
  done:     "\u{f00c}",  // 
  planned:  "\u{f04c}",  // 
  failed:   "\u{f071}",  // 
  download: "\u{f063}",  // 
  upload:   "\u{f062}",  // 
  clock:    "\u{f1da}",  // 
  estimate: "\u{f252}",  // 
  summary:  "\u{f04a0}", // 󰒠
};

/// Pick the icon set based on `NERD_FONTS` env var. Defaults to Unicode —
/// nerd-font auto-detection is unreliable across terminals, so we don't
/// guess.
pub fn detect() -> &'static Icons {
  match std::env::var("NERD_FONTS").as_deref() {
    Ok("1" | "true" | "yes") => &NERD,
    _ => &UNICODE,
  }
}
