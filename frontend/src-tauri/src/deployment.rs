/// Transactional agent deployment boundary.
///
/// M1 will implement stage -> validate -> atomic swap -> record semantics.
/// Authorization is resolved before this layer; deployment operates only on
/// already-authorized local skill material.
pub struct DeploymentEngine;
