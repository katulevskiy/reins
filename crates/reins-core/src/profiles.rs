//! The account profiles on this device (Decisions v2 · O1, 06 §2, 08 R6): the Personal account, and a Managed work
//! profile per organization.
//!
//! A work profile is a separate Reins account (the work email, an org's member), and each profile runs its own
//! [`crate::api::ReinsCore`] on its own data directory, its data key wrapped by its own Keystore alias / Keychain item,
//! with its own notification channels. So nothing crosses: a profile's store, Activity, connections and keys are its
//! own, an agent pairs with exactly one, and nothing of the Personal profile is ever sent to an org (the work core
//! never sees it).
//!
//! This registry only remembers which profiles exist and where each keeps its data; it holds no secret.
//! `<base>/profiles.json`; the Personal profile keeps today's paths (`<base>/core`, the platform's existing key alias),
//! so nothing moves for anyone without a work profile.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// The Personal profile's id.
pub const PERSONAL: &str = "personal";
/// Work profiles per device.
pub const MAX_MANAGED: usize = 4;
const FILE: &str = "profiles.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ProfileKind {
    Personal,
    Managed,
}

/// One profile, with everything a platform needs to run its core.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AccountProfile {
    pub id: String,
    pub kind: ProfileKind,
    /// `Personal`, or the org's name (the badge on cards and rows).
    pub label: String,
    pub org_id: Option<String>,
    pub email: Option<String>,
    pub server_url: Option<String>,
    /// The core's data directory.
    pub data_dir: String,
    /// Appended to the platform's data-key alias / Keychain account (`""` for Personal: the existing one).
    pub key_alias: String,
    /// Put in front of notification channel ids and thread identifiers (`""` for Personal).
    pub channel_prefix: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Managed {
    id: String,
    org_id: String,
    org_name: String,
    email: String,
    server_url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    personal_email: Option<String>,
    #[serde(default)]
    managed: Vec<Managed>,
}

/// The device's profiles (`<base>/profiles.json`).
#[derive(uniffi::Object)]
pub struct ProfileRegistry {
    base: PathBuf,
    stored: Mutex<Stored>,
}

/// A work profile's id: stable for one org on one server.
#[must_use]
pub fn managed_id(server_url: &str, org_id: &str) -> String {
    let mut input = server_url.trim_end_matches('/').to_ascii_lowercase().into_bytes();
    input.push(0);
    input.extend_from_slice(org_id.as_bytes());
    let digest = ring::digest::digest(&ring::digest::SHA256, &input);
    format!("work-{}", &data_encoding::HEXLOWER.encode(digest.as_ref())[..12])
}

impl ProfileRegistry {
    fn path(&self) -> PathBuf {
        self.base.join(FILE)
    }

    fn profiles_dir(&self) -> PathBuf {
        self.base.join("profiles")
    }

    fn personal(&self, stored: &Stored) -> AccountProfile {
        AccountProfile {
            id: PERSONAL.to_owned(),
            kind: ProfileKind::Personal,
            label: "Personal".to_owned(),
            org_id: None,
            email: stored.personal_email.clone(),
            server_url: None,
            data_dir: self.base.join("core").to_string_lossy().into_owned(),
            key_alias: String::new(),
            channel_prefix: String::new(),
        }
    }

    fn managed(&self, m: &Managed) -> AccountProfile {
        AccountProfile {
            id: m.id.clone(),
            kind: ProfileKind::Managed,
            label: m.org_name.clone(),
            org_id: Some(m.org_id.clone()),
            email: Some(m.email.clone()),
            server_url: Some(m.server_url.clone()),
            data_dir: self.profiles_dir().join(&m.id).join("core").to_string_lossy().into_owned(),
            key_alias: m.id.clone(),
            channel_prefix: format!("{}.", m.id),
        }
    }

