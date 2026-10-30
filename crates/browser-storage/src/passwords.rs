//! Password storage. Secrets live in the OS credential store (macOS Keychain,
//! Windows Credential Manager, Secret Service on Linux) via `keyring`; this
//! crate never writes a password to its own files.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential store: {0}")]
    Backend(String),
}

/// Abstraction so logic is testable without touching a real keychain.
pub trait SecretStore: Send + Sync {
    fn set(&self, origin: &str, username: &str, password: &str) -> Result<(), SecretError>;
    fn get(&self, origin: &str, username: &str) -> Result<Option<String>, SecretError>;
    fn delete(&self, origin: &str, username: &str) -> Result<(), SecretError>;
}

const SERVICE: &str = "browserust";

/// Credentials are keyed by exact origin (scheme+host+port) so a password saved
/// for one site is never offered to another.
fn account(origin: &str, username: &str) -> String {
    format!("{origin}\u{1f}{username}")
}

pub struct OsKeyring;

impl SecretStore for OsKeyring {
    fn set(&self, origin: &str, username: &str, password: &str) -> Result<(), SecretError> {
        keyring::Entry::new(SERVICE, &account(origin, username))
            .and_then(|e| e.set_password(password))
            .map_err(|e| SecretError::Backend(e.to_string()))
    }
    fn get(&self, origin: &str, username: &str) -> Result<Option<String>, SecretError> {
        match keyring::Entry::new(SERVICE, &account(origin, username)).and_then(|e| e.get_password()) {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
    fn delete(&self, origin: &str, username: &str) -> Result<(), SecretError> {
        match keyring::Entry::new(SERVICE, &account(origin, username)).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Backend(e.to_string())),
        }
    }
}

/// Process-memory store for tests and private browsing.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn set(&self, o: &str, u: &str, p: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account(o, u), p.into());
        Ok(())
    }
    fn get(&self, o: &str, u: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(&account(o, u)).cloned())
    }
    fn delete(&self, o: &str, u: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(&account(o, u));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_scoped_to_exact_origin_and_user() {
        let s = MemorySecrets::default();
        s.set("https://a.test", "bob", "pw").unwrap();
        assert_eq!(s.get("https://a.test", "bob").unwrap().as_deref(), Some("pw"));
        assert_eq!(s.get("https://evil.test", "bob").unwrap(), None);
        assert_eq!(s.get("http://a.test", "bob").unwrap(), None, "scheme matters");
        assert_eq!(s.get("https://a.test", "alice").unwrap(), None);
        s.delete("https://a.test", "bob").unwrap();
        assert_eq!(s.get("https://a.test", "bob").unwrap(), None);
    }
}
