use std::{
  collections::HashMap,
  fs::{self, File, OpenOptions},
  io::{BufReader, BufWriter},
  path::PathBuf,
  time::SystemTime,
};

use csv::{Reader, Writer};
use serde::{Deserialize, Serialize};

use crate::state::BuildReport;

/// Maximum number of historical builds to keep per derivation
const HISTORY_LIMIT: usize = 10;

/// Build report cache for CSV persistence
pub struct BuildReportCache {
  cache_path: PathBuf,
}

/// CSV row format for build reports
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuildReportRow {
  hostname:        String,
  derivation_name: String,
  utc_time:        String,
  build_seconds:   u64,
}

impl BuildReportCache {
  /// Create a new cache instance with the given path
  #[must_use]
  pub fn new(cache_path: PathBuf) -> Self {
    Self { cache_path }
  }

  // FIXME: just use the dirs crate for this
  /// Get the default cache directory path
  ///
  /// Uses `$XDG_STATE_HOME` if set, otherwise ``~/.local/state`
  #[must_use]
  pub fn default_cache_dir() -> PathBuf {
    if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
      PathBuf::from(xdg_state).join("rom")
    } else if let Ok(home) = std::env::var("HOME") {
      PathBuf::from(home).join(".local/state/rom")
    } else {
      PathBuf::from(".rom")
    }
  }

  /// Get the default cache file path
  #[must_use]
  pub fn default_cache_path() -> PathBuf {
    Self::default_cache_dir().join("build-reports.csv")
  }

  /// Load build reports from CSV
  ///
  /// Returns empty [`HashMap`] if file doesn't exist or parsing fails
  pub fn load(&self) -> HashMap<(String, String), Vec<BuildReport>> {
    if !self.cache_path.exists() {
      return HashMap::new();
    }

    let file = match File::open(&self.cache_path) {
      Ok(f) => f,
      Err(_) => return HashMap::new(),
    };

    let reader = BufReader::new(file);
    let mut csv_reader = Reader::from_reader(reader);

    let mut reports: HashMap<(String, String), Vec<BuildReport>> =
      HashMap::new();

    for result in csv_reader.deserialize() {
      let row: BuildReportRow = match result {
        Ok(r) => r,
        Err(_) => continue,
      };

      let completed_at = match parse_utc_time(&row.utc_time) {
        Some(t) => t,
        None => continue,
      };

      let report = BuildReport {
        derivation_name: row.derivation_name.clone(),
        duration_secs: row.build_seconds as f64,
        completed_at,
        host: row.hostname.clone(),
        success: true, // only successful builds are cached

        // FIXME: not stored in CSV. This is for simplicity, and because I'm
        // lazy
        platform: String::new(),
      };

      let key = (row.hostname, row.derivation_name);
      reports.entry(key).or_default().push(report);
    }

    // Sort each entry by timestamp (newest first) and limit to HISTORY_LIMIT
    for entries in reports.values_mut() {
      entries.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
      entries.truncate(HISTORY_LIMIT);
    }

    reports
  }

  /// Save build reports to CSV
  ///
  /// Merges with existing reports and enforces history limit
  pub fn save(
    &self,
    reports: &HashMap<(String, String), Vec<BuildReport>>,
  ) -> Result<(), std::io::Error> {
    // Ensure directory exists
    if let Some(parent) = self.cache_path.parent() {
      fs::create_dir_all(parent)?;
    }

    // Load existing reports to merge
    let mut merged = self.load();

    // Merge new reports
    for ((host, drv_name), new_reports) in reports {
      let key = (host.clone(), drv_name.clone());
      let existing = merged.entry(key).or_default();

      // Add new reports
      existing.extend(new_reports.iter().cloned());

      // Sort by timestamp (newest first)
      existing.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));

      // Keep only most recent HISTORY_LIMIT entries
      existing.truncate(HISTORY_LIMIT);
    }

    // Write to a temp file in the same directory, then rename atomically.
    // This prevents a concurrent save() from corrupting the cache file.
    let tmp_path = self.cache_path.with_extension("csv.tmp");

    let file = OpenOptions::new()
      .write(true)
      .create(true)
      .truncate(true)
      .open(&tmp_path)?;

    let writer = BufWriter::new(file);
    let mut csv_writer = Writer::from_writer(writer);

    // Flatten and write all reports
    for ((hostname, derivation_name), entries) in merged {
      for report in entries {
        let row = BuildReportRow {
          hostname:        hostname.clone(),
          derivation_name: derivation_name.clone(),
          utc_time:        format_utc_time(report.completed_at),
          build_seconds:   report.duration_secs as u64,
        };
        csv_writer.serialize(row)?;
      }
    }

    csv_writer.flush()?;
    drop(csv_writer);

    // Atomic replace
    fs::rename(&tmp_path, &self.cache_path)?;

    Ok(())
  }

  /// Calculate median build time from historical reports
  ///
  /// Returns [`None`] if there are no reports
  #[must_use]
  pub fn calculate_median(reports: &[BuildReport]) -> Option<u64> {
    if reports.is_empty() {
      return None;
    }

    let mut durations: Vec<u64> =
      reports.iter().map(|r| r.duration_secs as u64).collect();
    durations.sort_unstable();

    let len = durations.len();
    if len % 2 == 1 {
      Some(durations[len / 2])
    } else {
      let mid1 = durations[len / 2 - 1];
      let mid2 = durations[len / 2];
      Some((mid1 + mid2) / 2)
    }
  }

  /// Get median build time for a specific derivation on a host
  #[must_use]
  pub fn get_estimate(
    &self,
    reports: &HashMap<(String, String), Vec<BuildReport>>,
    host: &str,
    derivation_name: &str,
  ) -> Option<u64> {
    let key = (host.to_string(), derivation_name.to_string());
    let entries = reports.get(&key)?;
    Self::calculate_median(entries)
  }
}

