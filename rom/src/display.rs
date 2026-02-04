//! Display rendering for ROM

use std::{
  collections::{HashMap, HashSet},
  io::{self, Write},
};

use crossterm::{
  cursor,
  execute,
  style::{Color, ResetColor, SetForegroundColor},
};

use crate::state::{BuildStatus, DerivationId, State, current_time};

/// Format a duration in seconds to a human-readable string
#[must_use]
pub fn format_duration(secs: f64) -> String {
  if secs < 60.0 {
    format!("{secs:.0}s")
  } else if secs < 3600.0 {
    format!("{:.0}m{:.0}s", secs / 60.0, secs % 60.0)
  } else {
    format!("{:.0}h{:.0}m", secs / 3600.0, (secs % 3600.0) / 60.0)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendStyle {
  Compact,
  Table,
  Verbose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryStyle {
  Concise,
  Table,
  Full,
}

pub struct DisplayConfig {
  pub show_timers:       bool,
  pub max_tree_depth:    usize,
  pub max_visible_lines: usize,
  pub use_color:         bool,
  pub format:            crate::types::DisplayFormat,
  pub legend_style:      LegendStyle,
  pub summary_style:     SummaryStyle,
}

impl Default for DisplayConfig {
  fn default() -> Self {
    Self {
      show_timers:       true,
      max_tree_depth:    10,
      max_visible_lines: 100,
      use_color:         true,
      format:            crate::types::DisplayFormat::Tree,
      legend_style:      LegendStyle::Table,
      summary_style:     SummaryStyle::Concise,
    }
  }
}

pub struct Display<W: Write> {
  writer:     W,
  config:     DisplayConfig,
  last_lines: usize,
}

struct TreeNode {
  drv_id:   DerivationId,
  children: Vec<TreeNode>,
}

impl<W: Write> Display<W> {
  pub fn new(writer: W, config: DisplayConfig) -> io::Result<Self> {
    Ok(Self {
      writer,
      config,
      last_lines: 0,
    })
  }

  pub fn clear_previous(&mut self) -> io::Result<()> {
    // Move cursor up and clear each line like NOM does
    if self.last_lines > 0 {
      // Save current position by moving to start of line
      execute!(self.writer, cursor::MoveToColumn(0))?;

      // Move up to the first line we printed
      for _ in 0..self.last_lines {
        execute!(self.writer, cursor::MoveUp(1))?;
      }

      // Clear from cursor to end of screen
      execute!(
        self.writer,
        cursor::MoveToColumn(0),
        crossterm::terminal::Clear(
          crossterm::terminal::ClearType::FromCursorDown
        )
      )?;

      // Don't flush here - let render() handle it after printing
    }
    Ok(())
  }

  pub fn render(&mut self, state: &State, logs: &[String]) -> io::Result<()> {
    // Clear previous output first
    self.clear_previous()?;

    let mut lines = Vec::new();

    // Print build logs ABOVE the graph
    for log in logs {
      lines.push(log.clone());
    }

    // Render based on format
    match self.config.format {
      crate::types::DisplayFormat::Tree => {
        let tree_lines = self.render_tree_view(state);
        let has_tree = !tree_lines.is_empty();
        let legend_lines = self.render_legend(state, has_tree);

        // Tree and legend come pre-formatted with headers and frames
        lines.extend(tree_lines);
        lines.extend(legend_lines);
      },
      crate::types::DisplayFormat::Plain => {
        // Plain format - show flat list view
        lines.extend(self.render_plain_view(state));
      },
      crate::types::DisplayFormat::Dashboard => {
        // Dashboard format - show summary dashboard
        lines.extend(self.render_dashboard_view(state));
      },
    }

    // Track how many lines we're printing
    self.last_lines = lines.len();

    // Print all lines
    for line in lines {
      writeln!(self.writer, "{line}")?;
    }

    // Flush to ensure output is visible
    self.writer.flush()
  }

  pub fn render_final(&mut self, state: &State) -> io::Result<()> {
    tracing::debug!("render_final called");

    // Clear any previous render
    self.clear_previous()?;

    let mut lines = Vec::new();

    // Render final output based on format
    match self.config.format {
      crate::types::DisplayFormat::Tree => {
        let tree_lines = self.render_tree_view(state);
        if !tree_lines.is_empty() {
          lines.push(format!(
            "{} Dependency Graph:",
            self.colored("┏━", Color::Blue)
          ));
          lines.extend(tree_lines);
        }
        lines.extend(self.render_final_summary(state));
      },
      crate::types::DisplayFormat::Plain => {
        lines.extend(self.render_plain_view(state));
        lines.extend(self.render_final_summary(state));
      },
      crate::types::DisplayFormat::Dashboard => {
        lines.extend(self.render_dashboard_final(state));
      },
    }

    tracing::debug!("render_final: {} lines to print", lines.len());

    // Print final output (don't track last_lines since this is final)
    for line in lines {
      writeln!(self.writer, "{line}")?;
    }

    writeln!(self.writer)?;
    self.writer.flush()?;

    Ok(())
  }

  fn render_final_summary(&self, state: &State) -> Vec<String> {
    match self.config.summary_style {
      SummaryStyle::Concise => self.render_concise_summary(state),
      SummaryStyle::Table => self.render_table_summary(state),
      SummaryStyle::Full => self.render_full_summary(state),
    }
  }

  fn render_concise_summary(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();

    let duration = current_time() - state.start_time;

    // Always print summary (like NOM's "Finished at HH:MM:SS after Xs")
    if running > 0 || completed > 0 || failed > 0 || planned > 0 {
      lines.push(format!(
        "{} {} {} │ {} {} │ {} {} │ {} {} │ {} {}",
        self.colored("━", Color::Blue),
        self.colored("⏵", Color::Yellow),
        running,
        self.colored("✔", Color::Green),
        completed,
        self.colored("✗", Color::Red),
        failed,
        self.colored("⏸", Color::Grey),
        planned,
        self.colored("⏱", Color::Grey),
        self.format_duration(duration)
      ));
    } else {
      // Nothing built - just show "Finished after Xs"
      let now = chrono::Local::now();
      let time_str = now.format("%H:%M:%S");
      lines.push(format!(
        "{} {}",
        self.colored(&format!("Finished at {time_str}"), Color::Green),
        self.colored(
          &format!("after {}", self.format_duration(duration)),
          Color::Green
        )
      ));
    }

    lines
  }

  fn render_table_summary(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let downloading = state.full_summary.running_downloads.len();
    let uploading = state.full_summary.running_uploads.len();

    if running > 0
      || completed > 0
      || failed > 0
      || planned > 0
      || downloading > 0
      || uploading > 0
    {
      // Group builds by host
      let mut host_builds: std::collections::HashMap<
        String,
        (usize, usize, usize),
      > = std::collections::HashMap::new();

      for build in state.full_summary.running_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_builds.entry(host).or_insert((0, 0, 0));
        entry.0 += 1;
      }

      for build in state.full_summary.completed_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_builds.entry(host).or_insert((0, 0, 0));
        entry.1 += 1;
      }

      for build in state.full_summary.failed_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_builds.entry(host).or_insert((0, 0, 0));
        entry.2 += 1;
      }

      // Group downloads/uploads by host
      let mut host_transfers: std::collections::HashMap<
        String,
        (usize, usize),
      > = std::collections::HashMap::new();

      for transfer in state.full_summary.running_downloads.values() {
        let host = transfer.host.name().to_string();
        let entry = host_transfers.entry(host).or_insert((0, 0));
        entry.0 += 1;
      }

      for transfer in state.full_summary.running_uploads.values() {
        let host = transfer.host.name().to_string();
        let entry = host_transfers.entry(host).or_insert((0, 0));
        entry.1 += 1;
      }

      // Header
      if !host_builds.is_empty() || !host_transfers.is_empty() {
        lines.push(format!(
          "{} Builds          │ Host",
          self.colored("┏━━━", Color::Blue)
        ));

        // Show builds by host
        for (host, (run, done, fail)) in &host_builds {
          let mut parts = Vec::new();
          if *run > 0 {
            parts.push(format!("{} {}", self.colored("⏵", Color::Yellow), run));
          }
          if *done > 0 {
            parts.push(format!("{} {}", self.colored("✔", Color::Green), done));
          }
          if *fail > 0 {
            parts.push(format!("{} {}", self.colored("✗", Color::Red), fail));
          }

          let status = if parts.is_empty() {
            "       ".to_string()
          } else {
            parts.join(" │ ")
          };

          lines.push(format!(
            "{} {:14} │ {}",
            self.colored("┃", Color::Blue),
            status,
            host
          ));
        }

        // Show downloads by host
        for (host, (down, up)) in &host_transfers {
          let mut parts = Vec::new();
          if *down > 0 {
            parts.push(format!("{} {}", self.colored("↓", Color::Blue), down));
          }
          if *up > 0 {
            parts.push(format!("{} {}", self.colored("↑", Color::Green), up));
          }

          let status = if parts.is_empty() {
            "       ".to_string()
          } else {
            parts.join(" │ ")
          };

          lines.push(format!(
            "{} {:14} │ {}",
            self.colored("┃", Color::Blue),
            status,
            host
          ));
        }
      }

      // Summary line
      let duration = current_time() - state.start_time;

      lines.push(format!(
        "{} ∑ {} {} │ {} {} │ {} {} │ Finished after {}",
        self.colored("━", Color::Blue),
        self.colored("↓", Color::Blue),
        downloading,
        self.colored("↑", Color::Green),
        uploading,
        self.colored("⏸", Color::Grey),
        planned,
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_full_summary(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let downloading = state.full_summary.running_downloads.len();
    let uploading = state.full_summary.running_uploads.len();

    if running > 0
      || completed > 0
      || failed > 0
      || planned > 0
      || downloading > 0
      || uploading > 0
    {
      lines.push(self.colored(&"═".repeat(60), Color::Blue).clone());
      lines.push(format!("{} Build Summary", self.colored("┃", Color::Blue)));
      lines.push(self.colored(&"─".repeat(60), Color::Blue).clone());

      // Builds section
      if running + completed + failed > 0 {
        lines.push(format!(
          "{} Builds:        {} {} running  {} {} completed  {} {} failed",
          self.colored("┃", Color::Blue),
          self.colored("⏵", Color::Yellow),
          running,
          self.colored("✔", Color::Green),
          completed,
          self.colored("✗", Color::Red),
          failed
        ));
      }

      // Planned section
      if planned > 0 {
        lines.push(format!(
          "{} Planned:       {} {} waiting",
          self.colored("┃", Color::Blue),
          self.colored("⏸", Color::Grey),
          planned
        ));
      }

      // Transfers section
      if downloading + uploading > 0 {
        lines.push(format!(
          "{} Transfers:     {} {} downloading  {} {} uploading",
          self.colored("┃", Color::Blue),
          self.colored("↓", Color::Blue),
          downloading,
          self.colored("↑", Color::Green),
          uploading
        ));
      }

      // Duration
      let duration = current_time() - state.start_time;
      lines.push(format!(
        "{} Duration:      {} {}",
        self.colored("┃", Color::Blue),
        self.colored("⏱", Color::Grey),
        self.format_duration(duration)
      ));

      lines.push(self.colored(&"═".repeat(60), Color::Blue).clone());
    }

    lines
  }

  fn render_legend(&self, state: &State, has_tree: bool) -> Vec<String> {
    match self.config.legend_style {
      LegendStyle::Compact => self.render_compact_legend(state, has_tree),
      LegendStyle::Table => self.render_table_legend(state, has_tree),
      LegendStyle::Verbose => self.render_verbose_legend(state, has_tree),
    }
  }

  fn render_compact_legend(
    &self,
    state: &State,
    has_tree: bool,
  ) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();

    if running > 0 || completed > 0 || failed > 0 || planned > 0 {
      let duration = current_time() - state.start_time;
      let prefix = if has_tree { "━" } else { "┏━" };
      lines.push(format!(
        "{} {} {running} │ {} {completed} │ {} {failed} │ {} {planned} │ {} {}",
        self.colored(prefix, Color::Blue),
        self.colored("⏵", Color::Yellow),
        self.colored("✔", Color::Green),
        self.colored("✗", Color::Red),
        self.colored("⏸", Color::Grey),
        self.colored("⏱", Color::Grey),
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_table_legend(&self, state: &State, has_tree: bool) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();

    if running > 0 || completed > 0 || failed > 0 || planned > 0 {
      let duration = current_time() - state.start_time;

      // Group by host
      let mut host_counts: HashMap<String, (usize, usize, usize, usize)> =
        HashMap::new();

      for build in state.full_summary.running_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_counts.entry(host).or_insert((0, 0, 0, 0));
        entry.0 += 1;
      }

      for build in state.full_summary.completed_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_counts.entry(host).or_insert((0, 0, 0, 0));
        entry.1 += 1;
      }

      for build in state.full_summary.failed_builds.values() {
        let host = build.host.name().to_string();
        let entry = host_counts.entry(host).or_insert((0, 0, 0, 0));
        entry.2 += 1;
      }

      // Add separator if this follows a tree, otherwise header
      let header_prefix = if has_tree { "┣━━━" } else { "┏━" };
      lines.push(format!(
        "{} Builds",
        self.colored(header_prefix, Color::Blue)
      ));

      // Summary line
      let summary_prefix = if has_tree { "┗━" } else { "━" };
      lines.push(format!(
        "{} ∑ {} {} │ {} {} │ {} {} │ {} {} │ {} {}",
        self.colored(summary_prefix, Color::Blue),
        self.colored("⏵", Color::Yellow),
        running,
        self.colored("✔", Color::Green),
        completed,
        self.colored("✗", Color::Red),
        failed,
        self.colored("⏸", Color::Grey),
        planned,
        self.colored("⏱", Color::Grey),
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_verbose_legend(
    &self,
    state: &State,
    has_tree: bool,
  ) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();

    if running > 0 || completed > 0 || failed > 0 || planned > 0 {
      let prefix = if has_tree { "┣━━━" } else { "┏━" };
      lines.push(format!(
        "{} Build Summary:",
        self.colored(prefix, Color::Blue)
      ));
      lines.push(format!(
        "┃    {} Running: {running}",
        self.colored("⏵", Color::Yellow)
      ));
      lines.push(format!(
        "┃    {} Completed: {completed}",
        self.colored("✔", Color::Green)
      ));
      if failed > 0 {
        lines.push(format!(
          "┃    {} Failed: {failed}",
          self.colored("✗", Color::Red)
        ));
      }
      lines.push(format!(
        "┃    {} Planned: {planned}",
        self.colored("⏸", Color::Grey)
      ));

      let duration = current_time() - state.start_time;
      lines.push(format!(
        "{} Total time: {}",
        self.colored("━", Color::Blue),
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_plain_view(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    let running = state.full_summary.running_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let downloading = state.full_summary.running_downloads.len();
    let uploading = state.full_summary.running_uploads.len();
    let duration = current_time() - state.start_time;

    // Always show progress line with activity counts
    let mut progress_parts = Vec::new();

    if planned > 0 {
      progress_parts.push(format!(
        "{} {} planned",
        self.colored("⏸", Color::Grey),
        planned
      ));
    }
    if downloading > 0 {
      progress_parts.push(format!(
        "{} {} downloading",
        self.colored("↓", Color::Blue),
        downloading
      ));
    }
    if uploading > 0 {
      progress_parts.push(format!(
        "{} {} uploading",
        self.colored("↑", Color::Green),
        uploading
      ));
    }

    // Always show progress line, even if empty
    if running > 0 || planned > 0 || downloading > 0 || uploading > 0 {
      let progress_line = if progress_parts.is_empty() {
        format!(
          "{} {} {}",
          self.colored("━", Color::Blue),
          self.colored("⏱", Color::Grey),
          self.format_duration(duration)
        )
      } else {
        format!(
          "{} {} {} {}",
          self.colored("━", Color::Blue),
          self.colored("⏱", Color::Grey),
          progress_parts.join(" "),
          self.format_duration(duration)
        )
      };
      lines.push(progress_line);
    }

    // Show downloads
    for (path_id, transfer) in &state.full_summary.running_downloads {
      if let Some(path_info) = state.store_path_infos.get(path_id) {
        let name = &path_info.name.name;
        let size = if let Some(total) = transfer.total_bytes {
          self.format_bytes(transfer.bytes_transferred, total)
        } else {
          format!("{} B", transfer.bytes_transferred)
        };
        lines.push(format!(
          "  {} {} {}",
          self.colored("↓", Color::Blue),
          name,
          size
        ));
      }
    }

    // Show uploads
    for (path_id, transfer) in &state.full_summary.running_uploads {
      if let Some(path_info) = state.store_path_infos.get(path_id) {
        let name = &path_info.name.name;
        let size = if let Some(total) = transfer.total_bytes {
          self.format_bytes(transfer.bytes_transferred, total)
        } else {
          format!("{} B", transfer.bytes_transferred)
        };
        lines.push(format!(
          "  {} {} {}",
          self.colored("↑", Color::Green),
          name,
          size
        ));
      }
    }

    // Show running builds
    for (drv_id, build) in &state.full_summary.running_builds {
      if let Some(info) = state.get_derivation_info(*drv_id) {
        let name = &info.name.name;
        let elapsed = current_time() - build.start;

        // Format time info
        let mut time_info = String::new();
        if let Some(estimate_secs) = build.estimate {
          let remaining = estimate_secs.saturating_sub(elapsed as u64);
          time_info.push_str(&format!(
            "∅ {} ",
            self.format_duration(remaining as f64)
          ));
        }
        time_info.push_str(&self.format_duration(elapsed));

        lines.push(format!(
          "  {} {} {}",
          self.colored("⏵", Color::Yellow),
          name,
          time_info
        ));
      }
    }

    lines
  }

  fn render_dashboard_view(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    // Get primary build (first root or first running build)
    let primary_build = state
      .forest_roots
      .first()
      .and_then(|&id| state.get_derivation_info(id));

    if let Some(build_info) = primary_build {
      let name = &build_info.name.name;
      lines.push(format!("BUILD GRAPH: {name}"));
      lines.push("─".repeat(44));

      // Get host information from running/completed builds
      let host = if let Some((_, build)) =
        state.full_summary.running_builds.iter().next()
      {
        build.host.name()
      } else if let Some((_, build)) =
        state.full_summary.completed_builds.iter().next()
      {
        build.host.name()
      } else {
        "localhost"
      };

      // Determine status
      let running = state.full_summary.running_builds.len();
      let completed = state.full_summary.completed_builds.len();
      let failed = state.full_summary.failed_builds.len();

      let status = if running > 0 {
        format!("{} building", self.colored("⏵", Color::Yellow))
      } else if failed > 0 {
        format!("{} failed", self.colored("✗", Color::Red))
      } else if completed > 0 {
        format!("{} success", self.colored("✔", Color::Green))
      } else {
        format!("{} planned", self.colored("⏸", Color::Grey))
      };

      // Duration
      let duration = current_time() - state.start_time;

      // Format dashboard
      lines.push(format!("Host        │ {host}"));
      lines.push(format!("Status      │ {status}"));
      lines.push(format!("Duration    │ {}", self.format_duration(duration)));
      lines.push("─".repeat(44));

      // Summary stats
      let total_jobs = running + completed + failed;
      lines.push(format!(
        "Summary     │ jobs={}  ok={}  failed={}  total={}",
        total_jobs,
        completed,
        failed,
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_dashboard_final(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    // Get primary build
    let primary_build = state
      .forest_roots
      .first()
      .and_then(|&id| state.get_derivation_info(id));

    if let Some(build_info) = primary_build {
      let name = &build_info.name.name;
      lines.push(format!("BUILD GRAPH: {name}"));
      lines.push("─".repeat(44));

      // Get host from build reports or completed builds
      let host = if let Some((_, builds)) = state.build_reports.iter().next() {
        if let Some(report) = builds.first() {
          &report.host
        } else {
          "localhost"
        }
      } else if let Some((_, build)) =
        state.full_summary.completed_builds.iter().next()
      {
        build.host.name()
      } else {
        "localhost"
      };

      let completed = state.full_summary.completed_builds.len();
      let failed = state.full_summary.failed_builds.len();

      let status = if failed > 0 {
        format!("{} failed", self.colored("✗", Color::Red))
      } else if completed > 0 {
        format!("{} success", self.colored("✔", Color::Green))
      } else {
        "unknown".to_string()
      };

      let duration = current_time() - state.start_time;

      lines.push(format!("Host        │ {host}"));
      lines.push(format!("Status      │ {status}"));
      lines.push(format!("Duration    │ {}", self.format_duration(duration)));
      lines.push("─".repeat(44));

      let total_jobs = completed + failed;
      lines.push(format!(
        "Summary     │ jobs={}  ok={}  failed={}  total={}",
        total_jobs,
        completed,
        failed,
        self.format_duration(duration)
      ));
    }

    lines
  }

  fn render_tree_view(&self, state: &State) -> Vec<String> {
    let mut lines = Vec::new();

    // Filter roots to only show those that are actively building
    let active_roots: Vec<DerivationId> = state
      .forest_roots
      .iter()
      .copied()
      .filter(|&drv_id| {
        if let Some(info) = state.get_derivation_info(drv_id) {
          matches!(
            info.build_status,
            BuildStatus::Building(_) | BuildStatus::Failed { .. }
          )
        } else {
          false
        }
      })
      .collect();

    if active_roots.is_empty() {
      return lines;
    }

    let forest = self.build_active_forest(state, &active_roots);

    if forest.is_empty() {
      return lines;
    }

    // Add header as first line
    lines.push(format!(
      "{} Dependency Graph:",
      self.colored("┏━", Color::Blue)
    ));

    // Render each root with its tree
    for node in &forest {
      self.render_tree_node(state, node, &mut lines);
    }

    lines
  }

  fn build_active_forest(
    &self,
    state: &State,
    roots: &[DerivationId],
  ) -> Vec<TreeNode> {
    let mut forest = Vec::new();
    let mut visited = HashSet::new();

    for &root_id in roots {
      if let Some(node) = self.build_active_node(state, root_id, &mut visited) {
        forest.push(node);
      }
    }

    forest
  }

  fn build_active_node(
    &self,
    state: &State,
    drv_id: DerivationId,
    visited: &mut HashSet<DerivationId>,
  ) -> Option<TreeNode> {
    if visited.contains(&drv_id) {
      return None;
    }
    visited.insert(drv_id);

    let drv_info = state.get_derivation_info(drv_id)?;

    // Only include actively building or failed children
    let mut children = Vec::new();
    for input in &drv_info.input_derivations {
      let child_info = state.get_derivation_info(input.derivation)?;

      // Only show children that are actively building or failed
      let should_show = matches!(
        child_info.build_status,
        BuildStatus::Building(_) | BuildStatus::Failed { .. }
      );

      if should_show {
        if let Some(child) =
          self.build_active_node(state, input.derivation, visited)
        {
          children.push(child);
        }
      }
    }

    Some(TreeNode { drv_id, children })
  }

  fn render_tree_node(
    &self,
    state: &State,
    node: &TreeNode,
    lines: &mut Vec<String>,
  ) {
    let info = match state.get_derivation_info(node.drv_id) {
      Some(info) => info,
      None => return,
    };

    // Render children first (so they appear above root)
    for (i, child) in node.children.iter().enumerate() {
      let is_last = i == node.children.len() - 1;
      self.render_tree_child(state, child, lines, is_last, "┃ ");
    }

    // Then render the root node at bottom
    let mut line = String::new();
    line.push_str(&self.colored("┃ ", Color::Blue));

    // Status icon
    let (icon, color) = self.get_status_icon(&info.build_status);
    line.push_str(&self.colored(icon, color));
    line.push(' ');

    // Package name
    line.push_str(&self.truncate_name(&info.name.name, 50));

    // Phase information
    if let BuildStatus::Building(build_info) = &info.build_status {
      if let Some(activity_id) = build_info.activity_id {
        if let Some(activity) = state.activities.get(&activity_id) {
          if let Some(phase) = &activity.phase {
            line
              .push_str(&self.colored(&format!(" ({phase})"), Color::DarkGrey));
          }
        }
      }

      // Time information
      let elapsed = current_time() - build_info.start;

      // Show estimate if available
      if let Some(estimate_secs) = build_info.estimate {
        let remaining = estimate_secs.saturating_sub(elapsed as u64);
        line.push_str(&self.colored(
          &format!(" ∅ {}", self.format_duration(remaining as f64)),
          Color::DarkGrey,
        ));
      }

      // Show elapsed time
      line.push_str(&self.colored(
        &format!(" ⏱ {}", self.format_duration(elapsed)),
        Color::DarkGrey,
      ));
    }

    lines.push(line);
  }

  fn render_tree_child(
    &self,
    state: &State,
    node: &TreeNode,
    lines: &mut Vec<String>,
    is_last_child: bool,
    prefix: &str,
  ) {
    let info = match state.get_derivation_info(node.drv_id) {
      Some(info) => info,
      None => return,
    };

    // Render this node's children FIRST (they go above)
    for (i, child) in node.children.iter().enumerate() {
      let is_last = i == node.children.len() - 1;
      let child_prefix = if is_last_child {
        format!("{prefix}   ")
      } else {
        format!("{prefix}│  ")
      };

      self.render_tree_child(state, child, lines, is_last, &child_prefix);
    }

    // Then render this node
    let mut line = String::new();
    line.push_str(prefix);

    let connector = if is_last_child { "└─ " } else { "├─ " };
    line.push_str(&self.colored(connector, Color::Blue));

    let (icon, color) = self.get_status_icon(&info.build_status);
    line.push_str(&self.colored(icon, color));
    line.push(' ');
    line.push_str(&self.truncate_name(&info.name.name, 50));

    lines.push(line);
  }

  const fn get_status_icon(&self, status: &BuildStatus) -> (&str, Color) {
    match status {
      BuildStatus::Building(_) => ("⏵", Color::Yellow),
      BuildStatus::Planned => ("⏸", Color::Grey),
      BuildStatus::Built { .. } => ("✔", Color::Green),
      BuildStatus::Failed { .. } => ("✗", Color::Red),
      BuildStatus::Unknown => ("?", Color::Grey),
    }
  }

  fn colored(&self, text: &str, color: Color) -> String {
    if self.config.use_color {
      format!("{}{}{}", SetForegroundColor(color), text, ResetColor)
    } else {
      text.to_string()
    }
  }

  pub fn format_duration(&self, secs: f64) -> String {
    if secs < 60.0 {
      format!("{secs:.0}s")
    } else if secs < 3600.0 {
      format!("{:.0}m{:.0}s", secs / 60.0, secs % 60.0)
    } else {
      format!("{:.0}h{:.0}m", secs / 3600.0, (secs % 3600.0) / 60.0)
    }
  }

  fn truncate_name(&self, name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
      name.to_string()
    } else {
      format!("{}…", &name[..max_len - 1])
    }
  }

  fn format_bytes(&self, bytes: u64, total: u64) -> String {
    let format_size = |b: u64| -> String {
      if b < 1024 {
        format!("{b} B")
      } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
      } else if b < 1024 * 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
      } else {
        format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0))
      }
    };

    if total > 0 {
      let percent = (bytes as f64 / total as f64) * 100.0;
      format!(
        "{}/{} ({:.0}%)",
        format_size(bytes),
        format_size(total),
        percent
      )
    } else {
      format_size(bytes)
    }
  }
}
