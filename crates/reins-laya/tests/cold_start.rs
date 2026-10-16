//! Auto against the real Laya package, when `REINS_LAYA_PKG` names its directory (skipped otherwise): the
//! hand-labelled risk cases of `tools/laya/golden.json` (`risk.cases`, severity in context) judged as the phone judges
//! them, then decided by Auto's verdict ([`decide::verdict`]) at every stage of a class: day one, cold, warm by its
//! numbers, and unlocked by hand. The user's answers are the worst case for trust: the user approves the very request
//! every time. They become memory rows as the engine writes them (the shadow verdict, `risk_clear`), and the trust is
//! what [`gates::trust`] measures on them.
//!
//! Each case is rebuilt as the approval view the phone would show, from what the golden set keeps of it (the desktop
//! question, the push, the MCP call, the command secrets go to, the service and the action), and the engine's own
//! reading decides what it would: [`gates::request_floor`] and a push to the default branch (both asked before the
//! model) and [`risk::refine`]'s outcome (a tripwire or an uncertain reading is never approved). What the golden set does not keep (whether a catalog operation
//! is asked every time, the destructive mark an MCP server gives a tool) is left out, so a floor the phone would apply
//! may be missing here: the counts of what Auto approves are then the stricter test.
//!
//! What has to hold, at every stage: nothing labelled risky, severe or catastrophic is approved, and nothing on the
//! floor. Prints the table it measured.

use std::path::{Path, PathBuf};

use reins_core::McpCallView;
use reins_core::autopilot::decide::{self, Evidence, Probs};
use reins_core::autopilot::memory::{Embedding, MemoryRow, Source};
use reins_core::autopilot::{gates, risk, situation};
use reins_core::connector::desktop::read_ask;
use reins_core::types::{ApprovalKind, ApprovalView, AskView, GitFileView, GitPushView, GitRefView, SecretReleaseView};
use reins_core::{Preset, Verdict};
use reins_laya::Judge;
use serde_json::{Value, json};

fn package() -> Option<PathBuf> {
    let Some(dir) = std::env::var_os("REINS_LAYA_PKG").map(PathBuf::from) else {
        eprintln!("skipped: set REINS_LAYA_PKG to a Laya package directory to run this test");
        return None;
    };
    assert!(dir.join("model.onnx").exists(), "REINS_LAYA_PKG={} has no model.onnx", dir.display());
    Some(dir)
}

/// One golden case as Auto sees it.
struct Case {
    id: String,
    severity: usize,
    probs: Probs,
    escalate: f32,
    risk: Option<[f32; 4]>,
    /// Why the engine asks before the model (`gates::request_floor` of the rebuilt view).
    floor: Option<&'static str>,
    /// The engine's own reading: read with certainty, no tripwire (`risk::Outcome::Read`).
    certain: bool,
}

/// The approval head reads the request without the context line (only the risk reads it).
fn without_context(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with("context: ")).collect::<Vec<_>>().join("\n")
}

fn field<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines().find_map(|l| l.strip_prefix(&format!("{key}: "))).unwrap_or_default()
}

/// A push as `tools/laya/gen_risk.py` describes it (as `laya-context` reads it).
fn push_view(v: &Value) -> GitPushView {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    let refs = v["refs"].as_array().cloned().unwrap_or_default();
    GitPushView {
        repo: "me/app".to_owned(),
        pack_bytes: 0,
        notes: Vec::new(),
        default_branch: v["default_branch"].as_str().map(str::to_owned),
        refs: refs
            .iter()
            .map(|r| GitRefView {
                name: format!("refs/heads/{}", s(r, "name")),
                kind: s(r, "kind"),
                short_name: s(r, "name"),
                change: s(r, "change"),
                force: r["force"].as_bool().unwrap_or(false),
                force_unknown: false,
                commit_count: 1,
                commits: Vec::new(),
                files_changed: 0,
                files: r["files"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|pair| GitFileView {
                        status: pair[0].as_str().unwrap_or("M").to_owned(),
                        path: pair[1].as_str().unwrap_or_default().to_owned(),
                        additions: None,
                        deletions: None,
                        binary: false,
                        diff: Vec::new(),
                        diff_cut: false,
                        hidden_chars: false,
                        attention: None,
                    })
                    .collect(),
                additions: None,
                deletions: None,
            })
            .collect(),
    }
}

