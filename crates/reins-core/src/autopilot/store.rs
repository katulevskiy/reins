//! Autopilot's tables (migration 7). Modes are plain settings; everything learned or seen (profiles with their
//! adapters and class locks, the decision memory, what was evaluated for each request) is sealed with the data key,
//! each blob bound to its row; approved targets are kept only as keyed hashes.

use std::collections::{BTreeMap, HashSet};

use reins_policy::Grant;
use ring::rand::SecureRandom;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::adapter::Adapter;
use super::memory::{Embedding, MEMORY_CAP, MemoryRow};
use super::modes::ModeRow;
use super::types::{AutopilotMode, MAX_THRESHOLD, MIN_THRESHOLD, Preset, Verdict};
use crate::CoreError;
use crate::store::{Store, StoredGrant};

/// What was evaluated for a request is kept this long after it (corrections from Activity need it).
pub const SUGGESTION_RETENTION_SECS: i64 = 30 * 86_400;
const DAY_SECS: i64 = 86_400;
pub(crate) const SALT_AAD: &str = "autopilot.salt";
/// `meta` key prefix (then the scope) of when a running bypass started.
const BYPASS_SINCE_KEY: &str = "autopilot_bypass_since:";
/// `meta` key prefix (then the connection) of when the server says a connection was made.
const CREATED_KEY: &str = "autopilot_created:";
/// `meta` key prefixes (then the scope) of an Overnight's end, start, and the mode it returns to.
const OVERNIGHT_UNTIL_KEY: &str = "autopilot_overnight_until:";
const OVERNIGHT_SINCE_KEY: &str = "autopilot_overnight_since:";
const OVERNIGHT_RETURN_KEY: &str = "autopilot_overnight_return:";
/// `meta` key: how long a parked request waits past the morning, in hours.
pub const OVERNIGHT_EXPIRY_KEY: &str = "autopilot.overnight_expiry_hours";

/// A profile as stored (sealed).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// The last named preset picked (never `Custom`: older versions read this field and know only the named ones).
    #[serde(default)]
    pub preset: Preset,
    /// The user's own thresholds; when set, they apply instead of the preset's.
    #[serde(default)]
    pub custom: Option<Thresholds>,
    /// Class key → locked (`true`) or unlocked (`false`) by hand (`Engine::set_class_lock`).
    #[serde(default)]
    pub manual: BTreeMap<String, bool>,
    #[serde(default)]
    pub adapter: Option<Adapter>,
    /// Decisions remembered since the adapter was last trained.
    #[serde(default)]
    pub since_train: usize,
}

/// A profile's own confidence thresholds, each within [`MIN_THRESHOLD`, `MAX_THRESHOLD`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub approve: f32,
    pub deny: f32,
}

impl Thresholds {
    /// Both clamped to the allowed range and rounded to whole percent; `None` when either is not a number.
    pub fn clamped(approve: f32, deny: f32) -> Option<Self> {
        let clamp = |x: f32| x.is_finite().then(|| (x.clamp(MIN_THRESHOLD, MAX_THRESHOLD) * 100.0).round() / 100.0);
        Some(Self {
            approve: clamp(approve)?,
            deny: clamp(deny)?,
        })
    }
}

impl Profile {
    /// The preset shown: `Custom` when the user set thresholds of their own.
    pub fn shown_preset(&self) -> Preset {
        if self.custom.is_some() {
            Preset::Custom
        } else {
            self.preset
        }
    }

    /// (θ_a, θ_d) the Auto decision uses for this profile.
    pub fn thresholds(&self) -> (f32, f32) {
        self.custom
            .and_then(|t| Thresholds::clamped(t.approve, t.deny))
            .map_or_else(|| self.preset.thresholds(), |t| (t.approve, t.deny))
    }
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
    /// Severity after the two-pass rule (benign, risky, severe, catastrophic), when the model rates risk.
    #[serde(default)]
    pub risk: Option<[f32; 4]>,
    /// The verified package that judged it declares the risk output (whether or not this run returned it).
    #[serde(default)]
    pub risk_declared: bool,
    /// That package's Overnight threshold, when it names a valid one.
    #[serde(default)]
    pub risk_threshold: Option<f32>,
    /// The highest class any view (facts, full, neutral) gave on its own: the class shown is never below it.
    #[serde(default)]
    pub risk_view_max: Option<super::types::RiskLevel>,
    /// What Auto's verdict rested on besides the probabilities; `None` for evaluations made before the cold start
    /// (replayed with the rule of then, [`super::decide::would`]).
    #[serde(default)]
    pub evidence: Option<super::decide::Evidence>,
}

