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
/// Human decisions in a class before Overnight's own unlock of auto-approve (Auto needs none: [`auto_gate`]).
pub const APPROVE_UNLOCK_DECISIONS: usize = 20;
/// Human decisions in a class before auto-deny can unlock by itself.
pub const DENY_UNLOCK_DECISIONS: usize = 10;
/// Shadow accuracy below which auto-approve pauses in a class (and which Overnight's unlock needs).
pub const APPROVE_UNLOCK_ACCURACY: f32 = super::decide::TRUSTED_ACCURACY;
/// The window of the shadow accuracy…
const ACCURACY_WINDOW: usize = 50;
/// …in which Autopilot must have decided at least this many on its own for the accuracy to count.
const ACCURACY_MIN_DECIDED: usize = super::decide::MEASURED_DECIDED;
/// The window in which a single wrong automatic answer keeps the class paused.
pub const STRICT_WINDOW: usize = 20;

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

const FAR_REACHING: &str = "this is asked every time (secrets or a far-reaching change)";
const MCP_DESTRUCTIVE: &str = "the MCP server marks this tool destructive";
const PUSH_REWRITES: &str = "this push rewrites or deletes history";
const UPLOAD_RUNS: &str = "the uploaded file may hold something to run";

/// Whether Overnight may weigh a request's floor with the risk model instead of always asking (a request without a
/// floor: trivially). Only when every reason it has to wait is about what an action does: a destructive tool, a push
/// that rewrites or deletes history, a far-reaching change, an upload that may hold something to run. Never when one
/// is about who decides (a standing permission, the accounts an AI sees), money, secrets released or sign-ins, what
/// could not be read (a question the phone could not read, hidden characters), or sensitive items.
pub fn overnight_weighs(view: &ApprovalView) -> bool {
    floors(view).iter().all(|(_, weighable)| *weighable)
}

/// Why a request always waits for the user (spec §2), or `None`: the first of its reasons.
pub fn request_floor(view: &ApprovalView) -> Option<&'static str> {
    floors(view).first().map(|(why, _)| *why)
}

