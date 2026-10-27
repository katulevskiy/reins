//! Decisions v2 on the phone: the policy snapshot (the matrix, the rules and the context graph's facts), the setup
//! that guides the owner when they turn on Auto (`docs/design/decisions-v2/01-decision-engine.md` §9.1), the
//! "Auto needs information" questions (§3) and "Ask me next time" (§7).
//!
//! Facts are only what the owner confirmed or what Reins verified itself: the accounts connected to it, and the
//! people Reins itself sent mail to (as proposals, until confirmed). A proposal decides nothing: only [`Facts`] do, and
//! anything not in them is Outside. Everything is sealed with the data key; the server never sees it.
//!
//! The matrix runs in shadow until [`MATRIX_DECIDES_DEFAULT`] (or the stored switch) says it decides.

use std::collections::{BTreeMap, BTreeSet};

use reins_policy::context::{self as graph, FactKind, Facts, PUBLIC_MAIL_DOMAINS, Unknown, Zone};
use reins_policy::effects::{self, Analysis};
use reins_policy::effects::{EffectKind, Level};
use reins_policy::matrix::{self, Cell, Input, Issuer, Outcome, Rule, Snapshot, Then, When};
use reins_proto::gmail::ToolCall;
use reins_proto::relay::RelayRequest;
use serde::{Deserialize, Serialize};

use super::store::MatrixNote;
use super::types::{
    AutoQuestion, AutoSetup, AutopilotMode, CellChoice, FactChoice, MatrixRowView, MatrixView, Verdict, WorldItem,
    ZoneChoice,
};
use crate::engine::Engine;
use crate::store::unix_now;
use crate::{CoreError, OwnerCheck};

/// Whether the matrix decides by default (in Auto, once the owner finished Auto's setup). The stored switch
/// (`set_matrix_decides`) turns it off: the matrix then runs in shadow, and what it would have decided is recorded on
/// every entry.
pub const MATRIX_DECIDES_DEFAULT: bool = true;

const POLICY_KEY: &str = "autopilot.policy";
const PROPOSALS_KEY: &str = "autopilot.world_proposals";
const QUESTIONS_KEY: &str = "autopilot.questions";
const SETUP_KEY: &str = "autopilot.setup_done";
const DECIDES_KEY: &str = "autopilot.matrix_decides";
/// Open questions kept at most.
pub const MAX_QUESTIONS: usize = 50;
/// "Ask me next time" pauses Auto for that zone and kind of effect this long (01 §7).
pub const EXCEPTION_PAUSE_SECS: i64 = 86_400;
/// Activity entries read for the setup's counts and proposals.
const HISTORY: u32 = 2_000;
const COUNT_WINDOW_SECS: i64 = 30 * 86_400;
const SENT_WINDOW_SECS: i64 = 90 * 86_400;
/// People Reins sent to, proposed as Friends, at most.
const MAX_PEOPLE: usize = 20;

/// Requests that touched each thing lately.
type Counts = BTreeMap<(FactKind, String), u32>;

/// A proposed zone, waiting for the owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Proposal {
    pub kind: FactKind,
    pub key: String,
    pub zone: Zone,
}

/// A stored question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredQuestion {
    pub id: String,
    pub kind: FactKind,
    pub key: String,
    pub service: String,
    pub connection_label: String,
    #[serde(default)]
    pub request_id: Option<String>,
    pub at: i64,
    /// Answered or dismissed: never asked again.
    #[serde(default)]
    pub closed: bool,
}

/// The section a fact is listed in.
fn service_of(kind: FactKind, key: &str) -> &'static str {
    match kind {
        FactKind::Owner | FactKind::Repo => "github",
        FactKind::Domain => "email",
        FactKind::Person if key.contains('@') => "email",
        FactKind::Person | FactKind::Chat => "messages",
        FactKind::Host => "computers",
    }
}

fn level_of(level: u8) -> Result<Level, CoreError> {
    Ok(match level {
        0 => Level::L0,
        1 => Level::L1,
        2 => Level::L2,
        3 => Level::L3,
        _ => return Err(CoreError::invalid("a level is 0 to 3")),
    })
}

