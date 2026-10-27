//! The run loop: each pass refreshes the roster, claims jobs, verifies each one ([`crate::verify`]), runs it or mints
//! its lease, posts a signed receipt, and revokes what the server (or a lease's expiry) says to revoke.
//!
//! A request id never runs twice: `state_dir/done.log` records a job as started before anything is called upstream,
//! and its signed receipt when it ends. A job seen again is answered with its receipt; one that was started but never
//! finished (a crash mid-call) is failed, not retried.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reins_audit::verify_envelope;
use reins_proto::managed::{DecisionRecord, Envelope, Job, JobKind, Receipt, ReceiptStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::Client;
use crate::config::{ExecutorConfig, MAX_ROSTER_AGE_SECS, Pins};
use crate::connectors::{Connector, GithubApp, GoogleWorkspace};
use crate::leases::LeaseStore;
use crate::seal::seal;
use crate::verify::{HeldStep, Refusal, Verified, verify_job_with};

/// How often [`run`] polls.
pub const POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mark {
    Started,
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Line {
    request: String,
    mark: Mark,
    #[serde(default)]
    receipt: Option<Envelope>,
}

/// The idempotency log (`state_dir/done.log`, JSON lines, 0600).
pub struct DoneLog {
    path: PathBuf,
    seen: Mutex<HashMap<String, Line>>,
}

impl DoneLog {
    pub fn open(state_dir: &Path) -> Result<Self, String> {
        let path = state_dir.join("done.log");
        let mut seen = HashMap::new();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for l in text.lines().filter(|l| !l.trim().is_empty()) {
                    // A torn last line (a crash mid-write) is skipped; the marks before it stand.
                    if let Ok(line) = serde_json::from_str::<Line>(l) {
                        seen.insert(line.request.clone(), line);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        }
        Ok(Self {
            path,
            seen: Mutex::new(seen),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Line>> {
        self.seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn append(&self, line: Line) -> Result<(), String> {
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let err = |e: std::io::Error| format!("writing {}: {e}", self.path.display());
        let mut text = serde_json::to_string(&line).map_err(|e| e.to_string())?;
        text.push('\n');
        let mut f = opts.open(&self.path).map_err(err)?;
        f.write_all(text.as_bytes()).map_err(err)?;
        f.sync_data().map_err(err)?;
        self.lock().insert(line.request.clone(), line);
        Ok(())
    }

    /// Marks `request` as started, durably, before anything runs.
    pub fn start(&self, request: &str) -> Result<(), String> {
        self.append(Line {
            request: request.to_owned(),
            mark: Mark::Started,
            receipt: None,
        })
    }

    pub fn finish(&self, request: &str, receipt: &Envelope) -> Result<(), String> {
        self.append(Line {
            request: request.to_owned(),
            mark: Mark::Done,
            receipt: Some(receipt.clone()),
        })
    }

    /// `Some(Some(receipt))` when it ended, `Some(None)` when it started and never ended, `None` when never seen.
    #[must_use]
    pub fn state(&self, request: &str) -> Option<Option<Envelope>> {
        self.lock().get(request).map(|l| match l.mark {
            Mark::Done => l.receipt.clone(),
            Mark::Started => None,
        })
    }
}

/// Everything a pass needs.
pub struct Context {
    pub client: Client,
    pub pins: Pins,
    pub max_roster_age_secs: i64,
    pub github: Option<GithubApp>,
    pub google: Option<GoogleWorkspace>,
    pub leases: LeaseStore,
    pub done: DoneLog,
    policies: Mutex<HashMap<String, Envelope>>,
    /// Every issuer-signed rollout step seen, with when it was first seen: a step the server stops sending still
    /// counts (it can't take one back), across restarts too (`state_dir/rollouts.log`).
    rollouts: Mutex<Vec<HeldStep>>,
    rollouts_path: PathBuf,
}

impl Context {
    /// A context for `client`'s org, trusting `pins`, keeping its state in `state_dir`. Add connectors with
    /// [`Context::with_github`] and [`Context::with_google`].
    pub fn new(client: Client, pins: Pins, state_dir: &Path) -> Result<Self, String> {
        if pins.org != client.org() {
            return Err("the pins are for another organization".to_owned());
        }
        std::fs::create_dir_all(state_dir).map_err(|e| format!("creating {}: {e}", state_dir.display()))?;
        let rollouts_path = state_dir.join("rollouts.log");
        let mut held: Vec<HeldStep> = match std::fs::read_to_string(&rollouts_path) {
            Ok(text) => text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("reading {}: {e}", rollouts_path.display())),
        };
        // Only what the pinned issuer signed for this org is kept.
        held.retain(|h| {
            h.envelope.kind == reins_proto::managed::EnvelopeKind::Rollout
                && h.envelope.org == pins.org
                && verify_envelope(&h.envelope, std::slice::from_ref(&pins.issuer))
        });
        Ok(Self {
            leases: LeaseStore::open(state_dir)?,
            done: DoneLog::open(state_dir)?,
            client,
            pins,
            max_roster_age_secs: MAX_ROSTER_AGE_SECS,
            github: None,
            google: None,
            policies: Mutex::new(HashMap::new()),
            rollouts: Mutex::new(held),
            rollouts_path,
        })
    }

    #[must_use]
    pub fn with_github(mut self, app: GithubApp) -> Self {
        self.github = Some(app);
        self
    }

    #[must_use]
    pub fn with_google(mut self, gw: GoogleWorkspace) -> Self {
        self.google = Some(gw);
        self
    }

    /// From the configuration file: its key, pins, connectors and state directory.
    pub fn from_config(cfg: &ExecutorConfig) -> Result<Self, String> {
        let key = Arc::new(crate::client::load_key(&cfg.key_file)?);
        let client = Client::new(&cfg.server, &cfg.org, &cfg.executor_id, key)?;
        let mut ctx = Self::new(client, cfg.pins()?, &cfg.state_dir)?;
        ctx.max_roster_age_secs = cfg.max_roster_age_secs;
        if let Some(gh) = &cfg.github {
            let pem = gh.private_key.read()?;
            ctx = ctx.with_github(GithubApp::new(gh.app_id, gh.installation_id, &gh.owner, &pem, &gh.api_base)?);
        }
        if let Some(g) = &cfg.google {
            let key = g.service_account.read()?;
            ctx = ctx.with_google(GoogleWorkspace::from_service_account(&key, &g.api_base, g.token_uri.as_deref())?);
        }
        Ok(ctx)
    }

    fn connector(&self, tool: &str) -> Option<&dyn Connector> {
        if let Some(gh) = self.github.as_ref().filter(|c| c.handles(tool)) {
            return Some(gh);
        }
        self.google.as_ref().filter(|c| c.handles(tool)).map(|g| -> &dyn Connector { g })
    }

    /// The policy snapshot `hash`, cached once it verifies with the pinned issuer key.
    async fn policy(&self, hash: &str) -> Result<Envelope, String> {
        if let Some(env) = self.policies.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(hash) {
            return Ok(env.clone());
        }
        let env = self.client.policy(hash).await?;
        if verify_envelope(&env, std::slice::from_ref(&self.pins.issuer)) {
            self.policies
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(hash.to_owned(), env.clone());
        }
        Ok(env)
    }
}

/// Most rollout steps held (old ones go first).
const MAX_HELD_STEPS: usize = 2_000;

impl Context {
    /// Adds the issuer-signed rollout steps in `envs` not held yet, first seen `now`, and keeps them on disk.
    pub fn hold_rollouts(&self, envs: Vec<Envelope>, now: i64) -> Result<(), String> {
        let mut held = self.rollouts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut fresh = Vec::new();
        for env in envs {
            let signed = env.kind == reins_proto::managed::EnvelopeKind::Rollout
                && env.org == self.pins.org
                && verify_envelope(&env, std::slice::from_ref(&self.pins.issuer));
            if signed && !held.iter().any(|h| h.envelope == env) && !fresh.iter().any(|h: &HeldStep| h.envelope == env)
            {
                fresh.push(HeldStep {
                    envelope: env,
                    seen_at: now,
                });
            }
        }
        if !fresh.is_empty() {
            let mut opts = std::fs::OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let err = |e: std::io::Error| format!("writing {}: {e}", self.rollouts_path.display());
            let mut text = String::new();
            for h in &fresh {
                text.push_str(&serde_json::to_string(h).map_err(|e| e.to_string())?);
                text.push('\n');
            }
            let mut f = opts.open(&self.rollouts_path).map_err(err)?;
            f.write_all(text.as_bytes()).map_err(err)?;
            f.sync_data().map_err(err)?;
            held.extend(fresh);
        }
        let excess = held.len().saturating_sub(MAX_HELD_STEPS);
        held.drain(..excess);
        Ok(())
    }

    /// The rollout steps held.
    #[must_use]
    pub fn held_rollouts(&self) -> Vec<HeldStep> {
        self.rollouts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

/// What one pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunStats {
    pub claimed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub refused: usize,
    pub stale: usize,
    /// Jobs already handled (answered with their earlier receipt).
    pub repeated: usize,
    /// Leases revoked upstream.
    pub revoked: usize,
}

/// One pass: refresh the roster, claim, verify, act, receipt, then revocations.
pub async fn run_once(ctx: &Context) -> Result<RunStats, String> {
    let mut stats = RunStats::default();
    // Leases past their end first, before any job and without the server: an executor back from being offline revokes
    // the tokens that outlived their lease (GitHub keeps an installation token alive for an hour) before anything else.
    revoke_expired(ctx, &mut stats).await;
    let jobs = run_jobs(ctx, &mut stats).await;
    // Revocations run every pass, whatever happened to the jobs.
    let revoked = revocations(ctx, &mut stats).await;
    jobs.and(revoked).map(|()| stats)
}

/// [`run_once`] every [`POLL`], forever.
pub async fn run(ctx: &Context) {
    loop {
        match run_once(ctx).await {
            Ok(s) if s != RunStats::default() => log::info!("{s:?}"),
            Ok(_) => {}
            Err(e) => log::warn!("{e}"),
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn run_jobs(ctx: &Context, stats: &mut RunStats) -> Result<(), String> {
    let roster = ctx.client.roster().await?;
    // Refreshed with the roster, before any job: a decision is judged against every stage published so far.
    ctx.hold_rollouts(ctx.client.rollouts().await?, crate::now())?;
    let jobs = ctx.client.claim().await?;
    stats.claimed = jobs.len();
    for job in jobs {
        handle(ctx, &roster, &job, stats).await;
    }
    Ok(())
}

async fn handle(ctx: &Context, roster: &Envelope, job: &Job, stats: &mut RunStats) {
    let receipt = match ctx.done.state(&job.request) {
        Some(Some(receipt)) => {
            stats.repeated += 1;
            post(ctx, &receipt).await;
            return;
        }
        Some(None) => {
            let r = receipt(
                ctx,
                job,
                None,
                ReceiptStatus::Failed,
                Value::Null,
                "interrupted before it ended; not run again",
            );
            stats.failed += 1;
            r
        }
        None => decide_and_act(ctx, roster, job, stats).await,
    };
    let env = ctx.client.sign_receipt(&receipt);
    if let Err(e) = ctx.done.finish(&job.request, &env) {
        log::error!("recording {}: {e}", job.request);
    }
    post(ctx, &env).await;
}

async fn post(ctx: &Context, env: &Envelope) {
    if let Err(e) = ctx.client.receipt(env).await {
        log::warn!("posting a receipt: {e}");
    }
}

fn receipt(
    ctx: &Context,
    job: &Job,
    record: Option<&DecisionRecord>,
    status: ReceiptStatus,
    result: Value,
    reason: &str,
) -> Receipt {
    Receipt {
        org: ctx.pins.org.clone(),
        request: job.request.clone(),
        digest: record.map(|r| r.digest.clone()).unwrap_or_default(),
        executor: ctx.client.executor_id().to_owned(),
        status,
        result,
        reason: reason.to_owned(),
        at: crate::now(),
    }
}

async fn verify(ctx: &Context, roster: &Envelope, job: &Job) -> Result<Verified, Refusal> {
    // Unverified, only to know which snapshot to fetch: verify_job checks everything.
    let claimed: DecisionRecord = job.decision.parse().map_err(|_| Refusal::BadCountersig)?;
    let policy = ctx.policy(&claimed.snapshot).await.map_err(|_| Refusal::UnsignedPolicy)?;
    let current = ctx.policy(&job.current_policy).await.map_err(|_| Refusal::UnsignedPolicy)?;
    let held = ctx.held_rollouts();
    verify_job_with(job, &ctx.pins, &policy, &current, roster, &held, crate::now(), ctx.max_roster_age_secs)
}

async fn decide_and_act(ctx: &Context, roster: &Envelope, job: &Job, stats: &mut RunStats) -> Receipt {
    let verified = match verify(ctx, roster, job).await {
        Ok(v) => v,
        Err(refusal) => {
            let hint: Option<DecisionRecord> = job.decision.parse().ok();
            let outcome = if refusal.is_stale() {
                stats.stale += 1;
                ReceiptStatus::Stale
            } else {
                stats.refused += 1;
                ReceiptStatus::Refused
            };
            log::warn!("{}: {}", job.request, refusal.reason());
            return receipt(ctx, job, hint.as_ref(), outcome, Value::Null, &refusal.reason());
        }
    };
    let record = Some(&verified.record);
    if let Err(e) = ctx.done.start(&job.request) {
        stats.failed += 1;
        return receipt(ctx, job, record, ReceiptStatus::Failed, Value::Null, &e);
    }
    let outcome = match job.kind {
        JobKind::Execute => match ctx.connector(&job.tool) {
            Some(c) => c.run(&verified.member_email, &job.tool, &job.args).await,
            None => Err(format!("this executor has no connector for {}", job.tool)),
        },
        JobKind::Lease => mint(ctx, job).await,
    };
    match outcome {
        Ok(result) => {
            stats.succeeded += 1;
            receipt(ctx, job, record, ReceiptStatus::Succeeded, result, "")
        }
        Err(e) => {
            stats.failed += 1;
            receipt(ctx, job, record, ReceiptStatus::Failed, Value::Null, &e)
        }
    }
}

/// Mints the countersigned lease, records the token on disk, then seals it to the requester's box key.
async fn mint(ctx: &Context, job: &Job) -> Result<Value, String> {
    let lease = job.lease.as_ref().ok_or("the lease is missing")?;
    let gh = ctx.github.as_ref().ok_or("this executor has no GitHub App")?;
    let (token, token_expires) = gh.lease(&lease.repo, lease.access).await?;
    let kept = ctx.leases.hold(&lease.lease, &token, lease.expires_at);
    let sealed = kept.and_then(|()| seal(&lease.box_key, token.as_bytes()));
    match sealed {
        Ok(sealed) => Ok(json!({"sealed": sealed, "expires_at": lease.expires_at.min(token_expires)})),
        Err(e) => {
            // Never leave a token alive that nobody will revoke.
            if let Err(r) = gh.revoke(&token).await {
                log::error!("revoking an unissued token for {}: {r}", lease.lease);
            } else if let Err(r) = ctx.leases.revoked(&lease.lease, crate::now()) {
                log::error!("{r}");
            }
            Err(e)
        }
    }
}

/// Revokes every token of `lease` upstream; forgets them once all are revoked.
async fn revoke_lease(ctx: &Context, lease: &str) -> Result<usize, String> {
    let held = ctx.leases.tokens(lease);
    if !held.is_empty() {
        let gh = ctx.github.as_ref().ok_or("this executor has no GitHub App")?;
        for h in &held {
            gh.revoke(&h.token).await?;
        }
    }
    ctx.leases.revoked(lease, crate::now())?;
    Ok(held.len())
}

async fn revoke_expired(ctx: &Context, stats: &mut RunStats) {
    for lease in ctx.leases.expired(crate::now()) {
        match revoke_lease(ctx, &lease).await {
            Ok(n) => stats.revoked += n,
            Err(e) => log::warn!("revoking expired lease {lease}: {e}"),
        }
    }
}

async fn revocations(ctx: &Context, stats: &mut RunStats) -> Result<(), String> {
    for r in ctx.client.revocations().await? {
        if !ctx.leases.knows(&r.lease) {
            // Not one this executor issued: it has nothing to revoke, and must not say it did.
            log::debug!("revocation of unknown lease {}", r.lease);
            continue;
        }
        match revoke_lease(ctx, &r.lease).await {
            Ok(n) => {
                stats.revoked += n;
                ctx.client.revoked(&r.lease).await?;
            }
            Err(e) => log::warn!("revoking lease {} ({}): {e}", r.lease, r.reason),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use reins_proto::managed::EnvelopeKind;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::connectors::testing::test_pem;
    use crate::seal::Opener;
    use crate::testkit::*;

    struct World {
        server: MockServer,
        ctx: Context,
        executor_public: Vec<u8>,
        dir: tempfile::TempDir,
    }

    /// One mock that is both the Reins server (claiming `jobs` every time) and GitHub.
    async fn world(f: &Fixture, jobs: Vec<Job>) -> World {
        let server = MockServer::start().await;
        let ok = |v: Value| ResponseTemplate::new(200).set_body_json(v);
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/roster"))
            .respond_with(ok(serde_json::to_value(f.roster_env()).unwrap()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/rollouts"))
            .respond_with(ok(serde_json::json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/reins/org-exec/policy/{}", f.policy.hash())))
            .respond_with(ok(serde_json::to_value(f.policy_env(&f.policy)).unwrap()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/org-exec/claim"))
            .respond_with(ok(serde_json::to_value(jobs).unwrap()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/org-exec/receipt"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/app/installations/77/access_tokens"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"token": "ghs_t", "expires_at": "2099-01-01T00:00:00Z"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/installation/token"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        crate::connectors::github::tests::repo_mocks(&server).await;
        let key = Arc::new(Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap());
        let executor_public = key.public_key().as_ref().to_vec();
        let client = Client::new(&server.uri(), ORG, "ex-1", key).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::new(client, f.pins(), dir.path())
            .unwrap()
            .with_github(GithubApp::new(123, 77, "acme", &test_pem(), &server.uri()).unwrap());
        World {
            server,
            ctx,
            executor_public,
            dir,
        }
    }

    async fn no_revocations(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/revocations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(server)
            .await;
    }

    async fn hits(server: &MockServer, m: &str, p: &str) -> Vec<wiremock::Request> {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == m && r.url.path() == p)
            .collect()
    }

    /// The receipts posted, each verified with the executor's key.
    async fn receipts(w: &World) -> Vec<Receipt> {
        hits(&w.server, "POST", "/reins/org-exec/receipt")
            .await
            .iter()
            .map(|r| {
                let env: Envelope = serde_json::from_slice(&r.body).unwrap();
                assert_eq!(env.kind, EnvelopeKind::Receipt);
                let key = reins_audit::TrustedKey {
                    key_id: "ex-1".into(),
                    public: w.executor_public.clone().try_into().unwrap(),
                };
                assert!(verify_envelope(&env, &[key]), "signed by the executor");
                env.parse().unwrap()
            })
            .collect()
    }

    #[tokio::test]
    async fn held_rollout_steps_survive_a_restart() {
        let f = Fixture::new();
        let w = world(&f, Vec::new()).await;
        let steps = f.rollout_steps("ro_1", "S", "C", &[LEE], 100);
        let mut forged = Fixture::new();
        forged.issuer = ed25519(9);
        let fake = forged.rollout_steps("ro_1", "S", "C", &[LEE], 100);
        w.ctx.hold_rollouts(vec![steps[0].clone(), steps[1].clone(), fake[2].clone()], 500).unwrap();
        // Seen again later: still first seen at 500.
        w.ctx.hold_rollouts(vec![steps[1].clone()], 900).unwrap();
        let held = w.ctx.held_rollouts();
        assert_eq!(
            held.iter().map(|h| (h.envelope.clone(), h.seen_at)).collect::<Vec<_>>(),
            [(steps[0].clone(), 500), (steps[1].clone(), 500)]
        );
        // A restarted executor holds the same steps, even if the server stops sending them.
        let client =
            Client::new(&w.server.uri(), ORG, "ex-1", Arc::new(Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap()))
                .unwrap();
        let again = Context::new(client, f.pins(), w.dir.path()).unwrap();
        assert_eq!(again.held_rollouts(), held);
    }

    #[tokio::test]
    async fn a_request_id_never_runs_twice() {
        let f = Fixture::new();
        let w = world(&f, vec![f.auto_job()]).await;
        no_revocations(&w.server).await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/api/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 9, "html_url": "u"})))
            .expect(1)
            .mount(&w.server)
            .await;
        let first = run_once(&w.ctx).await.unwrap();
        assert_eq!((first.claimed, first.succeeded), (1, 1), "{first:?}");
        // The server offers the same job again: it is answered with the same receipt, not run.
        let second = run_once(&w.ctx).await.unwrap();
        assert_eq!((second.repeated, second.succeeded), (1, 0), "{second:?}");
        let r = receipts(&w).await;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], r[1]);
        assert_eq!(r[0].status, ReceiptStatus::Succeeded);
        assert_eq!(r[0].result, json!({"number": 9, "html_url": "u"}));
        assert_eq!(r[0].digest, f.auto_job().decision.parse::<DecisionRecord>().unwrap().digest);
        // Nor after a restart.
        let again = DoneLog::open(w.dir.path()).unwrap();
        assert!(matches!(again.state("or_1"), Some(Some(_))));
    }

    #[tokio::test]
    async fn a_job_interrupted_mid_call_is_failed_not_retried() {
        let f = Fixture::new();
        let w = world(&f, vec![f.auto_job()]).await;
        no_revocations(&w.server).await;
        w.ctx.done.start("or_1").unwrap();
        let stats = run_once(&w.ctx).await.unwrap();
        assert_eq!(stats.failed, 1);
        assert!(hits(&w.server, "POST", "/repos/acme/api/issues").await.is_empty());
        assert_eq!(receipts(&w).await[0].status, ReceiptStatus::Failed);
    }

    #[tokio::test]
    async fn a_refusal_is_receipted_and_nothing_is_called() {
        let f = Fixture::new();
        let mut tampered = f.auto_job();
        tampered.args["title"] = json!("Something else");
        let w = world(&f, vec![tampered]).await;
        no_revocations(&w.server).await;
        let stats = run_once(&w.ctx).await.unwrap();
        assert_eq!((stats.refused, stats.succeeded), (1, 0), "{stats:?}");
        // Nothing reached GitHub: no token, no call.
        let upstream: Vec<String> = w
            .server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.url.path().to_owned())
            .filter(|p| !p.starts_with("/reins/"))
            .collect();
        assert!(upstream.is_empty(), "{upstream:?}");
        let r = receipts(&w).await;
        assert_eq!(r[0].status, ReceiptStatus::Refused);
        assert!(r[0].reason.contains("digest"), "{}", r[0].reason);

        // Stale: the policy now active denies it. Receipted as stale, nothing called.
        let mut denying = f.policy.clone();
        denying.rules.push(org_rule("api-deny", "acme/api", reins_policy::matrix::Then::Deny, None));
        let mut stale = f.auto_job();
        stale.current_policy = denying.hash();
        let w = world(&f, vec![stale]).await;
        no_revocations(&w.server).await;
        Mock::given(method("GET"))
            .and(path(format!("/reins/org-exec/policy/{}", denying.hash())))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::to_value(f.policy_env(&denying)).unwrap()),
            )
            .mount(&w.server)
            .await;
        let stats = run_once(&w.ctx).await.unwrap();
        assert_eq!(stats.stale, 1, "{stats:?}");
        let r = receipts(&w).await;
        assert_eq!(r[0].status, ReceiptStatus::Stale);
        assert!(r[0].reason.contains("api-deny"), "{}", r[0].reason);
        assert!(hits(&w.server, "POST", "/app/installations/77/access_tokens").await.is_empty());
    }

    #[tokio::test]
    async fn a_lease_is_kept_before_it_is_sealed_and_revoked_when_the_server_says() {
        let f = Fixture::new();
        let (opener, box_key) = Opener::new().unwrap();
        let job = f.lease_job(&box_key);
        let w = world(&f, vec![job.clone()]).await;
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/revocations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .up_to_n_times(1)
            .mount(&w.server)
            .await;
        let stats = run_once(&w.ctx).await.unwrap();
        assert_eq!(stats.succeeded, 1, "{stats:?}");
        let r = &receipts(&w).await[0];
        let sealed = r.result["sealed"].as_str().unwrap();
        assert_eq!(opener.open(sealed).unwrap().as_slice(), b"ghs_t");
        assert_eq!(r.result["expires_at"], job.lease.unwrap().expires_at);
        assert_eq!(w.ctx.leases.tokens("ls_1")[0].token, "ghs_t");
        // Minted for exactly the countersigned repository's id (after a metadata-only lookup).
        let minted = &hits(&w.server, "POST", "/app/installations/77/access_tokens").await[1];
        assert_eq!(
            serde_json::from_slice::<Value>(&minted.body).unwrap(),
            json!({"repository_ids": [101], "permissions": {"contents": "read"}})
        );

        // Deprovisioned: the server lists the lease; the executor revokes the token upstream and says so.
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/revocations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"lease": "ls_1", "reason": "revoked"}, {"lease": "ls_other", "reason": "revoked"}
            ])))
            .mount(&w.server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/org-exec/revoked/ls_1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&w.server)
            .await;
        let stats = run_once(&w.ctx).await.unwrap();
        assert_eq!(stats.revoked, 1);
        // The lookup token was revoked at once; the lease's token now.
        let deleted = hits(&w.server, "DELETE", "/installation/token").await;
        assert_eq!(deleted.len(), 2);
        assert!(deleted.iter().all(|d| d.headers.get("authorization").unwrap() == "Bearer ghs_t"));
        assert!(w.ctx.leases.tokens("ls_1").is_empty());
        // A lease it never issued is not acknowledged.
        assert!(hits(&w.server, "POST", "/reins/org-exec/revoked/ls_other").await.is_empty());
    }
}
