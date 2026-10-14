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
//! ```

use std::io::{BufRead, Write};

use reins_core::autopilot::risk::{self, Context};
use reins_core::types::{GitFileView, GitPushView, GitRefView};
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
    json!({"lines": c.lines, "tripwire": c.tripwire.is_some(), "unreadable": c.unreadable.is_some(), "deletes": c.deletes})
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let c = if let Some(cmd) = v["command"].as_str() {
            risk::command_context(cmd)
        } else if let Some(files) = v["files"].as_array() {
            let mut c = Context::default();
            for f in files.iter().filter_map(Value::as_str) {
                let one = risk::file_context(f);
                for l in one.lines {
                    if !c.lines.contains(&l) {
                        c.lines.push(l);
                    }
                }
                c.tripwire = c.tripwire.or(one.tripwire);
                c.unreadable = c.unreadable.or(one.unreadable);
            }
            c
        } else if v["push"].is_object() {
            risk::push_context(&push_view(&v["push"]))
        } else {
            Context::default()
        };
        if writeln!(out, "{}", answer(&c)).and_then(|()| out.flush()).is_err() {
            break;
        }
    }
}
