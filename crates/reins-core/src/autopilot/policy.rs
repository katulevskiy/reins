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
/// What GitHub said each login is (login → its account id, or none, and when), sealed.
const GITHUB_IDS_KEY: &str = "autopilot.github_ids";
/// The connected GitHub accounts' own ids (login → id), as their tokens' `GET /user` said.
const GITHUB_SELF_KEY: &str = "autopilot.github_self";
/// How long what GitHub said a login is holds before it is asked again.
pub const GITHUB_ID_SECS: i64 = 3600;
/// Logins looked up at most per request (the rest stay unresolved, so Outside).
const MAX_LOOKUPS: usize = 8;
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
    /// The zone the card preselects (a fact kept by the chat's handle before ids: the owner confirms it for the id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed: Option<Zone>,
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

/// What GitHub said one login is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LoginSeen {
    /// The account id, `None` when no account has that login.
    id: Option<String>,
    at: i64,
}

/// A request as the matrix reads it: its parties by the services' stable ids.
pub(crate) struct Identified {
    /// The request, with a Telegram chat as written replaced by the id of the chat a send would reach.
    pub request: RelayRequest,
    /// Login → GitHub account id, for the logins the request names that GitHub could tell.
    pub github_ids: BTreeMap<String, String>,
    /// The Telegram chat id a send was resolved to (what an automatic approval sends to).
    pub chat: Option<String>,
}

/// `request` as an automatic approval runs it: a Telegram send goes to the chat id it was evaluated for, so a name
/// that resolves to another chat by then can't redirect it.
pub(crate) fn pin_chat(request: &mut RelayRequest, chat: Option<&str>) {
    if let (ToolCall::Connector(call), Some(id)) = (&mut request.call, chat)
        && call.service == reins_proto::connector::TELEGRAM
        && call.op == "send"
    {
        call.args.insert("chat".to_owned(), serde_json::Value::String(id.to_owned()));
    }
}

/// A GitHub owner fact kept by its login (before ids), on github.com.
fn legacy_github_login(key: &str) -> bool {
    !key.is_empty() && !key.starts_with('#') && !key.contains('/') && !key.contains('.')
}

impl Engine {
    /// Before the matrix reads a request: who its parties are, by id, through the services that connect them
    /// (Telegram resolves a chat exactly as a send would; GitHub says which account a login is now). What can't be
    /// told stays as written, so Outside. Also moves facts kept by a handle or a login onto ids as they're learned.
    pub(crate) async fn ap_identify(&self, request: &RelayRequest, now: i64) -> Identified {
        let telegram = self.connectors.get(reins_proto::connector::TELEGRAM);
        let github = self.connectors.get(reins_proto::connector::GITHUB);
        self.ap_identify_with(request, now, telegram.as_deref(), github.as_deref()).await
    }

    pub(crate) async fn ap_identify_with(
        &self,
        request: &RelayRequest,
        now: i64,
        telegram: Option<&dyn crate::connector::Connector>,
        github: Option<&dyn crate::connector::Connector>,
    ) -> Identified {
        let mut out = Identified {
            request: request.clone(),
            github_ids: BTreeMap::new(),
            chat: None,
        };
        if let (Some(tg), ToolCall::Connector(call)) = (telegram, &mut out.request.call)
            && call.service == reins_proto::connector::TELEGRAM
            && call.op == "send"
            && let Some(written) = call.str_arg("chat").map(str::to_owned)
            && let Some(account) = request
                .account
                .clone()
                .or_else(|| self.accounts_of(reins_proto::connector::TELEGRAM).into_iter().next())
        {
            match tg.party(&account, &written).await {
                Ok(Some(party)) => {
                    if let Err(e) = self.ap_learn_chat(&party, &request.connection_label, &request.id.0, now) {
                        log::warn!("Auto could not keep what Telegram said of a chat: {e}");
                    }
                    out.chat = Some(party.id.clone());
                    call.args.insert("chat".to_owned(), serde_json::Value::String(party.id));
                }
                Ok(None) => {}
                Err(e) => log::info!("Auto could not ask Telegram who a chat is: {e}"),
            }
        }
        if let Some(gh) = github
            && let Some(account) = request
                .account
                .clone()
                .filter(
                    |_| matches!(&request.call, ToolCall::Connector(c) if c.service == reins_proto::connector::GITHUB),
                )
                .or_else(|| self.accounts_of(reins_proto::connector::GITHUB).into_iter().next())
        {
            let mut logins: BTreeSet<String> = Self::ap_github_logins(&out.request);
            if let Ok(snap) = self.ap_snapshot() {
                logins.extend(snap.facts.owners.keys().filter(|k| legacy_github_login(k)).cloned());
            }
            out.github_ids = self.ap_github_ids(gh, &account, logins, now).await;
            if let Err(e) = self.ap_migrate_github_facts(&out.github_ids) {
                log::warn!("Auto could not move GitHub facts onto ids: {e}");
            }
        }
        out
    }

