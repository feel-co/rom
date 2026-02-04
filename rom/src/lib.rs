//! ROM - Rust Output Monitor
pub mod cache;
pub mod cli;
pub mod display;
pub mod error;
pub mod monitor;
pub mod state;
pub mod types;
pub mod update;

use std::io::{BufRead, Write};

pub use cli::{Cli, Commands};
pub use error::{Result, RomError};
pub use monitor::Monitor;
pub use types::{Config, InputMode};

/// Monitor a stream of nix output and display enhanced progress information.
///
/// # Arguments
///
/// * `config` - Configuration for the monitor
/// * `reader` - Input stream containing nix output
/// * `writer` - Output stream for enhanced display
///
/// # Errors
///
/// Returns an error if monitoring fails due to I/O issues or parsing errors.
pub fn monitor_stream<R, W>(config: Config, reader: R, writer: W) -> Result<()>
where
  R: BufRead,
  W: Write,
{
  let mut monitor = Monitor::new(config, writer)?;
  monitor.process_stream(reader)
}

/// Run the CLI application with the provided arguments.
///
/// This is the main entry point for the CLI application.
pub fn run() -> eyre::Result<()> {
  cli::run()
}

/// Create a new monitor instance with the given configuration.
pub fn create_monitor<W: Write>(
  config: Config,
  writer: W,
) -> Result<Monitor<W>> {
  Monitor::new(config, writer)
}
