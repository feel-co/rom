use std::{
  collections::HashMap,
  env,
  fs::{self, File, OpenOptions},
  io::{BufRead, BufReader, BufWriter, Write},
  path::PathBuf,
  time::{Duration, SystemTime},
};

use crate::state::BuildReport;

/// Maximum number of historical builds to keep per derivation
const HISTORY_LIMIT: usize = 10;

/// Build report cache for CSV persistence
pub struct BuildReportCache {
  cache_path: PathBuf,
}

impl BuildReportCache {
  /// Create a new cache instance with the given path
  #[must_use]
  pub const fn new(cache_path: PathBuf) -> Self {
    Self { cache_path }
  }

  /// Get the default cache file path: `$XDG_STATE_HOME/rom/build-reports.csv`
  /// (falls back to `~/.local/state/rom/build-reports.csv`).
  #[must_use]
  pub fn default_cache_path() -> PathBuf {
    state_dir().join("rom").join("build-reports.csv")
  }

  /// Load build reports from CSV.
  ///
  /// Returns an empty map if the file doesn't exist or can't be parsed.
  #[must_use]
  pub fn load(&self) -> HashMap<(String, String), Vec<BuildReport>> {
    let Ok(file) = File::open(&self.cache_path) else {
      return HashMap::new();
    };

    let reader = BufReader::new(file);
    let mut reports: HashMap<(String, String), Vec<BuildReport>> =
      HashMap::new();

    for (idx, line) in reader.lines().enumerate() {
      let Ok(line) = line else { continue };
      // Skip header if present
      if idx == 0 && line.starts_with("hostname,") {
        continue;
      }
      let Some(row) = parse_row(&line) else { continue };
      let Some(completed_at) = parse_utc_time(&row.utc_time) else {
        continue;
      };

      let report = BuildReport {
        derivation_name: row.derivation_name.clone(),
        duration_secs: row.build_seconds as f64,
        completed_at,
        host: row.hostname.clone(),
        success: true,
        platform: String::new(),
      };

      reports
        .entry((row.hostname, row.derivation_name))
        .or_default()
        .push(report);
    }

    for entries in reports.values_mut() {
      entries.sort_by_key(|r| std::cmp::Reverse(r.completed_at));
      entries.truncate(HISTORY_LIMIT);
    }

    reports
  }

  /// Save build reports to CSV.
  ///
  /// Merges with existing reports and enforces the history limit, writing
  /// atomically via tmp-file + rename.
  pub fn save(
    &self,
    reports: &HashMap<(String, String), Vec<BuildReport>>,
  ) -> std::io::Result<()> {
    if let Some(parent) = self.cache_path.parent() {
      fs::create_dir_all(parent)?;
    }

    let mut merged = self.load();
    for ((host, drv_name), new_reports) in reports {
      let entries = merged.entry((host.clone(), drv_name.clone())).or_default();
      entries.extend(new_reports.iter().cloned());
      entries.sort_by_key(|r| std::cmp::Reverse(r.completed_at));
      entries.truncate(HISTORY_LIMIT);
    }

    // Per-invocation unique tmp path so two concurrent rom runs don't
    // clobber each other's half-written file. PID + nanos-since-epoch is
    // plenty of entropy for a process-local cache.
    let tmp_path = {
      let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
      let pid = std::process::id();
      self
        .cache_path
        .with_extension(format!("csv.tmp.{pid}.{nanos}"))
    };
    let file = OpenOptions::new()
      .write(true)
      .create(true)
      .truncate(true)
      .open(&tmp_path)?;

    {
      let mut writer = BufWriter::new(file);
      writeln!(writer, "hostname,derivation_name,utc_time,build_seconds")?;
      for ((hostname, drv_name), entries) in merged {
        for report in entries {
          writeln!(
            writer,
            "{},{},{},{}",
            hostname,
            drv_name,
            format_utc_time(report.completed_at),
            report.duration_secs as u64,
          )?;
        }
      }
      writer.flush()?;
    }

    fs::rename(&tmp_path, &self.cache_path)
  }

  /// Median of historical build durations. Returns `None` if empty.
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
      Some(u64::midpoint(mid1, mid2))
    }
  }

  /// Median build time for a specific derivation on a given host.
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

struct Row {
  hostname:        String,
  derivation_name: String,
  utc_time:        String,
  build_seconds:   u64,
}

