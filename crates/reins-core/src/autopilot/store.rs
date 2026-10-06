//! Autopilot's tables (migration 7). Modes are plain settings; everything learned or seen (profiles with their
//! adapters and class locks, the decision memory, what was evaluated for each request) is sealed with the data key,
//! each blob bound to its row; approved targets are kept only as keyed hashes.

use std::collections::{BTreeMap, HashSet};

use ring::rand::SecureRandom;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::adapter::Adapter;
use super::memory::{Embedding, MEMORY_CAP, MemoryRow};
use super::modes::ModeRow;
use super::types::{AutopilotMode, Preset, Verdict};
use crate::CoreError;
use crate::store::Store;

/// What was evaluated for a request is kept this long after it (corrections from Activity need it).
pub const SUGGESTION_RETENTION_SECS: i64 = 30 * 86_400;
const DAY_SECS: i64 = 86_400;
const SALT_AAD: &str = "autopilot.salt";

/// A profile as stored (sealed).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub preset: Preset,
    /// Class key → unlocked (`true`) or locked (`false`) by hand.
    #[serde(default)]
    pub manual: BTreeMap<String, bool>,
    #[serde(default)]
    pub adapter: Option<Adapter>,
    /// Decisions remembered since the adapter was last trained.
    #[serde(default)]
    pub since_train: usize,
}

/// A remembered neighbour, as shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeighbourNote {
    pub label: String,
    pub verdict: Verdict,
    pub similarity: f32,
    pub at: i64,
}

/// What the model said about a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Judged {
    pub model_id: String,
    /// After the prompt-injection rule: approve, deny, ask.
    pub probs: [f32; 3],
    pub confidence: f32,
    pub escalate: f32,
    /// Calibrated logits of the two passes.
    pub base_facts: [f32; 3],
    pub base_full: [f32; 3],
    pub e_facts: Embedding,
    pub e_full: Embedding,
    pub neighbours: Vec<NeighbourNote>,
}

/// How a request was finally decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    /// "" (the user) | "autopilot" | "bypass" | "lockdown"
    pub by: String,
    pub verdict: Verdict,
    pub at: i64,
}

/// Everything the pass worked out about one parked request (sealed, bound to the request id).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub request_id: String,
    pub at: i64,
    /// "request" | "blob" | "pairing"
    pub kind: String,
    pub connection_id: String,
    pub connection_label: String,
    pub mode: AutopilotMode,
    pub profile_id: String,
    pub profile_name: String,
    pub class_key: String,
    pub label: String,
    /// Keyed hash of (connection, target); empty without a target.
    pub target_key: String,
    pub s_facts: String,
    pub s_full: String,
    pub novel: bool,
    /// Why it always waits for the user.
    pub floor: Option<String>,
    pub judged: Option<Judged>,
    /// What Autopilot would do (gates aside).
    pub verdict: Verdict,
    pub reason: String,
    pub decided: Option<Decided>,
}

/// A stored profile with its id.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredProfile {
    pub id: String,
    pub created_at: i64,
    pub profile: Profile,
}

fn profile_aad(id: &str) -> String {
    format!("autopilot.profile:{id}")
}

fn memory_aad(profile_id: &str, request_id: &str) -> String {
    format!("autopilot.memory:{profile_id}:{request_id}")
}

fn suggestion_aad(request_id: &str) -> String {
    format!("autopilot.suggestion:{request_id}")
}

fn to_json<T: Serialize>(v: &T) -> Result<Vec<u8>, CoreError> {
    serde_json::to_vec(v).map_err(|e| CoreError::storage(e.to_string()))
}

impl Store {
    // ---- modes ---------------------------------------------------------------------------------------------------

    /// The global setting (`scope` = "") or a connection's; the default when there is none.
    pub fn ap_mode_row(&self, scope: &str) -> Result<ModeRow, CoreError> {
        Ok(self.ap_mode_rows()?.into_iter().find(|(s, _)| s == scope).map(|(_, r)| r).unwrap_or_default())
    }

    pub fn ap_mode_rows(&self) -> Result<Vec<(String, ModeRow)>, CoreError> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT scope, mode, bypass_until, profile_id FROM autopilot_settings ORDER BY scope")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    ModeRow {
                        mode: r.get::<_, Option<String>>(1)?.as_deref().and_then(AutopilotMode::parse),
                        bypass_until: r.get(2)?,
                        profile_id: r.get(3)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn ap_set_mode_row(&self, scope: &str, row: &ModeRow, now: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        if row == &ModeRow::default() {
            conn.execute("DELETE FROM autopilot_settings WHERE scope = ?1", params![scope])?;
            return Ok(());
        }
        conn.execute(
            "INSERT INTO autopilot_settings (scope, mode, bypass_until, profile_id, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (scope) DO UPDATE SET mode = ?2, bypass_until = ?3, profile_id = ?4, updated_at = ?5",
            params![scope, row.mode.map(AutopilotMode::as_str), row.bypass_until, row.profile_id, now],
        )?;
        Ok(())
    }

