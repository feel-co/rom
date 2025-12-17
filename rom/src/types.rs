//! Core types for ROM

/// Display format for output
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayFormat {
  /// Show dependency tree graph
  Tree,
  /// Plain text output
  Plain,
  /// Dashboard summary view
  Dashboard,
}

/// Summary display style
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryStyle {
  /// Concise single-line summary
  Concise,
  /// Table with host breakdown
  Table,
  /// Full detailed summary
  Full,
}

impl SummaryStyle {
  #[must_use]
  pub fn from_str(s: &str) -> Self {
    match s.to_lowercase().as_str() {
      "concise" => Self::Concise,
      "table" => Self::Table,
      "full" => Self::Full,
      _ => Self::Concise,
    }
  }
}

impl DisplayFormat {
  #[must_use]
  pub fn from_str(s: &str) -> Self {
    match s.to_lowercase().as_str() {
      "tree" => Self::Tree,
      "plain" => Self::Plain,
      "dashboard" => Self::Dashboard,
      _ => Self::Tree,
    }
  }
}

/// Configuration for the monitor
#[derive(Debug, Clone)]
pub struct Config {
  /// Whether we're piping output through
  pub piping:        bool,
  /// Silent mode - minimal output
  pub silent:        bool,
  /// Input parsing mode
  pub input_mode:    InputMode,
  /// Show completion times
  pub show_timers:   bool,
  /// Terminal width override
  pub width:         Option<usize>,
  /// Display format
  pub format:        DisplayFormat,
  /// Legend display style
  pub legend_style:  String,
  /// Summary display style
  pub summary_style: String,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      piping:        false,
      silent:        false,
      input_mode:    InputMode::Human,
      show_timers:   true,
      width:         None,
      format:        DisplayFormat::Tree,
      legend_style:  "table".to_string(),
      summary_style: "concise".to_string(),
    }
  }
}

/// Input parsing mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
  /// Parse JSON output from nix --log-format=internal-json
  Json,
  /// Parse human-readable nix output
  Human,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_config_default() {
    let config = Config::default();
    assert!(!config.piping);
    assert!(!config.silent);
    assert_eq!(config.input_mode, InputMode::Human);
    assert!(config.show_timers);
    assert_eq!(config.format, DisplayFormat::Tree);
  }

  #[test]
  fn test_input_mode_comparison() {
    assert_eq!(InputMode::Json, InputMode::Json);
    assert_ne!(InputMode::Json, InputMode::Human);
  }
}
