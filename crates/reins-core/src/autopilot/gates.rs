//! What must hold before Autopilot acts on its own: the hard floor (spec §2) and, in Auto mode, the class gates and
//! the rate limit (spec §5.4).

use super::memory::MemoryRow;
use super::types::Verdict;
use crate::types::{ApprovalKind, ApprovalView, BlobView};

/// A connection younger than this always waits (and cannot be put in bypass).
pub const NEW_CONNECTION_SECS: i64 = 10 * 60;
/// Auto-approvals per connection: at most this many in 10 minutes…
pub const RATE_PER_10_MIN: u32 = 30;
/// …and this many in a day.
pub const RATE_PER_DAY: u32 = 200;
/// Human decisions in a class before auto-approve can unlock by itself.
pub const APPROVE_UNLOCK_DECISIONS: usize = 20;
/// Human decisions in a class before auto-deny can unlock by itself.
pub const DENY_UNLOCK_DECISIONS: usize = 10;
/// Shadow accuracy needed over the recent decisions.
pub const APPROVE_UNLOCK_ACCURACY: f32 = 0.95;
/// The window of the shadow accuracy…
const ACCURACY_WINDOW: usize = 50;
/// …in which Autopilot must have decided at least this many on its own for the accuracy to count.
const ACCURACY_MIN_DECIDED: usize = 10;
/// The window in which a single wrong automatic answer keeps the class locked.
const STRICT_WINDOW: usize = 20;

/// File types an upload is never shared automatically as (executables, scripts, archives that may hold them, and
/// anything the server could not tell).
const FLAGGED_TYPES: [&str; 11] = [
    "application/x-executable",
    "application/x-mach-binary",
    "application/x-msdownload",
    "application/vnd.android.package-archive",
    "application/java-archive",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/x-7z-compressed",
    "application/x-rar-compressed",
    "application/octet-stream",
];
const FLAGGED_EXTENSIONS: [&str; 34] = [
    "exe", "dll", "so", "dylib", "msi", "apk", "aab", "ipa", "jar", "bat", "cmd", "com", "ps1", "psm1", "vbs", "scr",
    "sh", "bash", "zsh", "app", "deb", "rpm", "dmg", "pkg", "bin", "run", "appimage", "elf", "zip", "tar", "gz", "tgz",
    "7z", "rar",
];

/// An upload that may hold something to run: never shared automatically.
pub fn blob_flagged(view: &BlobView) -> bool {
    let mime = view.content_type.split(';').next().unwrap_or_default().trim().to_lowercase();
    let ext = view.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    FLAGGED_TYPES.contains(&mime.as_str())
        || FLAGGED_EXTENSIONS.contains(&ext.as_str())
        || view.preview_text.as_deref().is_some_and(|t| t.trim_start().starts_with("#!"))
}

/// Why a request always waits for the user (spec §2), or `None`.
pub fn request_floor(view: &ApprovalView) -> Option<&'static str> {
    if matches!(view.kind, ApprovalKind::Grant) {
        return Some("a standing permission is only ever given by you");
    }
    if matches!(view.kind, ApprovalKind::Accounts) {
        return Some("which accounts an AI sees is only ever decided by you");
    }
    if view.secrets.is_some() {
        return Some("secrets for the desktop app are only ever released by you");
    }
    if view.ssh.is_some() {
        return Some("SSH sign-ins are only ever approved by you");
    }
    if view.purchase.is_some() || view.service == reins_proto::connector::PAYMENTS && view.action == "write" {
        return Some("purchases are only ever approved by you, or within a spend limit you set");
    }
    if view.no_standing {
        return Some("this is asked every time (secrets or a far-reaching change)");
    }
    if view.mcp.as_ref().is_some_and(|m| m.destructive) {
        return Some("the MCP server marks this tool destructive");
    }
    if view.git.as_ref().is_some_and(|g| g.refs.iter().any(|r| r.force || r.force_unknown || r.change == "delete")) {
        return Some("this push rewrites or deletes history");
    }
    if view.blob.as_ref().is_some_and(blob_flagged) {
        return Some("the uploaded file may hold something to run");
    }
    None
}