/// Parse UTC time string in format "%Y-%m-%d %H:%M:%S"
fn parse_utc_time(s: &str) -> Option<SystemTime> {
  // Simple parsing for "YYYY-MM-DD HH:MM:SS" format
  let parts: Vec<&str> = s.split([' ', '-', ':']).collect();
  if parts.len() != 6 {
    return None;
  }

  let year: i64 = parts[0].parse().ok()?;
  let month: u64 = parts[1].parse().ok()?;
  let day: u64 = parts[2].parse().ok()?;
  let hour: u64 = parts[3].parse().ok()?;
  let minute: u64 = parts[4].parse().ok()?;
  let second: u64 = parts[5].parse().ok()?;

  // Approximate conversion to Unix timestamp
  // This is a simplified calculation that doesn't handle leap years perfectly
  let days_since_epoch = (year - 1970) * 365
    + (year - 1969) / 4
    + days_until_month(month)
    + day as i64
    - 1;
  let seconds_since_epoch =
    days_since_epoch as u64 * 86400 + hour * 3600 + minute * 60 + second;

  Some(
    SystemTime::UNIX_EPOCH
      + std::time::Duration::from_secs(seconds_since_epoch),
  )
}

// FIXME: I'm really sure there's a library for this but lets just get
// this thing compiling
/// Calculate days until the start of a month (approximation)
const fn days_until_month(month: u64) -> i64 {
  match month {
    1 => 0,
    2 => 31,
    3 => 59,
    4 => 90,
    5 => 120,
    6 => 151,
    7 => 181,
    8 => 212,
    9 => 243,
    10 => 273,
    11 => 304,
    12 => 334,
    _ => 0,
  }
}

// FIXME: does Chrono do this?
/// Format SystemTime as UTC string in format "%Y-%m-%d %H:%M:%S"
fn format_utc_time(time: SystemTime) -> String {
  let duration = time
    .duration_since(SystemTime::UNIX_EPOCH)
    .unwrap_or_default();
  let secs = duration.as_secs();

  let days = secs / 86400;
  let remaining = secs % 86400;
  let hours = remaining / 3600;
  let minutes = (remaining % 3600) / 60;
  let seconds = remaining % 60;

  // Approximate conversion from days since epoch to date
  let mut year = 1970;
  let mut days_left = days as i64;

  // Subtract full years
  while days_left >= 365 {
    if is_leap_year(year) && days_left >= 366 {
      days_left -= 366;
      year += 1;
    } else if !is_leap_year(year) {
      days_left -= 365;
      year += 1;
    } else {
      break;
    }
  }

  // Calculate month and day
  let (month, day) = calculate_month_day(days_left as u64, is_leap_year(year));

  format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}")
}

const fn is_leap_year(year: i64) -> bool {
  (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn calculate_month_day(days: u64, is_leap: bool) -> (u8, u8) {
  let days_in_month: [u8; 12] = if is_leap {
    [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  } else {
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
  };

  let mut remaining = days as i32;
  for (i, &month_days) in days_in_month.iter().enumerate() {
    if remaining < i32::from(month_days) {
      return ((i + 1) as u8, (remaining + 1) as u8);
    }
    remaining -= i32::from(month_days);
  }

  (12, 31)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_calculate_median_odd() {
    let reports = vec![
      BuildReport {
        derivation_name: "test".to_string(),
        platform:        "x86_64-linux".to_string(),
        duration_secs:   10.0,
        completed_at:    SystemTime::UNIX_EPOCH,
        host:            "localhost".to_string(),
        success:         true,
      },
      BuildReport {
        derivation_name: "test".to_string(),
        platform:        "x86_64-linux".to_string(),
        duration_secs:   20.0,
        completed_at:    SystemTime::UNIX_EPOCH,
        host:            "localhost".to_string(),
        success:         true,
      },
      BuildReport {
        derivation_name: "test".to_string(),
        platform:        "x86_64-linux".to_string(),
        duration_secs:   30.0,
        completed_at:    SystemTime::UNIX_EPOCH,
        host:            "localhost".to_string(),
        success:         true,
      },
    ];

    assert_eq!(BuildReportCache::calculate_median(&reports), Some(20));
  }

  #[test]
  fn test_calculate_median_even() {
    let reports = vec![
      BuildReport {
        derivation_name: "test".to_string(),
        platform:        "x86_64-linux".to_string(),
        duration_secs:   10.0,
        completed_at:    SystemTime::UNIX_EPOCH,
        host:            "localhost".to_string(),
        success:         true,
      },
      BuildReport {
        derivation_name: "test".to_string(),
        platform:        "x86_64-linux".to_string(),
        duration_secs:   20.0,
        completed_at:    SystemTime::UNIX_EPOCH,
        host:            "localhost".to_string(),
        success:         true,
      },
    ];

    assert_eq!(BuildReportCache::calculate_median(&reports), Some(15));
  }

  #[test]
  fn test_calculate_median_empty() {
    let reports = vec![];
    assert_eq!(BuildReportCache::calculate_median(&reports), None);
  }

  #[test]
  fn test_format_parse_utc_time() {
    let time =
      SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    let formatted = format_utc_time(time);
    let parsed = parse_utc_time(&formatted).unwrap();

    // Allow small difference due to approximation
    let diff = parsed
      .duration_since(time)
      .unwrap_or_else(|e| e.duration())
      .as_secs();
    assert!(diff < 86400); // less than 1 day difference
  }
}