    fn save(&self, stored: &Stored) -> Result<(), CoreError> {
        std::fs::create_dir_all(&self.base).map_err(|e| CoreError::storage(format!("profiles: {e}")))?;
        let tmp = self.base.join(format!("{FILE}.tmp"));
        let text = serde_json::to_vec_pretty(stored).map_err(|e| CoreError::storage(e.to_string()))?;
        std::fs::write(&tmp, text).map_err(|e| CoreError::storage(format!("profiles: {e}")))?;
        std::fs::rename(&tmp, self.path()).map_err(|e| CoreError::storage(format!("profiles: {e}")))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Stored> {
        self.stored.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Opens the registry under `base` (a missing or unreadable file: only the Personal profile).
    #[must_use]
    pub fn open(base: &Path) -> Self {
        let stored = std::fs::read(base.join(FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
            .unwrap_or_default();
        Self {
            base: base.to_owned(),
            stored: Mutex::new(stored),
        }
    }
}

#[uniffi::export]
#[allow(clippy::needless_pass_by_value, reason = "UniFFI passes owned values")]
impl ProfileRegistry {
    /// The registry of the profiles kept under `base_dir` (the folder that holds today's `core` directory).
    #[uniffi::constructor]
    pub fn new(base_dir: String) -> Arc<Self> {
        Arc::new(Self::open(Path::new(&base_dir)))
    }

    /// Every profile, Personal first.
    pub fn list(&self) -> Vec<AccountProfile> {
        let stored = self.lock();
        std::iter::once(self.personal(&stored)).chain(stored.managed.iter().map(|m| self.managed(m))).collect()
    }

    /// The profile the app shows now (Personal unless another was chosen).
    pub fn active(&self) -> AccountProfile {
        let stored = self.lock();
        stored
            .active
            .as_deref()
            .and_then(|id| stored.managed.iter().find(|m| m.id == id))
            .map_or_else(|| self.personal(&stored), |m| self.managed(m))
    }

    /// Shows `id` from now on.
    pub fn set_active(&self, id: String) -> Result<(), CoreError> {
        let mut stored = self.lock();
        if id != PERSONAL && !stored.managed.iter().any(|m| m.id == id) {
            return Err(CoreError::invalid("No such profile"));
        }
        stored.active = (id != PERSONAL).then_some(id);
        self.save(&stored)
    }

    /// Remembers the Personal account's email (so a work profile can't be the same account).
    pub fn note_personal(&self, email: Option<String>) -> Result<(), CoreError> {
        let mut stored = self.lock();
        let email = email.map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty());
        if stored.personal_email == email {
            return Ok(());
        }
        stored.personal_email = email;
        self.save(&stored)
    }

    /// Adds (or renames) the work profile of `org_id`, signed in as `email` at `server_url`.
    pub fn add_managed(
        &self,
        org_id: String,
        org_name: String,
        email: String,
        server_url: String,
    ) -> Result<AccountProfile, CoreError> {
        let email = email.trim().to_lowercase();
        let org_name = crate::text::truncate_chars(&crate::text::one_line(org_name.trim()), 64);
        if org_id.trim().is_empty() || email.is_empty() || server_url.trim().is_empty() {
            return Err(CoreError::invalid("A work profile needs its organization, email and server"));
        }
        let mut stored = self.lock();
        if stored.personal_email.as_deref() == Some(email.as_str()) {
            return Err(CoreError::invalid("This is your personal account. Sign in with your work account."));
        }
        let id = managed_id(&server_url, &org_id);
        if let Some(m) = stored.managed.iter_mut().find(|m| m.id == id) {
            m.org_name = org_name;
            m.email = email;
        } else {
            if stored.managed.len() >= MAX_MANAGED {
                return Err(CoreError::invalid("This phone has as many work profiles as it can hold"));
            }
            stored.managed.push(Managed {
                id: id.clone(),
                org_id,
                org_name,
                email,
                server_url: server_url.trim_end_matches('/').to_owned(),
            });
        }
        self.save(&stored)?;
        let m = stored.managed.iter().find(|m| m.id == id).cloned().unwrap_or_default();
        Ok(self.managed(&m))
    }

    /// Removes a work profile and deletes everything it kept on this device (its data directory). The Personal
    /// profile can't be removed here.
    pub fn remove(&self, id: String) -> Result<(), CoreError> {
        if id == PERSONAL {
            return Err(CoreError::invalid("The personal profile can't be removed"));
        }
        let mut stored = self.lock();
        let before = stored.managed.len();
        stored.managed.retain(|m| m.id != id);
        if stored.managed.len() == before {
            return Ok(());
        }
        if stored.active.as_deref() == Some(id.as_str()) {
            stored.active = None;
        }
        self.save(&stored)?;
        let dir = self.profiles_dir().join(&id);
        // `id` is one this registry made (`work-` and hex), so the path stays under `profiles/`.
        if dir.starts_with(self.profiles_dir()) && dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| CoreError::storage(format!("profile data: {e}")))?;
        }
        Ok(())
    }
}

/// The bytes a work profile's device key signs to enroll the device ([`reins_proto::org::enrollment_message`]).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value, reason = "UniFFI passes owned values")]
pub fn org_enrollment_message(challenge: String) -> Vec<u8> {
    reins_proto::org::enrollment_message(&challenge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(dir: &Path) -> Arc<ProfileRegistry> {
        ProfileRegistry::new(dir.to_string_lossy().into_owned())
    }

    #[test]
    fn personal_keeps_todays_paths() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        let list = r.list();
        assert_eq!(list.len(), 1);
        let p = &list[0];
        assert_eq!(
            (p.id.as_str(), p.kind, p.key_alias.as_str(), p.channel_prefix.as_str()),
            (PERSONAL, ProfileKind::Personal, "", "")
        );
        assert_eq!(Path::new(&p.data_dir), dir.path().join("core"));
        assert_eq!(r.active(), *p);
    }

    #[test]
    fn managed_profiles_get_their_own_dir_alias_and_channels() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        let w = r
            .add_managed("org1".into(), "Acme".into(), "Dana@Acme.com".into(), "https://app.reins2fa.com/".into())
            .unwrap();
        assert_eq!(w.kind, ProfileKind::Managed);
        assert_eq!(w.label, "Acme");
        assert_eq!(w.email.as_deref(), Some("dana@acme.com"));
        assert!(w.id.starts_with("work-") && w.id.len() == 17, "{}", w.id);
        assert_eq!(w.key_alias, w.id);
        assert_eq!(w.channel_prefix, format!("{}.", w.id));
        assert_eq!(Path::new(&w.data_dir), dir.path().join("profiles").join(&w.id).join("core"));
        let personal = &r.list()[0];
        assert_ne!(personal.data_dir, w.data_dir);
        assert_ne!(personal.key_alias, w.key_alias);
        assert_eq!(w.id, managed_id("https://APP.reins2fa.com", "org1"));
    }