/// Why an upload always waits, or `None`.
pub fn blob_floor(view: &BlobView) -> Option<&'static str> {
    blob_flagged(view).then_some("the file may hold something to run (an executable, a script or an archive)")
}

/// The connection is too new for anything automatic (`age` unknown counts as new).
pub fn young(age: Option<i64>, limit: i64) -> bool {
    age.is_none_or(|a| a < limit)
}

/// What the memory says about one class, decisions newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassStats {
    pub decisions: usize,
    pub approved: usize,
    pub denied: usize,
    /// Among the last 50: how many Autopilot would have decided on its own, and how many of those it got right.
    pub decided: usize,
    pub correct: usize,
    /// Among the last 20: "would approve" where the user denied, and "would deny" where the user approved.
    pub wrong_approvals: usize,
    pub wrong_denials: usize,
}

impl ClassStats {
    /// `None` until it would have decided at least 10 of the recent ones.
    #[expect(clippy::cast_precision_loss, reason = "counts of at most 50")]
    pub fn accuracy(&self) -> Option<f32> {
        (self.decided >= ACCURACY_MIN_DECIDED).then(|| self.correct as f32 / self.decided as f32)
    }

    /// Auto-approve unlocked by the numbers (spec §5.4).
    pub fn approve_earned(&self) -> bool {
        self.decisions >= APPROVE_UNLOCK_DECISIONS
            && self.accuracy().is_some_and(|a| a >= APPROVE_UNLOCK_ACCURACY)
            && self.wrong_approvals == 0
    }

    /// Auto-deny unlocked by the numbers.
    pub fn deny_earned(&self) -> bool {
        self.decisions >= DENY_UNLOCK_DECISIONS && self.wrong_denials == 0
    }
}

/// The statistics of `class_key` over `rows` (newest first).
pub fn class_stats(rows: &[&MemoryRow], class_key: &str) -> ClassStats {
    let mut s = ClassStats::default();
    for (i, r) in rows.iter().filter(|r| r.class_key == class_key).enumerate() {
        s.decisions += 1;
        if r.approved() {
            s.approved += 1;
        } else {
            s.denied += 1;
        }
        if i < ACCURACY_WINDOW && r.would != Verdict::Ask {
            s.decided += 1;
            if r.would == r.label {
                s.correct += 1;
            }
        }
        if i < STRICT_WINDOW {
            match (r.would, r.label) {
                (Verdict::Approve, Verdict::Deny) => s.wrong_approvals += 1,
                (Verdict::Deny, Verdict::Approve) => s.wrong_denials += 1,
                _ => {}
            }
        }
    }
    s
}

