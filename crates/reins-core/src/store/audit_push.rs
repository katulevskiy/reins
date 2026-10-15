//! The push kept with an Activity entry (`AuditInfo::git`): what a push changed, ref by ref, commit by commit and file
//! by file. It is part of the entry's sealed info, so it syncs with the entry to every device of the account (Activity
//! is a grow-only union, `merge.rs`) and, like the rest of an entry, is never rewritten. The approval's own copy
//! (`audit_snapshots`, every diff as the desktop app sent it) stays on the device that showed it.
//!
//! It is the desktop app's push summary itself, shortened to [`SYNCED_PUSH_TARGET`] bytes: the files' changes line by
//! line only while the whole fits, then the lists (their counts stay). Its strings are cleaned when shown
//! (`views::push_view`), never trusted to be clean, and the whole is checked each time it is read, whichever device
//! wrote it: at most [`SYNCED_PUSH_MAX`] bytes, within the summary's bounds and formats (`PushSummary::validate`). A
//! copy that does not pass is dropped; the entry stays as it is.

use reins_proto::desktop::{GIT_PUSH_OP, GIT_TAG_PUSH_OP, PushSummary};
use reins_proto::gmail::ToolCall;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::CoreError;
use crate::types::{ActivityPush, ApprovalView, GitPushView};

/// The most bytes (JSON) of a push kept with an entry, as it is written.
pub const SYNCED_PUSH_TARGET: usize = 16 * 1024;
/// The most bytes (JSON) of a kept push as it is read, from any device: a larger one is dropped.
pub const SYNCED_PUSH_MAX: usize = 32 * 1024;
/// The longest repository name kept.
const MAX_REPO: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditPush {
    /// `owner/name`, as the call named it.
    pub repo: String,
    /// The desktop app's summary, without push options and maybe shortened.
    pub summary: PushSummary,
    /// The files' changes line by line were left out of this copy to keep it small.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diffs_dropped: bool,
}

fn json_len<T: Serialize>(v: &T) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// Takes every patch out; whether there was one.
fn strip_patches(summary: &mut PushSummary) -> bool {
    let mut had = false;
    for update in &mut summary.updates {
        for file in update.files.iter_mut().chain(update.commits.iter_mut().flat_map(|c| c.files.iter_mut())) {
            had |= file.patch.is_some();
            file.patch = None;
            file.patch_cut = false;
        }
    }
    had
}

impl AuditPush {
    /// The copy kept of the push `call` asks for; `None` for any other call.
    #[must_use]
    pub fn of_call(call: &ToolCall) -> Option<Self> {
        let ToolCall::Connector(call) = call else {
            return None;
        };
        if !matches!(call.op.as_str(), GIT_PUSH_OP | GIT_TAG_PUSH_OP) {
            return None;
        }
        let summary = crate::connector::git::summary_arg(call).ok()?;
        Self::fit(call.str_arg("repo")?.to_owned(), summary)
    }

    /// `summary` of `repo` shortened to [`SYNCED_PUSH_TARGET`] bytes; `None` when even empty lists do not fit.
    fn fit(repo: String, mut summary: PushSummary) -> Option<Self> {
        if repo.is_empty() || repo.chars().count() > MAX_REPO {
            return None;
        }
        summary.push_options.clear();
        let mut push = Self {
            repo,
            summary,
            diffs_dropped: false,
        };
        if json_len(&push) <= SYNCED_PUSH_TARGET {
            return Some(push);
        }
        push.diffs_dropped = strip_patches(&mut push.summary);
        let wrapper = json_len(&push).checked_sub(json_len(&push.summary))?;
        if !push.summary.fit(SYNCED_PUSH_TARGET.checked_sub(wrapper)?) {
            return None;
        }
        (json_len(&push) <= SYNCED_PUSH_TARGET).then_some(push)
    }

    /// A kept push as read back, from any device; `None` when it is too large or not a valid push.
    fn checked(value: &Value) -> Option<Self> {
        if json_len(value) > SYNCED_PUSH_MAX {
            return None;
        }
        let push = Self::deserialize(value).ok()?;
        if push.repo.is_empty() || push.repo.chars().count() > MAX_REPO {
            return None;
        }
        push.summary.validate().ok()?;
        Some(push)
    }

    /// The push as the screens show it; `diffs`: with the files' changes line by line (else without them, for lists;
    /// the hidden-character flags stay).
    #[must_use]
    pub fn view(&self, diffs: bool) -> GitPushView {
        let mut view = crate::views::push_view(&self.repo, &self.summary);
        if !diffs {
            for r in &mut view.refs {
                for f in r.files.iter_mut().chain(r.commits.iter_mut().flat_map(|c| c.files.iter_mut())) {
                    f.diff.clear();
                    f.diff_cut = false;
                }
            }
        }
        view
    }