async fn confirm(owner: &dyn OwnerCheck, reason: &str) -> Result<(), CoreError> {
    match owner.confirm(reason.to_owned()).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(CoreError::invalid("the owner did not confirm")),
        Err(e) => Err(CoreError::invalid(format!("the owner check failed: {e}"))),
    }
}

/// The zone a fact has now, Outside when none (what a change is compared with).
fn current(facts: &Facts, kind: FactKind, key: &str) -> Zone {
    if kind == FactKind::Person && graph::key(key).is_some_and(|k| facts.own_addresses.contains(&k)) {
        return Zone::Mine;
    }
    facts.get(kind, key).unwrap_or(Zone::Outside)
}

/// What the phone reads of a request: its analysis, the repository it acts on, and the host a bare `owner/name`
/// in it means (the service's).
#[derive(Clone, Debug)]
pub struct Reading {
    pub analysis: Analysis,
    pub base: Option<String>,
    pub bare_host: Option<&'static str>,
}

/// The host a git service's bare `owner/name` is on.
fn service_host(service: &str) -> Option<&'static str> {
    use reins_proto::connector::{BITBUCKET, CODEBERG, GITHUB, GITLAB};
    match service {
        GITHUB => Some(graph::GITHUB),
        GITLAB => Some("gitlab.com"),
        CODEBERG => Some("codeberg.org"),
        BITBUCKET => Some("bitbucket.org"),
        _ => None,
    }
}

/// What a request does, as the consequence engine reads it on the phone. `Ok(None)` for what the phone doesn't
/// evaluate: a hook's question from a computer (the computer evaluates those, where the facts are: 08 R1) and an
/// upload. `Err` for a request it should read and can't (a push without a readable summary, a tool without a spec):
/// where the matrix decides, that waits for the owner.
pub fn request_analysis(request: &RelayRequest) -> Result<Option<Reading>, String> {
    use reins_proto::connector::DESKTOP;
    use reins_proto::desktop::{GIT_FETCH_OP, GIT_PUSH_OP, GIT_TAG_PUSH_OP};
    let plain = |analysis: Analysis| {
        Ok(Some(Reading {
            analysis,
            base: None,
            bare_host: None,
        }))
    };
    match &request.call {
        ToolCall::GmailSearch {
            ..
        } => plain(effects::analyze_tool("gmail_search", &serde_json::Value::Null)),
        ToolCall::GmailRead {
            ..
        } => plain(effects::analyze_tool("gmail_read", &serde_json::Value::Null)),
        ToolCall::GmailSend {
            email,
        } => plain(effects::analyze_tool("gmail_send", &serde_json::json!({ "to": email.to, "cc": email.cc }))),
        ToolCall::RequestGrant {
            ..
        } => plain(effects::analyze_tool("reins_request_access", &serde_json::Value::Null)),
        ToolCall::ListAccounts {
            ..
        } => plain(effects::analyze_tool("reins_list_accounts", &serde_json::Value::Null)),
        ToolCall::Mcp(m) => plain(effects::analyze_mcp(&m.server, &m.tool, None)),
        ToolCall::Connector(call) => {
            let repo = call.str_arg("repo").map(str::to_owned);
            let bare_host = service_host(&call.service);
            match call.op.as_str() {
                GIT_PUSH_OP | GIT_TAG_PUSH_OP => {
                    let summary =
                        crate::connector::git::summary_arg(call).map_err(|_| "a push it couldn't read".to_owned())?;
                    let repo = repo.unwrap_or_default();
                    Ok(Some(Reading {
                        analysis: effects::analyze_push(&repo, &summary),
                        base: Some(repo),
                        bare_host,
                    }))
                }
                GIT_FETCH_OP => {
                    let repo = repo.unwrap_or_default();
                    Ok(Some(Reading {
                        analysis: effects::analyze_git_read(&repo),
                        base: Some(repo),
                        bare_host,
                    }))
                }
                _ if call.service == DESKTOP => Ok(None),
                _ => {
                    let spec = reins_proto::connector::specs()
                        .iter()
                        .find(|s| s.service == call.service && s.op == call.op)
                        .ok_or_else(|| "a tool Auto doesn't know".to_owned())?;
                    let args = serde_json::Value::Object(call.args.clone());
                    Ok(Some(Reading {
                        base: effects::tool_repo(&args),
                        analysis: effects::analyze_tool(spec.tool, &args),
                        bare_host,
                    }))
                }
            }
        }
        // An upload is weighed as a file (`crate::blob`), not here.
        ToolCall::RequestUpload {
            ..
        } => Ok(None),
    }
}

