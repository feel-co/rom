use rom_core::types::{Config, DisplayFormat, InputMode, LogPrefixStyle};

#[test]
fn test_config_default() {
  let config = Config::default();
  assert!(!config.piping);
  assert!(!config.silent);
  assert_eq!(config.input_mode, InputMode::Human);
  assert!(config.show_timers);
  assert_eq!(config.format, DisplayFormat::Tree);
  assert_eq!(config.log_prefix_style, LogPrefixStyle::Short);
  assert_eq!(config.log_line_limit, None);
}

#[test]
fn test_input_mode_comparison() {
  assert_eq!(InputMode::Json, InputMode::Json);
  assert_ne!(InputMode::Json, InputMode::Human);
}
