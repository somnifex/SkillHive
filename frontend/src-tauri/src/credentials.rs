/// OS-backed credential storage boundary.
///
/// Refresh/session secrets must remain in the privileged desktop core and must
/// never be persisted in WebView localStorage. Platform keychain integration
/// is added after the shell is proven buildable on supported targets.
pub struct CredentialStore;
