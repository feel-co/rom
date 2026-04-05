//! ROM CLI - command-line interface and nix process wrappers
mod cli;

pub use cli::{
  Cli,
  Commands,
  parse_args_with_separator,
  replace_command_with_exit,
  run,
};
