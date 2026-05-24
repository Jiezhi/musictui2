//! Credential storage abstraction.
//!
//! Production builds talk to the host OS keychain (Keychain on macOS,
//! Secret Service on Linux, Credential Manager on Windows) via the
//! [`keyring`] crate. Tests use [`InMemoryStore`] so they never touch a
//! real keychain. A common [`CredentialStore`] trait keeps call sites
//! storage-agnostic.

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;

const SERVICE: &str = "musictui2";

/// Errors raised by credential storage backends.
#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("Keyring backend error: {0}")]
    Backend(String),
}

#[allow(dead_code)]
impl CredentialError {
    pub fn backend(msg: impl Into<String>) -> Self {
        Self::Backend(msg.into())
    }
}

/// Storage backend for sensitive strings (WebDAV passwords, GitHub tokens, …).
///
/// All operations are synchronous because OS keychain APIs are themselves
/// synchronous and credentials are accessed off the hot path.
#[allow(dead_code)]
pub trait CredentialStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError>;
    fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self, key: &str) -> Result<(), CredentialError>;
}

/// OS keyring-backed store. Each entry lives under the `musictui2` service
/// name to avoid clashing with other applications on the same host.
#[allow(dead_code)]
pub struct KeyringStore;

#[allow(dead_code)]
impl KeyringStore {
    pub fn new() -> Self {
        Self
    }
}

#[allow(dead_code)]
impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        let entry = keyring::Entry::new(SERVICE, key)
            .map_err(|err| CredentialError::Backend(err.to_string()))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(CredentialError::Backend(err.to_string())),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(SERVICE, key)
            .map_err(|err| CredentialError::Backend(err.to_string()))?;
        entry
            .set_password(secret)
            .map_err(|err| CredentialError::Backend(err.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(SERVICE, key)
            .map_err(|err| CredentialError::Backend(err.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(CredentialError::Backend(err.to_string())),
        }
    }
}

/// Thread-safe in-memory store for unit tests.
#[derive(Default)]
pub struct InMemoryStore {
    entries: Mutex<HashMap<String, String>>,
}

#[allow(dead_code)]
impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for InMemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, CredentialError> {
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), CredentialError> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), CredentialError> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Canonical key for the WebDAV password belonging to a given source name.
pub fn webdav_password_key(source_name: &str) -> String {
    format!("webdav-password:{source_name}")
}

/// Canonical key for an optional GitHub token stored in the keyring.
pub const GITHUB_TOKEN_KEY: &str = "github-token";

/// Resolves the GitHub token from the environment first, then the keyring as
/// a fallback. Returns `Ok(None)` when neither source has a value.
#[allow(dead_code)]
pub fn resolve_github_token(
    env_value: Option<String>,
    store: &dyn CredentialStore,
) -> Result<Option<String>, CredentialError> {
    if let Some(value) = env_value.filter(|s| !s.is_empty()) {
        return Ok(Some(value));
    }
    store.get(GITHUB_TOKEN_KEY)
}

/// Resolves the WebDAV password for a source: keyring entry first, then the
/// caller-provided inline fallback (typically the value persisted in SQLite for
/// backward compatibility). Returns `Ok(None)` when neither source has a value.
#[allow(dead_code)]
pub fn resolve_webdav_password(
    source_name: &str,
    fallback_inline: Option<&str>,
    store: &dyn CredentialStore,
) -> Result<Option<String>, CredentialError> {
    if let Some(value) = store
        .get(&webdav_password_key(source_name))?
        .filter(|s| !s.is_empty())
    {
        return Ok(Some(value));
    }
    Ok(fallback_inline
        .filter(|s| !s.is_empty())
        .map(ToString::to_string))
}

