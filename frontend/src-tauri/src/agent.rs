use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub id: String,
    pub display_name: String,
    pub kind: AgentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Application,
    UnifiedAgentSkills,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub descriptor_id: String,
    pub display_name: String,
    pub skill_root: PathBuf,
    pub enabled: bool,
    pub detected: bool,
    pub skill_root_exists: bool,
}

pub trait AgentAdapter: Send + Sync {
    fn descriptor(&self) -> AgentDescriptor;
    fn discover(&self) -> Result<Vec<AgentInstance>, AgentAdapterError>;
    fn validate_skill_root(&self, path: &Path) -> Result<(), AgentAdapterError>;
}

#[derive(Clone)]
pub struct AgentRegistry {
    adapters: Vec<Arc<dyn AgentAdapter>>,
}

impl AgentRegistry {
    pub fn builtin() -> Self {
        Self {
            adapters: built_in_adapters()
                .into_iter()
                .map(|adapter| Arc::new(adapter) as Arc<dyn AgentAdapter>)
                .collect(),
        }
    }

    pub fn with_custom_profile(
        mut self,
        id: impl Into<String>,
        display_name: impl Into<String>,
        skill_root: impl Into<PathBuf>,
    ) -> Self {
        self.adapters.push(Arc::new(CustomAgentAdapter::new(
            id,
            display_name,
            skill_root,
        )));
        self
    }

    pub fn descriptors(&self) -> Vec<AgentDescriptor> {
        self.adapters.iter().map(|adapter| adapter.descriptor()).collect()
    }

    pub fn discover_all(&self) -> Vec<AgentDiscoveryResult> {
        self.adapters
            .iter()
            .map(|adapter| {
                let descriptor = adapter.descriptor();
                match adapter.discover() {
                    Ok(instances) => AgentDiscoveryResult {
                        descriptor,
                        instances,
                        error: None,
                    },
                    Err(error) => AgentDiscoveryResult {
                        descriptor,
                        instances: Vec::new(),
                        error: Some(error.to_string()),
                    },
                }
            })
            .collect()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDiscoveryResult {
    pub descriptor: AgentDescriptor,
    pub instances: Vec<AgentInstance>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuiltInAgentAdapter {
    descriptor: AgentDescriptor,
    relative_skill_root: PathBuf,
    detection_roots: Vec<PathBuf>,
}

impl BuiltInAgentAdapter {
    fn application(
        id: &'static str,
        display_name: &'static str,
        relative_skill_root: impl Into<PathBuf>,
        detection_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            descriptor: AgentDescriptor {
                id: id.to_owned(),
                display_name: display_name.to_owned(),
                kind: AgentKind::Application,
            },
            relative_skill_root: relative_skill_root.into(),
            detection_roots: detection_roots.into_iter().collect(),
        }
    }

    fn unified() -> Self {
        Self {
            descriptor: AgentDescriptor {
                id: "agent-skills".to_owned(),
                display_name: "Agent Skills (~/.agents/skills)".to_owned(),
                kind: AgentKind::UnifiedAgentSkills,
            },
            relative_skill_root: PathBuf::from(".agents").join("skills"),
            detection_roots: vec![PathBuf::from(".agents")],
        }
    }
}

impl AgentAdapter for BuiltInAgentAdapter {
    fn descriptor(&self) -> AgentDescriptor {
        self.descriptor.clone()
    }

    fn discover(&self) -> Result<Vec<AgentInstance>, AgentAdapterError> {
        let home = home_dir()?;
        let skill_root = home.join(&self.relative_skill_root);
        let detected = path_entry_exists(&skill_root)?
            || self
                .detection_roots
                .iter()
                .map(|candidate| path_entry_exists(&home.join(candidate)))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .any(|exists| exists);

        if !detected {
            return Ok(Vec::new());
        }

        self.validate_skill_root(&skill_root)?;
        Ok(vec![AgentInstance {
            id: format!("{}:default", self.descriptor.id),
            descriptor_id: self.descriptor.id.clone(),
            display_name: self.descriptor.display_name.clone(),
            skill_root_exists: path_entry_exists(&skill_root)?,
            skill_root,
            enabled: true,
            detected: true,
        }])
    }

    fn validate_skill_root(&self, path: &Path) -> Result<(), AgentAdapterError> {
        validate_skill_root(path)
    }
}

#[derive(Debug, Clone)]
pub struct CustomAgentAdapter {
    descriptor: AgentDescriptor,
    skill_root: PathBuf,
}

impl CustomAgentAdapter {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        skill_root: impl Into<PathBuf>,
    ) -> Self {
        let id = id.into();
        let display_name = display_name.into();
        Self {
            descriptor: AgentDescriptor {
                id: format!("custom:{id}"),
                display_name,
                kind: AgentKind::Custom,
            },
            skill_root: skill_root.into(),
        }
    }
}

impl AgentAdapter for CustomAgentAdapter {
    fn descriptor(&self) -> AgentDescriptor {
        self.descriptor.clone()
    }