    /// The push as an Activity entry's page shows it, from this copy.
    #[must_use]
    pub fn activity_push(&self) -> ActivityPush {
        let git = self.view(true);
        ActivityPush {
            diffs: has_diffs(&git),
            diffs_not_kept: self.diffs_dropped,
            this_device: false,
            git,
        }
    }
}

impl super::Store {
    /// Entry `seq`'s request as its approval showed it (this device's copy), else, for a push, a read-only view from
    /// the copy synced with the entry; `None` when there is neither.
    pub fn activity_approval(&self, seq: i64) -> Result<Option<ApprovalView>, CoreError> {
        if let Some(parked) = self.audit_snapshot(seq)? {
            return Ok(Some(crate::views::entry_approval_view(&parked)));
        }
        let Some(entry) = self.audit_entry(seq)? else {
            return Ok(None);
        };
        Ok(entry.info.git.as_deref().map(|git| crate::views::synced_push_approval_view(&entry, git.view(true))))
    }

    /// Entry `seq`'s push: this device's copy of its approval (every diff the desktop app sent) when it has one, else
    /// the copy synced with the entry; `None` when the entry is no push (or was logged before pushes were kept).
    pub fn activity_push(&self, seq: i64) -> Result<Option<ActivityPush>, CoreError> {
        if let Some(parked) = self.audit_snapshot(seq)?
            && let ToolCall::Connector(call) = &parked.request.call
            && let Some(git) = crate::views::git_push_view(call)
        {
            return Ok(Some(ActivityPush {
                diffs: has_diffs(&git),
                diffs_not_kept: false,
                this_device: true,
                git,
            }));
        }
        Ok(self.audit_entry(seq)?.and_then(|e| e.info.git.map(|g| g.activity_push())))
    }
}

/// Some file of `git` has its change line by line.
#[must_use]
pub fn has_diffs(git: &GitPushView) -> bool {
    git.refs.iter().any(|r| r.files.iter().chain(r.commits.iter().flat_map(|c| &c.files)).any(|f| !f.diff.is_empty()))
}