/// Writes a WebDAV password to the credential store under the canonical key for
/// the given source name. Best-effort callers should treat failures as a signal
/// to fall back to inline storage.
#[allow(dead_code)]
pub fn store_webdav_password(
    source_name: &str,
    password: &str,
    store: &dyn CredentialStore,
) -> Result<(), CredentialError> {
    store.set(&webdav_password_key(source_name), password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_round_trips() {
        let store = InMemoryStore::new();
        assert!(store.get("missing").unwrap().is_none());

        store.set("webdav-password:home-nas", "hunter2").unwrap();
        assert_eq!(
            store.get("webdav-password:home-nas").unwrap().as_deref(),
            Some("hunter2")
        );

        store.delete("webdav-password:home-nas").unwrap();
        assert!(store.get("webdav-password:home-nas").unwrap().is_none());
    }

    #[test]
    fn in_memory_store_overwrites_existing_value() {
        let store = InMemoryStore::new();
        store.set("k", "v1").unwrap();
        store.set("k", "v2").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn in_memory_store_delete_is_idempotent() {
        let store = InMemoryStore::new();
        store.delete("never-set").unwrap();
        store.delete("never-set").unwrap();
    }

    #[test]
    fn webdav_password_key_namespaces_by_source_name() {
        assert_eq!(webdav_password_key("home"), "webdav-password:home");
        assert_eq!(webdav_password_key("work"), "webdav-password:work");
        // Different sources must not collide.
        assert_ne!(webdav_password_key("a"), webdav_password_key("b"));
    }

    #[test]
    fn resolve_github_token_prefers_environment_over_keyring() {
        let store = InMemoryStore::new();
        store.set(GITHUB_TOKEN_KEY, "from-keyring").unwrap();

        let resolved = resolve_github_token(Some("from-env".to_string()), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("from-env"));
    }

    #[test]
    fn resolve_github_token_falls_back_to_keyring_when_env_missing() {
        let store = InMemoryStore::new();
        store.set(GITHUB_TOKEN_KEY, "from-keyring").unwrap();
        let resolved = resolve_github_token(None, &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("from-keyring"));
    }

    #[test]
    fn resolve_github_token_treats_empty_env_as_missing() {
        let store = InMemoryStore::new();
        store.set(GITHUB_TOKEN_KEY, "from-keyring").unwrap();
        let resolved = resolve_github_token(Some(String::new()), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("from-keyring"));
    }

    #[test]
    fn resolve_github_token_returns_none_when_no_source_has_value() {
        let store = InMemoryStore::new();
        assert!(resolve_github_token(None, &store).unwrap().is_none());
        assert!(resolve_github_token(Some(String::new()), &store)
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolve_webdav_password_prefers_keyring_over_inline_fallback() {
        let store = InMemoryStore::new();
        store_webdav_password("nas", "from-keyring", &store).unwrap();
        let resolved = resolve_webdav_password("nas", Some("inline-legacy"), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("from-keyring"));
    }

    #[test]
    fn resolve_webdav_password_falls_back_to_inline_when_keyring_missing() {
        let store = InMemoryStore::new();
        let resolved = resolve_webdav_password("nas", Some("inline-legacy"), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("inline-legacy"));
    }

    #[test]
    fn resolve_webdav_password_returns_none_when_no_source_has_value() {
        let store = InMemoryStore::new();
        assert!(resolve_webdav_password("nas", None, &store)
            .unwrap()
            .is_none());
        assert!(resolve_webdav_password("nas", Some(""), &store)
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolve_webdav_password_treats_empty_keyring_entry_as_missing() {
        let store = InMemoryStore::new();
        store_webdav_password("nas", "", &store).unwrap();
        let resolved = resolve_webdav_password("nas", Some("inline-legacy"), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("inline-legacy"));
    }

    #[test]
    fn store_webdav_password_namespaces_each_source_independently() {
        let store = InMemoryStore::new();
        store_webdav_password("home", "h-secret", &store).unwrap();
        store_webdav_password("work", "w-secret", &store).unwrap();
        assert_eq!(
            resolve_webdav_password("home", None, &store)
                .unwrap()
                .as_deref(),
            Some("h-secret")
        );
        assert_eq!(
            resolve_webdav_password("work", None, &store)
                .unwrap()
                .as_deref(),
            Some("w-secret")
        );
    }

    /// Trait-object safety check — call sites need to store a `Box<dyn CredentialStore>`.
    #[test]
    fn credential_store_is_object_safe_and_thread_shareable() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn CredentialStore>();
        let _boxed: Box<dyn CredentialStore> = Box::new(InMemoryStore::new());
    }
}
