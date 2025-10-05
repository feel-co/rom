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

pub enum Progress {
  JustStarted,
  InputReceived,
  Finished,
}

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

pub enum Host {
  Local,
  Host(String),
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
  progress: Progress,
}

impl State {
  pub fn imbibe(&mut self, update: Actions) {}
}
