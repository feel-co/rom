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

    let line = if failed > 0 {
      let noun = if failed == 1 { "failure" } else { "failures" };
      format!(
        "{} {} at {} after {}",
        self.colored("⚠", Color::Red),
        self
          .colored(&format!("Exited after {failed} build {noun}"), Color::Red),
        self.colored(&at.to_string(), Color::Red),
        self.colored(&dur, Color::Red),
      )
    } else if nix_errors > 0 {
      let noun = if nix_errors == 1 { "error" } else { "errors" };
      format!(
        "{} {} at {} after {}",
        self.colored("⚠", Color::Red),
        self
          .colored(&format!("Exited with {nix_errors} nix {noun}"), Color::Red),
        self.colored(&at.to_string(), Color::Red),
        self.colored(&dur, Color::Red),
      )
    } else {
      let mut s = format!(
        "{} after {}",
        self.colored(&format!("Finished at {at}"), Color::Green),
        self.colored(&dur, Color::Green),
      );
      if completed > 0 {
        s.push_str(&format!(
          "  {}",
          self.colored(&format!("✔ {completed}"), Color::Green)
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
    lines.push(format!(
      "{} {}",
      self.colored("━━━", Color::Blue),
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
          parts.push(format!("{} {done}", self.colored("✔", Color::Green)));
        }
        if fail > 0 {
          parts.push(format!("{} {fail}", self.colored("✗", Color::Red)));
        }
        lines.push(format!(
          "{}  {}  {}",
          self.colored("┃", Color::Blue),
          parts.join("  "),
          self.colored(host, Color::Magenta),
        ));
      }
    }

    // Final ∑ line
    let mut sum_parts = Vec::new();
    if completed > 0 {
      sum_parts
        .push(format!("{} {completed}", self.colored("✔", Color::Green)));
    }
    if failed > 0 {
      sum_parts.push(format!("{} {failed}", self.colored("✗", Color::Red)));
    }
    if dl_done > 0 {
      sum_parts.push(format!("{} {dl_done}", self.colored("↓", Color::Green)));
    }
    if ul_done > 0 {
      sum_parts.push(format!("{} {ul_done}", self.colored("↑", Color::Green)));
    }

    let finish = if failed > 0 || !state.nix_errors.is_empty() {
      self.colored(&format!("Exited at {at} after {dur}"), Color::Red)
    } else {
      self.colored(&format!("Finished at {at} after {dur}"), Color::Green)
    };
    sum_parts.push(finish);

    lines.push(format!(
      "{} ∑ {}",
      self.colored("┗━", Color::Blue),
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

    let v = self.colored("┃", Color::Blue);

    let mut lines = Vec::new();
    lines.push(format!(
      "{} Build Summary",
      self.colored("━━━", Color::Blue)
    ));

    if completed > 0 || failed > 0 {
      let mut bp = Vec::new();
      if completed > 0 {
        bp.push(format!(
          "{} {completed} built",
          self.colored("✔", Color::Green)
        ));
      }
      if failed > 0 {
        bp.push(format!("{} {failed} failed", self.colored("✗", Color::Red)));
      }
      lines.push(format!("{}  Builds:     {}", v, bp.join("  ")));
    }

    let total_dl = dl_done + dl_running;
    let total_ul = ul_done + ul_running;
    if total_dl > 0 {
      lines.push(format!(
        "{}  Downloads:  {} fetched",
        v,
        self.colored(&total_dl.to_string(), Color::Green)
      ));
    }
    if total_ul > 0 {
      lines.push(format!(
        "{}  Uploads:    {} pushed",
        v,
        self.colored(&total_ul.to_string(), Color::Green)
      ));
    }

    if !state.nix_errors.is_empty() {
      lines.push(format!(
        "{}  {} {} nix error(s)",
        v,
        self.colored("⚠", Color::Red),
        state.nix_errors.len()
      ));
    }

    let finish_label = if failed > 0 || !state.nix_errors.is_empty() {
      self.colored(&format!("Exited at {at}"), Color::Red)
    } else {
      self.colored(&format!("Finished at {at}"), Color::Green)
    };
    lines.push(format!(
      "{} {} after {}",
      self.colored("┗━", Color::Blue),
      finish_label,
      self.colored(&self.format_duration(duration), Color::DarkGrey),
    ));

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
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let dl = state.full_summary.running_downloads.len();
    let ul = state.full_summary.running_uploads.len();

    if running + completed + planned + dl + ul == 0 {
      return vec![];
    }

    let duration = current_time() - state.start_time;
    let prefix = if has_tree { "━" } else { "━" };

    let mut parts: Vec<String> = Vec::new();
    if running > 0 {
      parts.push(format!("{} {running}", self.colored("⏵", Color::Yellow)));
    }
    if completed > 0 {
      parts.push(format!("{} {completed}", self.colored("✔", Color::Green)));
    }
    if planned > 0 {
      parts.push(format!("{} {planned}", self.colored("⏸", Color::Blue)));
    }
    if dl > 0 {
      parts.push(format!("{} {dl}", self.colored("↓", Color::Yellow)));
    }
    if ul > 0 {
      parts.push(format!("{} {ul}", self.colored("↑", Color::Yellow)));
    }
    parts.push(self.colored(&self.format_duration(duration), Color::DarkGrey));

    vec![format!(
      "{} {}",
      self.colored(prefix, Color::Blue),
      parts.join("  ")
    )]
  }

  fn render_table_legend(&self, state: &State, has_tree: bool) -> Vec<String> {
    let running = state.full_summary.running_builds.len();
    let completed = state.full_summary.completed_builds.len();
    let planned = state.full_summary.planned_builds.len();
    let dl_running = state.full_summary.running_downloads.len();
    let dl_done = state.full_summary.completed_downloads.len();
    let ul_running = state.full_summary.running_uploads.len();

    let show_builds = running + completed + planned > 0;
    let show_dl = dl_running + dl_done > 0;
    let show_ul = ul_running > 0;

    if !show_builds && !show_dl && !show_ul {
      return vec![];
    }

    let duration = current_time() - state.start_time;

    // Collect unique hosts from running builds to decide whether to show
    // per-host rows
    let mut host_set: std::collections::HashSet<String> =
      std::collections::HashSet::new();
    for b in state.full_summary.running_builds.values() {
      host_set.insert(b.host.name().to_string());
    }
    for b in state.full_summary.completed_builds.values() {
      host_set.insert(b.host.name().to_string());
    }
    let many_hosts = host_set.len() > 1;

    // Build the header columns
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

    // Build summary counts for the ∑ row
    let mut sum_parts: Vec<String> = Vec::new();
    if show_builds {
      let mut bp = Vec::new();
      if running > 0 {
        bp.push(format!("{} {running}", self.colored("⏵", Color::Yellow)));
      }
      if completed > 0 {
        bp.push(format!("{} {completed}", self.colored("✔", Color::Green)));
      }
      if planned > 0 {
        bp.push(format!("{} {planned}", self.colored("⏸", Color::Blue)));
      }
      if !bp.is_empty() {
        sum_parts.push(bp.join(" "));
      }
    }
    if show_dl {
      let mut dp = Vec::new();
      if dl_running > 0 {
        dp.push(format!("{} {dl_running}", self.colored("↓", Color::Yellow)));
      }
      if dl_done > 0 {
        dp.push(format!("{} {dl_done}", self.colored("↓", Color::Green)));
      }
      if !dp.is_empty() {
        sum_parts.push(dp.join(" "));
      }
    }
    if show_ul {
      sum_parts
        .push(format!("{} {ul_running}", self.colored("↑", Color::Yellow)));
    }
    sum_parts
      .push(self.colored(&self.format_duration(duration), Color::DarkGrey));

    let mut lines = Vec::new();

    // ━━━  [header cols], or ┣━━━ if following a tree
    let header_sep = if has_tree {
      "┣━━━"
    } else {
      "━━━"
    };
    lines.push(format!(
      "{} {}",
      self.colored(header_sep, Color::Blue),
      header_parts.join("  ")
    ));

    // Per-host rows (only when multiple remote builders)
    if many_hosts {
      let mut hosts: Vec<String> = host_set.into_iter().collect();
      hosts.sort();
      for host in &hosts {
        let mut row_parts: Vec<String> = Vec::new();
        if show_builds {
          let r = state
            .full_summary
            .running_builds
            .values()
            .filter(|b| b.host.name() == host)
            .count();
          let d = state
            .full_summary
            .completed_builds
            .values()
            .filter(|b| b.host.name() == host)
            .count();
          let mut bp = Vec::new();
          if r > 0 {
            bp.push(format!("{} {r}", self.colored("⏵", Color::Yellow)));
          }
          if d > 0 {
            bp.push(format!("{} {d}", self.colored("✔", Color::Green)));
          }
          if !bp.is_empty() {
            row_parts.push(bp.join(" "));
          }
        }
        if !row_parts.is_empty() {
          lines.push(format!(
            "{}  {} {}",
            self.colored("┃", Color::Blue),
            row_parts.join(" │ "),
            self.colored(host, Color::Magenta),
          ));
        }
      }
    }

    // ┗━ ∑  [summary]
    let tail = if has_tree { "┗━" } else { "┗━" };
    lines.push(format!(
      "{} ∑ {}",
      self.colored(tail, Color::Blue),
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
    let planned = state.full_summary.planned_builds.len();
    let dl_running = state.full_summary.running_downloads.len();
    let ul_running = state.full_summary.running_uploads.len();

    if running + completed + planned + dl_running + ul_running == 0 {
      return vec![];
    }

    let now = current_time();
    let duration = now - state.start_time;
    let prefix = if has_tree {
      "┣━━━"
    } else {
      "━━━"
    };
    let v = self.colored("┃", Color::Blue);

    let mut lines = Vec::new();
    lines.push(format!("{} Builds", self.colored(prefix, Color::Blue)));

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
            format!("  {}", self.colored(h, Color::Magenta))
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

    for (name, elapsed, host) in &running_entries {
      lines.push(format!(
        "{}  {} {:<width$}  {}{}",
        v,
        self.colored("⏵", Color::Yellow),
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
          self.colored("↓", Color::Yellow),
          self.truncate_name(&pi.name.name, name_width),
          self.colored(&size, Color::DarkGrey),
          self.colored(&self.format_duration(elapsed), Color::DarkGrey),
          width = name_width,
        ));
      }
    }

    // Summary line
    let mut sum_parts: Vec<String> = Vec::new();
    if running > 0 {
      sum_parts.push(format!("{} {running}", self.colored("⏵", Color::Yellow)));
    }
    if completed > 0 {
      sum_parts
        .push(format!("{} {completed}", self.colored("✔", Color::Green)));
    }
    if planned > 0 {
      sum_parts.push(format!("{} {planned}", self.colored("⏸", Color::Blue)));
    }
    if dl_running > 0 {
      sum_parts
        .push(format!("{} {dl_running}", self.colored("↓", Color::Yellow)));
    }
    if ul_running > 0 {
      sum_parts
        .push(format!("{} {ul_running}", self.colored("↑", Color::Yellow)));
    }
    sum_parts
      .push(self.colored(&self.format_duration(duration), Color::DarkGrey));

    lines.push(format!(
      "{} ∑ {}",
      self.colored("┗━", Color::Blue),
      sum_parts.join("  ")
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

    for (name, build) in &builds {
      let elapsed = now - build.start;
      let mut suffix = String::new();
      if let Some(est) = build.estimate {
        let remaining = est.saturating_sub(elapsed as u64);
        suffix = format!(
          "  {} {}",
          self.colored("∅", Color::DarkGrey),
          self
            .colored(&self.format_duration(remaining as f64), Color::DarkGrey)
        );
      }
      let host_label = match &build.host {
        cognos::Host::Remote(h) => {
          format!("  {}", self.colored(h, Color::Magenta))
        },
        _ => String::new(),
      };
      lines.push(format!(
        "  {} {}  {}{}{}",
        self.colored("⏵", Color::Yellow),
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
          self.colored("↓", Color::Yellow),
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
          self.colored("↑", Color::Yellow),
          pi.name.name,
          self.colored(&size, Color::DarkGrey),
        ));
      }
    }

    // Summary bar at the bottom
    let mut parts: Vec<String> = Vec::new();
    if running > 0 {
      parts.push(format!("{} {running}", self.colored("⏵", Color::Yellow)));
    }
    if completed > 0 {
      parts.push(format!("{} {completed}", self.colored("✔", Color::Green)));
    }
    if planned > 0 {
      parts.push(format!("{} {planned}", self.colored("⏸", Color::Blue)));
    }
    if downloading > 0 {
      parts.push(format!(
        "{} {downloading}",
        self.colored("↓", Color::Yellow)
      ));
    }
    if uploading > 0 {
      parts.push(format!("{} {uploading}", self.colored("↑", Color::Yellow)));
    }
    parts.push(self.colored(&self.format_duration(duration), Color::DarkGrey));

    lines.push(format!(
      "{} {}",
      self.colored("━", Color::Blue),
      parts.join("  ")
    ));

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

      let host = state
        .full_summary
        .completed_builds
        .values()
        .next()
        .map_or("localhost", |b| b.host.name());

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
    line.push_str(&self.colored("┃ ", Color::Blue));

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
