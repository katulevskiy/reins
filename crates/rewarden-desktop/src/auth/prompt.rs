//! Asking the person at the computer: the pending list the CLI answers (`rewarden pending`, `approve`, `deny`) and a
//! desktop notification or dialog. A question is keyed by what it is about (a repository read, a push digest), so git
//! run again finds the question still open, or the answer given while git was not waiting any more.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// How long a question stays open, and how long its answer is remembered for git run again.
pub const REMEMBER: Duration = Duration::from_secs(600);

const MAX_OPEN: usize = 100;

/// One question waiting for an answer, as the control API lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingItem {
    pub id: String,
    /// "Read me/app", "Push to me/app".
    pub what: String,
    pub lines: Vec<String>,
    /// Unix seconds.
    pub created_at: i64,
}

/// Shows a question on the desktop and returns the answer: `Some(true)` approve, `Some(false)` deny, `None` no answer
/// (no desktop tool, dismissed). Dropping the future must take the prompt down.
#[async_trait::async_trait]
pub trait Prompter: Send + Sync {
    async fn ask(&self, item: &PendingItem) -> Option<bool>;
}

/// The questions waiting, and the answers recently given.
#[derive(Default)]
pub struct Pending {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    open: HashMap<String, Open>,
    answered: HashMap<String, (bool, Instant)>,
}

struct Open {
    item: PendingItem,
    key: String,
    since: Instant,
    answer: watch::Sender<Option<bool>>,
}

/// What [`Pending::ask`] found.
pub enum Asked {
    /// Answered earlier (within [`REMEMBER`]).
    Answered(bool),
    /// Waiting: the receiver changes when it is answered. `fresh` when this call opened the question.
    Open {
        id: String,
        answer: watch::Receiver<Option<bool>>,
        fresh: bool,
    },
}

fn new_id() -> String {
    let mut bytes = [0u8; 4];
    crypto_box::aead::rand_core::RngCore::fill_bytes(&mut crypto_box::aead::OsRng, &mut bytes);
    data_encoding::HEXLOWER.encode(&bytes)
}

impl Pending {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn sweep(inner: &mut Inner) {
        inner.open.retain(|_, o| o.since.elapsed() < REMEMBER);
        inner.answered.retain(|_, (_, at)| at.elapsed() < REMEMBER);
    }

    /// The earlier answer for `key`, the open question for it, or a new question (`what`, `lines`).
    pub fn ask(&self, key: &str, what: &str, lines: Vec<String>) -> Result<Asked, String> {
        let mut inner = self.lock();
        Self::sweep(&mut inner);
        if let Some((approved, _)) = inner.answered.get(key) {
            return Ok(Asked::Answered(*approved));
        }
        if let Some(open) = inner.open.values().find(|o| o.key == key) {
            return Ok(Asked::Open {
                id: open.item.id.clone(),
                answer: open.answer.subscribe(),
                fresh: false,
            });
        }
        if inner.open.len() >= MAX_OPEN {
            return Err("Too many approvals are waiting; answer them with `rewarden pending`.".to_owned());
        }
        let id = loop {
            let id = new_id();
            if !inner.open.contains_key(&id) {
                break id;
            }
        };
        let (tx, rx) = watch::channel(None);
        inner.open.insert(
            id.clone(),
            Open {
                item: PendingItem {
                    id: id.clone(),
                    what: what.to_owned(),
                    lines,
                    created_at: crate::now_unix(),
                },
                key: key.to_owned(),
                since: Instant::now(),
                answer: tx,
            },
        );
        Ok(Asked::Open {
            id,
            answer: rx,
            fresh: true,
        })
    }

    /// Answers the question `id`. `false` when there is no such open question.
    pub fn answer(&self, id: &str, approved: bool) -> bool {
        let mut inner = self.lock();
        Self::sweep(&mut inner);
        let Some(open) = inner.open.remove(id) else {
            return false;
        };
        inner.answered.insert(open.key, (approved, Instant::now()));
        open.answer.send_replace(Some(approved));
        true
    }

    /// The open questions, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<PendingItem> {
        let mut inner = self.lock();
        Self::sweep(&mut inner);
        let mut items: Vec<PendingItem> = inner.open.values().map(|o| o.item.clone()).collect();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        items
    }

    /// The open question `id`, for a prompt.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<PendingItem> {
        self.lock().open.get(id).map(|o| o.item.clone())
    }
}

