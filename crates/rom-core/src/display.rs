//! Display rendering for ROM
use std::{
  collections::HashSet,
  io::{self, Write},
};

use crossterm::{
  cursor,
  execute,
  style::{Color, ResetColor, SetForegroundColor},
};

use crate::{
  icons::Icons,
  state::{BuildStatus, DerivationId, State, current_time},
  types::{LegendStyle, SummaryStyle},
};

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

pub struct DisplayConfig {
  pub show_timers:       bool,
  pub max_tree_depth:    usize,
  pub max_visible_lines: usize,
  pub use_color:         bool,
  pub format:            crate::types::DisplayFormat,
  pub legend_style:      LegendStyle,
  pub summary_style:     SummaryStyle,
  pub icons:             &'static Icons,
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
      icons:             crate::icons::detect(),
    }
  }
}

pub struct Display<W: Write> {
  writer:            W,
  config:            DisplayConfig,
  /// Number of graph lines printed in the last render (cleared on next render)
  last_lines:        usize,
  /// Total log lines already printed (they scroll naturally, never cleared)
  printed_log_lines: usize,
}

struct TreeNode {
  drv_id:   DerivationId,
  children: Vec<Self>,
}

impl<W: Write> Display<W> {
  pub const fn new(writer: W, config: DisplayConfig) -> io::Result<Self> {
    Ok(Self {
      writer,
      config,
      last_lines: 0,
      printed_log_lines: 0,
    })
  }

  pub fn clear_previous(&mut self) -> io::Result<()> {
    if self.last_lines > 0 {
      // Move up in a single escape sequence, then clear to end of screen.
      // This is much cheaper than calling MoveUp(1) in a loop because it
      // produces one write + one flush instead of N.
      execute!(
        self.writer,
        cursor::MoveToColumn(0),
        cursor::MoveUp(self.last_lines as u16),
        cursor::MoveToColumn(0),
        crossterm::terminal::Clear(
          crossterm::terminal::ClearType::FromCursorDown
        )
      )?;
    }
    Ok(())
  }

