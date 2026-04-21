use std::process::ExitCode;

fn main() -> ExitCode {
  match rom::run() {
    Ok(()) => ExitCode::SUCCESS,
    Err(rom_core::RomError::BuildFailed) => ExitCode::FAILURE,
    Err(err) => {
      eprintln!("rom: {err}");
      ExitCode::FAILURE
    },
  }
}