/// The view of a kept note.
#[must_use]
pub fn matrix_view(n: &MatrixNote) -> MatrixView {
    MatrixView {
        outcome: match n.outcome {
            Outcome::Auto => Verdict::Approve,
            Outcome::Ask {
                ..
            }
            | Outcome::Park {
                ..
            } => Verdict::Ask,
            Outcome::Deny => Verdict::Deny,
        },
        hard: n.outcome.hard(),
        zone: n.zone.into(),
        level: n.level as u8,
        line: n.line.clone(),
        shadow: n.shadow,
    }
}

impl Engine {
    /// What the matrix says of a request, with what it named that Auto doesn't know. `seen`: the owner approved its
    /// target before. A request is evaluated against this core's own snapshot: each profile (Personal, or an org's
    /// Managed one) runs its own core on its own store (`crate::profiles`), and a connection pairs with exactly one, so
    /// no request is evaluated with another profile's facts or rules (08 R6).
    pub(crate) fn ap_evaluate(
        &self,
        request: &RelayRequest,
        seen: bool,
        now: i64,
        overnight: bool,
    ) -> Result<Option<(MatrixNote, Vec<Unknown>)>, CoreError> {
        let Some(reading) = request_analysis(request).map_err(CoreError::invalid)? else {
            return Ok(None);
        };
        let analysis = reading.analysis;
        let mut snap = self.ap_snapshot()?;
        // The live tasks are part of what the request is evaluated against (and of the snapshot's hash).
        self.ap_tasks_into(&mut snap, now);
        let resolved = graph::request_zone(
            &analysis,
            &snap.facts,
            &graph::Base {
                repo: reading.base.as_deref(),
                remotes: None,
                bare_host: reading.bare_host,
            },
        );
        let e = matrix::evaluate(
            &Input {
                analysis: &analysis,
                resolved: &resolved,
                connection: &request.connection_id.0,
                seen_resource: seen,
                now,
                overnight,
                // The phone plans no safety nets for its own requests (P3's are the computer's).
                netted: false,
            },
            &snap,
        );
        let by_rule = e.why.iter().any(|w| matches!(w, matrix::Why::Rule(_)));
        let mut kinds: Vec<EffectKind> = analysis.effects.iter().map(effects::Effect::kind).collect();
        kinds.sort();
        kinds.dedup();
        Ok(Some((
            MatrixNote {
                outcome: e.outcome,
                zone: e.zone,
                level: e.level,
                line: e.line(),
                snapshot: e.snapshot,
                shadow: !self.ap_matrix_deciding(),
                kinds,
                resource: resolved.resource,
                by_rule,
                by_task: e.why.iter().find_map(|w| match w {
                    matrix::Why::Task(id) => Some(id.clone()),
                    _ => None,
                }),
            },
            resolved.unknown,
        )))
    }

    // ---- storage -----------------------------------------------------------------------------------------------------