/// Shows `item` with `prompter` until it is answered (here or elsewhere) or forgotten; never blocks the caller.
pub fn spawn_prompt(
    pending: Arc<Pending>,
    prompter: Arc<dyn Prompter>,
    item: PendingItem,
    mut answer: watch::Receiver<Option<bool>>,
) {
    tokio::spawn(async move {
        let answered_elsewhere = async {
            while answer.borrow_and_update().is_none() {
                if answer.changed().await.is_err() {
                    return;
                }
            }
        };
        tokio::select! {
            got = prompter.ask(&item) => {
                if let Some(approved) = got {
                    pending.answer(&item.id, approved);
                }
            }
            () = answered_elsewhere => {}
            () = tokio::time::sleep(REMEMBER) => {}
        }
    });
}

/// The desktop's own way of asking: `notify-send` with actions on Linux, an `osascript` dialog on macOS, nothing
/// elsewhere (or when the tool is missing): the CLI still answers.
pub struct DesktopPrompter;

/// Notification servers read a little markup; the text comes from the agent's push.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl DesktopPrompter {
    fn command(item: &PendingItem) -> Option<tokio::process::Command> {
        let body = format!(
            "{}\n\nOr answer with `rewarden approve {}` / `rewarden deny {}`.",
            item.lines.join("\n"),
            item.id,
            item.id
        );
        if cfg!(target_os = "macos") {
            let mut c = tokio::process::Command::new("osascript");
            // The text goes in as an argument, never into the script.
            c.args([
                "-e",
                "on run argv",
                "-e",
                "display dialog (item 2 of argv) with title (item 1 of argv) buttons {\"Deny\", \"Approve\"} default button \"Deny\" giving up after 600",
                "-e",
                "end run",
                &format!("Rewarden: {}", item.what),
                &body,
            ]);
            Some(c)
        } else if cfg!(unix) {
            let mut c = tokio::process::Command::new("notify-send");
            c.args([
                "--app-name=Rewarden",
                "--urgency=critical",
                "--action=approve=Approve",
                "--action=deny=Deny",
                "--wait",
                &format!("Rewarden: {}", escape_markup(&item.what)),
                &escape_markup(&body),
            ]);
            Some(c)
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl Prompter for DesktopPrompter {
    async fn ask(&self, item: &PendingItem) -> Option<bool> {
        let mut cmd = Self::command(item)?;
        cmd.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let out = cmd.output().await.ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let text = text.trim();
        if text == "approve" || text.ends_with("button returned:Approve") {
            Some(true)
        } else if text == "deny" || text.ends_with("button returned:Deny") {
            Some(false)
        } else {
            None
        }
    }
}

/// Never shows anything (tests, and servers without a desktop).
pub struct NoPrompter;

#[async_trait::async_trait]
impl Prompter for NoPrompter {
    async fn ask(&self, _item: &PendingItem) -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_asked_once_and_its_answer_is_remembered_by_key() {
        let p = Pending::default();
        let Ok(Asked::Open {
            id,
            answer,
            fresh: true,
        }) = p.ask("push abc", "Push to me/app", vec!["Push 1 commit to main".into()])
        else {
            panic!("expected a fresh question")
        };
        let Ok(Asked::Open {
            id: again,
            fresh: false,
            ..
        }) = p.ask("push abc", "Push to me/app", vec![])
        else {
            panic!("expected the open question")
        };
        assert_eq!(id, again);
        assert_eq!(p.list().len(), 1);
        assert_eq!(p.get(&id).unwrap().lines, vec!["Push 1 commit to main"]);
        assert!(!p.answer("nope", true));
        assert!(p.answer(&id, false));
        assert_eq!(*answer.borrow(), Some(false));
        assert!(p.list().is_empty());
        assert!(matches!(p.ask("push abc", "", vec![]), Ok(Asked::Answered(false))));
        assert!(matches!(
            p.ask("push other", "", vec![]),
            Ok(Asked::Open {
                fresh: true,
                ..
            })
        ));
    }

    #[test]
    fn markup_from_the_agent_is_escaped() {
        assert_eq!(escape_markup("<b>a&b</b>"), "&lt;b&gt;a&amp;b&lt;/b&gt;");
    }

    #[tokio::test]
    async fn a_prompt_answer_resolves_the_question() {
        struct Yes;
        #[async_trait::async_trait]
        impl Prompter for Yes {
            async fn ask(&self, _item: &PendingItem) -> Option<bool> {
                Some(true)
            }
        }
        let p = Arc::new(Pending::default());
        let Ok(Asked::Open {
            id,
            mut answer,
            ..
        }) = p.ask("read me/app", "Read me/app", vec![])
        else {
            panic!()
        };
        spawn_prompt(Arc::clone(&p), Arc::new(Yes), p.get(&id).unwrap(), answer.clone());
        answer.changed().await.unwrap();
        assert_eq!(*answer.borrow(), Some(true));
    }
}
