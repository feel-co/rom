//! State update logic for processing nix messages

use cognos::{Actions, Activities, Id, Verbosity};
use tracing::{debug, trace};

use crate::state::{
  ActivityStatus,
  BuildFail,
  BuildInfo,
  BuildStatus,
  CompletedBuildInfo,
  CompletedTransferInfo,
  Derivation,
  DerivationId,
  FailType,
  FailedBuildInfo,
  Host,
  InputDerivation,
  OutputName,
  ProgressState,
  State,
  StorePath,
  StorePathId,
  StorePathState,
  TransferInfo,
  current_time,
};

/// Process a nix JSON message and update state
pub fn process_message(state: &mut State, action: Actions) -> bool {
  let now = current_time();
  let mut changed = false;

  // Mark that we've received input
  if state.progress_state == ProgressState::JustStarted {
    state.progress_state = ProgressState::InputReceived;
    changed = true;
  }

  trace!("Processing action: {:?}", action);

  match action {
    Actions::Start {
      id,
      level,
      parent,
      text,
      activity,
      fields,
    } => {
      changed |=
        handle_start(state, id, level, parent, text, activity, fields, now);
    },
    Actions::Stop { id } => {
      changed |= handle_stop(state, id, now);
    },
    Actions::Message { level, msg } => {
      changed |= handle_message(state, level, msg);
    },
    Actions::Result {
      id,
      activity,
      fields,
    } => {
      changed |= handle_result(state, id, activity as u8, fields, now);
    },
  }

  changed
}

fn handle_start(
  state: &mut State,
  id: Id,
  _level: Verbosity,
  parent: Id,
  text: String,
  activity: Activities,
  fields: Vec<serde_json::Value>,
  now: f64,
) -> bool {
  // Store activity status
  let parent_id = if parent == 0 { None } else { Some(parent) };

  let activity_u8 = activity as u8;

  state.activities.insert(id, ActivityStatus {
    activity: activity_u8,
    text:     text.clone(),
    parent:   parent_id,
    phase:    None,
  });

  let changed = match activity_u8 {
    104 | 105 => handle_build_start(state, id, parent_id, &text, &fields, now), /* Builds | Build */
    108 => handle_substitute_start(state, id, &text, &fields, now), /* Substitute */
    101 => handle_transfer_start(state, id, &text, &fields, now, false), /* FileTransfer */
    100 | 103 => handle_transfer_start(state, id, &text, &fields, now, true), /* CopyPath | CopyPaths */
    _ => false,
  };

  // Track parent-child relationships for dependency tree
  if changed
    && (activity_u8 == 104 || activity_u8 == 105)
    && parent_id.is_some()
  {
    let parent_act_id = parent_id.unwrap();

    // Find parent and child derivation IDs
    let parent_drv_id = find_derivation_by_activity(state, parent_act_id);
    let child_drv_id = find_derivation_by_activity(state, id);

    if let Some(parent_drv_id) = parent_drv_id {
      if let Some(child_drv_id) = child_drv_id {
        debug!(
          "Establishing parent-child relationship: parent={}, child={}",
          parent_drv_id, child_drv_id
        );

        // Add child as a dependency of parent
        if let Some(parent_info) = state.get_derivation_info_mut(parent_drv_id)
        {
          let input = InputDerivation {
            derivation: child_drv_id,
            outputs:    std::collections::HashSet::new(),
          };
          if !parent_info
            .input_derivations
            .iter()
            .any(|d| d.derivation == child_drv_id)
          {
            parent_info.input_derivations.push(input);
            debug!("Added child to parent's input_derivations");
          }
        }
        // Mark child as having a parent
        if let Some(child_info) = state.get_derivation_info_mut(child_drv_id) {
          child_info.derivation_parents.insert(parent_drv_id);
        }
        // Remove child from forest roots since it has a parent
        state.forest_roots.retain(|&id| id != child_drv_id);
      }
    }
  }

  changed
}

