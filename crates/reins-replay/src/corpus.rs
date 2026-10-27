//! The replay corpus: recorded requests, anonymized, each with the label a person gave it.
//!
//! A corpus is a folder with `context.json` (the workspace and the owner's own accounts and devices, as the setup
//! screens of Decisions v2 P2 would confirm them) and any number of `*.jsonl` files, one [`Case`] per line.

use std::path::Path;

use reins_proto::desktop::PushSummary;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What kind of request a case is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A harness hook before a shell command.
    Command,
    /// A harness hook before reading or changing files.
    File,
    /// A push through the desktop git proxy.
    GitPush,
    /// A clone or fetch through the proxy.
    GitRead,
    /// A tool call through `reins mcp` (connectors).
    Mcp,
    /// `reins ask`, a work session: a question meant for a person.
    Ask,
}

impl Kind {
    /// Developer work: commands, files and git (the 01 §13 ask-rate target of 3% is for these).
    #[must_use]
    pub fn developer(self) -> bool {
        matches!(self, Self::Command | Self::File | Self::GitPush | Self::GitRead)
    }
}

/// What a person says the request needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    /// Auto is right.
    Routine,
    /// A person should see it.
    NeedsHuman,
}

/// How the request ended when it was recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Original {
    /// `approved`, `denied`, `timed_out`, `failed`, `waiting`; empty for synthetic cases.
    #[serde(default)]
    pub outcome: String,
    /// The guard rule the computer reported (`command:rm -r`, `file:.env`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_rule: Option<String>,
}

/// One request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    /// Unix seconds.
    #[serde(default)]
    pub at: i64,
    pub kind: Kind,
    /// The harness or program that asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<bool>,
    /// `github.com/<owner>/<name>` for git; `<owner>/<name>` in tool arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default)]
    pub original: Original,
    pub label: Label,
    /// `real_risk`, `known_fp`, `owner_denied`, `timed_out`, `adversarial`, `repaired`, `burst:<n>`, …
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Case {
    #[must_use]
    pub fn tagged(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// A labelled real risk: never Auto (the recall gate).
    #[must_use]
    pub fn real_risk(&self) -> bool {
        self.tagged("real_risk")
    }
}

/// What the corpus knows about the computer and the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub home: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub scratch: Vec<String>,
    #[serde(default)]
    pub worktrees: Vec<String>,
    /// GitHub owners that are the owner's own (zone Mine).
    #[serde(default)]
    pub mine_owners: Vec<String>,
    /// The owner's own devices (zone Mine for ssh and network).
    #[serde(default)]
    pub mine_hosts: Vec<String>,
    /// The repositories' default branch.
    #[serde(default)]
    pub default_branch: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Corpus {
    pub context: Context,
    pub cases: Vec<Case>,
}

impl Corpus {
    /// Reads a corpus folder.
    ///
    /// # Errors
    /// An unreadable file or a line that is not a case (with its file and line number).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let ctx_path = dir.join("context.json");
        let context: Context =
            serde_json::from_slice(&std::fs::read(&ctx_path).map_err(|e| format!("{}: {e}", ctx_path.display()))?)
                .map_err(|e| format!("{}: {e}", ctx_path.display()))?;
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        let mut cases = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            for (n, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let case: Case = serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", f.display(), n + 1))?;
                cases.push(case);
            }
        }
        Ok(Self {
            context,
            cases,
        })
    }
}