    // ---- profiles ------------------------------------------------------------------------------------------------

    /// Oldest first; profiles that cannot be decrypted are skipped.
    pub fn ap_profiles(&self) -> Result<Vec<StoredProfile>, CoreError> {
        let rows: Vec<(String, i64, Vec<u8>)> = {
            let conn = self.lock()?;
            let mut stmt =
                conn.prepare("SELECT id, created_at, data FROM autopilot_profiles ORDER BY created_at, id")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(id, created_at, sealed)| {
                let plain = self.unseal(&profile_aad(&id), &sealed).ok()?;
                let profile = serde_json::from_slice(&plain).ok()?;
                Some(StoredProfile {
                    id,
                    created_at,
                    profile,
                })
            })
            .collect())
    }

    pub fn ap_profile(&self, id: &str) -> Result<Option<Profile>, CoreError> {
        Ok(self.ap_profiles()?.into_iter().find(|p| p.id == id).map(|p| p.profile))
    }

    pub fn ap_put_profile(&self, id: &str, created_at: i64, profile: &Profile) -> Result<(), CoreError> {
        let sealed = self.seal(&profile_aad(id), &to_json(profile)?)?;
        self.lock()?.execute(
            "INSERT INTO autopilot_profiles (id, created_at, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET data = ?3",
            params![id, created_at, sealed],
        )?;
        Ok(())
    }

    /// Deletes a profile, its memory, and the assignments to it.
    pub fn ap_delete_profile(&self, id: &str) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM autopilot_profiles WHERE id = ?1", params![id])?;
        conn.execute("DELETE FROM autopilot_memory WHERE profile_id = ?1", params![id])?;
        conn.execute("UPDATE autopilot_settings SET profile_id = NULL WHERE profile_id = ?1", params![id])?;
        conn.execute(
            "DELETE FROM autopilot_settings WHERE mode IS NULL AND bypass_until IS NULL AND profile_id IS NULL",
            [],
        )?;
        Ok(())
    }

    // ---- memory --------------------------------------------------------------------------------------------------

    /// A profile's memory, newest first (rows that cannot be decrypted are skipped).
    pub fn ap_memory(&self, profile_id: &str) -> Result<Vec<MemoryRow>, CoreError> {
        let rows: Vec<(String, Vec<u8>)> = {
            let conn = self.lock()?;
            let mut stmt = conn.prepare(
                "SELECT request_id, data FROM autopilot_memory WHERE profile_id = ?1 ORDER BY at DESC, id DESC",
            )?;
            stmt.query_map(params![profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(request_id, sealed)| {
                let plain = self.unseal(&memory_aad(profile_id, &request_id), &sealed).ok()?;
                serde_json::from_slice(&plain).ok()
            })
            .collect())
    }

    pub fn ap_memory_count(&self, profile_id: &str) -> Result<u32, CoreError> {
        Ok(self.lock()?.query_row(
            "SELECT COUNT(*) FROM autopilot_memory WHERE profile_id = ?1",
            params![profile_id],
            |r| r.get(0),
        )?)
    }

    /// Remembers a decision (replacing an earlier one about the same request) and evicts beyond the cap.
    pub fn ap_add_memory(&self, profile_id: &str, row: &MemoryRow) -> Result<(), CoreError> {
        let sealed = self.seal(&memory_aad(profile_id, &row.request_id), &to_json(row)?)?;
        let conn = self.lock()?;
        conn.execute("DELETE FROM autopilot_memory WHERE request_id = ?1", params![row.request_id])?;
        conn.execute(
            "INSERT INTO autopilot_memory (profile_id, request_id, at, data) VALUES (?1, ?2, ?3, ?4)",
            params![profile_id, row.request_id, row.at, sealed],
        )?;
        conn.execute(
            "DELETE FROM autopilot_memory WHERE profile_id = ?1 AND id NOT IN
                 (SELECT id FROM autopilot_memory WHERE profile_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2)",
            params![profile_id, i64::try_from(MEMORY_CAP).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    pub fn ap_clear_memory(&self, profile_id: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM autopilot_memory WHERE profile_id = ?1", params![profile_id])?;
        Ok(())
    }

    // ---- suggestions ---------------------------------------------------------------------------------------------

    pub fn ap_suggestion(&self, request_id: &str) -> Result<Option<Suggestion>, CoreError> {
        let sealed: Option<Vec<u8>> = self
            .lock()?
            .query_row("SELECT data FROM autopilot_suggestions WHERE request_id = ?1", params![request_id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(sealed
            .and_then(|s| self.unseal(&suggestion_aad(request_id), &s).ok())
            .and_then(|plain| serde_json::from_slice(&plain).ok()))
    }

    pub fn ap_put_suggestion(&self, s: &Suggestion) -> Result<(), CoreError> {
        let sealed = self.seal(&suggestion_aad(&s.request_id), &to_json(s)?)?;
        self.lock()?.execute(
            "INSERT INTO autopilot_suggestions (request_id, at, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (request_id) DO UPDATE SET data = ?3",
            params![s.request_id, s.at, sealed],
        )?;
        Ok(())
    }

    /// The requests the pass already went through.
    pub fn ap_evaluated(&self) -> Result<HashSet<String>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT request_id FROM autopilot_suggestions")?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<HashSet<_>, _>>()?;
        Ok(ids)
    }

    /// Forgets old evaluations and old rate-limit entries.
    pub fn ap_prune(&self, now: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM autopilot_suggestions WHERE at <= ?1", params![now - SUGGESTION_RETENTION_SECS])?;
        conn.execute("DELETE FROM autopilot_rate WHERE at <= ?1", params![now - DAY_SECS])?;
        Ok(())
    }

    // ---- rate limit ----------------------------------------------------------------------------------------------

    pub fn ap_rate_add(&self, connection_id: &str, at: i64) -> Result<(), CoreError> {
        self.lock()?
            .execute("INSERT INTO autopilot_rate (connection_id, at) VALUES (?1, ?2)", params![connection_id, at])?;
        Ok(())
    }

    /// Auto-approvals of a connection after `since`.
    pub fn ap_rate_count(&self, connection_id: &str, since: i64) -> Result<u32, CoreError> {
        Ok(self.lock()?.query_row(
            "SELECT COUNT(*) FROM autopilot_rate WHERE connection_id = ?1 AND at > ?2",
            params![connection_id, since],
            |r| r.get(0),
        )?)
    }

    // ---- targets -------------------------------------------------------------------------------------------------

    /// The secret the target hashes are keyed with (made on first use).
    fn ap_salt(&self) -> Result<Vec<u8>, CoreError> {
        let conn = self.lock()?;
        let existing: Option<Vec<u8>> =
            conn.query_row("SELECT value FROM meta WHERE key = 'autopilot_salt'", [], |r| r.get(0)).optional()?;
        if let Some(salt) = existing.and_then(|s| self.unseal(SALT_AAD, &s).ok()) {
            return Ok(salt);
        }
        let mut salt = vec![0_u8; 32];
        ring::rand::SystemRandom::new().fill(&mut salt).map_err(|_| CoreError::storage("no randomness"))?;
        let sealed = self.seal(SALT_AAD, &salt)?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('autopilot_salt', ?1) ON CONFLICT (key) DO UPDATE SET value = ?1",
            params![sealed],
        )?;
        Ok(salt)
    }

    /// The keyed hash of a connection's target ("" without a target).
    pub fn ap_target_key(&self, connection_id: &str, target: &str) -> Result<String, CoreError> {
        let target = target.trim().to_lowercase();
        if target.is_empty() {
            return Ok(String::new());
        }
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &self.ap_salt()?);
        let tag = ring::hmac::sign(&key, format!("{connection_id}\0{target}").as_bytes());
        Ok(data_encoding::HEXLOWER.encode(tag.as_ref()))
    }

    pub fn ap_target_seen(&self, key: &str) -> Result<bool, CoreError> {
        if key.is_empty() {
            return Ok(false);
        }
        let found: Option<i64> = self
            .lock()?
            .query_row("SELECT 1 FROM autopilot_targets WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    pub fn ap_target_forget(&self, key: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM autopilot_targets WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn ap_target_add(&self, key: &str, at: i64) -> Result<(), CoreError> {
        if key.is_empty() {
            return Ok(());
        }
        self.lock()?.execute(
            "INSERT INTO autopilot_targets (key, at) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET at = ?2",
            params![key, at],
        )?;
        Ok(())
    }

    // ---- connection facts ----------------------------------------------------------------------------------------

    /// (approved, denied) entries of a connection in the activity.
    pub fn ap_history(&self, connection_id: &str) -> Result<(u32, u32), CoreError> {
        Ok(self.lock()?.query_row(
            "SELECT
                 COALESCE(SUM(outcome IN ('released', 'sent', 'granted')), 0),
                 COALESCE(SUM(outcome = 'denied'), 0)
             FROM audit WHERE connection_id = ?1 AND action != 'pair'",
            params![connection_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// When this phone approved the connection's pairing (its own clock), if it did.
    pub fn ap_paired_at(&self, connection_id: &str) -> Result<Option<i64>, CoreError> {
        Ok(self.lock()?.query_row(
            "SELECT MAX(at) FROM audit WHERE connection_id = ?1 AND action = 'pair' AND outcome = 'released'",
            params![connection_id],
            |r| r.get(0),
        )?)
    }
}
