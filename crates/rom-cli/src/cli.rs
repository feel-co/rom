//! CLI interface for ROM
use std::{
  io,
  path::PathBuf,
  process::{Command, Stdio},
};

use clap::Parser;
use rom_core::{Result, RomError};

#[derive(Debug, Parser)]
#[command(name = "rom", version, about = "ROM - A Nix build output monitor")]
pub struct Cli {
  #[command(subcommand)]
  pub command: Option<Commands>,

  /// Parse JSON output from nix --log-format=internal-json
  #[arg(long, global = true)]
  pub json: bool,

  /// Minimal output
  #[arg(long, global = true)]
  pub silent: bool,

  /// Output format: tree, plain, dashboard
  #[arg(long, global = true, default_value = "tree")]
  pub format: String,

  /// Legend display style: compact, table, verbose
  #[arg(long, global = true, default_value = "table")]
  pub legend: String,

  /// Summary display style: concise, table, full
  #[arg(long, global = true, default_value = "concise")]
  pub summary: String,

  /// Log prefix style: short, full, none
  #[arg(long, global = true, default_value = "short")]
  pub log_prefix: String,

  /// Maximum number of log lines to display
  #[arg(long, global = true)]
  pub log_lines: Option<usize>,

  /// Nix-family evaluator to use. Auto-detected by default
  #[arg(long, global = true)]
  pub platform: Option<String>,

  /// Increase verbosity; controls nix log level and rom diagnostic output.
  /// Repeatable: -v (info), -vv (debug), -vvv (trace)
  #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
  pub verbose: u8,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
  /// Run nix build with monitoring
  Build {
    /// Packages or flake expressions to build
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },

  /// Run nix shell with monitoring
  Shell {
    /// Packages or flake expressions
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },

  /// Run nix develop with monitoring
  Develop {
    /// Packages or flake expressions
    packages: Vec<String>,

    /// Extra flags to pass directly to nix
    #[arg(last = true)]
    nix_flags: Vec<String>,
  },
}

struct WrapperConfig {
  platform:         cognos::Platform,
  silent:           bool,
  verbose:          u8,
  format:           rom_core::types::DisplayFormat,
  legend_style:     rom_core::types::LegendStyle,
  summary_style:    rom_core::types::SummaryStyle,
  log_prefix_style: rom_core::types::LogPrefixStyle,
  log_lines:        Option<usize>,
}