    /// The GitHub logins a request names (its repositories' owners).
    fn ap_github_logins(request: &RelayRequest) -> BTreeSet<String> {
        let Ok(Some(reading)) = request_analysis(request) else {
            return BTreeSet::new();
        };
        let resolved = graph::request_zone(
            &reading.analysis,
            &Facts::default(),
            &graph::Base {
                repo: reading.base.as_deref(),
                remotes: None,
                bare_host: reading.bare_host,
            },
        );
        let mut logins: BTreeSet<String> = resolved
            .parts
            .iter()
            .chain(reading.base.iter())
            .filter_map(|p| graph::parse_repo(p, reading.bare_host))
            .filter(|r| r.host == graph::GITHUB)
            .map(|r| r.owner)
            .collect();
        logins.extend(
            resolved
                .unknown
                .iter()
                .filter(|u| u.kind == FactKind::Owner && legacy_github_login(&u.key))
                .map(|u| u.key.clone()),
        );
        logins
    }

    /// Login → account id for `logins`, from what GitHub said within [`GITHUB_ID_SECS`], else asked now (at most
    /// [`MAX_LOOKUPS`]). A login GitHub can't tell about is left out.
    async fn ap_github_ids(
        &self,
        gh: &dyn crate::connector::Connector,
        account: &str,
        logins: BTreeSet<String>,
        now: i64,
    ) -> BTreeMap<String, String> {
        let mut seen: BTreeMap<String, LoginSeen> = self.ap_sealed(GITHUB_IDS_KEY).unwrap_or_default();
        let mut own: BTreeMap<String, String> = self.ap_sealed(GITHUB_SELF_KEY).unwrap_or_default();
        let (mut changed, mut asked) = (false, 0);
        let mut ids = BTreeMap::new();
        for login in logins {
            let fresh = seen.get(&login).filter(|s| now - s.at < GITHUB_ID_SECS).cloned();
            let found = match fresh {
                Some(s) => s.id,
                None if asked < MAX_LOOKUPS => {
                    asked += 1;
                    match gh.party(account, &login).await {
                        Ok(party) => {
                            let id = party.as_ref().map(|p| p.id.clone());
                            if let Some(p) = party.filter(|p| p.own) {
                                own.insert(login.clone(), p.id);
                            }
                            seen.insert(
                                login.clone(),
                                LoginSeen {
                                    id: id.clone(),
                                    at: now,
                                },
                            );
                            changed = true;
                            id
                        }
                        Err(e) => {
                            log::info!("Auto could not ask GitHub who {login} is: {e}");
                            None
                        }
                    }
                }
                None => None,
            };
            if let Some(id) = found {
                ids.insert(login, id);
            }
        }
        if changed {
            seen.retain(|_, s| now - s.at < 7 * GITHUB_ID_SECS);
            if let Err(e) =
                self.ap_put_sealed(GITHUB_IDS_KEY, &seen).and_then(|()| self.ap_put_sealed(GITHUB_SELF_KEY, &own))
            {
                log::warn!("Auto could not keep what GitHub said: {e}");
            }
        }
        ids
    }

