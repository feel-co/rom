use std::collections::HashMap;

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
pub use state::{
  BuildInfo,
  BuildStatus,
  Dependencies,
  Derivation,
  Host,
  OutputName,
  ProgressState,
  State,
};

/// Process a list of actions and return the resulting state
#[must_use]
pub fn process_actions(actions: Vec<Actions>) -> State {
  let mut state = State {
    progress:          ProgressState::JustStarted,
    derivations:       HashMap::new(),
    builds:            HashMap::new(),
    dependencies:      Dependencies {
      deps: HashMap::new(),
    },
    store_paths:       HashMap::new(),
    dependency_states: HashMap::new(),
  };
  for action in actions {
    state.imbibe(action);
  }
  state
}
