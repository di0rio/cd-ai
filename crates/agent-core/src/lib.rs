pub mod agent;
pub mod checkpoint;
pub mod eval;
pub mod events;
pub mod ollama;
pub mod permissions;
pub mod redactor;
pub mod sandbox;
pub mod skills;
pub mod syntax;
pub mod tool_call;
pub mod tools;
pub mod workspace;

use serde::Serialize;
use ts_rs::TS;

pub use agent::state::{RollbackResult, RollbackSkip};
pub use checkpoint::{
    CheckpointError, ShadowRepo, baseline_commit, record_checkpoint, rollback_task,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct AppInfo {
    #[ts(type = "string")]
    pub name: &'static str,
    #[ts(type = "string")]
    pub version: &'static str,
}

pub fn app_info() -> AppInfo {
    AppInfo {
        name: "cd-ai",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_uses_crate_version() {
        let info = app_info();
        assert_eq!(info.name, "cd-ai");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