/// Whether Auto mode may approve (`.0`) and deny (`.1`) in a class; the user's override wins.
pub fn unlocked(stats: &ClassStats, manual: Option<bool>) -> (bool, bool) {
    match manual {
        Some(open) => (open, open),
        None => (stats.approve_earned(), stats.deny_earned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autopilot::memory::tests::row;

    fn decided(would: Verdict, label: Verdict) -> MemoryRow {
        let mut r = row(label, &[1.0]);
        r.would = would;
        r.class_key = "github/write/push".to_owned();
        r
    }

    #[test]
    fn auto_approve_unlocks_after_twenty_accurate_decisions() {
        let mut rows: Vec<MemoryRow> = (0..19).map(|_| decided(Verdict::Approve, Verdict::Approve)).collect();
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let s = class_stats(&refs, "github/write/push");
        assert_eq!((s.decisions, s.decided, s.correct), (19, 19, 19));
        assert!(!s.approve_earned(), "19 is not 20");
        rows.push(decided(Verdict::Approve, Verdict::Approve));
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        assert!(class_stats(&refs, "github/write/push").approve_earned());
        assert!(!class_stats(&refs, "gmail/read").approve_earned(), "per class");
    }

    #[test]
    fn one_wrong_approval_in_the_last_twenty_locks_it() {
        let mut rows = vec![decided(Verdict::Approve, Verdict::Deny)];
        rows.extend((0..40).map(|_| decided(Verdict::Approve, Verdict::Approve)));
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let s = class_stats(&refs, "github/write/push");
        assert_eq!(s.wrong_approvals, 1);
        assert!(s.accuracy().unwrap() > APPROVE_UNLOCK_ACCURACY);
        assert!(!s.approve_earned());
        // Twenty good ones later it is out of the strict window.
        let mut newer: Vec<MemoryRow> = (0..20).map(|_| decided(Verdict::Approve, Verdict::Approve)).collect();
        newer.extend(rows);
        let refs: Vec<&MemoryRow> = newer.iter().collect();
        assert!(class_stats(&refs, "github/write/push").approve_earned());
    }

    #[test]
    fn accuracy_needs_enough_own_decisions() {
        let mut rows: Vec<MemoryRow> = (0..25).map(|_| decided(Verdict::Ask, Verdict::Approve)).collect();
        rows.extend((0..5).map(|_| decided(Verdict::Approve, Verdict::Approve)));
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let s = class_stats(&refs, "github/write/push");
        assert_eq!(s.accuracy(), None, "only 5 decided on its own");
        assert!(!s.approve_earned());
        let low: Vec<MemoryRow> = (0..30)
            .map(|i| {
                decided(
                    Verdict::Approve,
                    if i % 10 == 0 {
                        Verdict::Deny
                    } else {
                        Verdict::Approve
                    },
                )
            })
            .collect();
        let refs: Vec<&MemoryRow> = low.iter().collect();
        assert!(!class_stats(&refs, "github/write/push").approve_earned(), "90% is not enough");
    }

    #[test]
    fn auto_deny_unlocks_after_ten_and_locks_on_a_wrong_denial() {
        let rows: Vec<MemoryRow> = (0..10).map(|_| decided(Verdict::Deny, Verdict::Deny)).collect();
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let s = class_stats(&refs, "github/write/push");
        assert!(s.deny_earned() && !s.approve_earned());
        let mut wrong = vec![decided(Verdict::Deny, Verdict::Approve)];
        wrong.extend(rows);
        let refs: Vec<&MemoryRow> = wrong.iter().collect();
        assert!(!class_stats(&refs, "github/write/push").deny_earned());
        assert_eq!(unlocked(&class_stats(&refs, "github/write/push"), Some(true)), (true, true), "unlocked by hand");
        assert_eq!(unlocked(&ClassStats::default(), Some(false)), (false, false));
    }

    #[test]
    fn young_or_unknown_connections_wait() {
        assert!(young(None, NEW_CONNECTION_SECS));
        assert!(young(Some(599), NEW_CONNECTION_SECS));
        assert!(!young(Some(600), NEW_CONNECTION_SECS));
    }

    #[test]
    fn executables_scripts_and_archives_are_flagged() {
        let view = |name: &str, mime: &str, text: Option<&str>| BlobView {
            id: "b".to_owned(),
            connection_label: "c".to_owned(),
            name: name.to_owned(),
            size: 1,
            content_type: mime.to_owned(),
            sha256: String::new(),
            purpose: String::new(),
            preview_text: text.map(str::to_owned),
            preview_image: None,
            created_at: 0,
            expires_at: 0,
        };
        assert!(blob_flagged(&view("a.bin", "application/x-executable", None)));
        assert!(blob_flagged(&view("tool.zip", "application/zip", None)));
        assert!(blob_flagged(&view("run.sh", "text/plain; charset=utf-8", Some("echo"))));
        assert!(blob_flagged(&view("notes.txt", "text/plain; charset=utf-8", Some("#!/bin/sh\nrm -rf /"))));
        assert!(blob_flagged(&view("data", "application/octet-stream", None)), "unknown binary");
        assert!(!blob_flagged(&view("report.txt", "text/plain; charset=utf-8", Some("quarterly numbers"))));
        assert!(!blob_flagged(&view("photo.png", "image/png", None)));
        assert!(!blob_flagged(&view("paper.pdf", "application/pdf", None)));
    }
}