fn handle_stop(state: &mut State, id: Id, now: f64) -> bool {
  let activity = state.activities.get(&id).cloned();

  if let Some(activity_status) = activity {
    state.activities.remove(&id);

    match activity_status.activity {
      104 | 105 => handle_build_stop(state, id, now), // Builds | Build
      108 => handle_substitute_stop(state, id, now),  // Substitute
      101 | 100 | 103 => handle_transfer_stop(state, id, now), /* FileTransfer, CopyPath, CopyPaths */
      _ => false,
    }
  } else {
    false
  }
}

fn handle_message(state: &mut State, level: Verbosity, msg: String) -> bool {
  // Store all build logs for display
  state.build_logs.push(msg.clone());

  // Extract phase from log messages like "Running phase: configurePhase"
  if let Some(phase_start) = msg.find("Running phase: ") {
    let phase_name = &msg[phase_start + 15..]; // Skip "Running phase: "
    let phase = phase_name.trim().to_string();

    // Find the active build and update its phase
    for activity in state.activities.values_mut() {
      if activity.activity == 105 {
        // Build activity
        activity.phase = Some(phase.clone());
      }
    }
  }

  match level {
    Verbosity::Error => {
      // Track errors
      if msg.contains("error:") || msg.contains("failed") {
        state.nix_errors.push(msg.clone());

        // Try to extract which build failed
        if let Some(drv_path) = extract_derivation_from_error(&msg) {
          if let Some(drv) = Derivation::parse(&drv_path) {
            let drv_id = state.get_or_create_derivation_id(drv);

            // Get build info first
            let build_info_opt =
              state.get_derivation_info(drv_id).and_then(|info| {
                if let BuildStatus::Building(build_info) = &info.build_status {
                  Some(build_info.clone())
                } else {
                  None
                }
              });

            if let Some(build_info) = build_info_opt {
              let fail = BuildFail {
                at:        current_time(),
                fail_type: parse_fail_type(&msg),
              };

              state.update_build_status(drv_id, BuildStatus::Failed {
                info: build_info,
                fail,
              });
            }
          }
        }
        return true;
      }
      false
    },
    Verbosity::Info | Verbosity::Notice => {
      // Track info messages for evaluation progress
      if msg.contains("evaluating") || msg.contains("copying") {
        // Update evaluation state
        if let Some(file_name) = extract_file_name(&msg) {
          state.evaluation_state.last_file_name = Some(file_name);
          state.evaluation_state.count += 1;
          state.evaluation_state.at = current_time();
        }
      }
      true // return true since we stored the log
    },
    _ => {
      true // return true since we stored the log
    },
  }
}

fn handle_result(
  state: &mut State,
  id: Id,
  activity: u8,
  fields: Vec<serde_json::Value>,
  _now: f64,
) -> bool {
  match activity {
    101 | 108 => {
      // FileTransfer or Substitute
      // Fields contain progress information
      // XXX: Format: [bytes_transferred, total_bytes]
      if fields.len() >= 2 {
        update_transfer_progress(state, id, &fields);
      }
      false
    },
    104 => {
      // Builds activity type - contains phase information
      if !fields.is_empty() {
        if let Some(phase_str) = fields[0].as_str() {
          // Update the activity's phase field
          if let Some(activity) = state.activities.get_mut(&id) {
            activity.phase = Some(phase_str.to_string());
            return true;
          }
        }
      }
      false
    },
    105 => {
      // Build completed, fields contain output path
      complete_build(state, id)
    },
    _ => false,
  }
}

