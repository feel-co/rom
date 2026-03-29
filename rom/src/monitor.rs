//! Monitor module for orchestrating state updates and display rendering

use std::{
  io::{BufRead, Write},
  time::Duration,
};

use cognos::Host;
use tracing::debug;

use crate::{
  cache::BuildReportCache,
  display::{Display, DisplayConfig},
  error::{Result, RomError},
  state::{BuildStatus, Derivation, FailType, State, StorePath},
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
    let display_config = DisplayConfig {
      show_timers:       config.show_timers,
      max_tree_depth:    10,
      max_visible_lines: 100,
      use_color:         !config.piping,
      format:            config.format,
      legend_style:      config.legend_style,
      summary_style:     config.summary_style,
    };

    let display = Display::new(writer, display_config)?;
    let mut state = State::new();

    // Load build cache for predictions
    let cache_path = BuildReportCache::default_cache_path();
    let cache = BuildReportCache::new(cache_path);
    state.build_cache = cache.load();

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

    // Save build cache for future predictions
    let cache_path = BuildReportCache::default_cache_path();
    let cache = BuildReportCache::new(cache_path);
    if let Err(e) = cache.save(&self.state.build_cache) {
      debug!("Failed to save build cache: {}", e);
      // Don't fail the build if cache save fails
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
            println!("{msg}");
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
      println!("{line}");
      Ok(false)
    }
  }

  /// Process a human-readable line
  fn process_human_line(&mut self, line: &str) -> Result<bool> {
    let line = line.trim();

    // Skip empty lines
    if line.is_empty() {
      return Ok(false);
    }

    // Try to detect build-related messages
    if (line.starts_with("building") || line.contains("building '"))
      && let Some(drv_path) = extract_path_from_message(line)
      && let Some(drv) = crate::state::Derivation::parse(&drv_path)
    {
      let drv_id = self.state.get_or_create_derivation_id(drv);
      let now = crate::state::current_time();

      let build_info = crate::state::BuildInfo {
        start:       now,
        host:        Host::Localhost,
        estimate:    None,
        activity_id: None,
      };

      self.state.update_build_status(
        drv_id,
        crate::state::BuildStatus::Building(build_info),
      );
      return Ok(true);
    }

    // Detect downloads
    if (line.starts_with("downloading") || line.contains("downloading '"))
      && let Some(path_str) = extract_path_from_message(line)
      && let Some(path) = crate::state::StorePath::parse(&path_str)
    {
      let path_id = self.state.get_or_create_store_path_id(path);
      let now = crate::state::current_time();

      // Try to extract byte size from the message
      let total_bytes = extract_byte_size(line);

      let transfer = crate::state::TransferInfo {
        start: now,
        host: Host::Localhost,
        activity_id: 0, // no activity ID in human mode
        bytes_transferred: 0,
        total_bytes,
      };

      self
        .state
        .full_summary
        .running_downloads
        .insert(path_id, transfer);

      return Ok(true);
    }

    // Detect download completions with byte sizes
    if (line.starts_with("downloaded") || line.contains("downloaded '"))
      && let Some(path_str) = extract_path_from_message(line)
      && let Some(path) = StorePath::parse(&path_str)
      && let Some(&path_id) = self.state.store_path_ids.get(&path)
    {
      let now = crate::state::current_time();
      let total_bytes = extract_byte_size(line).unwrap_or(0);

      // Get start time from running download if it exists
      let start = self
        .state
        .full_summary
        .running_downloads
        .get(&path_id)
        .map_or(now, |t| t.start);

      let completed = crate::state::CompletedTransferInfo {
        start,
        end: now,
        host: Host::Localhost,
        total_bytes,
      };

      self.state.full_summary.running_downloads.remove(&path_id);
      self
        .state
        .full_summary
        .completed_downloads
        .insert(path_id, completed);

      return Ok(true);
    }

    // Detect "checking outputs of" messages
    if line.contains("checking outputs of")
      && let Some(drv_path) = extract_path_from_message(line)
      && let Some(drv) = crate::state::Derivation::parse(&drv_path)
    {
      let drv_id = self.state.get_or_create_derivation_id(drv);
      // Just mark it as "touched" - checking happens after build
      // Reminds me of Sako...
      self.state.touched_ids.insert(drv_id);
      return Ok(true);
    }

    // Detect "copying N paths" messages
    if line.starts_with("copying") && line.contains("paths") {
      // Extract number of paths if present
      let words: Vec<&str> = line.split_whitespace().collect();
      if words.len() >= 2
        && let Ok(count) = words[1].parse::<usize>()
      {
        debug!("Copying {} paths", count);
        return Ok(true);
      }
    }

    // Detect errors
    if line.starts_with("error:") || line.contains("error:") {
      self.state.nix_errors.push(line.to_string());

      // Try to determine the error type and associated derivation
      let fail_type = if line.contains("hash mismatch")
        || line.contains("output path")
          && (line.contains("hash") || line.contains("differs"))
      {
        FailType::HashMismatch
      } else if line.contains("timed out") || line.contains("timeout") {
        FailType::Timeout
      } else if line.contains("dependency failed")
        || line.contains("dependencies failed")
      {
        FailType::DependencyFailed
      } else if line.contains("builder for")
        && line.contains("failed with exit code")
      {
        // Try to extract exit code
        if let Some(code_pos) = line.find("exit code") {
          let after_code = &line[code_pos + 10..];
          let code_str = after_code
            .split_whitespace()
            .next()
            .map(|s| s.trim_end_matches(|c: char| !c.is_ascii_digit()));
          if let Some(code) = code_str.and_then(|s| s.parse::<i32>().ok()) {
            FailType::BuildFailed(code)
          } else {
            FailType::Unknown
          }
        } else {
          FailType::Unknown
        }
      } else {
        FailType::Unknown
      };

      // Try to find the associated derivation and mark it as failed
      if let Some(drv_path) = extract_path_from_message(line)
        && let Some(drv) = crate::state::Derivation::parse(&drv_path)
        && let Some(&drv_id) = self.state.derivation_ids.get(&drv)
        && let Some(info) = self.state.get_derivation_info(drv_id)
        && let crate::state::BuildStatus::Building(build_info) =
          &info.build_status
      {
        let now = crate::state::current_time();
        self.state.update_build_status(
          drv_id,
          crate::state::BuildStatus::Failed {
            info: build_info.clone(),
            fail: crate::state::BuildFail { at: now, fail_type },
          },
        );
      }

      return Ok(true);
    }

    // Detect build completions
    if (line.starts_with("built") || line.contains("built '"))
      && let Some(drv_path) = extract_path_from_message(line)
      && let Some(drv) = Derivation::parse(&drv_path)
      && let Some(&drv_id) = self.state.derivation_ids.get(&drv)
      && let Some(info) = self.state.get_derivation_info(drv_id)
      && let BuildStatus::Building(build_info) = &info.build_status
    {
      let now = crate::state::current_time();
      self.state.update_build_status(drv_id, BuildStatus::Built {
        info: build_info.clone(),
        end:  now,
      });
      return Ok(true);
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
  if let Some(start) = line.find('\'')
    && let Some(end) = line[start + 1..].find('\'')
  {
    return Some(line[start + 1..start + 1 + end].to_string());
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

/// Extract byte size from a message line (e.g., "downloaded 123 KiB")
fn extract_byte_size(line: &str) -> Option<u64> {
  // Look for patterns like "123 KiB", "6.7 MiB", etc.
  // Haha 6.7
  let words: Vec<&str> = line.split_whitespace().collect();
  for (i, word) in words.iter().enumerate() {
    if i + 1 < words.len() {
      let unit = words[i + 1];
      if matches!(unit, "B" | "KiB" | "MiB" | "GiB" | "TiB" | "PiB")
        && let Ok(value) = word.parse::<f64>()
      {
        let multiplier = match unit {
          "B" => 1_u64,
          "KiB" => 1024,
          "MiB" => 1024 * 1024,
          "GiB" => 1024 * 1024 * 1024,
          "TiB" => 1024_u64 * 1024 * 1024 * 1024,
          "PiB" => 1024_u64 * 1024 * 1024 * 1024 * 1024,
          _ => 1,
        };
        return Some((value * multiplier as f64) as u64);
      }
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

  #[test]
  fn test_extract_byte_size() {
    let line = "downloaded 123 KiB in 2 seconds";
    assert_eq!(extract_byte_size(line), Some(123 * 1024));

    let line2 = "downloading 4.5 MiB";
    assert_eq!(
      extract_byte_size(line2),
      Some((4.5 * 1024.0 * 1024.0) as u64)
    );

    let line3 = "no size here";
    assert_eq!(extract_byte_size(line3), None);
  }
}