    #[test]
    fn one_managed_profile_per_org_and_never_the_personal_account() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        r.note_personal(Some("dana@gmail.com".into())).unwrap();
        assert!(r.add_managed("o".into(), "Acme".into(), "DANA@gmail.com".into(), "https://s".into()).is_err());
        let a = r.add_managed("o".into(), "Acme".into(), "dana@acme.com".into(), "https://s".into()).unwrap();
        let b = r.add_managed("o".into(), "Acme Inc".into(), "dana@acme.com".into(), "https://s".into()).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(r.list().len(), 2);
        assert_eq!(r.list()[1].label, "Acme Inc");
        for i in 0..MAX_MANAGED - 1 {
            r.add_managed(format!("other{i}"), "X".into(), "x@x.io".into(), "https://s".into()).unwrap();
        }
        assert!(r.add_managed("one-more".into(), "X".into(), "x@x.io".into(), "https://s".into()).is_err());
    }

    #[test]
    fn removing_a_profile_wipes_only_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let r = registry(dir.path());
        std::fs::create_dir_all(dir.path().join("core")).unwrap();
        std::fs::write(dir.path().join("core").join("reins.sealed"), b"personal").unwrap();
        let w = r.add_managed("o".into(), "Acme".into(), "dana@acme.com".into(), "https://s".into()).unwrap();
        std::fs::create_dir_all(&w.data_dir).unwrap();
        std::fs::write(Path::new(&w.data_dir).join("reins.sealed"), b"work").unwrap();
        r.set_active(w.id.clone()).unwrap();
        assert_eq!(r.active().id, w.id);
        assert!(r.remove(PERSONAL.into()).is_err());
        r.remove(w.id.clone()).unwrap();
        assert!(!Path::new(&w.data_dir).exists());
        assert!(dir.path().join("core").join("reins.sealed").exists(), "the personal store stays");
        assert_eq!(r.active().id, PERSONAL);
        assert!(r.set_active(w.id).is_err());
    }

    #[test]
    fn the_registry_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let w = {
            let r = registry(dir.path());
            let w = r.add_managed("o".into(), "Acme".into(), "dana@acme.com".into(), "https://s".into()).unwrap();
            r.set_active(w.id.clone()).unwrap();
            w
        };
        let again = registry(dir.path());
        assert_eq!(again.active(), w);
        assert_eq!(again.list().len(), 2);
        // A damaged file leaves only Personal (no profile is guessed).
        std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
        assert_eq!(registry(dir.path()).list().len(), 1);
    }

    #[test]
    fn the_enrollment_message_is_the_protocols() {
        assert_eq!(org_enrollment_message("abc".into()), reins_proto::org::enrollment_message("abc"));
    }
}