  pub fn render(&mut self, state: &State, logs: &[String]) -> io::Result<()> {
    // Print any log lines that arrived since last render.
    // These are printed once and scroll up naturally, we never clear them.
    let new_logs = &logs[self.printed_log_lines.min(logs.len())..];
    if !new_logs.is_empty() {
      // Clear the current graph first so new logs appear above it
      self.clear_previous()?;
      let mut log_out = String::with_capacity(new_logs.len() * 80);
      for line in new_logs {
        log_out.push_str(line);
        log_out.push('\n');
      }
      self.writer.write_all(log_out.as_bytes())?;
      self.printed_log_lines = logs.len();
      self.last_lines = 0; // graph was cleared above
    }

    // Clear only the graph from the previous render
    self.clear_previous()?;

    // Build graph lines
    let mut graph_lines = match self.config.format {
      crate::types::DisplayFormat::Tree => {
        let tree_lines = self.render_tree_view(state);
        let has_tree = !tree_lines.is_empty();
        let mut g = tree_lines;
        g.extend(self.render_legend(state, has_tree));
        g
      },
      crate::types::DisplayFormat::Plain => self.render_plain_view(state),
      crate::types::DisplayFormat::Dashboard => {
        self.render_dashboard_view(state)
      },
    };

    if graph_lines.len() > self.config.max_visible_lines {
      graph_lines.truncate(self.config.max_visible_lines);
    }

    self.last_lines = graph_lines.len();

    let mut out = String::with_capacity(graph_lines.len() * 80);
    for line in &graph_lines {
      out.push_str(line);
      out.push('\n');
    }
    self.writer.write_all(out.as_bytes())?;
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
        // render_tree_view already includes its own header line; only extend if
        // there are actually active (building/failed) derivations to show
        let tree_lines = self.render_tree_view(state);
        lines.extend(tree_lines);
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
      SummaryStyle::Concise => self.render_finished_line(state),
      SummaryStyle::Table => self.render_table_summary(state),
      SummaryStyle::Full => self.render_full_summary(state),
    }
  }

  /// Final single-line summary. Matches NOM's finish markup:
  /// - success:  `Finished at HH:MM:SS after Xs  ✔ N`
  /// - failure:  `⚠ Exited after N build failures at HH:MM:SS after Xs`
  /// - errors:   `⚠ Exited with N nix errors at HH:MM:SS after Xs`
  fn render_finished_line(&self, state: &State) -> Vec<String> {
    let failed = state.full_summary.failed_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let nix_errors = state.nix_errors.len();
    let duration = current_time() - state.start_time;
    let now = chrono::Local::now();
    let at = now.format("%H:%M:%S");
    let dur = self.format_duration(duration);

    let ic = self.ic();
    let line = if failed > 0 {
      let noun = if failed == 1 { "failure" } else { "failures" };
      format!(
        "{} {} at {} after {}",
        self.colored(ic.failed, Color::DarkRed),
        self.colored(
          &format!("Exited after {failed} build {noun}"),
          Color::DarkRed
        ),
        self.colored(&at.to_string(), Color::DarkRed),
        self.colored(&dur, Color::DarkRed),
      )
    } else if nix_errors > 0 {
      let noun = if nix_errors == 1 { "error" } else { "errors" };
      format!(
        "{} {} at {} after {}",
        self.colored(ic.failed, Color::DarkRed),
        self.colored(
          &format!("Exited with {nix_errors} nix {noun}"),
          Color::DarkRed
        ),
        self.colored(&at.to_string(), Color::DarkRed),
        self.colored(&dur, Color::DarkRed),
      )
    } else {
      let mut s = format!(
        "{} after {}",
        self.colored(&format!("Finished at {at}"), Color::DarkGreen),
        self.colored(&dur, Color::DarkGreen),
      );
      if completed > 0 {
        s.push_str(&format!(
          "  {} {completed}",
          self.colored(ic.done, Color::DarkGreen)
        ));
      }
      s
    };

    vec![line]
  }

  fn render_table_summary(&self, state: &State) -> Vec<String> {
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let dl_done = state.full_summary.completed_downloads.len();
    let ul_done = state.full_summary.completed_uploads.len();
    let duration = current_time() - state.start_time;
    let now = chrono::Local::now();
    let at = now.format("%H:%M:%S");
    let dur = self.format_duration(duration);

    if completed + failed + dl_done + ul_done == 0 {
      return self.render_finished_line(state);
    }

    // Collect host breakdown
    let mut host_map: std::collections::HashMap<String, (usize, usize)> =
      std::collections::HashMap::new();
    for b in state.full_summary.completed_builds.values() {
      host_map.entry(b.host.name().to_string()).or_default().0 += 1;
    }
    for b in state.full_summary.failed_builds.values() {
      host_map.entry(b.host.name().to_string()).or_default().1 += 1;
    }
    let many_hosts = host_map.len() > 1;

    let mut lines = Vec::new();

    // Header
    let mut hdr_parts = Vec::new();
    if completed + failed > 0 {
      hdr_parts.push("Builds");
    }
    if dl_done > 0 {
      hdr_parts.push("Downloads");
    }
    if ul_done > 0 {
      hdr_parts.push("Uploads");
    }
    let ic = self.ic();
    lines.push(format!(
      "{} {}",
      self.colored("┏━━━", Color::DarkBlue),
      hdr_parts.join("  ")
    ));

    // Per-host rows when multiple hosts
    if many_hosts {
      let mut hosts: Vec<_> = host_map.keys().cloned().collect();
      hosts.sort();
      for host in &hosts {
        let (done, fail) = host_map[host];
        let mut parts = Vec::new();
        if done > 0 {
          parts.push(format!(
            "{} {done}",
            self.colored(ic.done, Color::DarkGreen)
          ));
        }
        if fail > 0 {
          parts.push(format!(
            "{} {fail}",
            self.colored(ic.failed, Color::DarkRed)
          ));
        }
        lines.push(format!(
          "{}  {}  {}",
          self.colored("┃", Color::DarkBlue),
          parts.join("  "),
          self.colored(host, Color::DarkMagenta),
        ));
      }
    }

    // Final ∑ line
    let mut sum_parts = Vec::new();
    if completed > 0 {
      sum_parts.push(format!(
        "{} {completed}",
        self.colored(ic.done, Color::DarkGreen)
      ));
    }
    if failed > 0 {
      sum_parts.push(format!(
        "{} {failed}",
        self.colored(ic.failed, Color::DarkRed)
      ));
    }
    if dl_done > 0 {
      sum_parts.push(format!(
        "{} {dl_done}",
        self.colored(ic.download, Color::DarkGreen)
      ));
    }
    if ul_done > 0 {
      sum_parts.push(format!(
        "{} {ul_done}",
        self.colored(ic.upload, Color::DarkGreen)
      ));
    }

    let finish = if failed > 0 || !state.nix_errors.is_empty() {
      self.colored(&format!("Exited at {at} after {dur}"), Color::DarkRed)
    } else {
      self.colored(&format!("Finished at {at} after {dur}"), Color::DarkGreen)
    };
    sum_parts.push(finish);

    lines.push(format!(
      "{} ∑ {}",
      self.colored("┗━", Color::DarkBlue),
      sum_parts.join("  │  ")
    ));

    lines
  }

  fn render_full_summary(&self, state: &State) -> Vec<String> {
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let dl_done = state.full_summary.completed_downloads.len();
    let dl_running = state.full_summary.running_downloads.len();
    let ul_done = state.full_summary.completed_uploads.len();
    let ul_running = state.full_summary.running_uploads.len();
    let duration = current_time() - state.start_time;
    let now = chrono::Local::now();
    let at = now.format("%H:%M:%S");

    let v = self.colored("┃", Color::DarkBlue);

    let mut lines = Vec::new();
    lines.push(format!(
      "{} Build Summary",
      self.colored("┏━━━", Color::DarkBlue)
    ));

    let ic = self.ic();
    if completed > 0 || failed > 0 {
      let mut bp = Vec::new();
      if completed > 0 {
        bp.push(format!(
          "{} {completed} built",
          self.colored(ic.done, Color::DarkGreen)
        ));
      }
      if failed > 0 {
        bp.push(format!(
          "{} {failed} failed",
          self.colored(ic.failed, Color::DarkRed)
        ));
      }
      lines.push(format!("{}  Builds:     {}", v, bp.join("  ")));
    }

    let total_dl = dl_done + dl_running;
    let total_ul = ul_done + ul_running;
    if total_dl > 0 {
      lines.push(format!(
        "{}  Downloads:  {} fetched",
        v,
        self.colored(&total_dl.to_string(), Color::DarkGreen)
      ));
    }
    if total_ul > 0 {
      lines.push(format!(
        "{}  Uploads:    {} pushed",
        v,
        self.colored(&total_ul.to_string(), Color::DarkGreen)
      ));
    }

    if !state.nix_errors.is_empty() {
      lines.push(format!(
        "{}  {} {} nix error(s)",
        v,
        self.colored(ic.failed, Color::DarkRed),
        state.nix_errors.len()
      ));
    }

    let finish_label = if failed > 0 || !state.nix_errors.is_empty() {
      self.colored(&format!("Exited at {at}"), Color::DarkRed)
    } else {
      self.colored(&format!("Finished at {at}"), Color::DarkGreen)
    };
    lines.push(format!(
      "{} {} after {}",
      self.colored("┗━", Color::DarkBlue),
      finish_label,
      self.colored(&self.format_duration(duration), Color::DarkGrey),
    ));

    lines
  }

  fn render_legend(&self, state: &State, has_tree: bool) -> Vec<String> {
    match self.config.legend_style {
      LegendStyle::Compact => self.render_compact_legend(state),
      LegendStyle::Table => self.render_table_legend(state, has_tree),
      LegendStyle::Verbose => self.render_verbose_legend(state, has_tree),
    }
  }

  fn render_compact_legend(&self, state: &State) -> Vec<String> {
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let dl = state.full_summary.running_downloads.len();
    let ul = state.full_summary.running_uploads.len();

    if running + completed + failed + planned + dl + ul == 0 {
      return vec![];
    }

    let duration = current_time() - state.start_time;
    let ic = self.ic();

    // Always emit ⏵ │ ✔ │ ✗ │ ⏸, dim zeros
    let mut parts: Vec<String> = Vec::new();
    parts.push(self.count_colored(ic.running, running, Color::DarkYellow));
    parts.push(self.count_colored(ic.done, completed, Color::DarkGreen));
    parts.push(self.count_colored(ic.failed, failed, Color::DarkRed));
    parts.push(self.count_colored(ic.planned, planned, Color::DarkBlue));
    if dl > 0 {
      parts.push(format!(
        "{} {dl}",
        self.colored(ic.download, Color::DarkYellow)
      ));
    }
    if ul > 0 {
      parts.push(format!(
        "{} {ul}",
        self.colored(ic.upload, Color::DarkYellow)
      ));
    }
    parts.push(format!(
      "{} {}",
      self.colored(ic.clock, Color::DarkGrey),
      self.colored(&self.format_duration(duration), Color::DarkGrey),
    ));

    vec![format!(
      "{} {}",
      self.colored("┗━", Color::DarkBlue),
      parts.join(" │ ")
    )]
  }

  fn render_table_legend(&self, state: &State, has_tree: bool) -> Vec<String> {
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let dl_running = state.full_summary.running_downloads.len();
    let dl_done = state.full_summary.completed_downloads.len();
    let ul_running = state.full_summary.running_uploads.len();
    let ul_done = state.full_summary.completed_uploads.len();

    let show_builds = running + completed + failed + planned > 0;
    let show_dl = dl_running + dl_done > 0;
    let show_ul = ul_running + ul_done > 0;

    if !show_builds && !show_dl && !show_ul {
      return vec![];
    }

    let now = current_time();
    let duration = now - state.start_time;
    let v = self.colored("┃", Color::DarkBlue);

    // Build header section label(s)
    let mut header_parts: Vec<&str> = Vec::new();
    if show_builds {
      header_parts.push("Builds");
    }
    if show_dl {
      header_parts.push("Downloads");
    }
    if show_ul {
      header_parts.push("Uploads");
    }

    let mut lines = Vec::new();

    // ┏━━━ header (or ┣━━━ when appended below a tree)
    let header_prefix = if has_tree {
      "┣━━━"
    } else {
      "┏━━━"
    };
    lines.push(format!(
      "{} {}",
      self.colored(header_prefix, Color::DarkBlue),
      header_parts.join("  ")
    ));

    // Per-running-build rows
    let mut running_entries: Vec<(String, f64, String)> = state
      .full_summary
      .running_builds
      .iter()
      .filter_map(|(drv_id, build)| {
        let info = state.get_derivation_info(*drv_id)?;
        let elapsed = now - build.start;
        let host_label = match &build.host {
          cognos::Host::Remote(h) => {
            format!("  on {}", self.colored(h, Color::DarkMagenta))
          },
          _ => String::new(),
        };
        Some((info.name.name.clone(), elapsed, host_label))
      })
      .collect();
    // Longest running first
    running_entries.sort_by(|a, b| {
      b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });

    let dl_name_width = state
      .full_summary
      .running_downloads
      .keys()
      .filter_map(|id| {
        state.store_path_infos.get(id).map(|pi| pi.name.name.len())
      })
      .max()
      .unwrap_or(0);

    let name_width = running_entries
      .iter()
      .map(|(n, ..)| n.len())
      .chain(std::iter::once(dl_name_width))
      .max()
      .unwrap_or(0)
      .min(48);

    // Show per-item rows only when not already shown in the tree above.
    // When has_tree=true the active builds are visible there; the legend
    // only needs to supply the ∑ summary line.
    if !has_tree {
      let ic = self.ic();
      for (name, elapsed, host_label) in &running_entries {
        lines.push(format!(
          "{}  {} {:<width$}  {} {}{}",
          v,
          self.colored(ic.running, Color::DarkYellow),
          self.truncate_name(name, name_width),
          self.colored(ic.clock, Color::DarkGrey),
          self.colored(&self.format_duration(*elapsed), Color::DarkGrey),
          host_label,
          width = name_width,
        ));
      }

      // Per-running-download rows
      for (path_id, transfer) in &state.full_summary.running_downloads {
        if let Some(pi) = state.store_path_infos.get(path_id) {
          let elapsed = now - transfer.start;
          let size_str = if let Some(total) = transfer.total_bytes {
            self.format_bytes(transfer.bytes_transferred, total)
          } else {
            format!("{} B", transfer.bytes_transferred)
          };
          lines.push(format!(
            "{}  {} {:<width$}  {} {} {}",
            v,
            self.colored(ic.download, Color::DarkYellow),
            self.truncate_name(&pi.name.name, name_width),
            self.colored(&size_str, Color::DarkGrey),
            self.colored(ic.clock, Color::DarkGrey),
            self.colored(&self.format_duration(elapsed), Color::DarkGrey),
            width = name_width,
          ));
        }
      }

      // Per-running-upload rows
      for (path_id, transfer) in &state.full_summary.running_uploads {
        if let Some(pi) = state.store_path_infos.get(path_id) {
          let elapsed = now - transfer.start;
          lines.push(format!(
            "{}  {} {:<width$}  {} {}",
            v,
            self.colored(ic.upload, Color::DarkYellow),
            self.truncate_name(&pi.name.name, name_width),
            self.colored(ic.clock, Color::DarkGrey),
            self.colored(&self.format_duration(elapsed), Color::DarkGrey),
            width = name_width,
          ));
        }
      }
    }

    // ∑ row: always emit all three build-state columns (NOM behaviour —
    // counts are shown even when zero, just dimmed to grey).
    let ic = self.ic();
    let mut sum_parts: Vec<String> = Vec::new();
    if show_builds {
      sum_parts.push(self.count_colored(
        ic.running,
        running,
        Color::DarkYellow,
      ));
      sum_parts.push(self.count_colored(ic.done, completed, Color::DarkGreen));
      sum_parts.push(self.count_colored(ic.failed, failed, Color::DarkRed));
      sum_parts.push(self.count_colored(ic.planned, planned, Color::DarkBlue));
    }
    if show_dl {
      // Two sub-columns: running (yellow) and done (green)
      if dl_running > 0 || dl_done > 0 {
        sum_parts.push(format!(
          "{} {}",
          self.colored(ic.download, Color::DarkGrey),
          [
            (dl_running > 0).then(|| {
              self.count_colored(ic.running, dl_running, Color::DarkYellow)
            }),
            (dl_done > 0)
              .then(|| self.count_colored(ic.done, dl_done, Color::DarkGreen)),
          ]
          .into_iter()
          .flatten()
          .collect::<Vec<_>>()
          .join(" "),
        ));
      }
    }
    if show_ul {
      if ul_running > 0 || ul_done > 0 {
        sum_parts.push(format!(
          "{} {}",
          self.colored(ic.upload, Color::DarkGrey),
          [
            (ul_running > 0).then(|| {
              self.count_colored(ic.running, ul_running, Color::DarkYellow)
            }),
            (ul_done > 0)
              .then(|| self.count_colored(ic.done, ul_done, Color::DarkGreen)),
          ]
          .into_iter()
          .flatten()
          .collect::<Vec<_>>()
          .join(" "),
        ));
      }
    }
    // Elapsed with clock icon
    sum_parts.push(format!(
      "{} {}",
      self.colored(ic.clock, Color::DarkGrey),
      self.colored(&self.format_duration(duration), Color::DarkGrey),
    ));

    // ┗━ ∑  [summary]
    lines.push(format!(
      "{} {} {}",
      self.colored("┗━", Color::DarkBlue),
      self.colored(ic.summary, Color::DarkGrey),
      sum_parts.join(" │ ")
    ));

    lines
  }

  fn render_verbose_legend(
    &self,
    state: &State,
    has_tree: bool,
  ) -> Vec<String> {
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let dl_running = state.full_summary.running_downloads.len();
    let ul_running = state.full_summary.running_uploads.len();

    if running + completed + failed + planned + dl_running + ul_running == 0 {
      return vec![];
    }

    let now = current_time();
    let duration = now - state.start_time;
    let prefix = if has_tree {
      "┣━━━"
    } else {
      "┏━━━"
    };
    let v = self.colored("┃", Color::DarkBlue);

    let mut lines = Vec::new();
    lines.push(format!(
      "{} Build Summary:",
      self.colored(prefix, Color::DarkBlue)
    ));

    // One row per running build: name left-aligned, time right
    let mut running_entries: Vec<(String, String, String)> = state
      .full_summary
      .running_builds
      .iter()
      .filter_map(|(drv_id, build)| {
        let info = state.get_derivation_info(*drv_id)?;
        let elapsed = now - build.start;
        let host = match &build.host {
          cognos::Host::Localhost => String::new(),
          cognos::Host::Remote(h) => {
            format!("  {}", self.colored(h, Color::DarkMagenta))
          },
        };
        Some((info.name.name.clone(), self.format_duration(elapsed), host))
      })
      .collect();
    running_entries.sort_by(|a, b| a.0.cmp(&b.0));

    let name_width = running_entries
      .iter()
      .map(|(n, ..)| n.len())
      .max()
      .unwrap_or(0)
      .min(48);

    let ic = self.ic();
    for (name, elapsed, host) in &running_entries {
      lines.push(format!(
        "{}  {} {:<width$}  {}{}",
        v,
        self.colored(ic.running, Color::DarkYellow),
        self.truncate_name(name, name_width),
        self.colored(elapsed, Color::DarkGrey),
        host,
        width = name_width,
      ));
    }

    // Running downloads
    for (path_id, transfer) in &state.full_summary.running_downloads {
      if let Some(pi) = state.store_path_infos.get(path_id) {
        let elapsed = now - transfer.start;
        let size = if let Some(total) = transfer.total_bytes {
          self.format_bytes(transfer.bytes_transferred, total)
        } else {
          format!("{} B", transfer.bytes_transferred)
        };
        lines.push(format!(
          "{}  {} {:<width$}  {} {}",
          v,
          self.colored(ic.download, Color::DarkYellow),
          self.truncate_name(&pi.name.name, name_width),
          self.colored(&size, Color::DarkGrey),
          self.colored(&self.format_duration(elapsed), Color::DarkGrey),
          width = name_width,
        ));
      }
    }

    let ic = self.ic();
    let mut sum_parts: Vec<String> = Vec::new();
    sum_parts.push(format!(
      "{} {running} running",
      self.colored(ic.running, Color::DarkYellow)
    ));
    sum_parts.push(format!(
      "{} {completed} completed",
      self.colored(ic.done, Color::DarkGreen)
    ));
    sum_parts.push(format!(
      "{} {failed} failed",
      self.colored(ic.failed, Color::DarkRed)
    ));
    sum_parts.push(format!(
      "{} {planned} planned",
      self.colored(ic.planned, Color::DarkBlue)
    ));
    if dl_running > 0 {
      sum_parts.push(format!(
        "{} {dl_running} downloading",
        self.colored(ic.download, Color::DarkYellow)
      ));
    }
    if ul_running > 0 {
      sum_parts.push(format!(
        "{} {ul_running} uploading",
        self.colored(ic.upload, Color::DarkYellow)
      ));
    }
    sum_parts.push(format!(
      "{} {}",
      self.colored(ic.clock, Color::DarkGrey),
      self.colored(&self.format_duration(duration), Color::DarkGrey),
    ));

    lines.push(format!(
      "{} {}",
      self.colored("┗━", Color::DarkBlue),
      sum_parts.join(" │ ")
    ));

    lines
  }

  fn render_plain_view(&self, state: &State) -> Vec<String> {
    let now = current_time();
    let duration = now - state.start_time;
    let running = state.full_summary.running_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let downloading = state.full_summary.running_downloads.len();
    let uploading = state.full_summary.running_uploads.len();

    if running + planned + completed + downloading + uploading == 0 {
      return vec![];
    }

    let mut lines = Vec::new();

    // Running builds
    let mut builds: Vec<_> = state
      .full_summary
      .running_builds
      .iter()
      .filter_map(|(drv_id, build)| {
        let info = state.get_derivation_info(*drv_id)?;
        Some((info.name.name.clone(), build.clone()))
      })
      .collect();
    builds.sort_by(|a, b| a.0.cmp(&b.0));

    let ic = self.ic();

    let mut header_parts: Vec<String> = Vec::new();
    if planned > 0 {
      header_parts.push(format!(
        "{} {planned} planned",
        self.colored(ic.planned, Color::DarkBlue)
      ));
    }
    if downloading > 0 {
      header_parts.push(format!(
        "{} {downloading} downloading",
        self.colored(ic.download, Color::DarkYellow)
      ));
    }
    if uploading > 0 {
      header_parts.push(format!(
        "{} {uploading} uploading",
        self.colored(ic.upload, Color::DarkYellow)
      ));
    }
    let duration_str = self.format_duration(duration);
    let header = if header_parts.is_empty() {
      format!(
        "{} {} {}",
        self.colored("━", Color::DarkBlue),
        self.colored(ic.clock, Color::DarkGrey),
        self.colored(&duration_str, Color::DarkGrey),
      )
    } else {
      format!(
        "{} {} {} {}",
        self.colored("━", Color::DarkBlue),
        self.colored(ic.clock, Color::DarkGrey),
        header_parts.join(" "),
        self.colored(&duration_str, Color::DarkGrey),
      )
    };
    lines.push(header);

    for (name, build) in &builds {
      let elapsed = now - build.start;
      let mut suffix = String::new();
      if let Some(est) = build.estimate {
        let remaining = est.saturating_sub(elapsed as u64);
        suffix = format!(
          "  {} {}",
          self.colored(ic.estimate, Color::DarkGrey),
          self
            .colored(&self.format_duration(remaining as f64), Color::DarkGrey)
        );
      }
      let host_label = match &build.host {
        cognos::Host::Remote(h) => {
          format!("  {}", self.colored(h, Color::DarkMagenta))
        },
        _ => String::new(),
      };
      lines.push(format!(
        "  {} {}  {}{}{}",
        self.colored(ic.running, Color::DarkYellow),
        name,
        self.colored(&self.format_duration(elapsed), Color::DarkGrey),
        suffix,
        host_label,
      ));
    }

    // Running downloads
    for (path_id, transfer) in &state.full_summary.running_downloads {
      if let Some(pi) = state.store_path_infos.get(path_id) {
        let size = if let Some(total) = transfer.total_bytes {
          self.format_bytes(transfer.bytes_transferred, total)
        } else {
          format!("{} B", transfer.bytes_transferred)
        };
        lines.push(format!(
          "  {} {}  {}",
          self.colored(ic.download, Color::DarkYellow),
          pi.name.name,
          self.colored(&size, Color::DarkGrey),
        ));
      }
    }

    // Running uploads
    for (path_id, transfer) in &state.full_summary.running_uploads {
      if let Some(pi) = state.store_path_infos.get(path_id) {
        let size = if let Some(total) = transfer.total_bytes {
          self.format_bytes(transfer.bytes_transferred, total)
        } else {
          format!("{} B", transfer.bytes_transferred)
        };
        lines.push(format!(
          "  {} {}  {}",
          self.colored(ic.upload, Color::DarkYellow),
          pi.name.name,
          self.colored(&size, Color::DarkGrey),
        ));
      }
    }

    lines
  }

  fn render_dashboard_view(&self, state: &State) -> Vec<String> {
    let now = current_time();
    let duration = now - state.start_time;
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let dl = state.full_summary.running_downloads.len();
    let ul = state.full_summary.running_uploads.len();

    if running + completed + planned + failed + dl + ul == 0 {
      return vec![];
    }

    let ic = self.ic();
    let sep = self.colored(&"─".repeat(44), Color::DarkBlue);
    let pipe = self.colored("│", Color::DarkBlue);

    let title = state
      .forest_roots
      .first()
      .and_then(|&id| state.get_derivation_info(id))
      .map_or_else(|| "Build".to_string(), |info| info.name.name.clone());

    let host = state
      .full_summary
      .running_builds
      .values()
      .find_map(|b| {
        match &b.host {
          cognos::Host::Remote(h) => Some(h.clone()),
          _ => None,
        }
      })
      .unwrap_or_else(|| "localhost".to_string());

    let (status_icon, status_color, status_label) = if running > 0 {
      (ic.running, Color::DarkYellow, "building")
    } else if planned > 0 || dl > 0 {
      (ic.planned, Color::DarkBlue, "waiting")
    } else if failed > 0 {
      (ic.failed, Color::DarkRed, "failed")
    } else {
      (ic.done, Color::DarkGreen, "done")
    };
    let status_str =
      format!("{} {status_label}", self.colored(status_icon, status_color));

    let duration_str = self.format_duration(duration);
    let host_s = self.colored(&host, Color::DarkMagenta);
    let dur_s = self.colored(&duration_str, Color::DarkGrey);
    let fail_s = if failed > 0 && self.config.use_color {
      format!(
        "{}\x1b[1m{failed}\x1b[0m{}",
        SetForegroundColor(Color::DarkRed),
        ResetColor
      )
    } else {
      failed.to_string()
    };
    let summary_str = format!(
      "jobs={}  ok={}  failed={fail_s}  total={dur_s}",
      self.num_str(running + completed + planned + failed),
      self.num_str(completed),
    );

    let header = format!(
      "{} BUILD GRAPH: {title}",
      self.colored("┏━", Color::DarkBlue)
    );

    vec![
      header,
      sep.clone(),
      format!("{:<12} {pipe} {host_s}", "Host"),
      format!("{:<12} {pipe} {status_str}", "Status"),
      format!("{:<12} {pipe} {dur_s}", "Duration"),
      sep,
      format!("{:<12} {pipe} {summary_str}", "Summary"),
    ]
  }

  fn render_dashboard_final(&self, state: &State) -> Vec<String> {
    let duration = current_time() - state.start_time;
    let completed = state.full_summary.completed_builds.len();
    let failed = state.full_summary.failed_builds.len();
    let now = chrono::Local::now();
    let at = now.format("%H:%M:%S");

    let ic = self.ic();
    let sep = self.colored(&"─".repeat(44), Color::DarkBlue);
    let pipe = self.colored("│", Color::DarkBlue);

    let title = state
      .forest_roots
      .first()
      .and_then(|&id| state.get_derivation_info(id))
      .map_or_else(|| "Build".to_string(), |info| info.name.name.clone());

    let host = state
      .full_summary
      .completed_builds
      .values()
      .find_map(|b| {
        match &b.host {
          cognos::Host::Remote(h) => Some(h.clone()),
          _ => None,
        }
      })
      .or_else(|| {
        state.full_summary.failed_builds.values().find_map(|b| {
          match &b.host {
            cognos::Host::Remote(h) => Some(h.clone()),
            _ => None,
          }
        })
      })
      .unwrap_or_else(|| "localhost".to_string());

    let (status_icon, status_color, status_label) =
      if failed > 0 || !state.nix_errors.is_empty() {
        (ic.failed, Color::DarkRed, format!("failed at {at}"))
      } else {
        (ic.done, Color::DarkGreen, format!("finished at {at}"))
      };
    let status_str =
      format!("{} {status_label}", self.colored(status_icon, status_color));

    let duration_str = self.format_duration(duration);
    let host_s = self.colored(&host, Color::DarkMagenta);
    let dur_s = self.colored(&duration_str, Color::DarkGrey);
    let jobs = completed + failed;
    let fail_s = if failed > 0 && self.config.use_color {
      format!(
        "{}\x1b[1m{failed}\x1b[0m{}",
        SetForegroundColor(Color::DarkRed),
        ResetColor
      )
    } else {
      failed.to_string()
    };
    let summary_str = format!(
      "jobs={}  ok={}  failed={fail_s}  total={dur_s}",
      self.num_str(jobs),
      self.num_str(completed),
    );

    let header = format!(
      "{} BUILD GRAPH: {title}",
      self.colored("┏━", Color::DarkBlue)
    );

    vec![
      header,
      sep.clone(),
      format!("{:<12} {pipe} {host_s}", "Host"),
      format!("{:<12} {pipe} {status_str}", "Status"),
      format!("{:<12} {pipe} {dur_s}", "Duration"),
      sep,
      format!("{:<12} {pipe} {summary_str}", "Summary"),
    ]
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
      self.colored("┏━", Color::DarkBlue)
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

      if should_show
        && let Some(child) =
          self.build_active_node(state, input.derivation, visited)
      {
        children.push(child);
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
    line.push_str(&self.colored("┃ ", Color::DarkBlue));

    // Status icon
    let (icon, color) = self.get_status_icon(&info.build_status);
    line.push_str(&self.colored(icon, color));
    line.push(' ');

    // Package name
    line.push_str(&self.truncate_name(&info.name.name, 50));

    // Phase information
    if let BuildStatus::Building(build_info) = &info.build_status {
      if let Some(activity_id) = build_info.activity_id
        && let Some(activity) = state.activities.get(&activity_id)
        && let Some(phase) = &activity.phase
      {
        line.push_str(&self.colored(&format!(" ({phase})"), Color::DarkGrey));
      }

      // Time information
      let elapsed = current_time() - build_info.start;

      let ic = self.ic();
      // Show estimate if available
      if let Some(estimate_secs) = build_info.estimate {
        let remaining = estimate_secs.saturating_sub(elapsed as u64);
        line.push_str(&self.colored(
          &format!(
            " {} {}",
            ic.estimate,
            self.format_duration(remaining as f64)
          ),
          Color::DarkGrey,
        ));
      }

      // Show elapsed time
      line.push_str(&self.colored(
        &format!(" {} {}", ic.clock, self.format_duration(elapsed)),
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
    line.push_str(&self.colored(connector, Color::DarkBlue));

    let (icon, color) = self.get_status_icon(&info.build_status);
    line.push_str(&self.colored(icon, color));
    line.push(' ');
    line.push_str(&self.truncate_name(&info.name.name, 48));

    // Show elapsed time for active children
    if let BuildStatus::Building(build_info) = &info.build_status {
      let elapsed = current_time() - build_info.start;
      let ic = self.ic();
      line.push_str(&self.colored(
        &format!("  {} {}", ic.clock, self.format_duration(elapsed)),
        Color::DarkGrey,
      ));
    }

    lines.push(line);
  }

  fn get_status_icon(&self, status: &BuildStatus) -> (&'static str, Color) {
    let ic = self.ic();
    match status {
      BuildStatus::Building(_) => (ic.running, Color::DarkYellow),
      BuildStatus::Planned => (ic.planned, Color::DarkBlue),
      BuildStatus::Built { .. } => (ic.done, Color::DarkGreen),
      BuildStatus::Failed { .. } => (ic.failed, Color::DarkRed),
      BuildStatus::Unknown => ("?", Color::Grey),
    }
  }

  /// Shorthand accessor for the configured icon set.
  fn ic(&self) -> &'static Icons {
    self.config.icons
  }

  fn colored(&self, text: &str, color: Color) -> String {
    if self.config.use_color {
      format!("{}{}{}", SetForegroundColor(color), text, ResetColor)
    } else {
      text.to_string()
    }
  }

  /// Render an icon + count matching NOM's `nonZeroBold` semantics:
  /// the icon keeps its active colour always; the number is bold when > 0.
  fn count_colored(&self, icon: &str, n: usize, active_color: Color) -> String {
    let icon_s = self.colored(icon, active_color);
    let num_s = if n > 0 && self.config.use_color {
      format!("\x1b[1m{n}\x1b[0m")
    } else {
      n.to_string()
    };
    format!("{icon_s} {num_s}")
  }

  /// Render a count as bold-when-nonzero with no icon, matching the number
  /// semantics of `count_colored` for use in the dashboard summary row.
  fn num_str(&self, n: usize) -> String {
    if n > 0 && self.config.use_color {
      format!("\x1b[1m{n}\x1b[0m")
    } else {
      n.to_string()
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    icons,
    state::{BuildInfo, CompletedBuildInfo, FailType, FailedBuildInfo, State},
    types::{DisplayFormat, LegendStyle, SummaryStyle},
  };

  fn render_to_string(
    format: DisplayFormat,
    use_color: bool,
    legend: LegendStyle,
    summary: SummaryStyle,
    state: &State,
    final_render: bool,
  ) -> String {
    let mut buf = Vec::new();
    {
      let mut d = Display::new(&mut buf, DisplayConfig {
        show_timers: false,
        max_tree_depth: 10,
        max_visible_lines: 100,
        use_color,
        format,
        legend_style: legend,
        summary_style: summary,
        icons: &icons::UNICODE,
      })
      .unwrap();
      if final_render {
        d.render_final(state).unwrap();
      } else {
        d.render(state, &[]).unwrap();
      }
    }
    String::from_utf8_lossy(&buf).into_owned()
  }

  fn state_running() -> State {
    let mut s = State::new();
    s.full_summary.running_builds.insert(0, BuildInfo {
      start:       0.0,
      host:        cognos::Host::Localhost,
      estimate:    None,
      activity_id: None,
    });
    s
  }

  fn state_completed() -> State {
    let mut s = State::new();
    s.full_summary
      .completed_builds
      .insert(0, CompletedBuildInfo {
        start: 0.0,
        end:   1.0,
        host:  cognos::Host::Localhost,
      });
    s
  }

  fn state_failed() -> State {
    let mut s = State::new();
    s.full_summary.failed_builds.insert(0, FailedBuildInfo {
      start:     0.0,
      end:       1.0,
      host:      cognos::Host::Localhost,
      fail_type: FailType::BuildFailed(-1),
    });
    s
  }

  // --- Dashboard: color-on emits ANSI, color-off is clean ---

  #[test]
  fn dashboard_color_on_emits_ansi() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      true,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_running(),
      false,
    );
    assert!(
      out.contains('\x1b'),
      "expected ANSI escapes in colored dashboard output"
    );
  }

  #[test]
  fn dashboard_color_off_no_ansi() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      false,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_running(),
      false,
    );
    assert!(
      !out.contains('\x1b'),
      "expected no ANSI escapes in plain dashboard output"
    );
  }

  // --- Dashboard: status label reflects build state ---

  #[test]
  fn dashboard_running_shows_building() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      false,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_running(),
      false,
    );
    assert!(
      out.contains("building"),
      "expected 'building' label for running state"
    );
  }

  #[test]
  fn dashboard_completed_shows_done() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      false,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_completed(),
      false,
    );
    assert!(
      out.contains("done"),
      "expected 'done' label when all builds completed"
    );
  }

  #[test]
  fn dashboard_failed_final_shows_failed() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      false,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_failed(),
      true,
    );
    assert!(
      out.contains("failed"),
      "expected 'failed' label in final dashboard with failures"
    );
  }

  // --- Dashboard: structural content ---

  #[test]
  fn dashboard_empty_state_no_graph_header() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      true,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &State::new(),
      false,
    );
    assert!(
      !out.contains("BUILD GRAPH"),
      "expected no BUILD GRAPH header for empty state"
    );
  }

  #[test]
  fn dashboard_nonempty_state_has_graph_header() {
    let out = render_to_string(
      DisplayFormat::Dashboard,
      false,
      LegendStyle::Table,
      SummaryStyle::Concise,
      &state_running(),
      false,
    );
    assert!(
      out.contains("BUILD GRAPH"),
      "expected BUILD GRAPH header in dashboard output"
    );
  }

  // --- Format × color permutations (no-panic) ---

  #[test]
  fn all_formats_color_on_render_and_final_without_panic() {
    let state = state_running();
    for format in [
      DisplayFormat::Tree,
      DisplayFormat::Plain,
      DisplayFormat::Dashboard,
    ] {
      render_to_string(
        format,
        true,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        false,
      );
      render_to_string(
        format,
        true,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  #[test]
  fn all_formats_color_off_render_and_final_without_panic() {
    let state = state_running();
    for format in [
      DisplayFormat::Tree,
      DisplayFormat::Plain,
      DisplayFormat::Dashboard,
    ] {
      render_to_string(
        format,
        false,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        false,
      );
      render_to_string(
        format,
        false,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  // --- Legend style × color permutations ---

  #[test]
  fn legend_compact_color_permutations() {
    let state = state_completed();
    for use_color in [true, false] {
      render_to_string(
        DisplayFormat::Tree,
        use_color,
        LegendStyle::Compact,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  #[test]
  fn legend_table_color_permutations() {
    let state = state_completed();
    for use_color in [true, false] {
      render_to_string(
        DisplayFormat::Tree,
        use_color,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  #[test]
  fn legend_verbose_color_permutations() {
    let state = state_completed();
    for use_color in [true, false] {
      render_to_string(
        DisplayFormat::Tree,
        use_color,
        LegendStyle::Verbose,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  // --- Summary style × format permutations ---

  #[test]
  fn summary_concise_all_formats() {
    let state = state_completed();
    for format in [
      DisplayFormat::Tree,
      DisplayFormat::Plain,
      DisplayFormat::Dashboard,
    ] {
      render_to_string(
        format,
        true,
        LegendStyle::Table,
        SummaryStyle::Concise,
        &state,
        true,
      );
    }
  }

  #[test]
  fn summary_table_all_formats() {
    let state = state_completed();
    for format in [
      DisplayFormat::Tree,
      DisplayFormat::Plain,
      DisplayFormat::Dashboard,
    ] {
      render_to_string(
        format,
        true,
        LegendStyle::Table,
        SummaryStyle::Table,
        &state,
        true,
      );
    }
  }

  #[test]
  fn summary_full_all_formats() {
    let state = state_completed();
    for format in [
      DisplayFormat::Tree,
      DisplayFormat::Plain,
      DisplayFormat::Dashboard,
    ] {
      render_to_string(
        format,
        true,
        LegendStyle::Table,
        SummaryStyle::Full,
        &state,
        true,
      );
    }
  }

  // --- Dashboard final: all build-state × color permutations ---

  #[test]
  fn dashboard_final_build_state_color_permutations() {
    for use_color in [true, false] {
      for state in [state_running(), state_completed(), state_failed()] {
        render_to_string(
          DisplayFormat::Dashboard,
          use_color,
          LegendStyle::Table,
          SummaryStyle::Concise,
          &state,
          true,
        );
      }
    }
  }
}