fn handle_build_start(
  state: &mut State,
  id: Id,
  parent_id: Option<Id>,
  text: &str,
  fields: &[serde_json::Value],
  now: f64,
) -> bool {
  debug!(
    "handle_build_start: id={}, text={}, fields={:?}",
    id, text, fields
  );

  // First try to get derivation path from fields
  let drv_path = if fields.is_empty() {
    extract_derivation_path(text)
  } else {
    fields[0].as_str().map(std::string::ToString::to_string)
  };

  if let Some(drv_path) = drv_path {
    debug!("Extracted derivation path: {}", drv_path);
    if let Some(drv) = Derivation::parse(&drv_path) {
      let drv_id = state.get_or_create_derivation_id(drv);
      let host = extract_host(text);

      let build_info = BuildInfo {
        start: now,
        host,
        estimate: None,
        activity_id: Some(id),
      };

      debug!("Setting derivation {} to Building status", drv_id);
      state.update_build_status(drv_id, BuildStatus::Building(build_info));
      debug!(
        "After update_build_status, state has {} derivations",
        state.derivation_infos.len()
      );

      // Parse .drv file to populate dependency tree
      state.populate_derivation_dependencies(drv_id);
      debug!(
        "After populate_derivation_dependencies, state has {} derivations",
        state.derivation_infos.len()
      );

      // Mark as forest root if no parent
      // Only add to forest roots if no parent
      if parent_id.is_none() && !state.forest_roots.contains(&drv_id) {
        state.forest_roots.push(drv_id);
      }

      // Store activity -> derivation mapping
      // Phase will be extracted from log messages
      return true;
    }
    debug!("Failed to parse derivation from path: {}", drv_path);
  } else {
    debug!(
      "No derivation path found - creating placeholder for activity {}",
      id
    );
    // For shell/develop commands, nix doesn't report specific derivation paths
    // Create a placeholder derivation to track that builds are happening
    use std::path::PathBuf;

    let placeholder_name = format!("building-{}", id);
    let placeholder_path = format!("/nix/store/placeholder-{}.drv", id);

    let placeholder_drv = Derivation {
      path: PathBuf::from(placeholder_path),
      name: placeholder_name,
    };

    let drv_id = state.get_or_create_derivation_id(placeholder_drv);
    let host = extract_host(text);

    let build_info = BuildInfo {
      start: now,
      host,
      estimate: None,
      activity_id: Some(id),
    };

    debug!(
      "Setting placeholder derivation {} to Building status",
      drv_id
    );
    state.update_build_status(drv_id, BuildStatus::Building(build_info));

    // Mark as forest root if no parent
    if parent_id.is_none() && !state.forest_roots.contains(&drv_id) {
      state.forest_roots.push(drv_id);
    }

    return true;
  }
  false
}

fn handle_build_stop(state: &mut State, id: Id, _now: f64) -> bool {
  // Find the derivation associated with this activity
  for (drv_id, info) in &state.derivation_infos {
    match &info.build_status {
      BuildStatus::Building(build_info)
        if build_info.activity_id == Some(id) =>
      {
        // Build was stopped but not marked as completed
        // It might be cancelled
        debug!("Build stopped for derivation {}", drv_id);
        return false;
      },
      _ => {},
    }
  }
  false
}

fn handle_substitute_start(
  state: &mut State,
  id: Id,
  text: &str,
  fields: &[serde_json::Value],
  now: f64,
) -> bool {
  // Extract store path
  let path_str = if fields.is_empty() {
    extract_store_path(text)
  } else {
    fields[0].as_str().map(std::string::ToString::to_string)
  };

  if let Some(path_str) = path_str {
    if let Some(path) = StorePath::parse(&path_str) {
      let path_id = state.get_or_create_store_path_id(path);
      let host = extract_host(text);

      let transfer = TransferInfo {
        start: now,
        host,
        activity_id: id,
        bytes_transferred: 0,
        total_bytes: None,
      };

      if let Some(path_info) = state.get_store_path_info_mut(path_id) {
        path_info
          .states
          .insert(StorePathState::Downloading(transfer.clone()));
      }

      state
        .full_summary
        .running_downloads
        .insert(path_id, transfer);

      return true;
    }
  }
  false
}