    fn discover(&self) -> Result<Vec<AgentInstance>, AgentAdapterError> {
        self.validate_skill_root(&self.skill_root)?;
        Ok(vec![AgentInstance {
            id: format!("{}:configured", self.descriptor.id),
            descriptor_id: self.descriptor.id.clone(),
            display_name: self.descriptor.display_name.clone(),
            skill_root_exists: path_entry_exists(&self.skill_root)?,
            skill_root: self.skill_root.clone(),
            enabled: true,
            detected: true,
        }])
    }

    fn validate_skill_root(&self, path: &Path) -> Result<(), AgentAdapterError> {
        validate_skill_root(path)
    }
}

fn built_in_adapters() -> Vec<BuiltInAgentAdapter> {
    vec![
        BuiltInAgentAdapter::application(
            "claude-code",
            "Claude Code",
            PathBuf::from(".claude").join("skills"),
            [PathBuf::from(".claude")],
        ),
        BuiltInAgentAdapter::application(
            "claude-desktop",
            "Claude Desktop",
            PathBuf::from(".claude-desktop").join("skills"),
            [PathBuf::from(".claude-desktop")],
        ),
        BuiltInAgentAdapter::application(
            "codex",
            "Codex",
            PathBuf::from(".codex").join("skills"),
            [PathBuf::from(".codex")],
        ),
        BuiltInAgentAdapter::application(
            "gemini",
            "Gemini CLI",
            PathBuf::from(".gemini").join("skills"),
            [PathBuf::from(".gemini")],
        ),
        BuiltInAgentAdapter::application(
            "opencode",
            "OpenCode",
            PathBuf::from(".config").join("opencode").join("skills"),
            [PathBuf::from(".config").join("opencode")],
        ),
        BuiltInAgentAdapter::application(
            "openclaw",
            "OpenClaw",
            PathBuf::from(".openclaw").join("skills"),
            [PathBuf::from(".openclaw")],
        ),
        BuiltInAgentAdapter::application(
            "grok-build",
            "Grok Build",
            PathBuf::from(".grok").join("skills"),
            [PathBuf::from(".grok")],
        ),
        BuiltInAgentAdapter::unified(),
    ]
}

/// Validates the identity/path shape of a profile before it is persisted.
///
/// Built-in profile identity and roots are derived exclusively from Rust
/// adapters. A WebView caller can therefore not redirect a built-in descriptor
/// to an arbitrary filesystem directory by forging `is_custom = false`.
/// Custom profiles remain user-configurable, but must live in the reserved
/// `custom:<id>` namespace so they cannot collide with built-in instances.
pub fn validate_persisted_profile(
    profile_id: &str,
    descriptor_id: &str,
    skill_root: &Path,
    is_custom: bool,
) -> Result<(), AgentAdapterError> {
    validate_skill_root(skill_root)?;

    if is_custom {
        let custom_key = descriptor_id
            .strip_prefix("custom:")
            .filter(|value| valid_custom_key(value))
            .ok_or_else(|| {
                AgentAdapterError::InvalidProfileIdentity(
                    "custom descriptor must be custom:<portable-id>".to_owned(),
                )
            })?;
        let expected_id = format!("custom:{custom_key}:configured");
        if profile_id != expected_id {
            return Err(AgentAdapterError::InvalidProfileIdentity(format!(
                "custom profile id must be {expected_id}"
            )));
        }
        return Ok(());
    }

    let adapter = built_in_adapters()
        .into_iter()
        .find(|adapter| adapter.descriptor.id == descriptor_id)
        .ok_or_else(|| {
            AgentAdapterError::InvalidProfileIdentity(format!(
                "unknown built-in descriptor: {descriptor_id}"
            ))
        })?;
    let expected_id = format!("{}:default", adapter.descriptor.id);
    if profile_id != expected_id {
        return Err(AgentAdapterError::InvalidProfileIdentity(format!(
            "built-in profile id must be {expected_id}"
        )));
    }

    let expected_root = home_dir()?.join(adapter.relative_skill_root);
    if skill_root != expected_root {
        return Err(AgentAdapterError::InvalidProfileIdentity(format!(
            "built-in profile {profile_id} must use {}",
            expected_root.display()
        )));
    }
    Ok(())
}

fn valid_custom_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn validate_skill_root(path: &Path) -> Result<(), AgentAdapterError> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(AgentAdapterError::InvalidPath(
            "skill root must be an absolute path".to_owned(),
        ));
    }
    if path.to_str().is_none() {
        return Err(AgentAdapterError::InvalidPath(
            "skill root must be valid UTF-8".to_owned(),
        ));
    }
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err(AgentAdapterError::InvalidPath(format!(
                "skill root is not lexically normalized: {}",
                path.display()
            )));
        }
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(AgentAdapterError::InvalidPath(format!(
                    "skill root must not be a symbolic link: {}",
                    path.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(AgentAdapterError::InvalidPath(format!(
                    "{} exists but is not a directory",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(AgentAdapterError::Discovery(error.to_string())),
    }
    Ok(())
}

fn path_entry_exists(path: &Path) -> Result<bool, AgentAdapterError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AgentAdapterError::Discovery(error.to_string())),
    }
}

fn home_dir() -> Result<PathBuf, AgentAdapterError> {
    #[cfg(windows)]
    let candidate = env::var_os("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        let mut combined = PathBuf::from(drive);
        combined.push(path);
        Some(combined.into_os_string())
    });

    #[cfg(not(windows))]
    let candidate = env::var_os("HOME");

    let path = candidate
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(AgentAdapterError::HomeDirectoryUnavailable)?;
    if !path.is_absolute() {
        return Err(AgentAdapterError::InvalidPath(
            "home directory must be absolute".to_owned(),
        ));
    }
    Ok(path)
}

