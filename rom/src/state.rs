//! State management for ROM

use std::{
  collections::{HashMap, HashSet},
  path::PathBuf,
  time::{Duration, SystemTime},
};

use cognos::{Host, Id, OutputName, ProgressState};
use indexmap::IndexMap;

/// Unique identifier for store paths
pub type StorePathId = usize;

/// Unique identifier for derivations
pub type DerivationId = usize;

/// Unique identifier for activities
pub type ActivityId = Id;

/// Store path representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorePath {
  pub path: PathBuf,
  pub hash: String,
  pub name: String,
}

impl StorePath {
  #[must_use]
  pub fn parse(path: &str) -> Option<Self> {
    if !path.starts_with("/nix/store/") {
      return None;
    }

    let path_buf = PathBuf::from(path);
    let file_name = path_buf.file_name()?.to_str()?;

    let parts: Vec<&str> = file_name.splitn(2, '-').collect();
    if parts.len() != 2 {
      return None;
    }

    Some(Self {
      path: path_buf.clone(),
      hash: parts[0].to_string(),
      name: parts[1].to_string(),
    })
  }
}

/// Derivation representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Derivation {
  pub path: PathBuf,
  pub name: String,
}

impl Derivation {
  #[must_use]
  pub fn parse(path: &str) -> Option<Self> {
    let path_buf = PathBuf::from(path);
    let file_name = path_buf.file_name()?.to_str()?;

    if !file_name.ends_with(".drv") {
      return None;
    }

    let name = file_name.strip_suffix(".drv")?;
    let parts: Vec<&str> = name.splitn(2, '-').collect();
    let display_name = if parts.len() == 2 {
      parts[1].to_string()
    } else {
      name.to_string()
    };

    Some(Self {
      path: path_buf,
      name: display_name,
    })
  }
}

/// Transfer information (download/upload)
#[derive(Debug, Clone)]
pub struct TransferInfo {
  pub start:             f64,
  pub host:              Host,
  pub activity_id:       ActivityId,
  pub bytes_transferred: u64,
  pub total_bytes:       Option<u64>,
}

/// Completed transfer information
#[derive(Debug, Clone)]
pub struct CompletedTransferInfo {
  pub start:       f64,
  pub end:         f64,
  pub host:        Host,
  pub total_bytes: u64,
}

/// Store path state
#[derive(Debug, Clone)]
pub enum StorePathState {
  DownloadPlanned,
  Downloading(TransferInfo),
  Uploading(TransferInfo),
  Downloaded(CompletedTransferInfo),
  Uploaded(CompletedTransferInfo),
}

/// Store path information
#[derive(Debug, Clone)]
pub struct StorePathInfo {
  pub name:      StorePath,
  pub states:    HashSet<StorePathState>,
  pub producer:  Option<DerivationId>,
  pub input_for: HashSet<DerivationId>,
}

impl PartialEq for StorePathState {
  fn eq(&self, other: &Self) -> bool {
    std::mem::discriminant(self) == std::mem::discriminant(other)
  }
}

impl Eq for StorePathState {}

impl std::hash::Hash for StorePathState {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    std::mem::discriminant(self).hash(state);
  }
}

/// Build information
#[derive(Debug, Clone)]
pub struct BuildInfo {
  pub start:       f64,
  pub host:        Host,
  pub estimate:    Option<u64>,
  pub activity_id: Option<ActivityId>,
}

/// Build failure information
#[derive(Debug, Clone)]
pub struct BuildFail {
  pub at:        f64,
  pub fail_type: FailType,
}

/// Failure type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailType {
  BuildFailed(i32),
  Timeout,
  HashMismatch,
  DependencyFailed,
  Unknown,
}

/// Build status
#[derive(Debug, Clone)]
pub enum BuildStatus {
  Unknown,
  Planned,
  Building(BuildInfo),
  Built { info: BuildInfo, end: f64 },
  Failed { info: BuildInfo, fail: BuildFail },
}