fn handle_substitute_stop(state: &mut State, id: Id, now: f64) -> bool {
  // Find the store path associated with this activity
  for (path_id, transfer_info) in &state.full_summary.running_downloads.clone()
  {
    if transfer_info.activity_id == id {
      state.full_summary.running_downloads.remove(path_id);

      let completed = CompletedTransferInfo {
        start:       transfer_info.start,
        end:         now,
        host:        transfer_info.host.clone(),
        total_bytes: transfer_info.bytes_transferred,
      };

      state
        .full_summary
        .completed_downloads
        .insert(*path_id, completed);

      if let Some(path_info) = state.get_store_path_info_mut(*path_id) {
        path_info
          .states
          .remove(&StorePathState::Downloading(transfer_info.clone()));
        path_info.states.insert(StorePathState::Downloaded(
          CompletedTransferInfo {
            start:       transfer_info.start,
            end:         now,
            host:        transfer_info.host.clone(),
            total_bytes: transfer_info.bytes_transferred,
          },
        ));
      }

      return true;
    }
  }
  false
}

fn handle_transfer_start(
  state: &mut State,
  id: Id,
  text: &str,
  fields: &[serde_json::Value],
  now: f64,
  is_copy: bool,
) -> bool {
  let path_str = if fields.is_empty() {
    extract_store_path(text)
  } else {
    fields[0].as_str().map(std::string::ToString::to_string)
  };

  if let Some(path_str) = path_str {
    if let Some(path) = StorePath::parse(&path_str) {
      let path_id = state.get_or_create_store_path_id(path);
      let host = extract_host(text);

      let transfer = TransferInfo {
        start: now,
        host,
        activity_id: id,
        bytes_transferred: 0,
        total_bytes: None,
      };

      if is_copy {
        state.full_summary.running_uploads.insert(path_id, transfer);
      } else {
        state
          .full_summary
          .running_downloads
          .insert(path_id, transfer);
      }

      return true;
    }
  }
  false
}

fn handle_transfer_stop(state: &mut State, id: Id, now: f64) -> bool {
  // Check downloads
  for (path_id, transfer_info) in &state.full_summary.running_downloads.clone()
  {
    if transfer_info.activity_id == id {
      state.full_summary.running_downloads.remove(path_id);

      let completed = CompletedTransferInfo {
        start:       transfer_info.start,
        end:         now,
        host:        transfer_info.host.clone(),
        total_bytes: transfer_info.bytes_transferred,
      };

      state
        .full_summary
        .completed_downloads
        .insert(*path_id, completed);
      return true;
    }
  }

  // Check uploads
  for (path_id, transfer_info) in &state.full_summary.running_uploads.clone() {
    if transfer_info.activity_id == id {
      state.full_summary.running_uploads.remove(path_id);

      let completed = CompletedTransferInfo {
        start:       transfer_info.start,
        end:         now,
        host:        transfer_info.host.clone(),
        total_bytes: transfer_info.bytes_transferred,
      };

      state
        .full_summary
        .completed_uploads
        .insert(*path_id, completed);
      return true;
    }
  }

  false
}

fn update_transfer_progress(
  state: &mut State,
  id: Id,
  fields: &[serde_json::Value],
) {
  if fields.len() < 2 {
    return;
  }

  let bytes_transferred = fields[0].as_u64().unwrap_or(0);
  let total_bytes = fields[1].as_u64();

  // Update running downloads
  for transfer_info in state.full_summary.running_downloads.values_mut() {
    if transfer_info.activity_id == id {
      transfer_info.bytes_transferred = bytes_transferred;
      transfer_info.total_bytes = total_bytes;
      return;
    }
  }

  // Update running uploads
  for transfer_info in state.full_summary.running_uploads.values_mut() {
    if transfer_info.activity_id == id {
      transfer_info.bytes_transferred = bytes_transferred;
      transfer_info.total_bytes = total_bytes;
      return;
    }
  }
}

fn complete_build(state: &mut State, id: Id) -> bool {
  // Find the derivation that just completed
  for (drv_id, info) in &state.derivation_infos.clone() {
    if let BuildStatus::Building(build_info) = &info.build_status {
      if build_info.activity_id == Some(id) {
        let end = current_time();
        state.update_build_status(*drv_id, BuildStatus::Built {
          info: build_info.clone(),
          end,
        });
        return true;
      }
    }
  }
  false
}

