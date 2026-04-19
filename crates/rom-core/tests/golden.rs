//! Replay a sequence of `@nix`-formatted JSON actions through the state
//! machine and assert the resulting state is correct. This is rom's smoke
//! test for end-to-end JSON ingestion.
//!
//! The fixture is inline rather than a captured file so the test is
//! self-contained and deterministic across environments.

use std::collections::HashSet;

use rom_core::{
  state::{BuildStatus, ProgressState, State},
  update,
};

fn replay(lines: &[&str]) -> State {
  let mut state = State::new();
  for line in lines {
    // The outer wire format is `@nix <json>`; inline fixture omits the
    // prefix since this test only exercises the JSON parser directly.
    let Ok(action) = serde_json::from_str::<cognos::Actions>(line) else {
      panic!("fixture line failed to parse: {line}");
    };
    update::process_message(&mut state, action);
    update::maintain_state(&mut state, rom_core::state::current_time());
  }
  update::summaries(&mut state);
  update::finish_state(&mut state);
  update::summaries(&mut state);
  state
}

/// Activity ids are arbitrary u64s; the message order matters more than
/// the values.
const HELLO_BUILD_DRV: &str =
  "/nix/store/hhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhh-hello-2.12.2.drv";

/// Minimal successful single-build trace.
///
/// Start Builds (top-level), start Build for hello, Stop hello, Stop Builds.
fn standard_fixture() -> Vec<String> {
  vec![
    // Start the top-level "builds" activity.
    r#"{"action":"start","id":1,"level":3,"parent":0,"text":"building 1 derivation","type":104,"fields":[]}"#
      .to_string(),
    // Start a concrete build of hello.
    format!(
      r#"{{"action":"start","id":2,"level":3,"parent":1,"text":"building '{HELLO_BUILD_DRV}'","type":105,"fields":["{HELLO_BUILD_DRV}","","",0,0]}}"#
    ),
    // Stop the build — success path.
    r#"{"action":"stop","id":2}"#.to_string(),
    // Stop the top-level Builds activity.
    r#"{"action":"stop","id":1}"#.to_string(),
  ]
}

/// Build that fails with an exit code.
fn fail_fixture() -> Vec<String> {
  let drv = "/nix/store/ffffffffffffffffffffffffffffffff-broken-0.1.drv";
  vec![
    r#"{"action":"start","id":1,"level":3,"parent":0,"text":"building 1 derivation","type":104,"fields":[]}"#
      .to_string(),
    format!(
      r#"{{"action":"start","id":2,"level":3,"parent":1,"text":"building '{drv}'","type":105,"fields":["{drv}","","",0,0]}}"#
    ),
    // Builder log line announcing failure.
    format!(
      r#"{{"action":"msg","level":0,"msg":"error: builder for '{drv}' failed with exit code 1"}}"#
    ),
    r#"{"action":"stop","id":2}"#.to_string(),
    r#"{"action":"stop","id":1}"#.to_string(),
  ]
}

#[test]
fn standard_replay_processes_without_panic() {
  let fixture = standard_fixture();
  let lines: Vec<&str> = fixture.iter().map(String::as_str).collect();
  let state = replay(&lines);

  assert_eq!(state.progress_state, ProgressState::Finished);
  assert!(
    !state.derivation_infos.is_empty(),
    "expected at least one derivation to be tracked"
  );
}

#[test]
fn standard_replay_has_no_builds_left_running() {
  let fixture = standard_fixture();
  let lines: Vec<&str> = fixture.iter().map(String::as_str).collect();
  let state = replay(&lines);

  let still_running: Vec<_> = state
    .derivation_infos
    .iter()
    .filter(|(_, info)| matches!(info.build_status, BuildStatus::Building(_)))
    .collect();
  assert!(
    still_running.is_empty(),
    "{} build(s) left in Building state after finish",
    still_running.len()
  );
}

#[test]
fn standard_replay_records_completion() {
  let fixture = standard_fixture();
  let lines: Vec<&str> = fixture.iter().map(String::as_str).collect();
  let state = replay(&lines);

  assert!(
    !state.full_summary.completed_builds.is_empty(),
    "expected at least one completed build in summary"
  );
  assert!(
    state.full_summary.failed_builds.is_empty(),
    "standard fixture should not record any failures"
  );
}

