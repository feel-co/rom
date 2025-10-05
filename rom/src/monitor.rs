//! Monitor module for orchestrating state updates and display rendering

use std::{
  io::{BufRead, Write},
  time::Duration,
};

use crate::{
  display::{Display, DisplayConfig},
  error::{Result, RomError},
  state::State,
  types::{Config, InputMode},
  update,
};

/// Main monitor that processes nix output and displays progress
pub struct Monitor<W: Write> {
  state:   State,
  display: Display<W>,
  config:  Config,
}

impl<W: Write> Monitor<W> {
  /// Create a new monitor
  pub fn new(config: Config, writer: W) -> Result<Self> {
    let legend_style = match config.legend_style.to_lowercase().as_str() {
      "compact" => crate::display::LegendStyle::Compact,
      "verbose" => crate::display::LegendStyle::Verbose,
      _ => crate::display::LegendStyle::Table,
    };

    let summary_style = match config.summary_style.to_lowercase().as_str() {
      "table" => crate::display::SummaryStyle::Table,
      "full" => crate::display::SummaryStyle::Full,
      _ => crate::display::SummaryStyle::Concise,
    };

    let display_config = DisplayConfig {
      show_timers: config.show_timers,
      max_tree_depth: 10,
      max_visible_lines: 100,
      use_color: !config.piping,
      format: config.format.clone(),
      legend_style,
      summary_style,
    };

    let display = Display::new(writer, display_config)?;
    let state = State::new();

    Ok(Self {
      state,
      display,
      config,
    })
  }

  /// Process a stream of input
  pub fn process_stream<R: BufRead>(&mut self, reader: R) -> Result<()> {
    let mut last_render = std::time::Instant::now();
    let render_interval = Duration::from_millis(100);

    for line in reader.lines() {
      let line = line.map_err(RomError::Io)?;

      // Process the line
      self.process_line(&line)?;

      // Render periodically
      if last_render.elapsed() >= render_interval {
        if !self.config.silent {
          self.display.render(&self.state, &[])?;
        }
        last_render = std::time::Instant::now();
      }
    }

    // Mark as finished and do final render
    crate::update::finish_state(&mut self.state);

    if !self.config.silent {
      self.display.render_final(&self.state)?;
    }

    // Return error code if there were failures
    if self.state.has_errors() {
      return Err(RomError::BuildFailed);
    }

    Ok(())
  }

  /// Process a single line of input
  fn process_line(&mut self, line: &str) -> Result<bool> {
    // Auto-detect format: lines starting with "@nix " are JSON
    if line.starts_with("@nix ") {
      self.process_json_line(line)
    } else {
      match self.config.input_mode {
        InputMode::Json => self.process_json_line(line),
        InputMode::Human => self.process_human_line(line),
      }
    }
  }

  /// Process a JSON-formatted line
  fn process_json_line(&mut self, line: &str) -> Result<bool> {
    // Nix JSON lines are prefixed with "@nix "
    if let Some(json_str) = line.strip_prefix("@nix ") {
      match serde_json::from_str::<cognos::Actions>(json_str) {
        Ok(action) => {
          // Handle message passthrough - print directly to stdout
          if let cognos::Actions::Message { msg, .. } = &action {
            println!("{}", msg);
          }

          let changed = update::process_message(&mut self.state, action);
          Ok(changed)
        },
        Err(e) => {
          // Log parsing errors but don't fail
          tracing::debug!("Failed to parse JSON message: {}", e);
          Ok(false)
        },
      }
    } else {
      // Non-JSON lines in JSON mode are passed through
      println!("{}", line);
      Ok(false)
    }
  }