fn extract_derivation_path(text: &str) -> Option<String> {
  // Look for .drv paths in the text
  if let Some(start) = text.find("/nix/store/") {
    if let Some(end) = text[start..].find(".drv") {
      return Some(text[start..start + end + 4].to_string());
    }
  }
  None
}

fn extract_store_path(text: &str) -> Option<String> {
  // Look for store paths in the text
  if let Some(start) = text.find("/nix/store/") {
    // Find the end of the path (space or end of string)
    let rest = &text[start..];
    let end = rest
      .find(|c: char| c.is_whitespace() || c == '\'' || c == '"')
      .unwrap_or(rest.len());
    return Some(rest[..end].to_string());
  }
  None
}

fn extract_host(text: &str) -> Host {
  if text.contains("on ") {
    // Format: "building X on hostname"
    if let Some(pos) = text.rfind("on ") {
      let rest = &text[pos + 3..];
      let hostname = rest
        .split_whitespace()
        .next()
        .unwrap_or("localhost")
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();
      return Host::Remote(hostname);
    }
  }
  Host::Localhost
}

fn extract_derivation_from_error(msg: &str) -> Option<String> {
  extract_derivation_path(msg)
}

fn extract_file_name(msg: &str) -> Option<String> {
  // Try to extract file name from evaluation messages
  if let Some(start) = msg.find('\'') {
    if let Some(end) = msg[start + 1..].find('\'') {
      return Some(msg[start + 1..start + 1 + end].to_string());
    }
  }
  None
}

fn parse_fail_type(msg: &str) -> FailType {
  if msg.contains("timeout") {
    FailType::Timeout
  } else if msg.contains("hash mismatch") || msg.contains("hash") {
    FailType::HashMismatch
  } else if msg.contains("dependency failed") {
    FailType::DependencyFailed
  } else {
    FailType::Unknown
  }
}

fn find_derivation_by_activity(
  state: &State,
  activity_id: Id,
) -> Option<DerivationId> {
  // Try to find in running builds first
  for (drv_id, build_info) in &state.full_summary.running_builds {
    if build_info.activity_id == Some(activity_id) {
      return Some(*drv_id);
    }
  }

  // Search through all derivations
  for (drv_id, info) in &state.derivation_infos {
    match &info.build_status {
      BuildStatus::Building(build_info)
        if build_info.activity_id == Some(activity_id) =>
      {
        return Some(*drv_id);
      },
      BuildStatus::Built { info, .. }
        if info.activity_id == Some(activity_id) =>
      {
        return Some(*drv_id);
      },
      BuildStatus::Failed { info, .. }
        if info.activity_id == Some(activity_id) =>
      {
        return Some(*drv_id);
      },
      _ => {},
    }
  }

  None
}

/// Maintain state consistency
pub fn maintain_state(state: &mut State, now: f64) {
  // Clear touched IDs - they've been processed
  if !state.touched_ids.is_empty() {
    state.touched_ids.clear();
  }

  // Update summaries
  update_summaries(state, now);
}