/// How a request was finally decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    /// "" (the user) | "autopilot" | "overnight" | "bypass" | "lockdown"
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
    /// The risk, when the model rated it.
    #[serde(default)]
    pub risk: Option<super::risk::RiskNote>,
    /// It was read to delete, move away or overwrite something (the connection's last hour counts these).
    #[serde(default)]
    pub deletes: bool,
    /// What the matrix decided, or would have decided in shadow (Decisions v2).
    #[serde(default)]
    pub matrix: Option<MatrixNote>,
    /// The task its agent was working on (Decisions v2, 01 §12).
    #[serde(default)]
    pub task: Option<String>,
    /// Overnight parked it for the morning (01 §11).
    #[serde(default)]
    pub parked: Option<Parked>,
    /// "Approve all" may cover it: set only where the matrix asked it softly (never for a closer look, a risk hold, a
    /// tripwire, a pause, the floor or anything always asked).
    #[serde(default)]
    pub batchable: bool,
}

/// An Overnight park: a Deferred ticket until `deadline`, shown in the morning digest at `until`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parked {
    pub until: i64,
    pub deadline: i64,
    /// "Approve all" may cover it (not hard, not L3, nothing always asked).
    pub batchable: bool,
}

/// What the matrix said of a request (`reins_policy::matrix::Evaluation`, as kept).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixNote {
    pub outcome: reins_policy::matrix::Outcome,
    pub zone: reins_policy::context::Zone,
    pub level: reins_policy::effects::Level,
    /// "Auto · Mine · Undoable".
    pub line: String,
    /// The policy snapshot's hash.
    pub snapshot: String,
    /// Recorded only: something else decided.
    pub shadow: bool,
    /// The kinds of effect it had, and the resource it acted on ("Ask me next time" writes its exception from them).
    #[serde(default)]
    pub kinds: Vec<reins_policy::effects::EffectKind>,
    #[serde(default)]
    pub resource: Option<String>,
    /// An explicit rule of the owner (or a repo, an org) asks or denies it: that holds in every automatic mode.
    #[serde(default)]
    pub by_rule: bool,
    /// The task whose envelope made it Auto (a use is spent before it runs).
    #[serde(default)]
    pub by_task: Option<String>,
    /// The Telegram chat (its id) this was evaluated for: an automatic approval sends to exactly that chat, never to
    /// whatever the name it was written with resolves to later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    /// What the matrix judged it by (analysis, zone, connection), so a change can be previewed on the phone's own
    /// history (02 §4). Kept only on the sealed suggestion (30 days), never copied into Activity or sent anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Box<super::preview::PreviewInput>>,
}

/// A stored profile with its id.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredProfile {
    pub id: String,
    pub created_at: i64,
    pub profile: Profile,
}

pub(crate) fn profile_aad(id: &str) -> String {
    format!("autopilot.profile:{id}")
}

pub(crate) fn memory_aad(profile_id: &str, request_id: &str) -> String {
    format!("autopilot.memory:{profile_id}:{request_id}")
}

fn suggestion_aad(request_id: &str) -> String {
    format!("autopilot.suggestion:{request_id}")
}

fn to_json<T: Serialize>(v: &T) -> Result<Vec<u8>, CoreError> {
    serde_json::to_vec(v).map_err(|e| CoreError::storage(e.to_string()))
}

/// A row of `autopilot_settings`: its mode and bypass end.
type SettingRow = (Option<String>, Option<i64>);

