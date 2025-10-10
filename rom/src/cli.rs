//! CLI interface for ROM
use std::{
  io,
  path::PathBuf,
  process::{Command, Stdio},
};

use clap::Parser;
use cognos::ProgressState;

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

  /// Output format: tree, plain
  #[arg(long, global = true, default_value = "tree")]
  pub format: String,

  /// Legend display style: compact, table, verbose
  #[arg(long, global = true, default_value = "table")]
  pub legend: String,

  /// Summary display style: concise, table, full
  #[arg(long, global = true, default_value = "concise")]
  pub summary: String,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
  /// Run nix build with monitoring
  Build {
    /// Package/flake to build and arguments to pass to Nix
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },

  /// Run nix shell with monitoring
  Shell {
    /// Package/flake and arguments to pass to Nix
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },

  /// Run nix develop with monitoring
  Develop {
    /// Package/flake and arguments to pass to nix Nix
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
  },
}

/// Run the CLI application
pub fn run() -> eyre::Result<()> {
  let cli = Cli::parse();

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

  match (&program_name[..], cli.command) {
    // rom-build symlink
    ("rom-build", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (package_and_rom_args, nix_args) = parse_args_with_separator(&args);
      run_nix_build_wrapper(
        package_and_rom_args,
        nix_args,
        cli.silent,
        cli.format.clone(),
        cli.legend.clone(),
        cli.summary.clone(),
      )?;
      Ok(())
    },

    // nom-shell symlink
    ("rom-shell", _) => {
      let args: Vec<String> = std::env::args().skip(1).collect();
      let (package_and_rom_args, nix_args) = parse_args_with_separator(&args);
      run_nix_shell_wrapper(
        package_and_rom_args,
        nix_args,
        cli.silent,
        cli.format.clone(),
        cli.legend.clone(),
        cli.summary.clone(),
      )?;
      Ok(())
    },

    // rom build command
    (_, Some(Commands::Build { args })) => {
      // If no args provided and --json is set, use piping mode from stdin
      if args.is_empty() && cli.json {
        let config = crate::types::Config {
          piping:        false,
          silent:        cli.silent,
          input_mode:    crate::types::InputMode::Json,
          show_timers:   true,
          width:         None,
          format:        crate::types::DisplayFormat::from_str(&cli.format),
          legend_style:  cli.legend.clone(),
          summary_style: cli.summary.clone(),
        };

        let stdin = io::stdin();
        let stdout = io::stdout();

        return Ok(crate::monitor_stream(config, stdin.lock(), stdout.lock())?);
      }
      let (package_and_rom_args, nix_args) = parse_args_with_separator(&args);
      if package_and_rom_args.is_empty() {
        eyre::bail!(
          "No package or flake specified for nix build\nUsage: rom build \
           <package> [-- <nix-flags>]\nExample: rom build nixpkgs#hello -- \
           --rebuild"
        );
      }
      run_nix_build_wrapper(
        package_and_rom_args,
        nix_args,
        cli.silent,
        cli.format.clone(),
        cli.legend.clone(),
        cli.summary.clone(),
      )?;
      Ok(())
    },

    // rom shell command
    (_, Some(Commands::Shell { args })) => {
      // If no args provided and --json is set, use piping mode from stdin
      if args.is_empty() && cli.json {
        let config = crate::types::Config {
          piping:        false,
          silent:        cli.silent,
          input_mode:    crate::types::InputMode::Json,
          show_timers:   true,
          width:         None,
          format:        crate::types::DisplayFormat::from_str(&cli.format),
          legend_style:  cli.legend.clone(),
          summary_style: cli.summary.clone(),
        };

        let stdin = io::stdin();
        let stdout = io::stdout();

        return Ok(crate::monitor_stream(config, stdin.lock(), stdout.lock())?);
      }
      let (package_and_rom_args, nix_args) = parse_args_with_separator(&args);
      if package_and_rom_args.is_empty() {
        eyre::bail!(
          "No package or flake specified for nix shell\nUsage: rom shell \
           <package> [-- <nix-flags>]\nExample: rom shell nixpkgs#python3 -- \
           --pure"
        );
      }
      run_nix_shell_wrapper(
        package_and_rom_args,
        nix_args,
        cli.silent,
        cli.format.clone(),
        cli.legend.clone(),
        cli.summary.clone(),
      )?;
      Ok(())
    },

    // rom develop command
    (_, Some(Commands::Develop { args })) => {
      // If no args provided and --json is set, use piping mode from stdin
      if args.is_empty() && cli.json {
        let config = crate::types::Config {
          piping:        false,
          silent:        cli.silent,
          input_mode:    crate::types::InputMode::Json,
          show_timers:   true,
          width:         None,
          format:        crate::types::DisplayFormat::from_str(&cli.format),
          legend_style:  cli.legend.clone(),
          summary_style: cli.summary.clone(),
        };

        let stdin = io::stdin();
        let stdout = io::stdout();

        return Ok(crate::monitor_stream(config, stdin.lock(), stdout.lock())?);
      }
      let (package_and_rom_args, nix_args) = parse_args_with_separator(&args);
      if package_and_rom_args.is_empty() {
        eyre::bail!(
          "No package or flake specified for nix develop\nUsage: rom develop \
           <package> [-- <nix-flags>]\nExample: rom develop nixpkgs#hello -- \
           --impure"
        );
      }
      run_nix_develop_wrapper(
        package_and_rom_args,
        nix_args,
        cli.silent,
        cli.format.clone(),
        cli.legend.clone(),
        cli.summary.clone(),
      )?;
      Ok(())
    },

    // Direct piping mode, read from stdin
    (_, None) => {
      let input_mode = if cli.json {
        crate::types::InputMode::Json
      } else {
        crate::types::InputMode::Human
      };

      let config = crate::types::Config {
        piping: false,
        silent: cli.silent,
        input_mode,
        show_timers: true,
        width: None,
        format: crate::types::DisplayFormat::from_str(&cli.format),
        legend_style: cli.legend.clone(),
        summary_style: cli.summary.clone(),
      };

      let stdin = io::stdin();
      let stdout = io::stdout();

      Ok(crate::monitor_stream(config, stdin.lock(), stdout.lock())?)
    },
  }
}