    fn ap_sealed<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> Result<T, CoreError> {
        match self.store.meta_get_sealed(key)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| CoreError::storage("corrupt Auto policy")),
            None => Ok(T::default()),
        }
    }

    fn ap_put_sealed<T: Serialize>(&self, key: &str, value: &T) -> Result<(), CoreError> {
        let json = serde_json::to_vec(value).map_err(|e| CoreError::storage(e.to_string()))?;
        self.store.meta_set_sealed(key, &json)
    }

    /// The policy snapshot decisions are evaluated against (the 01 §4 default until the owner changes it).
    pub(crate) fn ap_snapshot(&self) -> Result<Snapshot, CoreError> {
        self.ap_sealed(POLICY_KEY)
    }

    pub(crate) fn ap_put_snapshot(&self, snap: &Snapshot) -> Result<(), CoreError> {
        self.ap_put_sealed(POLICY_KEY, snap)
    }

    fn ap_proposals(&self) -> Result<Vec<Proposal>, CoreError> {
        self.ap_sealed(PROPOSALS_KEY)
    }

    fn ap_questions(&self) -> Result<Vec<StoredQuestion>, CoreError> {
        self.ap_sealed(QUESTIONS_KEY)
    }

    /// Whether the matrix decides now: it's switched on and the owner finished Auto's setup (until then Auto behaves as
    /// before, minus auto-deny: 07 §2). Else it runs in shadow.
    pub(crate) fn ap_matrix_deciding(&self) -> bool {
        self.ap_matrix_decides() && self.ap_setup_done()
    }

    /// Whether the matrix is switched on (the stored switch, else [`MATRIX_DECIDES_DEFAULT`]).
    pub(crate) fn ap_matrix_decides(&self) -> bool {
        match self.store.meta_get(DECIDES_KEY).ok().flatten().as_deref() {
            Some("1") => true,
            Some("0") => false,
            _ => MATRIX_DECIDES_DEFAULT,
        }
    }

    fn ap_setup_done(&self) -> bool {
        self.store.meta_get(SETUP_KEY).ok().flatten().as_deref() == Some("1")
    }

    // ---- facts and proposals ---------------------------------------------------------------------------------------

    /// Verified facts from the connected accounts, written at once (they never widen anything they shouldn't: the
    /// owner's own addresses and GitHub logins are theirs), and proposals from what Reins itself sent and saw, which
    /// wait for the owner.
    fn ap_refresh_world(&self, now: i64) -> Result<(Snapshot, Vec<Proposal>, Counts), CoreError> {
        let mut snap = self.ap_snapshot()?;
        let mut proposals = self.ap_proposals()?;
        let mut propose = |snap: &Snapshot, kind: FactKind, key: &str, zone: Zone| {
            let Some(k) = graph::key(key) else {
                return;
            };
            if snap.facts.get(kind, &k).is_some() || proposals.iter().any(|p| p.kind == kind && p.key == k) {
                return;
            }
            if kind == FactKind::Domain && PUBLIC_MAIL_DOMAINS.contains(&k.as_str()) {
                return;
            }
            proposals.push(Proposal {
                kind,
                key: k,
                zone,
            });
        };
        for a in self.store.accounts()? {
            match a.service.as_str() {
                crate::views::SERVICE_GMAIL => {
                    // An address the owner gave a zone of its own stays as they set it.
                    if let Some(k) = graph::key(&a.account)
                        && snap.facts.get(FactKind::Person, &k).is_none()
                    {
                        snap.facts.own_addresses.insert(k.clone());
                        if let Some((_, domain)) = k.rsplit_once('@') {
                            propose(&snap, FactKind::Domain, domain, Zone::Work);
                        }
                    }
                }
                reins_proto::connector::GITHUB if snap.facts.get(FactKind::Owner, &a.account).is_none() => {
                    snap.facts.set(FactKind::Owner, &a.account, Zone::Mine);
                }
                _ => {}
            }
        }
        // What Reins sent and the repositories requests touched: counts, and proposals.
        let mut counts: Counts = BTreeMap::new();
        let mut sent: BTreeMap<String, u32> = BTreeMap::new();
        for r in self.store.activity(HISTORY)? {
            let recent = r.at > now - COUNT_WINDOW_SECS;
            // Only mail the owner approved themselves: what a mode sent on its own (Bypass, Auto) can't make
            // someone a proposed Friend.
            if let Some(email) = &r.info.email
                && r.outcome == "sent"
                && r.info.decided_by.is_empty()
                && r.at > now - SENT_WINDOW_SECS
            {
                for to in email.to.iter().chain(&email.cc) {
                    if let Some(k) = graph::key(to) {
                        *sent.entry(k.clone()).or_default() += 1;
                        if recent {
                            *counts.entry((FactKind::Person, k)).or_default() += 1;
                        }
                    }
                }
            }
            if recent && r.service == reins_proto::connector::GITHUB {
                for t in &r.info.targets {
                    if let Some(r) = graph::parse_repo(t, Some(graph::GITHUB)) {
                        *counts.entry((FactKind::Owner, r.owner_key())).or_default() += 1;
                    }
                }
            }
        }
        let mut people: Vec<(String, u32)> =
            sent.into_iter().filter(|(k, _)| !snap.facts.own_addresses.contains(k)).collect();
        people.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (k, _) in people.into_iter().take(MAX_PEOPLE) {
            // Written to before (≥ 1 sent): Friends (01 §3), once the owner confirms.
            propose(&snap, FactKind::Person, &k, Zone::Friends);
        }
        for (kind, key) in counts.keys() {
            if *kind == FactKind::Owner {
                propose(&snap, FactKind::Owner, key, Zone::Outside);
            }
        }
        proposals.retain(|p| snap.facts.get(p.kind, &p.key).is_none());
        self.ap_put_snapshot(&snap)?;
        self.ap_put_sealed(PROPOSALS_KEY, &proposals)?;
        Ok((snap, proposals, counts))
    }

    // ---- the setup ---------------------------------------------------------------------------------------------------

    /// What the setup shows: the owner's world (facts and proposals) and the matrix with real names.
    pub fn auto_setup(&self) -> Result<AutoSetup, CoreError> {
        let now = unix_now();
        let (snap, proposals, counts) = self.ap_refresh_world(now)?;
        let count = |kind: FactKind, key: &str| counts.get(&(kind, key.to_owned())).copied().unwrap_or(0);
        let item = |kind: FactKind, key: &str, zone: Zone, confirmed: bool| WorldItem {
            kind: kind.into(),
            key: key.to_owned(),
            label: key.to_owned(),
            service: service_of(kind, key).to_owned(),
            zone: zone.into(),
            confirmed,
            count: count(kind, key),
        };
        let mut world = Vec::new();
        for a in &snap.facts.own_addresses {
            world.push(item(FactKind::Person, a, Zone::Mine, true));
        }
        for (kind, map) in [
            (FactKind::Owner, &snap.facts.owners),
            (FactKind::Repo, &snap.facts.repos),
            (FactKind::Domain, &snap.facts.domains),
            (FactKind::Person, &snap.facts.people),
            (FactKind::Host, &snap.facts.hosts),
            (FactKind::Chat, &snap.facts.chats),
        ] {
            for (k, z) in map {
                world.push(item(kind, k, *z, true));
            }
        }
        for p in &proposals {
            world.push(item(p.kind, &p.key, p.zone, false));
        }
        let names = |zone: Zone| -> Vec<String> {
            let mut v: Vec<(u32, String)> =
                world.iter().filter(|w| w.zone == zone.into()).map(|w| (w.count, w.label.clone())).collect();
            v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            v.into_iter().take(3).map(|(_, n)| n).collect()
        };
        let matrix = Zone::ALL
            .iter()
            .map(|z| MatrixRowView {
                zone: (*z).into(),
                cells: [Level::L0, Level::L1, Level::L2, Level::L3]
                    .iter()
                    .map(|l| CellChoice::from(snap.matrix.cell(*z, *l)))
                    .collect(),
                names: names(*z),
            })
            .collect();
        let questions = self.ap_questions()?.iter().filter(|q| !q.closed).count();
        Ok(AutoSetup {
            done: self.ap_setup_done(),
            matrix_decides: self.ap_matrix_decides(),
            world,
            matrix,
            questions: u32::try_from(questions).unwrap_or(u32::MAX),
        })
    }

    /// Sets the zone of one thing. Toward Mine (looser) asks the owner first; toward Outside needs nothing.
    pub async fn set_world_zone(
        &self,
        kind: FactChoice,
        key: &str,
        zone: ZoneChoice,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        let (kind, zone): (FactKind, Zone) = (kind.into(), zone.into());
        let k = graph::key(key).ok_or_else(|| CoreError::invalid("that is not a name Auto can keep"))?;
        let snap = self.ap_snapshot()?;
        if zone < current(&snap.facts, kind, &k) {
            confirm(owner, &format!("Treat {k} as {}", zone.label())).await?;
        }
        // Read again after the check: another change may have landed meanwhile.
        let mut snap = self.ap_snapshot()?;
        if kind == FactKind::Person {
            snap.facts.own_addresses.remove(&k);
        }
        if !snap.facts.set(kind, &k, zone) {
            return Err(CoreError::invalid("a public mail domain can't have a zone"));
        }
        self.ap_put_snapshot(&snap)?;
        let mut proposals = self.ap_proposals()?;
        proposals.retain(|p| !(p.kind == kind && p.key == k));
        self.ap_put_sealed(PROPOSALS_KEY, &proposals)?;
        self.ap_close_questions(kind, &k)
    }

    /// Sets one cell. Looser asks the owner first; stricter needs nothing.
    pub async fn set_matrix_cell(
        &self,
        zone: ZoneChoice,
        level: u8,
        cell: CellChoice,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        let (zone, level, cell): (Zone, Level, Cell) = (zone.into(), level_of(level)?, cell.into());
        if self.ap_snapshot()?.matrix.loosens(zone, level, cell) {
            confirm(owner, &format!("Let Auto do more in {}", zone.label())).await?;
        }
        let mut snap = self.ap_snapshot()?;
        snap.matrix.set(zone, level, cell);
        self.ap_put_snapshot(&snap)
    }

    /// Finishes the setup: confirms every proposal as shown, and turns on Auto. Asks the owner first.
    pub async fn finish_auto_setup(&self, owner: &dyn OwnerCheck) -> Result<(), CoreError> {
        confirm(owner, "Turn on Auto").await?;
        let mut snap = self.ap_snapshot()?;
        for p in self.ap_proposals()? {
            if snap.facts.get(p.kind, &p.key).is_none() {
                snap.facts.set(p.kind, &p.key, p.zone);
            }
        }
        self.ap_put_snapshot(&snap)?;
        self.ap_put_sealed(PROPOSALS_KEY, &Vec::<Proposal>::new())?;
        self.store.meta_set(SETUP_KEY, "1")?;
        self.set_autopilot_mode(None, Some(AutopilotMode::Auto), None).await
    }

    /// Turns the matrix's deciding on (asks the owner) or off (shadow; needs nothing).
    pub async fn set_matrix_decides(&self, on: bool, owner: &dyn OwnerCheck) -> Result<(), CoreError> {
        if on && !self.ap_matrix_decides() {
            confirm(owner, "Let the matrix decide").await?;
        }
        self.store.meta_set(
            DECIDES_KEY,
            if on {
                "1"
            } else {
                "0"
            },
        )
    }

    // ---- questions -------------------------------------------------------------------------------------------------

    /// Keeps a question for each thing a request named that Auto doesn't know (deduplicated; one answered or
    /// dismissed is never asked again). Never a notification: the cards are listed with the waiting items.
    pub(crate) fn ap_note_unknowns(
        &self,
        unknown: &[Unknown],
        connection_label: &str,
        request_id: &str,
        now: i64,
    ) -> Result<(), CoreError> {
        if unknown.is_empty() {
            return Ok(());
        }
        let mut qs = self.ap_questions()?;
        let mut changed = false;
        for u in unknown {
            if qs.iter().any(|q| q.kind == u.kind && q.key == u.key) {
                continue;
            }
            qs.push(StoredQuestion {
                id: format!("{}:{}", service_of(u.kind, &u.key), u.key),
                kind: u.kind,
                key: u.key.clone(),
                service: service_of(u.kind, &u.key).to_owned(),
                connection_label: connection_label.to_owned(),
                request_id: Some(request_id.to_owned()),
                at: now,
                closed: false,
            });
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        // The oldest open ones go first; closed ones are kept (so they aren't asked again) up to twice the cap.
        let open = qs.iter().filter(|q| !q.closed).count();
        if open > MAX_QUESTIONS {
            let mut drop = open - MAX_QUESTIONS;
            qs.retain(|q| {
                if !q.closed && drop > 0 {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
        while qs.len() > 2 * MAX_QUESTIONS {
            match qs.iter().position(|q| q.closed) {
                Some(i) => {
                    qs.remove(i);
                }
                None => break,
            }
        }
        self.ap_put_sealed(QUESTIONS_KEY, &qs)
    }

    fn ap_close_questions(&self, kind: FactKind, key: &str) -> Result<(), CoreError> {
        let mut qs = self.ap_questions()?;
        let mut changed = false;
        for q in qs.iter_mut().filter(|q| q.kind == kind && q.key == key && !q.closed) {
            q.closed = true;
            changed = true;
        }
        if changed {
            self.ap_put_sealed(QUESTIONS_KEY, &qs)?;
        }
        Ok(())
    }

    /// The open "Auto needs information" questions, newest first.
    pub fn auto_questions(&self) -> Result<Vec<AutoQuestion>, CoreError> {
        let snap = self.ap_snapshot()?;
        let mut qs: Vec<AutoQuestion> = self
            .ap_questions()?
            .into_iter()
            .filter(|q| !q.closed && snap.facts.get(q.kind, &q.key).is_none())
            .map(|q| AutoQuestion {
                id: q.id,
                kind: q.kind.into(),
                key: q.key,
                service: q.service,
                connection_label: q.connection_label,
                request_id: q.request_id,
                at: q.at,
                proposed: ZoneChoice::Outside,
            })
            .collect();
        qs.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
        Ok(qs)
    }

    /// Answers a question: the thing gets `zone` (anything but Outside asks the owner first).
    pub async fn answer_auto_question(
        &self,
        id: &str,
        zone: ZoneChoice,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        let q = self.ap_questions()?.into_iter().find(|q| q.id == id && !q.closed).ok_or(CoreError::NotFound)?;
        self.set_world_zone(q.kind.into(), &q.key, zone, owner).await
    }

    /// Dismisses a question without an answer: the thing stays Outside, and isn't asked about again.
    pub fn dismiss_auto_question(&self, id: &str) -> Result<(), CoreError> {
        let mut qs = self.ap_questions()?;
        let q = qs.iter_mut().find(|q| q.id == id).ok_or(CoreError::NotFound)?;
        q.closed = true;
        self.ap_put_sealed(QUESTIONS_KEY, &qs)
    }

    // ---- Ask me next time ------------------------------------------------------------------------------------------

    /// "Ask me next time" on something Auto did (01 §7): an exception for that zone, kind of effect and resource from
    /// now on, and a 24-hour pause for that zone and kind. Both are the owner's hard reviews (no permit covers them).
    /// Stricter only, so no owner check.
    pub fn ask_me_next_time(&self, request_id: &str) -> Result<(), CoreError> {
        let s = self.store.ap_suggestion(request_id)?.ok_or(CoreError::NotFound)?;
        let note = s.matrix.ok_or_else(|| CoreError::invalid("Auto has nothing to change for this entry"))?;
        let now = unix_now();
        let mut snap = self.ap_snapshot()?;
        let kinds: BTreeSet<EffectKind> = note.kinds.iter().copied().collect();
        let kinds: Vec<EffectKind> = kinds.into_iter().collect();
        let explain = "You asked to be asked".to_owned();
        if let Some(resource) = &note.resource {
            snap.rules.push(Rule {
                id: format!("exception-{request_id}"),
                issuer: Issuer::Owner,
                when: When {
                    zones: vec![note.zone],
                    effects: kinds.clone(),
                    resource: Some(resource.clone()),
                    ..When::default()
                },
                then: Then::Review,
                not_after: None,
                accepted: true,
                explain: explain.clone(),
            });
        }
        snap.rules.push(Rule {
            id: format!("pause-{request_id}"),
            issuer: Issuer::Owner,
            when: When {
                zones: vec![note.zone],
                effects: kinds,
                ..When::default()
            },
            then: Then::Review,
            not_after: Some(now + EXCEPTION_PAUSE_SECS),
            accepted: true,
            explain,
        });
        // Expired pauses go.
        snap.rules.retain(|r| r.not_after.is_none_or(|t| t > now));
        self.ap_put_snapshot(&snap)
    }
}