/// Whether a permission with this origin may be used now: always, unless a bypass or an Overnight granted it; then
/// only while that bypass or Overnight (the global one, or the connection's own) is in force and no lockdown holds.
/// Read with `conn`, inside the caller's transaction, so that one stopped a moment ago is seen stopped.
pub(crate) fn origin_in_force(conn: &Connection, origin: &str, now: i64) -> Result<bool, CoreError> {
    let Some((kind, scope)) = super::modes::temporary_scope(origin) else {
        return Ok(true);
    };
    let row = |scope: &str| -> Result<Option<SettingRow>, CoreError> {
        Ok(conn
            .query_row("SELECT mode, bypass_until FROM autopilot_settings WHERE scope = ?1", params![scope], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?)
    };
    let lockdown = |r: &Option<SettingRow>| {
        r.as_ref().is_some_and(|(mode, _)| mode.as_deref() == Some(AutopilotMode::Lockdown.as_str()))
    };
    let global = row("")?;
    if lockdown(&global) {
        return Ok(false);
    }
    let own = if scope.is_empty() {
        global
    } else {
        row(scope)?
    };
    if lockdown(&own) {
        return Ok(false);
    }
    Ok(match kind {
        super::modes::Temporary::Bypass => own.and_then(|(_, until)| until).is_some_and(|until| until > now),
        super::modes::Temporary::Overnight => {
            let overnight =
                own.as_ref().is_some_and(|(mode, _)| mode.as_deref() == Some(AutopilotMode::Overnight.as_str()));
            let until: Option<String> = conn
                .query_row(
                    "SELECT value FROM meta WHERE key = ?1",
                    params![format!("{OVERNIGHT_UNTIL_KEY}{scope}")],
                    |r| r.get(0),
                )
                .optional()?;
            overnight && until.and_then(|v| v.parse::<i64>().ok()).is_none_or(|until| until > now)
        }
    })
}

/// Revokes, in `conn`'s transaction, every active permission a bypass granted whose bypass is no longer in force;
/// returns them (each only once: a revoked one is not active any more).
fn revoke_ended_bypass_grants(conn: &Connection, now: i64) -> Result<Vec<StoredGrant>, CoreError> {
    let rows: Vec<(String, String, String, String, Option<i64>, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, connection_id, connection_label, grant_json, last_used_at, origin FROM grants \
             WHERE origin = ?1 OR origin LIKE ?2 OR origin = ?3 OR origin LIKE ?4",
        )?;
        stmt.query_map(
            params![super::modes::BYPASS_ORIGIN, "bypass:%", super::modes::OVERNIGHT_ORIGIN, "overnight:%"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut revoked = Vec::new();
    for (id, connection_id, connection_label, json, last_used_at, origin) in rows {
        let Ok(mut grant) = serde_json::from_str::<Grant>(&json) else {
            continue;
        };
        if grant.id.0 != id || grant.connection_id.0 != connection_id || !grant.is_active(now) {
            continue;
        }
        if origin_in_force(conn, &origin, now)? {
            continue;
        }
        grant.revoked = true;
        let json = serde_json::to_string(&grant).map_err(|e| CoreError::storage(e.to_string()))?;
        conn.execute("UPDATE grants SET grant_json = ?2 WHERE id = ?1", params![id, json])?;
        revoked.push(StoredGrant {
            grant,
            connection_label,
            last_used_at,
            origin,
        });
    }
    Ok(revoked)
}

impl Store {
    /// Takes back every permission whose bypass has ended (stopped, run out, locked down, replaced), in one
    /// transaction; idempotent. Returns what it revoked, for the Activity entries.
    pub fn ap_sweep_bypass_grants(&self, now: i64) -> Result<Vec<StoredGrant>, CoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revoked = revoke_ended_bypass_grants(&tx, now)?;
        tx.commit()?;
        Ok(revoked)
    }

    // ---- modes ---------------------------------------------------------------------------------------------------

    /// The global setting (`scope` = "") or a connection's; the default when there is none.
    pub fn ap_mode_row(&self, scope: &str) -> Result<ModeRow, CoreError> {
        Ok(self.ap_mode_rows()?.into_iter().find(|(s, _)| s == scope).map(|(_, r)| r).unwrap_or_default())
    }

    pub fn ap_mode_rows(&self) -> Result<Vec<(String, ModeRow)>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.scope, s.mode, s.bypass_until, s.profile_id, m.value, ou.value, os.value, orr.value
                 FROM autopilot_settings s
                 LEFT JOIN meta m ON m.key = ?1 || s.scope
                 LEFT JOIN meta ou ON ou.key = ?2 || s.scope
                 LEFT JOIN meta os ON os.key = ?3 || s.scope
                 LEFT JOIN meta orr ON orr.key = ?4 || s.scope
                 ORDER BY s.scope",
        )?;
        let rows = stmt
            .query_map(
                params![BYPASS_SINCE_KEY, OVERNIGHT_UNTIL_KEY, OVERNIGHT_SINCE_KEY, OVERNIGHT_RETURN_KEY],
                |r| {
                    let bypass_until: Option<i64> = r.get(2)?;
                    let mode = r.get::<_, Option<String>>(1)?.as_deref().and_then(AutopilotMode::parse);
                    let overnight = mode == Some(AutopilotMode::Overnight);
                    let meta = |i: usize| -> Option<String> {
                        r.get::<_, Option<String>>(i).ok().flatten().filter(|_| overnight)
                    };
                    Ok((
                        r.get::<_, String>(0)?,
                        ModeRow {
                            mode,
                            bypass_since: bypass_until
                                .and(r.get::<_, Option<String>>(4).ok().flatten())
                                .and_then(|v| v.parse().ok()),
                            bypass_until,
                            profile_id: r.get(3)?,
                            overnight_until: meta(5).and_then(|v| v.parse().ok()),
                            overnight_since: meta(6).and_then(|v| v.parse().ok()),
                            overnight_return: meta(7).as_deref().and_then(AutopilotMode::parse),
                        },
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Writes a mode row. When a bypass started is kept beside it in `meta`, so that the table (and the account
    /// snapshots made of it by other versions) keeps its columns. In the same transaction, every permission a bypass
    /// granted whose bypass is no longer in force is taken back; those are returned.
    pub fn ap_set_mode_row(&self, scope: &str, row: &ModeRow, now: i64) -> Result<Vec<StoredGrant>, CoreError> {
        let mut guard = self.lock()?;
        let conn = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let put = |key: String, value: Option<String>| -> Result<usize, rusqlite::Error> {
            match value {
                Some(v) => conn.execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
                    params![key, v],
                ),
                None => conn.execute("DELETE FROM meta WHERE key = ?1", params![key]),
            }
        };
        put(
            format!("{BYPASS_SINCE_KEY}{scope}"),
            row.bypass_since.filter(|_| row.bypass_until.is_some()).map(|t| t.to_string()),
        )?;
        let overnight = row.mode == Some(AutopilotMode::Overnight);
        put(format!("{OVERNIGHT_UNTIL_KEY}{scope}"), row.overnight_until.filter(|_| overnight).map(|t| t.to_string()))?;
        put(format!("{OVERNIGHT_SINCE_KEY}{scope}"), row.overnight_since.filter(|_| overnight).map(|t| t.to_string()))?;
        put(
            format!("{OVERNIGHT_RETURN_KEY}{scope}"),
            row.overnight_return.filter(|_| overnight).map(|m| m.as_str().to_owned()),
        )?;
        if row.mode.is_none() && row.bypass_until.is_none() && row.profile_id.is_none() {
            conn.execute("DELETE FROM autopilot_settings WHERE scope = ?1", params![scope])?;
        } else {
            conn.execute(
                "INSERT INTO autopilot_settings (scope, mode, bypass_until, profile_id, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (scope) DO UPDATE SET mode = ?2, bypass_until = ?3, profile_id = ?4, updated_at = ?5",
                params![scope, row.mode.map(AutopilotMode::as_str), row.bypass_until, row.profile_id, now],
            )?;
        }
        let revoked = revoke_ended_bypass_grants(&conn, now)?;
        conn.commit()?;
        Ok(revoked)
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

    /// Every evaluation made after `since`, oldest first (those that cannot be decrypted are skipped).
    pub fn ap_suggestions_since(&self, since: i64) -> Result<Vec<Suggestion>, CoreError> {
        let rows: Vec<(String, Vec<u8>)> = {
            let conn = self.lock()?;
            let mut stmt = conn
                .prepare("SELECT request_id, data FROM autopilot_suggestions WHERE at > ?1 ORDER BY at, request_id")?;
            stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows
            .into_iter()
            .filter_map(|(id, sealed)| {
                let plain = self.unseal(&suggestion_aad(&id), &sealed).ok()?;
                serde_json::from_slice(&plain).ok()
            })
            .collect())
    }

    /// Forgets the evaluation of a request, so that the next pass looks at it again.
    pub fn ap_forget_suggestion(&self, request_id: &str) -> Result<(), CoreError> {
        self.lock()?.execute("DELETE FROM autopilot_suggestions WHERE request_id = ?1", params![request_id])?;
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
    /// The keyed hash of a target as one connection reaches it in one class (`service/action/class`): approving a
    /// read of something never makes a send or a write to it known. (Keys made before the class was part of it no
    /// longer match: those targets ask once again.)
    pub fn ap_target_key(&self, connection_id: &str, class_key: &str, target: &str) -> Result<String, CoreError> {
        let target = target.trim().to_lowercase();
        if target.is_empty() {
            return Ok(String::new());
        }
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &self.ap_salt()?);
        let tag = ring::hmac::sign(&key, format!("{connection_id}\0{class_key}\0{target}").as_bytes());
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

    /// When the server says the connection was made, as last read (kept: it never changes).
    pub fn ap_created_at(&self, connection_id: &str) -> Result<Option<i64>, CoreError> {
        let value: Option<String> = self
            .lock()?
            .query_row("SELECT value FROM meta WHERE key = ?1", params![format!("{CREATED_KEY}{connection_id}")], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(value.and_then(|v| v.parse().ok()))
    }

    pub fn ap_set_created_at(&self, connection_id: &str, at: i64) -> Result<(), CoreError> {
        self.lock()?.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![format!("{CREATED_KEY}{connection_id}"), at.to_string()],
        )?;
        Ok(())
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