/// Parse arguments, separating those before and after `--`
/// Returns (`args_before_separator`, `args_after_separator`)
///
/// Everything before `--` is for the package name and rom arguments.
/// Everything after `--` goes directly to nix.
#[must_use] pub fn parse_args_with_separator(
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

/// Run nix build with monitoring
fn run_nix_build_wrapper(
  package_and_rom_args: Vec<String>,
  user_nix_args: Vec<String>,
  silent: bool,
  format: String,
  legend_style: String,
  summary_style: String,
) -> eyre::Result<()> {
  // Validate that at least one package/flake is specified
  if package_and_rom_args.is_empty() {
    eyre::bail!(
      "No package or flake specified for nix build\nUsage: rom build \
       <package> [-- <nix-flags>]\nExample: rom build nixpkgs#hello -- \
       --rebuild"
    );
  }

  let mut nix_args = vec![
    "build".to_string(),
    "-v".to_string(),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];

  // Add package/flake argument(s)
  nix_args.extend(package_and_rom_args);

  // Add user-provided nix flags (after --)
  nix_args.extend(user_nix_args);

  let exit_code = run_monitored_command(
    "nix",
    nix_args,
    silent,
    format,
    legend_style,
    summary_style,
  )?;
  if exit_code != 0 {
    std::process::exit(exit_code);
  }
  Ok(())
}