    /// The account id GitHub gives `login` now, through the first connected GitHub account.
    async fn ap_github_ids_now(&self, login: &str, now: i64) -> Option<String> {
        let gh = self.connectors.get(reins_proto::connector::GITHUB)?;
        let account = self.accounts_of(reins_proto::connector::GITHUB).into_iter().next()?;
        let logins = BTreeSet::from([login.to_owned()]);
        self.ap_github_ids(gh.as_ref(), &account, logins, now).await.remove(login)
    }

    /// GitHub owner facts kept by login move onto the account's id once GitHub said which account the login is (the
    /// login stays as its label). Nothing changes zone.
    fn ap_migrate_github_facts(&self, ids: &BTreeMap<String, String>) -> Result<(), CoreError> {
        let mut snap = self.ap_snapshot()?;
        let legacy: Vec<(String, Zone)> =
            snap.facts.owners.iter().filter(|(k, _)| legacy_github_login(k)).map(|(k, z)| (k.clone(), *z)).collect();
        let mut changed = false;
        for (login, zone) in legacy {
            let Some(id) = ids.get(&login) else {
                continue;
            };
            snap.facts.owners.remove(&login);
            if !snap.facts.owners.contains_key(&graph::github_owner_key(id)) {
                snap.facts.set_github_owner(id, &login, zone);
            }
            changed = true;
        }
        if changed {
            self.ap_put_snapshot(&snap)?;
        }
        Ok(())
    }

    /// What Telegram said of a chat a request named: the owner's own chat (Saved Messages) is Mine by its id, and the
    /// id gets its label. A fact kept by the `@handle` the request named (before ids) is never moved silently: when
    /// Telegram resolved that very handle, a question for the id preselects the handle's zone, and the owner confirms
    /// it. Nothing else (a title, an address, a number) is ever carried over.
    fn ap_learn_chat(
        &self,
        party: &crate::connector::Party,
        connection_label: &str,
        request_id: &str,
        now: i64,
    ) -> Result<(), CoreError> {
        let Some(id) = graph::key(&party.id) else {
            return Ok(());
        };
        let mut snap = self.ap_snapshot()?;
        let before = snap.facts.clone();
        if party.own && snap.facts.get(FactKind::Chat, &id).is_none() {
            snap.facts.set(FactKind::Chat, &id, Zone::Mine);
        }
        snap.facts.set_label(FactKind::Chat, &id, &party.label);
        let old = party
            .handle
            .as_deref()
            .filter(|h| h.starts_with('@') && snap.facts.get(FactKind::Chat, &id).is_none())
            .and_then(|h| snap.facts.get(FactKind::Chat, h).or_else(|| snap.facts.get(FactKind::Person, h)));
        if snap.facts != before {
            self.ap_put_snapshot(&snap)?;
        }
        if let Some(zone) = old {
            self.ap_propose_question(FactKind::Chat, &id, zone, connection_label, request_id, now)?;
        }
        Ok(())
    }