/// Input derivation for dependency tracking
#[derive(Debug, Clone)]
pub struct InputDerivation {
  pub derivation: DerivationId,
  pub outputs:    HashSet<OutputName>,
}

/// Derivation information
#[derive(Debug, Clone)]
pub struct DerivationInfo {
  pub name:               Derivation,
  pub outputs:            HashMap<OutputName, StorePathId>,
  pub input_derivations:  Vec<InputDerivation>,
  pub input_sources:      HashSet<StorePathId>,
  pub build_status:       BuildStatus,
  pub dependency_summary: DependencySummary,
  pub cached:             bool,
  pub derivation_parents: HashSet<DerivationId>,
  pub pname:              Option<String>,
  pub platform:           Option<String>,
}

/// Dependency summary for tracking build progress
#[derive(Debug, Clone, Default)]
pub struct DependencySummary {
  pub planned_builds:      HashSet<DerivationId>,
  pub running_builds:      HashMap<DerivationId, BuildInfo>,
  pub completed_builds:    HashMap<DerivationId, CompletedBuildInfo>,
  pub failed_builds:       HashMap<DerivationId, FailedBuildInfo>,
  pub planned_downloads:   HashSet<StorePathId>,
  pub completed_downloads: HashMap<StorePathId, CompletedTransferInfo>,
  pub completed_uploads:   HashMap<StorePathId, CompletedTransferInfo>,
  pub running_downloads:   HashMap<StorePathId, TransferInfo>,
  pub running_uploads:     HashMap<StorePathId, TransferInfo>,
}

impl DependencySummary {
  pub fn merge(&mut self, other: &Self) {
    self
      .planned_builds
      .extend(other.planned_builds.iter().copied());
    self
      .running_builds
      .extend(other.running_builds.iter().map(|(k, v)| (*k, v.clone())));
    self
      .completed_builds
      .extend(other.completed_builds.iter().map(|(k, v)| (*k, v.clone())));
    self
      .failed_builds
      .extend(other.failed_builds.iter().map(|(k, v)| (*k, v.clone())));
    self
      .planned_downloads
      .extend(other.planned_downloads.iter().copied());
    self.completed_downloads.extend(
      other
        .completed_downloads
        .iter()
        .map(|(k, v)| (*k, v.clone())),
    );
    self
      .completed_uploads
      .extend(other.completed_uploads.iter().map(|(k, v)| (*k, v.clone())));
    self
      .running_downloads
      .extend(other.running_downloads.iter().map(|(k, v)| (*k, v.clone())));
    self
      .running_uploads
      .extend(other.running_uploads.iter().map(|(k, v)| (*k, v.clone())));
  }

  pub fn clear_derivation(
    &mut self,
    id: DerivationId,
    old_status: &BuildStatus,
  ) {
    match old_status {
      BuildStatus::Unknown => {},
      BuildStatus::Planned => {
        self.planned_builds.remove(&id);
      },
      BuildStatus::Building(_) => {
        self.running_builds.remove(&id);
      },
      BuildStatus::Built { .. } => {
        self.completed_builds.remove(&id);
      },
      BuildStatus::Failed { .. } => {
        self.failed_builds.remove(&id);
      },
    }
  }

  pub fn update_derivation(
    &mut self,
    id: DerivationId,
    new_status: &BuildStatus,
  ) {
    match new_status {
      BuildStatus::Unknown => {},
      BuildStatus::Planned => {
        self.planned_builds.insert(id);
      },
      BuildStatus::Building(info) => {
        self.running_builds.insert(id, info.clone());
      },
      BuildStatus::Built { info, end } => {
        self.completed_builds.insert(id, CompletedBuildInfo {
          start: info.start,
          end:   *end,
          host:  info.host.clone(),
        });
      },
      BuildStatus::Failed { info, fail } => {
        self.failed_builds.insert(id, FailedBuildInfo {
          start:     info.start,
          end:       fail.at,
          host:      info.host.clone(),
          fail_type: fail.fail_type.clone(),
        });
      },
    }
  }
}