/// Run nix shell with monitoring
fn run_nix_shell_wrapper(
  package_and_rom_args: Vec<String>,
  user_nix_args: Vec<String>,
  silent: bool,
  format: String,
  legend_style: String,
  summary_style: String,
) -> eyre::Result<()> {
  // Validate that at least one package/flake is specified
  if package_and_rom_args.is_empty() {
    eyre::bail!(
      "No package or flake specified for nix shell\nUsage: rom shell \
       <package> [-- <nix-flags>]\nExample: rom shell nixpkgs#python3 -- \
       --pure"
    );
  }

  // For nix shell, we need to run it twice:
  // 1. First with --command exit to monitor the build
  // 2. Then normally to actually enter the shell
  let mut monitor_args = vec![
    "shell".to_string(),
    "-v".to_string(),
    "--log-format".to_string(),
    "internal-json".to_string(),
  ];

  // Replace or append --command with exit
  let args_without_command = replace_command_with_exit(&package_and_rom_args);
  monitor_args.extend(args_without_command);

  // Add user-provided nix flags
  monitor_args.extend(user_nix_args.clone());

  // Run first pass with monitoring
  let exit_code = run_monitored_command(
    "nix",
    monitor_args,
    silent,
    format,
    legend_style,
    summary_style,
  )?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // If monitoring succeeded and not silent, run the actual shell command
  if !silent {
    let mut shell_args = vec!["shell".to_string()];
    shell_args.extend(package_and_rom_args);
    shell_args.extend(user_nix_args);

    let status = Command::new("nix")
      .args(&shell_args)
      .status()
      .map_err(crate::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

/// Run nix develop with monitoring
fn run_nix_develop_wrapper(
  package_and_rom_args: Vec<String>,
  user_nix_args: Vec<String>,
  silent: bool,
  format: String,
  legend_style: String,
  summary_style: String,
) -> eyre::Result<()> {
  // Validate that at least one package/flake is specified (can be empty for
  // current flake) develop without args is valid (uses current directory's
  // flake)

  // Similar to shell - run twice
  let mut monitor_args = vec![
    "develop".to_string(),
    "-v".to_string(),
    "--log-format".to_string(),
    "internal-json".to_string(),
    "--command".to_string(),
    "true".to_string(),
  ];

  monitor_args.extend(package_and_rom_args.clone());

  // Add user-provided nix flags
  monitor_args.extend(user_nix_args.clone());

  // Run first pass with monitoring
  let exit_code = run_monitored_command(
    "nix",
    monitor_args,
    silent,
    format,
    legend_style,
    summary_style,
  )?;

  if exit_code != 0 {
    std::process::exit(exit_code);
  }

  // Run the actual develop command
  if !silent {
    let mut develop_args = vec!["develop".to_string()];
    develop_args.extend(package_and_rom_args);
    develop_args.extend(user_nix_args);

    let status = Command::new("nix")
      .args(&develop_args)
      .status()
      .map_err(crate::error::RomError::Io)?;

    std::process::exit(status.code().unwrap_or(1));
  }

  Ok(())
}

/// Run a nix command with output monitoring
fn run_monitored_command(
  command: &str,
  args: Vec<String>,
  silent: bool,
  format_str: String,
  legend_style_str: String,
  summary_style_str: String,
) -> eyre::Result<i32> {
  use std::{
    io::{BufRead, BufReader},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
  };

  let mut child = Command::new(command)
    .args(&args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(crate::error::RomError::Io)?;

  let stderr = child.stderr.take().expect("Failed to capture stderr");
  let stdout = child.stdout.take().expect("Failed to capture stdout");

  // Create shared state
  let state = Arc::new(Mutex::new(crate::state::State::new()));
  let state_clone = state.clone();
  let render_state = state;

  // Track whether we're done processing
  let processing_done = Arc::new(Mutex::new(false));
  let processing_done_clone = processing_done.clone();

  // Track start time for initial timer
  let start_time = Arc::new(Mutex::new(crate::state::current_time()));
  let start_time_clone = start_time.clone();

  // Spawn thread to read and parse stderr (where nix outputs logs)
  let stderr_thread = thread::spawn(move || {
    use tracing::debug;
    let reader = BufReader::new(stderr);
    let mut json_count = 0;
    let mut non_json_count = 0;

    for line in reader.lines().map_while(Result::ok) {
      // Try to parse as JSON message
      if let Some(json_line) = line.strip_prefix("@nix ") {
        json_count += 1;
        if let Ok(action) = serde_json::from_str::<cognos::Actions>(json_line) {
          debug!("Parsed JSON message #{}: {:?}", json_count, action);

          // Print messages immediately to stdout
          if let cognos::Actions::Message { msg, .. } = &action {
            println!("{msg}");
          }

          let mut state = state_clone.lock().unwrap();
          let derivation_count_before = state.derivation_infos.len();
          crate::update::process_message(&mut state, action);
          crate::update::maintain_state(
            &mut state,
            crate::state::current_time(),
          );
          let derivation_count_after = state.derivation_infos.len();
          if derivation_count_after != derivation_count_before {
            debug!(
              "Derivation count changed: {} -> {}",
              derivation_count_before, derivation_count_after
            );
          }
        } else {
          debug!("Failed to parse JSON: {}", json_line);
        }
      } else {
        // Non-JSON lines, pass through
        non_json_count += 1;
        println!("{line}");
      }
    }
    debug!(
      "Stderr thread finished: {} JSON messages, {} non-JSON lines",
      json_count, non_json_count
    );
    *processing_done_clone.lock().unwrap() = true;
  });

  // Read stdout (final nix output)
  let stdout_lines = Arc::new(Mutex::new(Vec::new()));
  let stdout_lines_clone = stdout_lines.clone();

  let stdout_thread = thread::spawn(move || {
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
      stdout_lines_clone.lock().unwrap().push(line);
    }
  });

  // Render loop - this is what displays the build graph
  let render_thread = thread::spawn(move || {
    use crate::display::{Display, DisplayConfig};

    let legend_style = match legend_style_str.to_lowercase().as_str() {
      "compact" => crate::display::LegendStyle::Compact,
      "verbose" => crate::display::LegendStyle::Verbose,
      _ => crate::display::LegendStyle::Table,
    };

    let format = crate::types::DisplayFormat::from_str(&format_str);

    let summary_style = match summary_style_str.to_lowercase().as_str() {
      "table" => crate::display::SummaryStyle::Table,
      "full" => crate::display::SummaryStyle::Full,
      _ => crate::display::SummaryStyle::Concise,
    };

    let display_config = DisplayConfig {
      show_timers: !silent,
      max_tree_depth: 10,
      max_visible_lines: 100,
      use_color: !silent,
      format,
      legend_style,
      summary_style,
    };

    let mut display = Display::new(io::stderr(), display_config).unwrap();
    let mut last_timer_display: Option<String> = None;

    // Render loop
    loop {
      thread::sleep(Duration::from_millis(100));
      let done = *processing_done.lock().unwrap();

      let state = render_state.lock().unwrap();
      let has_activity = !state.derivation_infos.is_empty()
        || !state.full_summary.running_builds.is_empty()
        || !state.full_summary.planned_builds.is_empty();

      if !silent {
        if has_activity
          || state.progress_state != ProgressState::JustStarted
        {
          // Clear any previous timer display
          if last_timer_display.is_some() {
            display.clear_previous().ok();
            last_timer_display = None;
          }
          let _ = display.render(&state, &[]);
        } else {
          // Show initial timer while waiting for activity
          let start = *start_time_clone.lock().unwrap();
          let elapsed = crate::state::current_time() - start;
          let timer_text =
            format!("⏱ {}", crate::display::format_duration(elapsed));

          // Only update if changed (to avoid flicker)
          if last_timer_display.as_ref() != Some(&timer_text) {
            display.clear_previous().ok();
            eprintln!("{timer_text}");
            last_timer_display = Some(timer_text);
          }
        }
      }

      if done {
        break;
      }
    }

    // Give it a moment for final state updates
    thread::sleep(Duration::from_millis(50));

    // Final render
    if !silent {
      let mut state = render_state.lock().unwrap();
      crate::update::finish_state(&mut state);
      let _ = display.render_final(&state);
    }
  });

  // Wait for process to complete
  let status = child.wait().map_err(crate::error::RomError::Io)?;

  // Wait for threads to finish
  let _ = stderr_thread.join();
  let _ = stdout_thread.join();
  let _ = render_thread.join();

  // Print captured stdout (nix's final output)
  let stdout_lines = stdout_lines.lock().unwrap();
  for line in stdout_lines.iter() {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{line}");
  }

  Ok(status.code().unwrap_or(1))
}

/// Replace --command/-c arguments with "sh -c exit" for monitoring pass
fn replace_command_with_exit(args: &[String]) -> Vec<String> {
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_replace_command_with_exit() {
    let args = vec![
      "nixpkgs#hello".to_string(),
      "--command".to_string(),
      "bash".to_string(),
    ];

    let result = replace_command_with_exit(&args);
    assert_eq!(result[0], "nixpkgs#hello");
    assert!(result.contains(&"--command".to_string()));
    assert!(result.contains(&"exit".to_string()));
    assert!(!result.contains(&"bash".to_string()));
  }

  #[test]
  fn test_replace_command_short_form() {
    let args = vec![
      "nixpkgs#hello".to_string(),
      "-c".to_string(),
      "echo test".to_string(),
    ];

    let result = replace_command_with_exit(&args);
    assert_eq!(result[0], "nixpkgs#hello");
    assert!(result.contains(&"exit".to_string()));
    assert!(!result.contains(&"echo test".to_string()));
  }

  #[test]
  fn test_parse_args_with_separator() {
    // Test with separator
    let args = vec![
      "nixpkgs#hello".to_string(),
      "--".to_string(),
      "--help".to_string(),
    ];
    let (before, after) = parse_args_with_separator(&args);
    assert_eq!(before, vec!["nixpkgs#hello".to_string()]);
    assert_eq!(after, vec!["--help".to_string()]);

    // Test without separator (backward compatibility)
    let args = vec!["nixpkgs#hello".to_string(), "--help".to_string()];
    let (before, after) = parse_args_with_separator(&args);
    assert_eq!(before, vec![
      "nixpkgs#hello".to_string(),
      "--help".to_string()
    ]);
    assert_eq!(after, Vec::<String>::new());

    // Test with multiple nix args after separator
    let args = vec![
      "nixpkgs#hello".to_string(),
      "--".to_string(),
      "--option".to_string(),
      "foo".to_string(),
      "bar".to_string(),
    ];
    let (before, after) = parse_args_with_separator(&args);
    assert_eq!(before, vec!["nixpkgs#hello".to_string()]);
    assert_eq!(after, vec![
      "--option".to_string(),
      "foo".to_string(),
      "bar".to_string()
    ]);

    // Test with only separator (edge case)
    let args = vec!["--".to_string(), "--help".to_string()];
    let (before, after) = parse_args_with_separator(&args);
    assert_eq!(before, Vec::<String>::new());
    assert_eq!(after, vec!["--help".to_string()]);
  }
}
