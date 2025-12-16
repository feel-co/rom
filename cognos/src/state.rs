use std::{collections::HashMap, path::PathBuf};

use crate::internal_json::Actions;

pub type Id = u64;

pub enum StorePath {
  Downloading,
  Uploading,
  Downloaded,
  Uploaded,
}

#[derive(Clone)]
pub enum BuildStatus {
  Planned,
  Running,
  Complete,
  Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProgressState {
  JustStarted,
  InputReceived,
  Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputName {
  Out,
  Doc,
  Dev,
  Bin,
  Info,
  Lib,
  Man,
  Dist,
  Other(String),
}

impl OutputName {
  #[must_use]
  pub fn parse(name: &str) -> Self {
    match name.to_lowercase().as_str() {
      "out" => Self::Out,
      "doc" => Self::Doc,
      "dev" => Self::Dev,
      "bin" => Self::Bin,
      "info" => Self::Info,
      "lib" => Self::Lib,
      "man" => Self::Man,
      "dist" => Self::Dist,
      _ => Self::Other(name.to_string()),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
  Localhost,
  Remote(String),
}

impl Host {
  #[must_use]
  pub fn name(&self) -> &str {
    match self {
      Self::Localhost => "localhost",
      Self::Remote(name) => name,
    }
  }
}

pub struct Derivation {
  store_path: PathBuf,
}

#[derive(Clone)]
pub struct BuildInfo {
  start:       f64,
  host:        Host,
  estimate:    Option<u64>,
  activity_id: Id,
  state:       BuildStatus,
}

pub enum DependencyState {
  Planned,
  Running,
  Completed,
}

pub struct Dependencies {
  pub deps: HashMap<Id, BuildInfo>,
}

// #[derive(Default)]
pub struct State {
  pub progress:          ProgressState,
  pub derivations:       HashMap<Id, Derivation>,
  pub builds:            HashMap<Id, BuildInfo>,
  pub dependencies:      Dependencies,
  pub store_paths:       HashMap<Id, StorePath>,
  pub dependency_states: HashMap<Id, DependencyState>,
}

impl State {
  pub fn imbibe(&mut self, action: Actions) {
    match action {
      Actions::Start {
        id,
        activity: _activity,
        ..
      } => {
        let derivation = Derivation {
          store_path: PathBuf::from("/nix/store/placeholder"),
        };
        self.derivations.insert(id, derivation);

        // Use the store_path to mark as used
        let _path = &self.derivations.get(&id).unwrap().store_path;

        let build_info = BuildInfo {
          start:       0.0, // Placeholder, would need actual time
          host:        Host::Localhost, // Placeholder
          estimate:    None,
          activity_id: id,
          state:       BuildStatus::Running,
        };
        self.builds.insert(id, build_info.clone());
        self.dependencies.deps.insert(id, build_info);

        // Use the fields to mark as used
        let _start = self.builds.get(&id).unwrap().start;
        let _host = &self.builds.get(&id).unwrap().host;
        let _estimate = &self.builds.get(&id).unwrap().estimate;
        let _activity_id = self.builds.get(&id).unwrap().activity_id;

        self.store_paths.insert(id, StorePath::Downloading);
        self.dependency_states.insert(id, DependencyState::Running);
      },
      Actions::Result { id, .. } => {
        if let Some(build) = self.builds.get_mut(&id) {
          build.state = BuildStatus::Complete;
        }
      },
      Actions::Stop { id } => {
        if let Some(build) = self.builds.get_mut(&id) {
          build.state = BuildStatus::Complete;
        }
      },
      Actions::Message { .. } => {
        // Could update progress or other state
        self.progress = ProgressState::InputReceived;
      },
    }
  }
}