fn update_summaries(state: &mut State, _now: f64) {
  use tracing::debug;

  // Update build summaries
  state.full_summary.planned_builds.clear();
  state.full_summary.running_builds.clear();
  state.full_summary.completed_builds.clear();
  state.full_summary.failed_builds.clear();

  debug!(
    "update_summaries: processing {} derivations",
    state.derivation_infos.len()
  );

  let mut building_count = 0;
  let mut planned_count = 0;

  for (drv_id, info) in &state.derivation_infos {
    debug!("  derivation {} status: {:?}", drv_id, info.build_status);
    match &info.build_status {
      BuildStatus::Planned => {
        // Only count explicitly planned builds, not unknown ones
        state.full_summary.planned_builds.insert(*drv_id);
        planned_count += 1;
      },
      BuildStatus::Unknown => {
        // Unknown derivations are cached/already built, don't count them
      },
      BuildStatus::Building(build_info) => {
        debug!("  → Adding {} to running_builds", drv_id);
        state
          .full_summary
          .running_builds
          .insert(*drv_id, build_info.clone());
        building_count += 1;
      },
      BuildStatus::Built { info, end } => {
        state.full_summary.completed_builds.insert(
          *drv_id,
          CompletedBuildInfo {
            start: info.start,
            end:   *end,
            host:  info.host.clone(),
          },
        );
      },
      BuildStatus::Failed { info, fail } => {
        state
          .full_summary
          .failed_builds
          .insert(*drv_id, FailedBuildInfo {
            start:     info.start,
            end:       fail.at,
            host:      info.host.clone(),
            fail_type: fail.fail_type.clone(),
          });
      },
    }
  }

  debug!(
    "update_summaries complete: {} running (counted {}), {} planned (counted \
     {}), {} completed, {} failed",
    state.full_summary.running_builds.len(),
    building_count,
    state.full_summary.planned_builds.len(),
    planned_count,
    state.full_summary.completed_builds.len(),
    state.full_summary.failed_builds.len()
  );
}

fn complete_build_success(state: &mut State, drv_id: DerivationId, now: f64) {
  let build_info = state.get_derivation_info(drv_id).and_then(|info| {
    if let BuildStatus::Building(build_info) = &info.build_status {
      Some(build_info.clone())
    } else {
      None
    }
  });

  if let Some(build_info) = build_info {
    state.update_build_status(drv_id, BuildStatus::Built {
      info: build_info,
      end:  now,
    });
  }
}

pub fn finish_state(state: &mut State) {
  state.progress_state = ProgressState::Finished;

  let building: Vec<DerivationId> = state
    .derivation_infos
    .iter()
    .filter_map(|(drv_id, info)| {
      if matches!(info.build_status, BuildStatus::Building(_)) {
        Some(*drv_id)
      } else {
        None
      }
    })
    .collect();

  for drv_id in building {
    complete_build_success(state, drv_id, current_time());
  }

  let downloading: Vec<StorePathId> = state
    .full_summary
    .running_downloads
    .keys()
    .copied()
    .collect();
  for path_id in downloading {
    if let Some(transfer) =
      state.full_summary.running_downloads.remove(&path_id)
    {
      let completed = CompletedTransferInfo {
        start:       transfer.start,
        end:         current_time(),
        host:        transfer.host,
        total_bytes: transfer.total_bytes.unwrap_or(0),
      };
      state
        .full_summary
        .completed_downloads
        .insert(path_id, completed.clone());

      if let Some(path_info) = state.get_store_path_info_mut(path_id) {
        path_info.states.clear();
        path_info
          .states
          .insert(StorePathState::Downloaded(completed));
      }
    }
  }

  let uploading: Vec<StorePathId> =
    state.full_summary.running_uploads.keys().copied().collect();
  for path_id in uploading {
    if let Some(transfer) = state.full_summary.running_uploads.remove(&path_id)
    {
      let completed = CompletedTransferInfo {
        start:       transfer.start,
        end:         current_time(),
        host:        transfer.host,
        total_bytes: transfer.total_bytes.unwrap_or(0),
      };
      state
        .full_summary
        .completed_uploads
        .insert(path_id, completed.clone());

      if let Some(path_info) = state.get_store_path_info_mut(path_id) {
        path_info.states.clear();
        path_info.states.insert(StorePathState::Uploaded(completed));
      }
    }
  }
}

/// Parse output name string to `OutputName` enum
fn parse_output_name(s: &str) -> Option<OutputName> {
  match s {
    "out" => Some(OutputName::Out),
    "doc" => Some(OutputName::Doc),
    "dev" => Some(OutputName::Dev),
    "bin" => Some(OutputName::Bin),
    "info" => Some(OutputName::Info),
    "lib" => Some(OutputName::Lib),
    "man" => Some(OutputName::Man),
    "dist" => Some(OutputName::Dist),
    other => Some(OutputName::Other(other.to_string())),
  }
}