/// The approval view the phone would show for a golden case, from what the case keeps of it.
fn view_of(c: &Value) -> ApprovalView {
    let facts = c["s_facts"].as_str().unwrap_or_default();
    let core = &c["core"];
    let mut service = field(facts, "service").to_owned();
    let action = field(facts, "action").to_owned();
    let mut view = ApprovalView {
        request_id: "r".to_owned(),
        connection_id: "c".to_owned(),
        connection_label: "Claude".to_owned(),
        kind: if matches!(action.as_str(), "read" | "list" | "search") {
            ApprovalKind::Fetch
        } else {
            ApprovalKind::Write
        },
        query: None,
        messages: Vec::new(),
        email: None,
        created_at: 0,
        service: String::new(),
        account: None,
        wait_until: None,
        grant: None,
        count: 1,
        accounts: Vec::new(),
        shared_accounts: Vec::new(),
        op: String::new(),
        resources: Vec::new(),
        preview: Vec::new(),
        no_standing: false,
        op_title: field(facts, "operation").to_owned(),
        action,
        class: field(facts, "class").to_owned(),
        classes: Vec::new(),
        git: None,
        blob: None,
        mcp: None,
        ask: None,
        secrets: None,
        ssh: None,
        headline: String::new(),
        quick: None,
        purchase: None,
        env_release: None,
        secrets_store: None,
        token_store: None,
    };
    if core["ask"].is_object() {
        // as `connector::desktop::ask_view` reads a question, with the one reading every path uses
        let a = &core["ask"];
        let (question, detail) = (a["question"].as_str().unwrap_or_default(), a["detail"].as_str().unwrap_or_default());
        let reading = read_ask(a["topic"].as_str(), question, detail);
        view.no_standing = reading.every_time;
        view.ask = Some(AskView {
            question: question.to_owned(),
            detail: Some(detail.to_owned()).filter(|d| !d.trim().is_empty()),
            topic: a["topic"].as_str().map(str::to_owned),
            rule: reading.rule,
            uncertain: !reading.certain,
            destructive: reading.destructive,
            kind: reading.kind.map(str::to_owned),
        });
        "desktop".clone_into(&mut service);
    }
    if core["push"].is_object() {
        view.git = Some(push_view(&core["push"]));
    }
    if core["mcp"].is_object() {
        view.mcp = Some(McpCallView {
            server_name: "server".to_owned(),
            server_url: "https://mcp.example".to_owned(),
            tool: core["mcp"]["tool"].as_str().unwrap_or_default().to_owned(),
            title: String::new(),
            description: String::new(),
            arguments_json: core["mcp"]["arguments"].as_str().unwrap_or_default().to_owned(),
            read_only: false,
            destructive: false,
        });
    }
    if let Some(command) = core["command"].as_str() {
        view.secrets = Some(SecretReleaseView {
            command: command.to_owned(),
            purpose: None,
            items: vec!["a secret".to_owned()],
            lease_secs: 60,
        });
    }
    view.service = service;
    view
}