  /// Process a human-readable line
  fn process_human_line(&mut self, line: &str) -> Result<bool> {
    // Parse human-readable nix output
    // This is a simplified version - the full implementation would need
    // comprehensive parsing of nix's output format

    let line = line.trim();

    // Skip empty lines
    if line.is_empty() {
      return Ok(false);
    }

    // Try to detect build-related messages
    if line.starts_with("building") || line.contains("building '") {
      if let Some(drv_path) = extract_path_from_message(line) {
        if let Some(drv) = crate::state::Derivation::parse(&drv_path) {
          let drv_id = self.state.get_or_create_derivation_id(drv);
          let now = crate::state::current_time();

          let build_info = crate::state::BuildInfo {
            start:       now,
            host:        crate::state::Host::Localhost,
            estimate:    None,
            activity_id: None,
          };

          self.state.update_build_status(
            drv_id,
            crate::state::BuildStatus::Building(build_info),
          );
          return Ok(true);
        }
      }
    }

    // Detect downloads
    if line.starts_with("downloading") || line.contains("downloading '") {
      if let Some(path_str) = extract_path_from_message(line) {
        if let Some(path) = crate::state::StorePath::parse(&path_str) {
          let path_id = self.state.get_or_create_store_path_id(path);
          let now = crate::state::current_time();

          let transfer = crate::state::TransferInfo {
            start:             now,
            host:              crate::state::Host::Localhost,
            activity_id:       0, // No activity ID in human mode
            bytes_transferred: 0,
            total_bytes:       None,
          };

          if let Some(path_info) = self.state.get_store_path_info_mut(path_id) {
            path_info
              .states
              .insert(crate::state::StorePathState::Downloading(
                transfer.clone(),
              ));
          }

          self
            .state
            .full_summary
            .running_downloads
            .insert(path_id, transfer);

          return Ok(true);
        }
      }
    }

    // Detect errors
    if line.starts_with("error:") || line.contains("error:") {
      self.state.nix_errors.push(line.to_string());
      return Ok(true);
    }

    // Detect build completions
    if line.starts_with("built") || line.contains("built '") {
      if let Some(drv_path) = extract_path_from_message(line) {
        if let Some(drv) = crate::state::Derivation::parse(&drv_path) {
          if let Some(&drv_id) = self.state.derivation_ids.get(&drv) {
            if let Some(info) = self.state.get_derivation_info(drv_id) {
              if let crate::state::BuildStatus::Building(build_info) =
                &info.build_status
              {
                let now = crate::state::current_time();
                self.state.update_build_status(
                  drv_id,
                  crate::state::BuildStatus::Built {
                    info: build_info.clone(),
                    end:  now,
                  },
                );
                return Ok(true);
              }
            }
          }
        }
      }
    }

    Ok(false)
  }

  /// Get a reference to the current state
  pub const fn state(&self) -> &State {
    &self.state
  }

  /// Get a mutable reference to the current state
  pub const fn state_mut(&mut self) -> &mut State {
    &mut self.state
  }
}

/// Extract a path from a message line
fn extract_path_from_message(line: &str) -> Option<String> {
  // Look for quoted paths
  if let Some(start) = line.find('\'') {
    if let Some(end) = line[start + 1..].find('\'') {
      return Some(line[start + 1..start + 1 + end].to_string());
    }
  }

  // Look for unquoted store paths
  for word in line.split_whitespace() {
    if word.starts_with("/nix/store/") {
      return Some(
        word
          .trim_matches(|c: char| {
            !c.is_ascii_alphanumeric() && c != '/' && c != '-' && c != '.'
          })
          .to_string(),
      );
    }
  }

  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_monitor_creation() {
    let config = Config::default();
    let output = Vec::new();
    let monitor = Monitor::new(config, output);
    assert!(monitor.is_ok());
  }

  #[test]
  fn test_extract_path_from_message() {
    let line = "building '/nix/store/abc123-hello-1.0.drv'";
    let path = extract_path_from_message(line);
    assert!(path.is_some());
    assert!(path.unwrap().contains("hello-1.0.drv"));
  }

  #[test]
  fn test_extract_path_unquoted() {
    let line = "building /nix/store/abc123-hello-1.0.drv locally";
    let path = extract_path_from_message(line);
    assert!(path.is_some());
  }
}
