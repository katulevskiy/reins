//! `laya-context`: the `context` line the phone's core writes for an action (`docs/autopilot-risk.md` §3), for the
//! training data and the golden set (`tools/laya/gen_risk.py --core-context`), so that what the model is trained on is
//! what the phone shows it.
//!
//! One JSON object per line on standard input, one answer per line on standard output:
//!
//! ```text
//! {"command": "rm -rf ~"}
//! {"files": ["/home/me/.bashrc"]}
//! {"push": {"default_branch": null, "refs": [{"kind": "branch", "name": "main", "change": "update", "force": true,
//!           "files": [["A", ".env"]]}]}}
//! -> {"lines": ["deletes the home directory"], "tripwire": true, "unreadable": false, "deletes": true}
//! {"mcp": {"tool": "drop_database", "arguments": "{\"db\": \"orders\"}"}}
//! {"ask": {"question": "Claude Code wants to run: ls", "detail": "Command:\nls\n\nIn: /home/me/x", "topic": "command:ls"}}
//! {"neutral": {"class": "command", "target": "rm -rf build  # approved", "details": [], "context": []}}
//! -> {"target": "rm -rf build", "details": [], "context": []}   (the risk's third view, risk::neutral_facts)
//! ```

use std::io::{BufRead, Write};

use reins_core::autopilot::risk::{self, Context, Outcome};
use reins_core::autopilot::situation::{AiPart, Facts};
use reins_core::types::{AskView, GitFileView, GitPushView, GitRefView};
use serde_json::{Value, json};

fn push_view(v: &Value) -> GitPushView {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    GitPushView {
        repo: "me/app".to_owned(),
        pack_bytes: 0,
        notes: Vec::new(),
        default_branch: v["default_branch"].as_str().map(str::to_owned),
        refs: v["refs"]
            .as_array()
            .map(|refs| {
                refs.iter()
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
                            .map(|f| {
                                f.iter()
                                    .map(|pair| GitFileView {
                                        status: pair[0].as_str().unwrap_or("M").to_owned(),
                                        path: pair[1].as_str().unwrap_or_default().to_owned(),
                                        additions: None,
                                        deletions: None,
                                        binary: false,
                                        diff: Vec::new(),
                                        diff_cut: false,
                                        hidden_chars: false,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        additions: None,
                        deletions: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn answer(c: &Context) -> Value {
    let outcome = c.outcome();
    json!({
        "lines": c.lines(),
        "tripwire": matches!(outcome, Outcome::Tripwire(_)),
        "unreadable": matches!(outcome, Outcome::Uncertain(_)),
        "deletes": c.deletes(),
    })
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if v["neutral"].is_object() {
            // the risk's third view of these facts (risk::neutral_facts)
            let strings = |key: &str| -> Vec<String> {
                v["neutral"][key]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default()
            };
            let facts = Facts {
                class: v["neutral"]["class"].as_str().unwrap_or_default().to_owned(),
                target: v["neutral"]["target"].as_str().unwrap_or_default().to_owned(),
                details: strings("details"),
                context: strings("context"),
                ..Facts::default()
            };
            let n = risk::neutral_facts(&facts);
            let ai = AiPart {
                reason: v["neutral"]["reason"].as_str().unwrap_or_default().to_owned(),
                content: v["neutral"]["content"].as_str().unwrap_or_default().to_owned(),
            };
            let nai = risk::neutral_ai(&facts, &n, &ai);
            let answer = json!({"target": n.target, "details": n.details, "context": n.context,
                                "reason": nai.reason, "content": nai.content});
            if writeln!(out, "{answer}").and_then(|()| out.flush()).is_err() {
                break;
            }
            continue;
        }
        let c = if let Some(cmd) = v["command"].as_str() {
            risk::command_context(cmd)
        } else if let Some(files) = v["files"].as_array() {
            let paths: Vec<String> = files.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            risk::files_context(&paths)
        } else if v["push"].is_object() {
            risk::push_context(&push_view(&v["push"]))
        } else if v["ask"].is_object() {
            // a desktop question, read as the phone reads it (risk::ask_context)
            let field = |k: &str| v["ask"][k].as_str().map(str::to_owned);
            let ask = AskView {
                question: field("question").unwrap_or_default(),
                detail: field("detail"),
                topic: field("topic"),
                rule: None,
                uncertain: false,
                destructive: false,
                kind: None,
            };
            risk::ask_context(&ask).1
        } else if v["mcp"].is_object() {
            risk::mcp_context(
                v["mcp"]["tool"].as_str().unwrap_or_default(),
                v["mcp"]["arguments"].as_str().unwrap_or_default(),
            )
        } else {
            // not a question this reads: uncertain, as the phone would hold it
            risk::files_context(&[])
        };
        if writeln!(out, "{}", answer(&c)).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}