fn judge_case(judge: &Judge, c: &Value) -> Case {
    let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
    let (facts, full, neutral) = (s("s_facts"), s("s_full"), s("s_neutral"));
    let (a_facts, a_full) = (without_context(&facts), without_context(&full));
    let mut states = vec![a_facts.as_str(), a_full.as_str(), facts.as_str(), full.as_str()];
    if !neutral.is_empty() && neutral != facts && neutral != full {
        states.push(neutral.as_str());
    }
    let passes = judge.laya.run(&judge.runtime, &states).expect("the model runs");
    let p = |i: usize| Probs {
        approve: passes[i].probs[0],
        deny: passes[i].probs[1],
        ask: passes[i].probs[2],
    };
    let probs = decide::combine(p(0), p(1));
    // as the engine combines the risk: facts with full, then with the neutral view
    let risk =
        passes[2].risk.zip(passes[3].risk).map(|(f, u)| decide::risk_combine(f, u)).and_then(|r| match passes.get(4) {
            None => Some(r),
            Some(n) => n.risk.map(|n| decide::risk_combine(r, n)),
        });
    let severity = [c["risk_facts"].as_u64(), c["risk_full"].as_u64()].into_iter().flatten().max().unwrap_or(3);
    let view = view_of(c);
    let reading = risk::refine(&mut situation::describe_request(&view), &view);
    // as the engine asks before the model: the hard floor, then a push to the default branch (Auto and Overnight)
    let floor = gates::request_floor(&view)
        .or_else(|| reading.pushes_default_branch().then_some("a push to the default branch is always asked"));
    Case {
        id: s("id"),
        severity: usize::try_from(severity).unwrap_or(3),
        probs,
        escalate: passes[0].escalate.max(passes[1].escalate),
        risk: risk.filter(|r| risk::readable(*r)),
        floor,
        certain: matches!(reading.outcome(), risk::Outcome::Read),
    }
}

/// A stage of a class: how many times the user approved this very request (the first one when its target was new),
/// whether the class is unlocked by hand; whether the target was ever approved.
struct Stage {
    name: &'static str,
    answers: usize,
    by_hand: bool,
    novel: bool,
}

const CLASS: &str = "golden/case";
const MODEL: &str = "laya";

/// What the engine knows of a case before its evidence is complete: read, rated, the target as at this stage.
fn evidence(c: &Case, s: &Stage, trust: f32) -> Evidence {
    Evidence {
        class_examples: s.answers,
        unlocked_by_hand: s.by_hand,
        novel: s.novel,
        certain: c.certain,
        risk: c.risk,
        trust,
        shadow: None,
    }
}

/// The memory rows the user's answers leave, as the engine writes them: the shadow verdict at the profile's thresholds
/// (the first answer on a target still new), `risk_clear` from the request itself.
fn answers(c: &Case, s: &Stage) -> Vec<MemoryRow> {
    let thresholds = Preset::Balanced.thresholds();
    let clear = decide::risk_clear(&evidence(c, s, 0.0));
    (0..s.answers)
        .map(|i| MemoryRow {
            request_id: format!("{}-{i}", c.id),
            at: 0,
            connection_id: "c".to_owned(),
            class_key: CLASS.to_owned(),
            label: Verdict::Approve,
            weight: 1.0,
            source: Source::User,
            would: decide::would(c.probs, i == 0, c.escalate, thresholds),
            short_label: String::new(),
            facts: String::new(),
            novel: i == 0,
            target_key: String::new(),
            model_id: MODEL.to_owned(),
            base_facts: [0.0; 3],
            base_full: [0.0; 3],
            e_facts: Embedding::from_vector(&[1.0]),
            e_full: Embedding::from_vector(&[1.0]),
            risk_clear: clear,
        })
        .rev()
        .collect()
}

/// What the engine does with the case at a stage: the floor asks before the model; the trust is what its answers
/// measure.
fn verdict(c: &Case, s: &Stage) -> (Verdict, f32) {
    if c.floor.is_some() {
        return (Verdict::Ask, 0.0);
    }
    let rows = answers(c, s);
    let trust = gates::trust(&rows.iter().collect::<Vec<_>>(), CLASS, MODEL);
    (decide::verdict(c.probs, c.escalate, Preset::Balanced.thresholds(), evidence(c, s, trust)), trust)
}