/// Every reason a request always waits, in order, each with whether Overnight may weigh it.
fn floors(view: &ApprovalView) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    if matches!(view.kind, ApprovalKind::Grant) {
        out.push(("a standing permission is only ever given by you", false));
    }
    if matches!(view.kind, ApprovalKind::Accounts) {
        out.push(("which accounts an AI sees is only ever decided by you", false));
    }
    if view.secrets.is_some() || view.env_release.is_some() {
        out.push(("secrets for the desktop app are only ever released by you", false));
    }
    if view.secrets_store.is_some() || view.token_store.is_some() {
        out.push(("secrets sent from the computer are only ever saved by you", false));
    }
    if view.ssh.is_some() {
        out.push(("SSH sign-ins are only ever approved by you", false));
    }
    if view.purchase.is_some() || view.service == reins_proto::connector::PAYMENTS && view.action == "write" {
        out.push(("purchases are only ever approved by you, or within a spend limit you set", false));
    }
    if view.no_standing {
        out.push((FAR_REACHING, !view.messages.iter().any(|m| m.sensitive)));
    }
    if view.ask.as_ref().is_some_and(|a| a.uncertain) {
        out.push(("the phone could not read this question with certainty", false));
    }
    if view.mcp.as_ref().is_some_and(|m| m.destructive) {
        out.push((MCP_DESTRUCTIVE, true));
    }
    if view.git.as_ref().is_some_and(|g| g.refs.iter().any(|r| r.force || r.force_unknown || r.change == "delete")) {
        out.push((PUSH_REWRITES, true));
    }
    if view.git.as_ref().is_some_and(|g| {
        g.refs.iter().any(|r| r.commits.iter().any(|c| c.hidden_chars) || r.files.iter().any(|f| f.hidden_chars))
    }) {
        out.push(("this push holds hidden characters that can make code read unlike what it does", false));
    }
    if view.blob.as_ref().is_some_and(blob_flagged) {
        out.push((UPLOAD_RUNS, true));
    }
    out
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

    /// Overnight's unlock of auto-approve by the numbers (before the cold start, Auto's too): 20 decisions, shadow
    /// accuracy at least 95%, no wrong approval among the last 20.
    pub fn approve_earned(&self) -> bool {
        self.decisions >= APPROVE_UNLOCK_DECISIONS
            && self.accuracy().is_some_and(|a| a >= APPROVE_UNLOCK_ACCURACY)
            && self.wrong_approvals == 0
    }

    /// Auto-approve is paused in the class: among the last 20 decisions Autopilot approved, or would have approved,
    /// something the user denied ("This was wrong" on an automatic approval counts), or its shadow accuracy fell
    /// below 95% once it can be measured. It resumes once enough decisions that agree with the user follow.
    pub fn demoted(&self) -> bool {
        self.wrong_approvals > 0 || self.accuracy().is_some_and(|a| a < APPROVE_UNLOCK_ACCURACY)
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

/// Overnight's class check: whether it may approve (`.0`) and deny (`.1`) in a class by the numbers it always used;
/// the user's override (`Some(true)` unlocked, `Some(false)` locked by hand) wins. Overnight also approves whatever
/// Auto would approve on its own ([`auto_gate`]).
pub fn unlocked(stats: &ClassStats, manual: Option<bool>) -> (bool, bool) {
    match manual {
        Some(open) => (open, open),
        None => (stats.approve_earned(), stats.deny_earned()),
    }
}

/// The user's measured agreement in a class with the model in use ([`super::decide::trust`]): how far it relaxes the
/// risk check. Over the class's last 50 decisions remembered with `model_id`, only those on requests that themselves
/// passed the fully relaxed risk check (`MemoryRow::risk_clear`) and that Autopilot would have decided: a decision on
/// a request held for its risk never helps release that hold, and another model's agreement is not this one's.
#[expect(clippy::cast_precision_loss, reason = "counts of at most 50")]
pub fn trust(rows: &[&MemoryRow], class_key: &str, model_id: &str) -> f32 {
    let (mut decided, mut correct) = (0_usize, 0_usize);
    for r in rows.iter().filter(|r| r.class_key == class_key && r.model_id == model_id).take(ACCURACY_WINDOW) {
        if r.risk_clear && r.would != Verdict::Ask {
            decided += 1;
            if r.would == r.label {
                correct += 1;
            }
        }
    }
    let accuracy = (decided >= ACCURACY_MIN_DECIDED).then(|| correct as f32 / decided as f32);
    super::decide::trust(accuracy, decided)
}

/// What Auto may do on its own in one class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "independent facts about one class")]
pub struct AutoGate {
    /// It may approve (when the request's own verdict says so: [`super::decide::verdict`]).
    pub approve: bool,
    /// It may deny any request its verdict denies (else only a surely catastrophic one: [`AutoGate::deny_catastrophic`]).
    pub deny: bool,
    /// It may deny what the risk head rates surely catastrophic, though the class has not earned auto-deny.
    pub deny_catastrophic: bool,
    /// Auto-approve paused after a wrong approval ([`ClassStats::demoted`]).
    pub paused: bool,
    /// Unlocked by hand: the profile's own thresholds from the first request.
    pub unlocked_by_hand: bool,
}

/// Auto's class check: approving needs no decisions first, only no recent wrong approval ([`ClassStats::demoted`]);
/// denying any request needs the numbers ([`ClassStats::deny_earned`]), else only a surely catastrophic one with no
/// wrong denial among the last 20. The user's override (`Some(true)` unlocked, `Some(false)` locked by hand) wins.
pub fn auto_gate(stats: &ClassStats, manual: Option<bool>) -> AutoGate {
    if let Some(open) = manual {
        return AutoGate {
            approve: open,
            deny: open,
            deny_catastrophic: open,
            paused: false,
            unlocked_by_hand: open,
        };
    }
    let paused = stats.demoted();
    AutoGate {
        approve: !paused,
        deny: stats.deny_earned(),
        deny_catastrophic: stats.wrong_denials == 0,
        paused,
        unlocked_by_hand: false,
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
    fn overnights_own_unlock_needs_twenty_accurate_decisions() {
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
    fn auto_approves_from_the_first_request_and_pauses_after_a_wrong_approval() {
        let empty = class_stats(&[], "github/write/push");
        assert_eq!(empty.decisions, 0);
        let gate = auto_gate(&empty, None);
        assert!(gate.approve && !gate.paused && !gate.deny && gate.deny_catastrophic, "{gate:?}");

        // "This was wrong" on an automatic approval (or a denial of what it would have approved) pauses the class…
        let mut rows = vec![decided(Verdict::Approve, Verdict::Deny)];
        rows.extend((0..5).map(|_| decided(Verdict::Approve, Verdict::Approve)));
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let gate = auto_gate(&class_stats(&refs, "github/write/push"), None);
        assert!(gate.paused && !gate.approve, "{gate:?}");
        // …until it leaves the strict window behind answers that agree with the user.
        let mut later: Vec<MemoryRow> =
            (0..STRICT_WINDOW).map(|_| decided(Verdict::Approve, Verdict::Approve)).collect();
        later.extend(rows);
        let refs: Vec<&MemoryRow> = later.iter().collect();
        let gate = auto_gate(&class_stats(&refs, "github/write/push"), None);
        assert!(gate.approve && !gate.paused, "{gate:?}");

        // Shadow accuracy under 95% once measurable pauses it too, without one wrong approval in the window.
        let mut low: Vec<MemoryRow> = (0..STRICT_WINDOW).map(|_| decided(Verdict::Ask, Verdict::Approve)).collect();
        low.extend((0..10).map(|i| {
            if i < 2 {
                decided(Verdict::Deny, Verdict::Approve)
            } else {
                decided(Verdict::Approve, Verdict::Approve)
            }
        }));
        let refs: Vec<&MemoryRow> = low.iter().collect();
        let s = class_stats(&refs, "github/write/push");
        assert_eq!((s.wrong_approvals, s.wrong_denials), (0, 0), "out of the strict window");
        assert!(s.accuracy().unwrap() < APPROVE_UNLOCK_ACCURACY && auto_gate(&s, None).paused);
    }

    #[test]
    fn locks_by_hand_win_and_auto_deny_stays_conservative() {
        let mut rows = vec![decided(Verdict::Approve, Verdict::Deny)];
        rows.extend((0..30).map(|_| decided(Verdict::Deny, Verdict::Deny)));
        let refs: Vec<&MemoryRow> = rows.iter().collect();
        let paused = class_stats(&refs, "github/write/push");
        let locked = auto_gate(&ClassStats::default(), Some(false));
        assert!(!locked.approve && !locked.deny && !locked.deny_catastrophic && !locked.paused, "{locked:?}");
        let opened = auto_gate(&paused, Some(true));
        assert!(opened.approve && opened.unlocked_by_hand && !opened.paused, "an unlock by hand beats the pause");
        // Deny: 10 decisions without a wrong denial, else only the surely catastrophic, and not after a wrong denial.
        assert!(!auto_gate(&ClassStats::default(), None).deny);
        assert!(auto_gate(&paused, None).deny);
        let mut wrong = vec![decided(Verdict::Deny, Verdict::Approve)];
        wrong.extend((0..10).map(|_| decided(Verdict::Deny, Verdict::Deny)));
        let refs: Vec<&MemoryRow> = wrong.iter().collect();
        let gate = auto_gate(&class_stats(&refs, "github/write/push"), None);
        assert!(!gate.deny && !gate.deny_catastrophic, "{gate:?}");
    }

    /// Agreement is measured only on decisions about requests clear of risk, with the model in use.
    #[test]
    fn trust_counts_only_risk_clear_decisions_of_the_model_in_use() {
        let agreeing = |n: usize, clear: bool, model: &str| -> Vec<MemoryRow> {
            (0..n)
                .map(|_| {
                    let mut r = decided(Verdict::Approve, Verdict::Approve);
                    r.risk_clear = clear;
                    model.clone_into(&mut r.model_id);
                    r
                })
                .collect()
        };
        let trust_of = |rows: &[MemoryRow], model: &str| {
            let refs: Vec<&MemoryRow> = rows.iter().collect();
            trust(&refs, "github/write/push", model)
        };
        assert!((trust_of(&agreeing(20, true, "m"), "m") - 1.0).abs() < 1e-6);
        assert!(trust_of(&agreeing(40, false, "m"), "m") == 0.0, "decisions on requests held for risk never count");
        assert!(trust_of(&agreeing(20, true, "old"), "m") == 0.0, "another model's agreement is not this one's");
        let mut mixed = agreeing(15, true, "m");
        mixed.extend(agreeing(30, false, "m"));
        assert!((trust_of(&mixed, "m") - 0.5).abs() < 1e-6, "15 clear decisions: halfway");
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
