pub mod aterm;
mod internal_json;
mod state;

pub use aterm::{
  ParsedDerivation,
  extract_pname,
  extract_version,
  parse_drv_file,
};
pub use internal_json::{Actions, Activities, Id, Verbosity};
pub use state::{BuildInfo, BuildStatus, Derivation, Host, OutputName, State, ProgressState};

/// Process a list of actions and return the resulting state
pub fn process_actions(actions: Vec<Actions>) -> State {
  let mut state = State { progress: ProgressState::JustStarted };
  for action in actions {
    state.imbibe(action);
  }
  state
}