/// Completed build information
#[derive(Debug, Clone)]
pub struct CompletedBuildInfo {
  pub start: f64,
  pub end:   f64,
  pub host:  Host,
}

/// Failed build information
#[derive(Debug, Clone)]
pub struct FailedBuildInfo {
  pub start:     f64,
  pub end:       f64,
  pub host:      Host,
  pub fail_type: FailType,
}

/// Activity status tracking
#[derive(Debug, Clone)]
pub struct ActivityStatus {
  pub activity: u8,
  pub text:     String,
  pub parent:   Option<ActivityId>,
  pub phase:    Option<String>,
  pub progress: Option<ActivityProgress>,
}

/// Activity progress for downloads/uploads/builds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityProgress {
  /// Bytes completed
  pub done:     u64,
  /// Total bytes expected
  pub expected: u64,
  /// Currently running transfers
  pub running:  u64,
  /// Failed transfers
  pub failed:   u64,
}

/// Build report for caching
#[derive(Debug, Clone)]
pub struct BuildReport {
  pub derivation_name: String,
  pub platform:        String,
  pub duration_secs:   f64,
  pub completed_at:    SystemTime,
  pub host:            String,
  pub success:         bool,
}

/// Evaluation information
#[derive(Debug, Clone, Default)]
pub struct EvalInfo {
  pub last_file_name: Option<String>,
  pub count:          usize,
  pub at:             f64,
}

/// Main state for ROM
#[derive(Debug, Clone)]
pub struct State {
  pub derivation_infos: IndexMap<DerivationId, DerivationInfo>,
  pub store_path_infos: IndexMap<StorePathId, StorePathInfo>,
  pub full_summary:     DependencySummary,
  pub forest_roots:     Vec<DerivationId>,
  pub build_reports:    HashMap<String, Vec<BuildReport>>,
  pub start_time:       f64,
  pub progress_state:   ProgressState,
  pub store_path_ids:   HashMap<StorePath, StorePathId>,
  pub derivation_ids:   HashMap<Derivation, DerivationId>,
  pub touched_ids:      HashSet<DerivationId>,
  pub activities:       HashMap<ActivityId, ActivityStatus>,
  pub nix_errors:       Vec<String>,
  pub build_logs:       Vec<String>,
  pub traces:           Vec<String>,
  pub build_platform:   Option<String>,
  pub evaluation_state: EvalInfo,
  next_store_path_id:   StorePathId,
  next_derivation_id:   DerivationId,
}

impl Default for State {
  fn default() -> Self {
    Self::new()
  }
}

impl State {
  #[must_use]
  pub fn new() -> Self {
    Self {
      derivation_infos:   IndexMap::new(),
      store_path_infos:   IndexMap::new(),
      full_summary:       DependencySummary::default(),
      forest_roots:       Vec::new(),
      build_reports:      HashMap::new(),
      start_time:         current_time(),
      progress_state:     ProgressState::JustStarted,
      store_path_ids:     HashMap::new(),
      derivation_ids:     HashMap::new(),
      touched_ids:        HashSet::new(),
      activities:         HashMap::new(),
      nix_errors:         Vec::new(),
      build_logs:         Vec::new(),
      traces:             Vec::new(),
      build_platform:     None,
      evaluation_state:   EvalInfo::default(),
      next_store_path_id: 0,
      next_derivation_id: 0,
    }
  }

  #[must_use]
  pub fn with_platform(platform: Option<String>) -> Self {
    let mut state = Self::new();
    state.build_platform = platform;
    state
  }

  pub fn get_or_create_store_path_id(
    &mut self,
    path: StorePath,
  ) -> StorePathId {
    if let Some(&id) = self.store_path_ids.get(&path) {
      return id;
    }

    let id = self.next_store_path_id;
    self.next_store_path_id += 1;

    self.store_path_infos.insert(id, StorePathInfo {
      name:      path.clone(),
      states:    HashSet::new(),
      producer:  None,
      input_for: HashSet::new(),
    });
    self.store_path_ids.insert(path, id);

    id
  }

