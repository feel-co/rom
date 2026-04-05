use rom_core::state::{Derivation, ProgressState, State, StorePath};

#[test]
fn test_store_path_parse() {
  let path = "/nix/store/abc123-hello-1.0";
  let sp = StorePath::parse(path).unwrap();
  assert_eq!(sp.hash, "abc123");
  assert_eq!(sp.name, "hello-1.0");
}

#[test]
fn test_derivation_parse() {
  let path = "/nix/store/abc123-hello-1.0.drv";
  let drv = Derivation::parse(path).unwrap();
  assert_eq!(drv.name, "hello-1.0");
}

#[test]
fn test_state_creation() {
  let state = State::new();
  assert_eq!(state.progress_state, ProgressState::JustStarted);
  assert_eq!(state.total_builds(), 0);
}

#[test]
fn test_get_or_create_ids() {
  let mut state = State::new();
  let path = StorePath::parse("/nix/store/abc123-hello-1.0").unwrap();
  let id1 = state.get_or_create_store_path_id(path.clone());
  let id2 = state.get_or_create_store_path_id(path);
  assert_eq!(id1, id2);
}