    /// A question for `kind`/`key` that preselects `zone` (once; one answered or dismissed isn't asked again).
    fn ap_propose_question(
        &self,
        kind: FactKind,
        key: &str,
        zone: Zone,
        connection_label: &str,
        request_id: &str,
        now: i64,
    ) -> Result<(), CoreError> {
        let mut qs = self.ap_questions()?;
        if qs.iter().any(|q| q.kind == kind && q.key == key) {
            return Ok(());
        }
        qs.push(StoredQuestion {
            id: format!("{}:{key}", service_of(kind, key)),
            kind,
            key: key.to_owned(),
            service: service_of(kind, key).to_owned(),
            connection_label: connection_label.to_owned(),
            request_id: Some(request_id.to_owned()),
            at: now,
            closed: false,
            proposed: Some(zone),
        });
        self.ap_put_sealed(QUESTIONS_KEY, &qs)
    }

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
        self.ap_evaluate_identified(request, &BTreeMap::new(), seen, now, overnight)
    }

    /// [`Engine::ap_evaluate`] with what GitHub said each login the request names is ([`Engine::ap_identify`]).
    pub(crate) fn ap_evaluate_identified(
        &self,
        request: &RelayRequest,
        github_ids: &BTreeMap<String, String>,
        seen: bool,
        now: i64,
        overnight: bool,
    ) -> Result<Option<(MatrixNote, Vec<Unknown>)>, CoreError> {
        let Some(reading) = request_analysis(request).map_err(CoreError::invalid)? else {
            return Ok(None);
        };
        let analysis = reading.analysis;
        let mut snap = self.ap_snapshot()?;
        snap.facts.github_ids.clone_from(github_ids);
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
        let input = super::preview::PreviewInput {
            analysis: analysis.clone(),
            resolved: resolved.clone(),
            connection: request.connection_id.0.clone(),
            seen_resource: seen,
        };
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
                chat: None,
                input: Some(Box::new(input)),
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
        let own_ids: BTreeMap<String, String> = self.ap_sealed(GITHUB_SELF_KEY).unwrap_or_default();
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
                reins_proto::connector::GITHUB => {
                    // By the account's id once GitHub said it (`GET /user`); by its login until then (moved onto the
                    // id when GitHub is next asked).
                    match own_ids.get(&a.account.to_ascii_lowercase()) {
                        Some(id) => {
                            let k = graph::github_owner_key(id);
                            if !snap.facts.owners.contains_key(&k) {
                                snap.facts.set_github_owner(id, &a.account, Zone::Mine);
                            }
                            if snap.facts.owners.get(&a.account.to_ascii_lowercase()) == Some(&Zone::Mine) {
                                snap.facts.remove(FactKind::Owner, &a.account);
                            }
                        }
                        None if snap.facts.get(FactKind::Owner, &a.account).is_none() => {
                            snap.facts.set(FactKind::Owner, &a.account, Zone::Mine);
                        }
                        None => {}
                    }
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
        let label = |kind: FactKind, key: &str| snap.facts.label(kind, key).unwrap_or(key).to_owned();
        let count = |kind: FactKind, key: &str| counts.get(&(kind, label(kind, key))).copied().unwrap_or(0);
        let item = |kind: FactKind, key: &str, zone: Zone, confirmed: bool| WorldItem {
            kind: kind.into(),
            key: key.to_owned(),
            label: label(kind, key),
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
        // A GitHub login is kept by the account it names now (its id), the login only its label; by the login when
        // GitHub can't be asked (it moves onto the id later).
        let github_id = if kind == FactKind::Owner && legacy_github_login(&k) {
            self.ap_github_ids_now(&k, unix_now()).await
        } else {
            None
        };
        // Read again after the check: another change may have landed meanwhile.
        let mut snap = self.ap_snapshot()?;
        if kind == FactKind::Person {
            snap.facts.own_addresses.remove(&k);
        }
        if let Some(id) = github_id {
            snap.facts.owners.remove(&k);
            snap.facts.set_github_owner(&id, &k, zone);
        } else if !snap.facts.set(kind, &k, zone) {
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
                proposed: None,
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
                // What the card names it (a chat's handle or title; the id stays in the store, and the answer goes to
                // it by the question's id).
                key: snap.facts.label(q.kind, &q.key).map_or(q.key.clone(), str::to_owned),
                service: q.service,
                connection_label: q.connection_label,
                request_id: q.request_id,
                at: q.at,
                proposed: q.proposed.map_or(ZoneChoice::Outside, Into::into),
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
        snap.rules.extend(exception_rules(request_id, &note, now));
        // Expired pauses go.
        snap.rules.retain(|r| r.not_after.is_none_or(|t| t > now));
        self.ap_put_snapshot(&snap)
    }
}

/// The rules "Ask me next time" on `request_id` writes (its preview applies the same).
pub(crate) fn exception_rules(request_id: &str, note: &MatrixNote, now: i64) -> Vec<Rule> {
        let mut out = Vec::new();
        let kinds: BTreeSet<EffectKind> = note.kinds.iter().copied().collect();
        let kinds: Vec<EffectKind> = kinds.into_iter().collect();
        let explain = "You asked to be asked".to_owned();
        if let Some(resource) = &note.resource {
            out.push(Rule {
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
        out.push(Rule {
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
        out
}