  pub fn get_or_create_derivation_id(
    &mut self,
    drv: Derivation,
  ) -> DerivationId {
    if let Some(&id) = self.derivation_ids.get(&drv) {
      return id;
    }

    let id = self.next_derivation_id;
    self.next_derivation_id += 1;

    self.derivation_infos.insert(id, DerivationInfo {
      name:               drv.clone(),
      outputs:            HashMap::new(),
      input_derivations:  Vec::new(),
      input_sources:      HashSet::new(),
      build_status:       BuildStatus::Unknown,
      dependency_summary: DependencySummary::default(),
      cached:             false,
      derivation_parents: HashSet::new(),
      pname:              None,
      platform:           None,
    });
    self.derivation_ids.insert(drv, id);

    id
  }

  /// Populate derivation dependencies by parsing its .drv file
  pub fn populate_derivation_dependencies(&mut self, drv_id: DerivationId) {
    use cognos::aterm;
    use tracing::debug;

    // Check if we've already parsed this derivation's dependencies
    // to avoid infinite recursion in circular dependency graphs
    let already_parsed = {
      if let Some(info) = self.get_derivation_info(drv_id) {
        !info.input_derivations.is_empty()
      } else {
        false
      }
    };

    if already_parsed {
      debug!("Skipping already-parsed derivation {}", drv_id);
      return;
    }

    let drv_path = {
      let info = match self.get_derivation_info(drv_id) {
        Some(i) => i,
        None => return,
      };
      // Path already includes .drv extension from Derivation::parse
      info.name.path.display().to_string()
    };

    debug!("Attempting to parse .drv file: {}", drv_path);

    let parsed = match aterm::parse_drv_file(&drv_path) {
      Ok(p) => {
        debug!(
          "Successfully parsed .drv file: {} with {} input derivations",
          drv_path,
          p.input_drvs.len()
        );
        p
      },
      Err(e) => {
        debug!("Failed to parse .drv file {}: {}", drv_path, e);
        return;
      },
    };

    // Extract metadata
    if let Some(pname) = aterm::extract_pname(&parsed.env) {
      if let Some(info) = self.get_derivation_info_mut(drv_id) {
        info.pname = Some(pname);
      }
    }

    if let Some(info) = self.get_derivation_info_mut(drv_id) {
      info.platform = Some(parsed.platform);
    }

    // Check if parent derivation is actively building
    let parent_is_building = {
      if let Some(parent_info) = self.get_derivation_info(drv_id) {
        matches!(parent_info.build_status, BuildStatus::Building(_))
      } else {
        false
      }
    };

    // Process input derivations
    for (input_drv_path, outputs) in parsed.input_drvs {
      if let Some(input_drv) = Derivation::parse(&input_drv_path) {
        let input_drv_id = self.get_or_create_derivation_id(input_drv);

        // Mark dependencies as Planned if parent is Building and input is
        // Unknown This ensures we only count real dependencies that
        // will be built
        if parent_is_building {
          if let Some(input_info) = self.get_derivation_info(input_drv_id) {
            if matches!(input_info.build_status, BuildStatus::Unknown) {
              debug!(
                "Marking input derivation {} as Planned (parent {} is \
                 Building)",
                input_drv_id, drv_id
              );
              self.update_build_status(input_drv_id, BuildStatus::Planned);
            } else {
              debug!(
                "Input derivation {} current status: {:?}",
                input_drv_id, input_info.build_status
              );
            }
          }
        }

        // Create output set
        let mut output_set = HashSet::new();
        for output in outputs {
          output_set.insert(OutputName::parse(&output));
        }

        // Add to parent's input derivations
        if let Some(parent_info) = self.get_derivation_info_mut(drv_id) {
          let input = InputDerivation {
            derivation: input_drv_id,
            outputs:    output_set,
          };
          if parent_info
            .input_derivations
            .iter()
            .any(|d| d.derivation == input_drv_id)
          {
            debug!(
              "Input derivation {} already in parent {}",
              input_drv_id, drv_id
            );
          } else {
            parent_info.input_derivations.push(input);
            debug!(
              "Added input derivation {} to {} (parent now has {} inputs)",
              input_drv_id,
              drv_id,
              parent_info.input_derivations.len()
            );
          }
        } else {
          debug!(
            "Parent derivation {} not found when trying to add input {}",
            drv_id, input_drv_id
          );
        }

        // Mark child as having this parent
        if let Some(child_info) = self.get_derivation_info_mut(input_drv_id) {
          child_info.derivation_parents.insert(drv_id);
        }

        // Remove from forest roots if it has a parent
        self.forest_roots.retain(|&id| id != input_drv_id);

        // Recursively populate child dependencies
        self.populate_derivation_dependencies(input_drv_id);
      }
    }
  }