fn parse_row(line: &str) -> Option<Row> {
  // 4-column CSV, no escaping needed — hostnames, derivation names, and our
  // fixed-format timestamps never contain commas.
  let mut parts = line.splitn(4, ',');
  let hostname = parts.next()?.to_string();
  let derivation_name = parts.next()?.to_string();
  let utc_time = parts.next()?.to_string();
  let build_seconds = parts.next()?.trim().parse().ok()?;
  Some(Row {
    hostname,
    derivation_name,
    utc_time,
    build_seconds,
  })
}

/// Resolve `$XDG_STATE_HOME` with the spec's fallback to `~/.local/state`.
fn state_dir() -> PathBuf {
  if let Some(p) = env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
    return PathBuf::from(p);
  }
  if let Some(home) = env::var_os("HOME") {
    return PathBuf::from(home).join(".local/state");
  }
  PathBuf::from(".")
}

/// Parse "YYYY-MM-DD HH:MM:SS" as a UTC `SystemTime`.
///
/// Rejects:
/// - Any separator not in the exact `-`, `-`, ` `, `:`, `:` positions
/// - Hour > 23, minute > 59, second > 59
/// - Impossible calendar dates (leap-year-aware), e.g. 2100-02-29
pub fn parse_utc_time(s: &str) -> Option<SystemTime> {
  let bytes = s.as_bytes();
  if bytes.len() != 19
    || bytes[4] != b'-'
    || bytes[7] != b'-'
    || bytes[10] != b' '
    || bytes[13] != b':'
    || bytes[16] != b':'
  {
    return None;
  }
  let year: i64 = s.get(0..4)?.parse().ok()?;
  let month: u32 = s.get(5..7)?.parse().ok()?;
  let day: u32 = s.get(8..10)?.parse().ok()?;
  let hour: u32 = s.get(11..13)?.parse().ok()?;
  let minute: u32 = s.get(14..16)?.parse().ok()?;
  let second: u32 = s.get(17..19)?.parse().ok()?;

  if hour > 23 || minute > 59 || second > 59 {
    return None;
  }
  if !is_valid_date(year, month, day) {
    return None;
  }

  let secs = days_from_civil(year, month, day)? * 86_400
    + i64::from(hour) * 3600
    + i64::from(minute) * 60
    + i64::from(second);
  if secs < 0 {
    return None;
  }
  Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// Reject calendar-impossible dates. Leap rule: divisible by 4, except
/// centuries, except those divisible by 400.
fn is_valid_date(year: i64, month: u32, day: u32) -> bool {
  if !(1..=12).contains(&month) || day < 1 {
    return false;
  }
  let days_in_month = match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 => {
      if year.rem_euclid(400) == 0
        || (year.rem_euclid(4) == 0 && year.rem_euclid(100) != 0)
      {
        29
      } else {
        28
      }
    },
    _ => return false,
  };
  day <= days_in_month
}

/// Format a UTC `SystemTime` as "YYYY-MM-DD HH:MM:SS".
pub fn format_utc_time(time: SystemTime) -> String {
  let secs = time
    .duration_since(SystemTime::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);
  let (y, mo, d, h, mi, s) = civil_from_days(secs.div_euclid(86_400))
    .map(|(y, mo, d)| {
      let rem = secs.rem_euclid(86_400);
      (y, mo, d, rem / 3600, (rem % 3600) / 60, rem % 60)
    })
    .unwrap_or((1970, 1, 1, 0, 0, 0));
  format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// Days from the civil epoch 1970-01-01 for a given civil date, or `None`
/// if the date is invalid. Howard Hinnant's civil date algorithm.
fn days_from_civil(y: i64, m: u32, d: u32) -> Option<i64> {
  if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
    return None;
  }
  let y = if m <= 2 { y - 1 } else { y };
  let era = y.div_euclid(400);
  let yoe = (y - era * 400) as u32;
  let m = i64::from(m);
  let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d)
    - 1;
  let doe = i64::from(yoe) * 365 + i64::from(yoe / 4 - yoe / 100) + doy;
  Some(era * 146_097 + doe - 719_468)
}

/// Inverse of `days_from_civil`. Returns `(year, month, day)`.
fn civil_from_days(z: i64) -> Option<(i64, u32, u32)> {
  let z = z + 719_468;
  let era = z.div_euclid(146_097);
  let doe = (z - era * 146_097) as u32;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
  let y = i64::from(yoe) + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = if m <= 2 { y + 1 } else { y };
  Some((y, m, d))
}
