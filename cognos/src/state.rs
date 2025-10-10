use std::{collections::HashMap, path::PathBuf};

use crate::internal_json::Actions;

pub type Id = u64;

pub enum StorePath {
  Downloading,
  Uploading,
  Downloaded,
  Uploaded,
}

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
  deps: HashMap<Id, BuildInfo>,
}

// #[derive(Default)]
pub struct State {
  pub progress: ProgressState,
}

impl State {
  pub fn imbibe(&mut self, update: Actions) {}
}