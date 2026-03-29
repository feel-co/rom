//! ROM - Rust Output Monitor
pub use rom_core::{
  Config,
  InputMode,
  Monitor,
  Result,
  RomError,
  cache,
  create_monitor,
  display,
  error,
  monitor,
  monitor_stream,
  state,
  types,
  update,
};

pub mod cli {
  pub use rom_cli::{Cli, Commands, parse_args_with_separator};
}

/// Run the CLI application with the provided arguments.
///
/// This is the main entry point for the CLI application.
pub fn run() -> eyre::Result<()> {
  rom_cli::run()
}
