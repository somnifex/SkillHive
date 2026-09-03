use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable identifier for an agent implementation known by SkillHive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub id: String,
    pub display_name: String,
}

/// A concrete local installation/profile of an agent.
///
/// Multiple instances of the same descriptor are allowed (for example native
/// Windows and WSL installations). Authorization is deliberately absent from
/// this type; adapters describe local capabilities only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub descriptor_id: String,
    pub display_name: String,
    pub skill_root: PathBuf,
    pub enabled: bool,
}

pub trait AgentAdapter: Send + Sync {
    fn descriptor(&self) -> AgentDescriptor;
    fn discover(&self) -> Result<Vec<AgentInstance>, AgentAdapterError>;
    fn validate_skill_root(&self, path: &std::path::Path) -> Result<(), AgentAdapterError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AgentAdapterError {
    #[error("agent skill path is invalid: {0}")]
    InvalidPath(String),
    #[error("agent discovery failed: {0}")]
    Discovery(String),
}