  #[must_use]
  pub fn get_derivation_info(
    &self,
    id: DerivationId,
  ) -> Option<&DerivationInfo> {
    self.derivation_infos.get(&id)
  }

  pub fn get_derivation_info_mut(
    &mut self,
    id: DerivationId,
  ) -> Option<&mut DerivationInfo> {
    self.derivation_infos.get_mut(&id)
  }

  #[must_use]
  pub fn get_store_path_info(&self, id: StorePathId) -> Option<&StorePathInfo> {
    self.store_path_infos.get(&id)
  }

  pub fn get_store_path_info_mut(
    &mut self,
    id: StorePathId,
  ) -> Option<&mut StorePathInfo> {
    self.store_path_infos.get_mut(&id)
  }

  pub fn update_build_status(
    &mut self,
    id: DerivationId,
    new_status: BuildStatus,
  ) {
    if let Some(info) = self.derivation_infos.get_mut(&id) {
      let old_status =
        std::mem::replace(&mut info.build_status, new_status.clone());
      self.full_summary.clear_derivation(id, &old_status);
      self.full_summary.update_derivation(id, &new_status);
      self.touched_ids.insert(id);
    }
  }

  #[must_use]
  pub fn has_errors(&self) -> bool {
    !self.nix_errors.is_empty() || !self.full_summary.failed_builds.is_empty()
  }

  #[must_use]
  pub fn total_builds(&self) -> usize {
    self.full_summary.planned_builds.len()
      + self.full_summary.running_builds.len()
      + self.full_summary.completed_builds.len()
      + self.full_summary.failed_builds.len()
  }

  #[must_use]
  pub fn running_builds_for_host(
    &self,
    host: &Host,
  ) -> Vec<(DerivationId, &BuildInfo)> {
    self
      .full_summary
      .running_builds
      .iter()
      .filter(|(_, info)| &info.host == host)
      .map(|(id, info)| (*id, info))
      .collect()
  }

  /// Check if a derivation has a platform mismatch
  #[must_use]
  pub fn has_platform_mismatch(&self, id: DerivationId) -> bool {
    if let (Some(build_platform), Some(info)) =
      (&self.build_platform, self.get_derivation_info(id))
    {
      if let Some(drv_platform) = &info.platform {
        return build_platform != drv_platform;
      }
    }
    false
  }

  /// Get all derivations with platform mismatches
  #[must_use]
  pub fn platform_mismatches(&self) -> Vec<DerivationId> {
    self
      .derivation_infos
      .keys()
      .filter(|&&id| self.has_platform_mismatch(id))
      .copied()
      .collect()
  }
}

#[must_use]
pub fn current_time() -> f64 {
  SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .unwrap_or(Duration::ZERO)
    .as_secs_f64()
}

#[cfg(test)]
mod tests {
  use super::*;

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
}