#[test]
fn fail_replay_captures_error_message() {
  let fixture = fail_fixture();
  let lines: Vec<&str> = fixture.iter().map(String::as_str).collect();
  let state = replay(&lines);

  assert_eq!(state.progress_state, ProgressState::Finished);
  assert!(
    state
      .nix_errors
      .iter()
      .any(|e| e.contains("failed with exit code")),
    "expected an error mentioning the exit code, got: {:?}",
    state.nix_errors
  );
}

#[test]
fn fail_replay_deduplicates_errors() {
  let fixture = fail_fixture();
  let lines: Vec<&str> = fixture.iter().map(String::as_str).collect();
  let state = replay(&lines);

  use std::collections::HashSet;
  let mut seen = HashSet::new();
  for err in &state.nix_errors {
    assert!(seen.insert(err.clone()), "duplicate error tracked: {err}");
  }
}

#[test]
fn topological_summaries_propagate_across_diamond() {
  // Diamond dependency: root depends on both mid_a and mid_b, which both
  // depend on leaf. After a full summary recompute, root's summary should
  // include leaf exactly once (no double-count) and every mid.
  use std::path::PathBuf;

  use rom_core::state::{DependencySummary, Derivation, InputDerivation};

  let _ = DependencySummary::default(); // import-only use
  let mut state = State::new();

  let root_id = state.get_or_create_derivation_id(Derivation {
    path: PathBuf::from("/nix/store/rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr-root.drv"),
    name: "root".to_string(),
  });
  let mid_a_id = state.get_or_create_derivation_id(Derivation {
    path: PathBuf::from(
      "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-mid-a.drv",
    ),
    name: "mid-a".to_string(),
  });
  let mid_b_id = state.get_or_create_derivation_id(Derivation {
    path: PathBuf::from(
      "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-mid-b.drv",
    ),
    name: "mid-b".to_string(),
  });
  let leaf_id = state.get_or_create_derivation_id(Derivation {
    path: PathBuf::from("/nix/store/llllllllllllllllllllllllllllllll-leaf.drv"),
    name: "leaf".to_string(),
  });

  // Wire dependencies: root -> {mid_a, mid_b}; mid_a -> leaf; mid_b -> leaf.
  let root = state.derivation_infos.get_mut(&root_id).unwrap();
  root.input_derivations.push(InputDerivation {
    derivation: mid_a_id,
    outputs:    HashSet::default(),
  });
  root.input_derivations.push(InputDerivation {
    derivation: mid_b_id,
    outputs:    HashSet::default(),
  });
  let mid_a = state.derivation_infos.get_mut(&mid_a_id).unwrap();
  mid_a.input_derivations.push(InputDerivation {
    derivation: leaf_id,
    outputs:    HashSet::default(),
  });
  let mid_b = state.derivation_infos.get_mut(&mid_b_id).unwrap();
  mid_b.input_derivations.push(InputDerivation {
    derivation: leaf_id,
    outputs:    HashSet::default(),
  });

  // Parent backlinks, same direction as input_derivations but reversed.
  state
    .derivation_infos
    .get_mut(&mid_a_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);
  state
    .derivation_infos
    .get_mut(&mid_b_id)
    .unwrap()
    .derivation_parents
    .insert(root_id);
  state
    .derivation_infos
    .get_mut(&leaf_id)
    .unwrap()
    .derivation_parents
    .insert(mid_a_id);
  state
    .derivation_infos
    .get_mut(&leaf_id)
    .unwrap()
    .derivation_parents
    .insert(mid_b_id);

  // Mark leaf as planned so it has something to aggregate.
  state
    .derivation_infos
    .get_mut(&leaf_id)
    .unwrap()
    .build_status = BuildStatus::Planned;

  update::summaries(&mut state);

  let root_summary = &state
    .derivation_infos
    .get(&root_id)
    .unwrap()
    .dependency_summary;
  // Leaf shows up once in the planned set even though it's reachable via
  // two distinct parents — HashSet semantics enforce this, and the topo
  // walk is what makes the dedup load-bearing.
  assert!(
    root_summary.planned_builds.contains(&leaf_id),
    "root's summary should include leaf"
  );
  assert_eq!(
    root_summary.planned_builds.len(),
    1,
    "diamond dependency should not double-count leaf"
  );
}
