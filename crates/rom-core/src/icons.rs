//! Icon sets for ROM display output.
//!
//! Two sets are available: Unicode (standard, widely supported) and Nerd Fonts
//! (requires a patched font, detected automatically via `has-nerd-font`).

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

/// Standard Unicode icons — always available, no special font required.
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

/// Nerd Fonts icons (v3 codepoints).
///
/// Requires a Nerd Font–patched terminal font.  Detected automatically via
/// the `has-nerd-font` crate, but can be forced with `NERD_FONTS=1` (or
/// disabled with `NERD_FONTS=0`).
pub static NERD: Icons = Icons {
  running:  "\u{f144}", // nf-fa-play
  done:     "\u{f00c}", // nf-fa-check
  planned:  "\u{f04c}", // nf-fa-pause
  failed:   "\u{f071}", // nf-fa-exclamation-triangle
  download: "\u{f019}", // nf-fa-download
  upload:   "\u{f093}", // nf-fa-upload
  clock:    "\u{f017}", // nf-fa-clock-o
  estimate: "\u{f252}", // nf-fa-hourglass-half
  summary:  "∑",
};

/// Detect the best icon set for the current terminal session.
///
/// Checks `NERD_FONTS` env override first (`1` forces Nerd, `0` forces
/// Unicode), then delegates to `has-nerd-font` for automatic detection.
pub fn detect() -> &'static Icons {
  // Manual override takes precedence
  if let Ok(v) = std::env::var("NERD_FONTS") {
    match v.trim() {
      "1" | "true" | "yes" => return &NERD,
      "0" | "false" | "no" => return &UNICODE,
      _ => {},
    }
  }

  let vars: Vec<(String, String)> = std::env::vars().collect();
  match has_nerd_font::detect(&vars).detected {
    Some(true) => &NERD,
    _ => &UNICODE,
  }
}
