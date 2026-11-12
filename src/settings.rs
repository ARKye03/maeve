use serde::{Deserialize, Serialize};

use crate::domain::CostSettings;
use crate::i18n::Lang;

const KEYCHAIN_SERVICE: &str = "maeve";
const KEYCHAIN_USER: &str = "gemini_api_key";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// Deliberately has no API-key field: the key lives only in the Keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub theme: ThemePref,
    pub advocate_model: String,
    pub judge_model: String,
    pub rpm: u32,
    pub cost: CostSettings,
    pub fx_manual: Option<f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::Es,
            theme: ThemePref::System,
            advocate_model: "gemini-flash-latest".into(),
            judge_model: "gemini-flash-latest".into(),
            rpm: 10,
            cost: CostSettings::default(),
            fx_manual: None,
        }
    }
}

/// Secret storage seam so tests never touch the real Keychain.
pub trait SecretStore {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, secret: &str) -> Result<(), String>;
}

pub struct Keychain {
    service: String,
    user: String,
}

impl Keychain {
    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            user: user.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &self.user).map_err(|e| e.to_string())
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
    }
}

impl SecretStore for Keychain {
    fn get(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, secret: &str) -> Result<(), String> {
        self.entry()?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }
}

/// Blank values count as absent.
pub fn load_key_from(store: &impl SecretStore) -> Result<Option<String>, String> {
    Ok(store
        .get()?
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty()))
}

pub fn save_key_to(store: &impl SecretStore, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("API key must not be empty".into());
    }
    store.set(key)
}

pub fn load_api_key() -> Result<Option<String>, String> {
    load_key_from(&Keychain::default())
}

pub fn save_api_key(k: &str) -> Result<(), String> {
    save_key_to(&Keychain::default(), k)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct FakeKeychain(RefCell<Option<String>>);

    impl SecretStore for FakeKeychain {
        fn get(&self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().clone())
        }

        fn set(&self, secret: &str) -> Result<(), String> {
            *self.0.borrow_mut() = Some(secret.to_owned());
            Ok(())
        }
    }

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.lang, Lang::Es);
        assert_eq!(s.theme, ThemePref::System);
        assert_eq!(s.advocate_model, "gemini-flash-latest");
        assert_eq!(s.judge_model, "gemini-flash-latest");
        assert_eq!(s.rpm, 10);
        assert_eq!(s.cost, CostSettings::default());
        assert_eq!(s.fx_manual, None);
    }

    #[test]
    fn settings_json_has_no_key_field() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert!(
            v.as_object()
                .unwrap()
                .keys()
                .all(|f| !f.contains("key") && !f.contains("secret"))
        );
    }

    #[test]
    fn partial_settings_json_falls_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"rpm": 5}"#).unwrap();
        assert_eq!(
            s,
            Settings {
                rpm: 5,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn key_round_trips_through_store() {
        let fake = FakeKeychain::default();
        assert_eq!(load_key_from(&fake), Ok(None));
        save_key_to(&fake, "  secret-value \n").unwrap();
        assert_eq!(load_key_from(&fake), Ok(Some("secret-value".into())));
    }

    #[test]
    fn empty_key_is_rejected_and_blank_reads_as_absent() {
        let fake = FakeKeychain::default();
        assert!(save_key_to(&fake, "   ").is_err());
        assert_eq!(fake.get(), Ok(None));
        fake.set("  ").unwrap();
        assert_eq!(load_key_from(&fake), Ok(None));
    }

    /// Hits the real Keychain with a throwaway service; run manually.
    #[test]
    #[ignore = "touches the macOS Keychain"]
    fn real_keychain_round_trip() {
        let service = format!("maeve-test-{}", std::process::id());
        let kc = Keychain::new(&service, "throwaway");
        assert_eq!(kc.get(), Ok(None));
        kc.set("dummy-value").unwrap();
        assert_eq!(kc.get(), Ok(Some("dummy-value".into())));
        kc.entry().unwrap().delete_credential().unwrap();
    }
}