/// `AuditInfo::git` as read: absent, malformed or too large is `None`, never a failure of the entry's other details.
pub(super) fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Box<AuditPush>>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(AuditPush::checked).map(Box::new))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use reins_proto::desktop::{CommitInfo, FileChange, FileStatus, RefChange, RefUpdate};

    pub(crate) const A: &str = "1111111111111111111111111111111111111111";
    pub(crate) const B: &str = "2222222222222222222222222222222222222222";

    /// A push of `main` with `commits` commits of `files` files each, every file with a patch of `lines` lines.
    pub(crate) fn summary(commits: usize, files: usize, lines: usize) -> PushSummary {
        let patch = |lines: usize| {
            let body: Vec<String> = (0..lines).map(|i| format!("+line {i}")).collect();
            format!("@@ -1,{lines} +1,{lines} @@\n{}", body.join("\n"))
        };
        let file = |i: usize| FileChange {
            patch: (lines > 0).then(|| patch(lines)),
            ..FileChange::new(format!("src/file{i}.rs"), FileStatus::Modified, Some(3), Some(1), false)
        };
        PushSummary {
            updates: vec![RefUpdate {
                name: "refs/heads/main".to_owned(),
                change: RefChange::Update,
                old: A.to_owned(),
                new: B.to_owned(),
                fast_forward: Some(true),
                commit_count: u32::try_from(commits).unwrap(),
                commits: (0..commits)
                    .map(|c| CommitInfo {
                        sha: format!("{c:040x}").replace('0', "a"),
                        subject: format!("Fix number {c}"),
                        author: "Ann <ann@example.com>".to_owned(),
                        date: 1_700_000_000,
                        files_changed: Some(u32::try_from(files).unwrap()),
                        files: (0..files).map(file).collect(),
                        additions: Some(3),
                        deletions: Some(1),
                    })
                    .collect(),
                files_changed: u32::try_from(files).unwrap(),
                files: (0..files)
                    .map(|i| FileChange::new(format!("src/file{i}.rs"), FileStatus::Modified, Some(3), Some(1), false))
                    .collect(),
                additions: Some(3),
                deletions: Some(1),
            }],
            pack_bytes: 1_234,
            push_options: vec!["ci.skip".to_owned()],
            notes: Vec::new(),
            default_branch: Some("main".to_owned()),
        }
    }

    /// The tool call a push of `summary` to `me/app` is.
    pub(crate) fn push_call(summary: &PushSummary) -> ToolCall {
        ToolCall::Connector(
            serde_json::from_value(serde_json::json!({"service": "github", "op": "git_push",
                "args": {"repo": "me/app", "summary": serde_json::to_string(summary).unwrap()}}))
            .unwrap(),
        )
    }

    #[test]
    fn a_small_push_is_kept_whole_with_its_diffs() {
        let push = AuditPush::of_call(&push_call(&summary(2, 2, 5))).expect("a push");
        assert!(!push.diffs_dropped);
        assert!(push.summary.push_options.is_empty(), "push options are not kept");
        let view = push.view(true);
        assert_eq!(view.repo, "me/app");
        assert_eq!(view.refs[0].commits.len(), 2);
        assert_eq!(view.refs[0].commits[0].files[0].diff.len(), 6, "the hunk and its lines");
        assert!(has_diffs(&view));
        assert!(!has_diffs(&push.view(false)), "lists carry no diff lines");
        assert_eq!(push.view(false).refs[0].commits[0].files[0].additions, Some(3), "but the counts");
        let other = ToolCall::Connector(
            serde_json::from_value(serde_json::json!({"service": "github", "op": "issue_get", "args": {}})).unwrap(),
        );
        assert_eq!(AuditPush::of_call(&other), None);
    }

    #[test]
    fn a_large_push_loses_its_diffs_then_list_items_but_never_its_counts() {
        let push = AuditPush::of_call(&push_call(&summary(3, 4, 300))).expect("a push");
        assert!(push.diffs_dropped);
        assert!(json_len(&push) <= SYNCED_PUSH_TARGET);
        assert!(!has_diffs(&push.view(true)));
        assert_eq!(push.view(true).refs[0].commits.len(), 3, "commits and files are all there");
        let huge = AuditPush::of_call(&push_call(&summary(50, 100, 0))).expect("a push");
        assert!(json_len(&huge) <= SYNCED_PUSH_TARGET);
        let r = &huge.view(false).refs[0];
        assert_eq!(r.commit_count, 50, "the count stays");
        assert!(r.commits.len() < 50 || r.commits.iter().any(|c| c.files.len() < 100), "lists were shortened");
        assert!(!huge.view(false).notes.is_empty(), "and a note says so");
    }

    #[test]
    fn a_copy_read_back_is_checked_whoever_wrote_it() {
        let good = AuditPush::of_call(&push_call(&summary(1, 1, 3))).unwrap();
        let value = serde_json::to_value(&good).unwrap();
        assert_eq!(AuditPush::checked(&value), Some(good.clone()));
        // Larger than any device writes.
        let mut big = good.clone();
        big.summary.updates[0].commits[0].files[0].patch =
            Some(format!("@@ -1 +1 @@\n{}", vec!["+x".repeat(100); 300].join("\n")));
        assert!(json_len(&big) > SYNCED_PUSH_MAX);
        assert_eq!(AuditPush::checked(&serde_json::to_value(&big).unwrap()), None);
        // Past the summary's bounds or formats.
        let mut bad = good.clone();
        bad.summary.updates[0].commits[0].sha = "not a sha".to_owned();
        assert_eq!(AuditPush::checked(&serde_json::to_value(&bad).unwrap()), None);
        let mut bad = good.clone();
        bad.summary.updates[0].commit_count = 0;
        assert_eq!(AuditPush::checked(&serde_json::to_value(&bad).unwrap()), None);
        let mut bad = good;
        bad.repo = String::new();
        assert_eq!(AuditPush::checked(&serde_json::to_value(&bad).unwrap()), None);
        // Not a push at all.
        assert_eq!(AuditPush::checked(&serde_json::json!({"repo": 3})), None);
        assert_eq!(AuditPush::checked(&serde_json::json!("x")), None);
    }

    #[test]
    fn strings_are_cleaned_when_shown_and_hidden_characters_flagged() {
        let mut s = summary(1, 1, 2);
        s.updates[0].commits[0].subject = "Fix\u{202E}gnp.exe".to_owned();
        s.updates[0].commits[0].files[0].patch = Some("@@ -1 +1 @@\n+a\u{200B}b".to_owned());
        let push = AuditPush::fit("me/app\u{0007}".to_owned(), s).unwrap();
        let view = push.view(true);
        let c = &view.refs[0].commits[0];
        assert!(c.hidden_chars);
        assert!(!c.subject.contains('\u{202E}'));
        assert_eq!(c.files[0].diff[1].text, "a\u{fffd}b");
        assert!(c.files[0].hidden_chars);
        assert_eq!(view.repo, "me/app", "one line, no control characters");
        assert!(push.view(false).refs[0].commits[0].files[0].hidden_chars, "the flag stays without the diff");
    }
}