/// Run the CLI application
pub fn run() -> Result<()> {
  let cli = Cli::parse();

  // Pre-parse typed display values before any moves of cli
  let format = rom_core::types::DisplayFormat::from_str(&cli.format);
  let legend_style = rom_core::types::LegendStyle::from_str(&cli.legend);
  let summary_style = rom_core::types::SummaryStyle::from_str(&cli.summary);
  let log_prefix_style =
    rom_core::types::LogPrefixStyle::from_str(&cli.log_prefix);
  let log_lines = cli.log_lines;
  let silent = cli.silent;
  let verbose = cli.verbose;
  let json = cli.json;
  let platform = cli
    .platform
    .as_deref()
    .and_then(cognos::Platform::from_str)
    .unwrap_or_else(cognos::Platform::detect);

  // Check if we're being called as a symlink (rom-build, rom-shell)
  let program_name = std::env::args()
    .next()
    .and_then(|path| {
      PathBuf::from(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(std::string::ToString::to_string)
    })
    .unwrap_or_else(|| "rom".to_string());

  let make_config = |input_mode: rom_core::types::InputMode| {
    rom_core::types::Config {
      piping: false,
      silent,
      input_mode,
      show_timers: true,
      width: None,
      format,
      legend_style,
      summary_style,
      log_prefix_style,
      log_line_limit: log_lines,
    }
  };

  let cfg = WrapperConfig {
    platform,
    silent,
    verbose,
    format,
    legend_style,
    summary_style,
    log_prefix_style,
    log_lines,
  };

  match (&program_name[..], cli.command) {
    // rom-build symlink
    ("rom-build", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (packages, nix_flags) = parse_args_with_separator(&args);
      run_build_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom-shell symlink
    ("rom-shell", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (packages, nix_flags) = parse_args_with_separator(&args);
      run_shell_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom build command
    (
      _,
      Some(Commands::Build {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() && json {
        let stdin = io::stdin();
        let stdout = io::stdout();
        return rom_core::monitor_stream(
          make_config(rom_core::types::InputMode::Json),
          stdin.lock(),
          stdout.lock(),
        );
      }
      if packages.is_empty() {
        return Err(RomError::config(
          "No package or flake specified for build\nUsage: rom build \
           <package> [-- <flags>]\nExample: rom build nixpkgs#hello -- \
           --rebuild",
        ));
      }
      run_build_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom shell command
    (
      _,
      Some(Commands::Shell {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() && json {
        let stdin = io::stdin();
        let stdout = io::stdout();
        return rom_core::monitor_stream(
          make_config(rom_core::types::InputMode::Json),
          stdin.lock(),
          stdout.lock(),
        );
      }
      if packages.is_empty() {
        return Err(RomError::config(
          "No package or flake specified for shell\nUsage: rom shell \
           <package> [-- <flags>]\nExample: rom shell nixpkgs#python3 -- \
           --pure",
        ));
      }
      run_shell_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // rom develop command
    (
      _,
      Some(Commands::Develop {
        packages,
        nix_flags,
      }),
    ) => {
      if packages.is_empty() && json {
        let stdin = io::stdin();
        let stdout = io::stdout();
        return rom_core::monitor_stream(
          make_config(rom_core::types::InputMode::Json),
          stdin.lock(),
          stdout.lock(),
        );
      }
      if packages.is_empty() {
        return Err(RomError::config(
          "No package or flake specified for develop\nUsage: rom develop \
           <package> [-- <flags>]\nExample: rom develop nixpkgs#hello -- \
           --impure",
        ));
      }
      run_develop_wrapper(packages, nix_flags, &cfg)?;
      Ok(())
    },

    // Direct piping mode, read from stdin
    (_, None) => {
      let input_mode = if json {
        rom_core::types::InputMode::Json
      } else {
        rom_core::types::InputMode::Human
      };
      let stdin = io::stdin();
      let stdout = io::stdout();
      Ok(rom_core::monitor_stream(
        make_config(input_mode),
        stdin.lock(),
        stdout.lock(),
      )?)
    },
  }
}

/// Parse arguments, separating those before and after `--`
/// Returns (`args_before_separator`, `args_after_separator`)
///
/// Everything before `--` is for the package name and rom arguments.
/// Everything after `--` goes directly to nix.
#[must_use]
pub fn parse_args_with_separator(
  args: &[String],
) -> (Vec<String>, Vec<String>) {
  if let Some(pos) = args.iter().position(|arg| arg == "--") {
    // Arguments before -- are package/rom args
    let before = args[..pos].to_vec();

    // Arguments after -- go to nix
    let after = args[pos + 1..].to_vec();
    (before, after)
  } else {
    // No separator found - all args are package/rom args for backward
    // compatibility
    (args.to_vec(), Vec::new())
  }
}

/// Returns the nix verbosity flag for the given level.
/// Always produces at least `-v` so build events are emitted via
/// `--log-format internal-json`.
fn nix_verbosity_flag(verbose: u8) -> String {
  format!("-{}", "v".repeat(verbose.max(1) as usize))
}

fn run_build_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> Result<()> {
  if packages.is_empty() {
    return Err(RomError::config(
      "No package or flake specified for build\nUsage: rom build <package> \
       [-- <flags>]\nExample: rom build nixpkgs#hello -- --rebuild",
    ));
  }

  let mut cmd_args = vec![
    "build".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];
  cmd_args.extend(packages);
  cmd_args.extend(nix_flags);

  let exit_code = run_monitored_command(cfg.platform.binary(), cmd_args, cfg)?;
  if exit_code != 0 {
    std::process::exit(exit_code);
  }
  Ok(())
}

fn run_shell_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> Result<()> {
  if packages.is_empty() {
    return Err(RomError::config(
      "No package or flake specified for shell\nUsage: rom shell <package> \
       [-- <flags>]\nExample: rom shell nixpkgs#python3 -- --pure",
    ));
  }

  // First pass: monitor the build phase with --command exit
  let mut monitor_args = vec![
    "shell".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];
  let shell_args: Vec<String> =
    packages.iter().chain(nix_flags.iter()).cloned().collect();
  monitor_args.extend(replace_command_with_exit(&shell_args));

  let exit_code =
    run_monitored_command(cfg.platform.binary(), monitor_args, cfg)?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // Second pass: enter the actual shell
  if !cfg.silent {
    let mut shell_args = vec!["shell".to_string()];
    shell_args.extend(packages);
    shell_args.extend(nix_flags);

    let status = Command::new(cfg.platform.binary())
      .args(&shell_args)
      .status()
      .map_err(rom_core::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

fn run_develop_wrapper(
  packages: Vec<String>,
  nix_flags: Vec<String>,
  cfg: &WrapperConfig,
) -> Result<()> {
  // First pass: monitor with --command true
  let mut monitor_args = vec![
    "develop".to_string(),
    nix_verbosity_flag(cfg.verbose),
    "--log-format".to_string(),
    "internal-json".to_string(),
    "--command".to_string(),
    "true".to_string(),
  ];
  monitor_args.extend(packages.clone());
  monitor_args.extend(nix_flags.clone());

  let exit_code =
    run_monitored_command(cfg.platform.binary(), monitor_args, cfg)?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // Second pass: enter the actual dev shell
  if !cfg.silent {
    let mut develop_args = vec!["develop".to_string()];
    develop_args.extend(packages);
    develop_args.extend(nix_flags);

    let status = Command::new(cfg.platform.binary())
      .args(&develop_args)
      .status()
      .map_err(rom_core::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

/// Events the main thread consumes from the input threads.
enum Event {
  /// Parsed nix JSON message.
  Json(Box<cognos::Actions>),
  /// Raw line to buffer as a log (non-JSON stderr).
  RawLog(String),
  /// Final stdout line (nix's summary output).
  Stdout(String),
  /// Input thread finished (EOF from child's stderr).
  StderrEof,
  /// Stdout thread finished (EOF from child's stdout).
  StdoutEof,
}

fn run_monitored_command(
  command: &str,
  args: Vec<String>,
  cfg: &WrapperConfig,
) -> Result<i32> {
  use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
  };

  let silent = cfg.silent;
  let format = cfg.format;
  let legend_style = cfg.legend_style;
  let summary_style = cfg.summary_style;
  let log_prefix_style = cfg.log_prefix_style;
  let log_line_limit = cfg.log_lines;

  // Hide the cursor while rendering; install handlers so Ctrl+C restores it
  // before the process exits via the signal's default disposition. The
  // RAII guard covers every early-return path (spawn failure, child.wait
  // error, panic unwind).
  let _cursor_guard = if silent {
    rom_core::term::CursorGuard::noop()
  } else {
    rom_core::term::install_signal_handlers();
    rom_core::term::CursorGuard::hide()
  };

  let mut child = Command::new(command)
    .args(&args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(RomError::Io)?;

  let stderr = child.stderr.take().expect("Failed to capture stderr");
  let stdout = child.stdout.take().expect("Failed to capture stdout");

  let (tx, rx) = mpsc::channel::<Event>();

  // stderr reader: parse @nix JSON, forward everything else as RawLog.
  let stderr_tx = tx.clone();
  let stderr_thread = thread::spawn(move || {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
      let Ok(line) = line else { break };
      let event = if let Some(json) = line.strip_prefix("@nix ") {
        match serde_json::from_str::<cognos::Actions>(json) {
          Ok(action) => Event::Json(Box::new(action)),
          Err(_) => Event::RawLog(line),
        }
      } else {
        Event::RawLog(line)
      };
      if stderr_tx.send(event).is_err() {
        break;
      }
    }
    let _ = stderr_tx.send(Event::StderrEof);
  });

  // stdout reader: just forward lines; they get printed after the build.
  let stdout_tx = tx.clone();
  let stdout_thread = thread::spawn(move || {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
      let Ok(line) = line else { break };
      if stdout_tx.send(Event::Stdout(line)).is_err() {
        break;
      }
    }
    let _ = stdout_tx.send(Event::StdoutEof);
  });
  drop(tx);

  // Main thread owns State and Display. No locks, no clones.
  let display_config = rom_core::display::DisplayConfig {
    show_timers: true,
    max_tree_depth: 10,
    max_visible_lines: 100,
    use_color: !silent,
    format,
    legend_style,
    summary_style,
    icons: rom_core::icons::detect(),
  };
  let mut display =
    rom_core::display::Display::new(io::stderr(), display_config)?;
  let mut state = rom_core::state::State::new();
  let mut logs: VecDeque<String> = VecDeque::new();
  let mut stdout_lines: Vec<String> = Vec::new();
  let mut stderr_done = false;
  let mut stdout_done = false;
  let mut dirty = false;
  let mut last_render = Instant::now();

  // Adaptive render cadence: frequent when state is changing, slow when idle.
  let fast = Duration::from_millis(60);
  let slow = Duration::from_secs(1);

  while !(stderr_done && stdout_done) {
    let timeout = if dirty { fast } else { slow };
    match rx.recv_timeout(timeout) {
      Ok(Event::Json(action)) => {
        let action = *action;
        // Update state first so activity prefixes are resolvable.
        rom_core::update::process_message(&mut state, action.clone());
        rom_core::update::maintain_state(
          &mut state,
          rom_core::state::current_time(),
        );

        // Extract any log line from this action.
        match &action {
          cognos::Actions::Message { msg, raw_msg, .. } => {
            // Prefer raw_msg (Lix) to avoid embedded ANSI escapes.
            let text = raw_msg.as_deref().unwrap_or(msg.as_str());
            logs.push_back(text.to_string());
          },
          cognos::Actions::Result {
            fields,
            result_type,
            id,
          } if matches!(result_type, cognos::ResultType::BuildLogLine)
            && !fields.is_empty() =>
          {
            if let Some(log_text) = fields[0].as_str() {
              let prefix = state
                .get_activity_prefix(*id, &log_prefix_style, !silent)
                .unwrap_or_default();
              logs.push_back(format!("{prefix}{log_text}"));
            }
          },
          _ => {},
        }
        if let Some(limit) = log_line_limit {
          while logs.len() > limit {
            logs.pop_front();
          }
        }
        dirty = true;
      },
      Ok(Event::RawLog(line)) => {
        logs.push_back(line);
        if let Some(limit) = log_line_limit {
          while logs.len() > limit {
            logs.pop_front();
          }
        }
        dirty = true;
      },
      Ok(Event::Stdout(line)) => stdout_lines.push(line),
      Ok(Event::StderrEof) => stderr_done = true,
      Ok(Event::StdoutEof) => stdout_done = true,
      Err(mpsc::RecvTimeoutError::Timeout) => {}, // fall through to render
      Err(mpsc::RecvTimeoutError::Disconnected) => break,
    }

    // Render at most once per `fast` interval when dirty; once per `slow`
    // otherwise so the elapsed-time ticker stays fresh.
    let since = last_render.elapsed();
    let should_render = (dirty && since >= fast) || since >= slow;
    if should_render {
      // Full topological recompute of transitive dep summaries before
      // rendering — O(V + E), cheap even for large graphs, and guaranteed
      // correct under diamond deps.
      rom_core::update::summaries(&mut state);
      let logs_vec: Vec<String> = if silent {
        Vec::new()
      } else {
        logs.iter().cloned().collect()
      };
      let _ = display.render(&state, &logs_vec);
      last_render = Instant::now();
      dirty = false;
    }
  }

  // Final render with the finish pass. Summaries twice — once before
  // finish_state so in-flight nodes are accurate, once after so completions
  // recorded during finish are propagated.
  rom_core::update::summaries(&mut state);
  rom_core::update::finish_state(&mut state);
  rom_core::update::summaries(&mut state);
  let _ = display.render_final(&state);

  let status = child.wait().map_err(RomError::Io)?;
  let _ = stderr_thread.join();
  let _ = stdout_thread.join();

  // _cursor_guard drops here and restores the cursor.

  use std::io::Write;
  for line in &stdout_lines {
    let _ = writeln!(std::io::stdout(), "{line}");
  }

  Ok(status.code().unwrap_or(1))
}

/// Replace --command/-c arguments with "sh -c exit" for monitoring pass
pub fn replace_command_with_exit(args: &[String]) -> Vec<String> {
  let mut result = Vec::new();
  let mut skip_next = false;

  for arg in args {
    if skip_next {
      skip_next = false;
      continue;
    }

    if arg == "--command" || arg == "-c" {
      // Skip this and the next argument
      skip_next = true;
      continue;
    }

    result.push(arg.clone());
  }

  // Add our exit command
  result.push("--command".to_string());
  result.push("sh".to_string());
  result.push("-c".to_string());
  result.push("exit".to_string());

  result
}
