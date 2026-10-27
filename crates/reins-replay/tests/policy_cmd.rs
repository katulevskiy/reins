//! `reins-replay policy check|render|simulate` on a copy of the sample policy repo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use reins_policy::org::{ManagedRequest, SimInput, evaluate_managed};
use reins_policy::simulate::{Retained, simulate_org};
use reins_replay::policy_cmd;
use serde_json::json;

fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../reins-policy/tests/fixtures/policy_repo")
}

/// A fresh copy of the sample repo under the target directory.
fn copy(name: &str) -> PathBuf {
    let to = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("policy-{name}"));
    std::fs::remove_dir_all(&to).ok();
    for (rel, text) in policy_cmd::read_repo(&sample()).unwrap() {
        let p = to.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    std::fs::write(to.join("README.md"), "not policy\n").unwrap();
    to
}

fn cli(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_reins-replay")).arg("policy").args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn check_reports_file_and_line() {
    let dir = copy("check");
    let (code, out) = cli(&["check", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("ok: 3 rules"), "{out}");
    std::fs::write(dir.join("rules/bad.toml"), "[[rule]]\nid = \"bad\"\nwhen = { zones = [\"moon\"] }\ndeny = true\n")
        .unwrap();
    let (code, out) = cli(&["check", dir.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(out.starts_with("rules/bad.toml:3: "), "{out}");
    assert!(out.contains("moon"), "{out}");
}

#[test]
fn render_is_idempotent() {
    let dir = copy("render");
    let before = policy_cmd::check(&dir).unwrap().unwrap();
    // A messy rule file is normalized, and the README left alone.
    std::fs::write(
        dir.join("rules/payments.toml"),
        "[[rule]]\n  id='payments-review'\nexplain = \"Finance approves payments\"\nwhen={effects=['money']}\n\
         review={class='deferred',window='1d',reviewers='any of Finance'}\n",
    )
    .unwrap();
    let (code, out) = cli(&["render", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    let once = policy_cmd::read_repo(&dir).unwrap();
    assert!(policy_cmd::render(&dir).unwrap().is_ok());
    assert_eq!(policy_cmd::read_repo(&dir).unwrap(), once);
    assert_eq!(policy_cmd::check(&dir).unwrap().unwrap().hash, before.hash);
    assert!(once["rules/payments.toml"].contains("effects = [\"Money\"]"), "{once:?}");
    assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap(), "not policy\n");
}

#[test]
fn simulate_matches_the_library() {
    let base_dir = copy("sim-base");
    let draft_dir = copy("sim-draft");
    // The draft adds a review on every write to acme/api.
    std::fs::write(
        draft_dir.join("rules/api.toml"),
        "[[rule]]\nid = \"api-review\"\nwhen = { resource = \"acme/api\" }\nreview = { reviewers = \"any of Security\", class = \"deferred\" }\n",
    )
    .unwrap();
    let base = policy_cmd::check(&base_dir).unwrap().unwrap().policy;
    let draft = policy_cmd::check(&draft_dir).unwrap().unwrap().policy;
    let history: Vec<Retained> = ["acme/api", "acme/web", "acme/api", "someone/x"]
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let args = json!({"repo": repo, "title": "x"});
            let req = ManagedRequest {
                org: "acme",
                member: "lee",
                connection: "c1",
                tool: "github_create_issue",
                args: &args,
                now: 100 + i64::try_from(i).unwrap(),
            };
            let v = evaluate_managed(&req, &base, None);
            Retained {
                id: format!("or_{i}"),
                input: SimInput::from_request(&req),
                groups: vec!["Platform".into()],
                outcome: v.outcome,
                snapshot: v.snapshot,
                member_policy: None,
            }
        })
        .collect();
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let hist = tmp.join("policy-history.jsonl");
    let lines: Vec<String> = history.iter().map(|r| serde_json::to_string(r).unwrap()).collect();
    std::fs::write(&hist, lines.join("\n") + "\n").unwrap();
    let out = tmp.join("policy-report.json");
    let (code, text) = cli(&[
        "simulate",
        draft_dir.to_str().unwrap(),
        "--history",
        hist.to_str().unwrap(),
        "--base",
        base_dir.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{text}");
    let lib = simulate_org(&history, &BTreeMap::new(), &base, &draft);
    assert_eq!(text.trim(), lib.line());
    let written: reins_policy::simulate::SimulationReport =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(written, lib);
    assert_eq!((lib.auto_to_review, lib.drift), (2, 0));
}
