//! Only opaque references enter configuration; passwords stay in the OS vault.
use crate::i18n::{Message, Text};
use anyhow::{Result, ensure};
use zeroize::Zeroizing;

pub const SERVICE: &str = "ssh-tui";

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id),
        Message::plain(Text::CredentialInvalid)
    );
    Ok(())
}

pub fn validate_password(password: &str) -> Result<()> {
    ensure!(
        !password.is_empty() && password.len() <= 1023 && !password.contains(['\r', '\n', '\0']),
        Message::plain(Text::SshPasswordInvalid)
    );
    Ok(())
}

pub trait CredentialStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Zeroizing<String>>;
    fn set(&self, id: &str, password: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

pub struct SystemStore;
impl SystemStore {
    fn entry(id: &str) -> Result<keyring::Entry> {
        validate_id(id)?;
        keyring::Entry::new(SERVICE, id)
            .map_err(|_| anyhow::anyhow!(Message::plain(Text::CredentialUnavailable)))
    }
}
impl CredentialStore for SystemStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>> {
        let password = Self::entry(id)?.get_password().map_err(|error| {
            anyhow::anyhow!(Message::plain(
                if matches!(error, keyring::Error::NoEntry) {
                    Text::CredentialMissing
                } else {
                    Text::CredentialUnavailable
                }
            ))
        })?;
        let password = Zeroizing::new(password);
        validate_password(&password)?;
        Ok(password)
    }
    fn set(&self, id: &str, password: &str) -> Result<()> {
        validate_password(password)?;
        Self::entry(id)?
            .set_password(password)
            .map_err(|_| anyhow::anyhow!(Message::plain(Text::CredentialUnavailable)))
    }
    fn delete(&self, id: &str) -> Result<()> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(anyhow::anyhow!(Message::plain(Text::CredentialUnavailable))),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    pub struct MemoryStore {
        pub values: Mutex<HashMap<String, Zeroizing<String>>>,
        pub fail: std::sync::atomic::AtomicBool,
    }
    impl MemoryStore {
        fn check(&self) -> Result<()> {
            ensure!(
                !self.fail.load(std::sync::atomic::Ordering::Relaxed),
                "vault unavailable"
            );
            Ok(())
        }
    }
    impl CredentialStore for MemoryStore {
        fn get(&self, id: &str) -> Result<Zeroizing<String>> {
            self.check()?;
            self.values
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!(Message::plain(Text::CredentialMissing)))
        }
        fn set(&self, id: &str, password: &str) -> Result<()> {
            self.check()?;
            validate_password(password)?;
            self.values
                .lock()
                .unwrap()
                .insert(id.into(), Zeroizing::new(password.into()));
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<()> {
            self.check()?;
            self.values.lock().unwrap().remove(id);
            Ok(())
        }
    }

    #[test]
    fn passwords_preserve_spaces_and_reject_line_breaks_and_oversize_input() {
        assert!(validate_password(" 中文 password ").is_ok());
        for value in ["", "p\nq", "p\rq", "p\0q"] {
            assert!(validate_password(value).is_err());
        }
        assert!(validate_password(&"x".repeat(1024)).is_err());
        assert!(validate_id(&new_id()).is_ok());
        assert!(validate_id("profile-name").is_err());
    }
}