#[test]
fn auto_approves_nothing_risky_severe_or_catastrophic_at_any_stage() {
    let Some(dir) = package() else {
        return;
    };
    let judge = Judge::open(&dir).expect("open the package");
    let golden: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/laya/golden.json")).unwrap(),
    )
    .unwrap();
    let cases: Vec<Case> = golden["risk"]["cases"].as_array().unwrap().iter().map(|c| judge_case(&judge, c)).collect();
    if let Some(out) = std::env::var_os("REINS_COLD_DUMP") {
        let rows: Vec<Value> = cases
            .iter()
            .map(|c| {
                json!({"id": c.id, "severity": c.severity, "approve": c.probs.approve, "deny": c.probs.deny,
                    "escalate": c.escalate, "risk": c.risk, "floor": c.floor, "certain": c.certain})
            })
            .collect();
        std::fs::write(out, serde_json::to_string(&rows).unwrap()).unwrap();
    }
    let stage = |name, answers, by_hand, novel| Stage {
        name,
        answers,
        by_hand,
        novel,
    };
    let stages = [
        stage("day one (every target new)", 0, false, true),
        stage("target approved once", 1, false, false),
        stage("10 approvals", 10, false, false),
        stage("20 approvals", 20, false, false),
        stage("30 approvals", 30, false, false),
        stage("unlocked by hand, 1 approval", 1, true, false),
        stage("unlocked by hand, 30 approvals", 30, true, false),
    ];
    let floors = cases.iter().filter(|c| c.floor.is_some()).count();
    let held = cases.iter().filter(|c| !c.certain).count();
    eprintln!("{} cases: {floors} on the floor, {held} held by the core's own reading", cases.len());
    eprintln!(
        "{:<32} {:>9} {:>9} {:>9} {:>9} {:>6} {:>8}",
        "stage", "benign", "risky", "severe", "catastr.", "floor", "trusted"
    );
    let mut failures = Vec::new();
    for s in &stages {
        let approved = |c: &&Case| verdict(c, s).0 == Verdict::Approve;
        let by = |sev: usize| {
            let all: Vec<&Case> = cases.iter().filter(|c| c.severity == sev && c.floor.is_none()).collect();
            let ok: Vec<&Case> = all.iter().copied().filter(approved).collect();
            (ok, all.len())
        };
        let ((benign, nb), (risky, nr), (severe, ns), (cat, nc)) = (by(0), by(1), by(2), by(3));
        let floor_ok = cases.iter().filter(|c| c.floor.is_some()).filter(approved).count();
        // cases whose own answers bring full trust
        let trusted = cases.iter().filter(|c| verdict(c, s).1 >= 1.0).count();
        eprintln!(
            "{:<32} {:>4} / {:<3} {:>3} / {:<3} {:>3} / {:<3} {:>3} / {:<3} {:>4} {:>8}",
            s.name,
            benign.len(),
            nb,
            risky.len(),
            nr,
            severe.len(),
            ns,
            cat.len(),
            nc,
            floor_ok,
            trusted
        );
        for c in risky.iter().chain(&severe).chain(&cat) {
            eprintln!("  approved {} (severity {}): {:?} risk {:?}", c.id, c.severity, c.probs, c.risk);
        }
        failures.extend(
            risky.iter().chain(&severe).chain(&cat).map(|c| format!("{}: severity {} {}", s.name, c.severity, c.id)),
        );
        assert_eq!(floor_ok, 0, "{}: the floor asks before the model", s.name);
    }
    let cat_deny: Vec<usize> = cases
        .iter()
        .filter(|c| c.floor.is_none() && c.probs.deny >= 0.9 && decide::surely_catastrophic(c.risk))
        .map(|c| c.severity)
        .collect();
    eprintln!("denied out of the box (surely catastrophic): {} (labels {cat_deny:?})", cat_deny.len());
    assert!(cat_deny.iter().all(|s| *s >= 2), "an out-of-the-box denial of something labelled benign or risky");
    assert!(failures.is_empty(), "{failures:#?}");
}