#[derive(Debug, thiserror::Error)]
pub enum AgentAdapterError {
    #[error("agent skill path is invalid: {0}")]
    InvalidPath(String),
    #[error("agent profile identity is invalid: {0}")]
    InvalidProfileIdentity(String),
    #[error("agent discovery failed: {0}")]
    Discovery(String),
    #[error("home directory is unavailable")]
    HomeDirectoryUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_profile_is_bound_to_configured_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("my-agent").join("skills");
        let adapter = CustomAgentAdapter::new("my-agent", "My Agent", &root);

        let discovered = adapter.discover().expect("discover");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].skill_root, root);
        assert_eq!(discovered[0].descriptor_id, "custom:my-agent");
        assert!(!discovered[0].skill_root_exists);
    }

    #[test]
    fn custom_profile_rejects_file_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("not-a-directory");
        std::fs::write(&file, b"file").expect("write");
        let adapter = CustomAgentAdapter::new("broken", "Broken", &file);

        assert!(matches!(
            adapter.discover(),
            Err(AgentAdapterError::InvalidPath(_))
        ));
    }

    #[test]
    fn registry_contains_unified_agent_skills_adapter() {
        let descriptors = AgentRegistry::builtin().descriptors();
        assert!(descriptors.iter().any(|descriptor| {
            descriptor.id == "agent-skills" && descriptor.kind == AgentKind::UnifiedAgentSkills
        }));
    }

    #[test]
    fn forged_builtin_root_is_rejected() {
        let home = home_dir().expect("home");
        let wrong = home.join("forged-skills");
        assert!(matches!(
            validate_persisted_profile(
                "claude-code:default",
                "claude-code",
                &wrong,
                false,
            ),
            Err(AgentAdapterError::InvalidProfileIdentity(_))
        ));
    }

    #[test]
    fn custom_profile_requires_reserved_identity_namespace() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(validate_persisted_profile(
            "custom:my-agent:configured",
            "custom:my-agent",
            &temp.path().join("skills"),
            true,
        )
        .is_ok());
        assert!(validate_persisted_profile(
            "claude-code:default",
            "claude-code",
            &temp.path().join("skills"),
            true,
        )
        .is_err());
    }
}
