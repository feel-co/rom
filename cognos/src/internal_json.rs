use serde::Deserialize;
use serde_repr::Deserialize_repr;

#[derive(Deserialize_repr, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Activities {
  Unknown       = 0,
  CopyPath      = 100,
  FileTransfer  = 101,
  Realise       = 102,
  CopyPaths     = 103,
  Builds        = 104,
  Build         = 105,
  OptimiseStore = 106,
  VerifyPath    = 107,
  Substitute    = 108,
  QueryPathInfo = 109,
  PostBuildHook = 110,
  BuildWaiting  = 111,
  FetchTree     = 112,
}

#[derive(
  Deserialize_repr, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
)]
#[repr(u8)]
pub enum Verbosity {
  Error     = 0,
  Warning   = 1,
  Notice    = 2,
  Info      = 3,
  Talkative = 4,
  Chatty    = 5,
  Debug     = 6,
  Vomit     = 7,
}

pub type Id = u64;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "action")]
pub enum Actions {
  #[serde(rename = "start")]
  Start {
    id:       Id,
    level:    Verbosity,
    #[serde(default)]
    parent:   Id,
    text:     String,
    #[serde(rename = "type")]
    activity: Activities,
    #[serde(default)]
    fields:   Vec<serde_json::Value>,
  },
  #[serde(rename = "stop")]
  Stop { id: Id },
  #[serde(rename = "msg")]
  Message { level: Verbosity, msg: String },
  #[serde(rename = "result")]
  Result {
    #[serde(default)]
    fields:   Vec<serde_json::Value>,
    id:       Id,
    #[serde(rename = "type")]
    activity: Activities,
  },
}
