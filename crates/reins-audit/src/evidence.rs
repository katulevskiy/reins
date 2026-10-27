//! Evidence for auditors from a verified export: what `reins-audit report` prints.
//!
//! [`build`] maps an org's audit events onto the controls of one framework (SOC 2's Trust Services Criteria, 06 §7,
//! or ISO/IEC 27001:2022 Annex A): per control, how many events of each kind and a few sample sequence numbers an
//! auditor can look up in the export. It also lists exceptions: events that show a control was weakened.
//!
//! Only events from an export that verified reach it ([`crate::verify_collect`]); the report names the export's
//! range and head hash so it can be tied back to the export. The output holds no wall-clock time: the same export
//! and options always give the same bytes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use reins_proto::managed::AuditEvent;
use serde::Serialize;
use serde_json::Value;

use crate::Report;

/// A control framework a report is made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Framework {
    Soc2,
    Iso27001,
}

impl Framework {
    /// `soc2` or `iso27001` (as `reins-audit report --framework` takes it).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "soc2" => Some(Self::Soc2),
            "iso27001" => Some(Self::Iso27001),
            _ => None,
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Soc2 => "SOC 2 (Trust Services Criteria)",
            Self::Iso27001 => "ISO/IEC 27001:2022 (Annex A)",
        }
    }

    fn short(self) -> &'static str {
        match self {
            Self::Soc2 => "SOC 2",
            Self::Iso27001 => "ISO 27001",
        }
    }

    fn catalogue(self) -> &'static [Control] {
        match self {
            Self::Soc2 => SOC2,
            Self::Iso27001 => ISO27001,
        }
    }
}

/// One control and the event kinds that evidence it. A pattern ending in `*` is a prefix.
struct Control {
    id: &'static str,
    title: &'static str,
    kinds: &'static [&'static str],
}

const SOC2: &[Control] = &[
    Control {
        id: "CC6.1",
        title: "Logical access",
        kinds: &[
            "org.created",
            "domain.*",
            "member.provisioned",
            "member.joined",
            "member.linked",
            "group.*",
            "key.created",
            "device.enrolled",
        ],
    },
    Control {
        id: "CC6.2",
        title: "Registration of users and devices",
        kinds: &[
            "member.provisioned",
            "member.restored",
            "device.enrolled",
            "device.reenroll_refused",
            "device.posture",
        ],
    },
    Control {
        id: "CC6.3",
        title: "Access removal and least privilege",
        kinds: &["member.deprovisioned", "group.deprovisioned", "group.member_removed", "directory.*", "lease.*"],
    },
    Control {
        id: "CC6.6",
        title: "Boundary protection",
        kinds: &["executor.*", "request.executed", "request.refused"],
    },
    Control {
        id: "CC6.7",
        title: "Signed decisions",
        kinds: &["decision.approved", "review.approved", "review.web", "passkey.*"],
    },
    Control {
        id: "CC7.2",
        title: "Monitoring",
        kinds: &["audit.*", "request.failed", "request.unknown", "request.stale"],
    },
    Control {
        id: "CC7.3",
        title: "Incident response",
        kinds: &["lease.revoked*", "decision.denied", "review.denied", "policy.rolled_back", "policy.rollout_paused"],
    },
    Control {
        id: "CC8.1",
        title: "Change management",
        kinds: &["change.*", "policy.*", "policy_repo.*", "routing.changed"],
    },
    Control {
        id: "A1.2",
        title: "Availability",
        kinds: &["review.expired", "review.escalated", "request.expired"],
    },
    Control {
        id: "C1.2",
        title: "Disposal",
        kinds: &["retention.set", "audit.pruned"],
    },
];

const ISO27001: &[Control] = &[
    Control {
        id: "A.5.15",
        title: "Access control",
        kinds: &["policy.activated", "member_policy.set", "change.applied"],
    },
    Control {
        id: "A.5.16",
        title: "Identity management",
        kinds: &["member.provisioned", "member.joined", "member.linked", "group.provisioned", "directory.*"],
    },
    Control {
        id: "A.5.18",
        title: "Access rights",
        kinds: &["lease.*", "member.deprovisioned", "group.member_*"],
    },
    Control {
        id: "A.5.26",
        title: "Response to incidents",
        kinds: &["lease.revoked*", "policy.rolled_back"],
    },
    Control {
        id: "A.8.2",
        title: "Privileged access rights",
        kinds: &["change.*", "org.owner_*"],
    },
    Control {
        id: "A.8.5",
        title: "Secure authentication",
        kinds: &["device.*", "passkey.*"],
    },
    Control {
        id: "A.8.10",
        title: "Information deletion",
        kinds: &["retention.set", "audit.pruned"],
    },
    Control {
        id: "A.8.15",
        title: "Logging",
        kinds: &["audit.*", "key.created"],
    },
    Control {
        id: "A.8.16",
        title: "Monitoring activities",
        kinds: &["request.refused", "request.failed", "decision.denied", "review.expired"],
    },
    Control {
        id: "A.8.32",
        title: "Change management",
        kinds: &["policy.*", "change.*", "routing.changed"],
    },
];

fn matches(pattern: &str, kind: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => kind.starts_with(prefix),
        None => kind == pattern,
    }
}

/// An event that shows a control was weakened.
struct ExceptionRule {
    kind: &'static str,
    /// The detail field that must be `true` (none: every event of `kind`).
    flag: Option<&'static str>,
    why: &'static str,
    soc2: &'static str,
    iso27001: &'static str,
}

const EXCEPTIONS: &[ExceptionRule] = &[
    ExceptionRule {
        kind: "change.applied",
        flag: Some("sole_holder"),
        why: "applied without a second person",
        soc2: "CC8.1",
        iso27001: "A.8.32",
    },
    ExceptionRule {
        kind: "member.deprovisioned",
        flag: Some("kept_as_last_owner"),
        why: "the last owner was kept",
        soc2: "CC6.3",
        iso27001: "A.5.18",
    },
    // Informational: an admin read request bodies through the console.
    ExceptionRule {
        kind: "audit.read",
        flag: None,
        why: "request bodies were read",
        soc2: "C1.2",
        iso27001: "A.8.15",
    },
];

/// The time a report covers: events with `at` in `[from, to)` (unix seconds; `None` is unbounded).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Period {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl Period {
    #[must_use]
    pub fn contains(&self, at: i64) -> bool {
        self.from.is_none_or(|f| at >= f) && self.to.is_none_or(|t| at < t)
    }
}

/// The verified export a report was made from (the whole export, whatever the period).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub first_seq: i64,
    pub last_seq: i64,
    pub events: u64,
    pub head_hash: String,
    pub checkpoints: u64,
}

/// Headline counts over the period.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Events in the period.
    pub events: u64,
    pub members_provisioned: u64,
    pub members_deprovisioned: u64,
    pub devices_enrolled: u64,
    pub admin_changes_applied: u64,
    /// Of `admin_changes_applied`: approved by a second admin.
    pub admin_changes_two_person: u64,
    pub policy_activations: u64,
    pub decisions_auto: u64,
    pub decisions_approved: u64,
    pub decisions_denied: u64,
    pub reviews_opened: u64,
    pub reviews_expired: u64,
    pub executions: u64,
    pub leases_issued: u64,
    pub leases_revoked: u64,
}

/// One control's events in the period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlEvidence {
    pub id: &'static str,
    pub title: &'static str,
    pub total: u64,
    /// Events per kind.
    pub counts: BTreeMap<String, u64>,
    /// Up to 5 sequence numbers: the first, the last and evenly between.
    pub samples: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Exception {
    pub seq: i64,
    pub kind: String,
    /// The control it weakens.
    pub control: &'static str,
    pub why: &'static str,
}

/// A per-control evidence report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub framework: Framework,
    pub org: String,
    pub period: Period,
    pub export: ExportSummary,
    pub summary: Summary,
    pub controls: Vec<ControlEvidence>,
    pub exceptions: Vec<Exception>,
}

const MAX_SAMPLES: usize = 5;

/// Up to [`MAX_SAMPLES`] of `seqs`: the first, the last and evenly between.
fn samples(seqs: &[i64]) -> Vec<i64> {
    if seqs.len() <= MAX_SAMPLES {
        return seqs.to_vec();
    }
    let last = seqs.len() - 1;
    (0..MAX_SAMPLES).map(|i| seqs[i * last / (MAX_SAMPLES - 1)]).collect()
}

fn flag(detail: Option<&Value>, key: &str) -> bool {
    detail.and_then(|d| d.get(key)) == Some(&Value::Bool(true))
}

/// The evidence of `events` (from [`crate::verify_collect`], which made `report`) for `framework`, over `period`.
#[must_use]
pub fn build(framework: Framework, report: &Report, events: &[AuditEvent], period: Period) -> Evidence {
    let catalogue = framework.catalogue();
    let mut seqs: Vec<Vec<i64>> = vec![Vec::new(); catalogue.len()];
    let mut counts: Vec<BTreeMap<String, u64>> = vec![BTreeMap::new(); catalogue.len()];
    let mut summary = Summary::default();
    let mut exceptions: Vec<Exception> = Vec::new();
    for e in events.iter().filter(|e| period.contains(e.at)) {
        summary.events += 1;
        for (i, control) in catalogue.iter().enumerate() {
            if control.kinds.iter().any(|p| matches(p, &e.kind)) {
                seqs[i].push(e.seq);
                *counts[i].entry(e.kind.clone()).or_default() += 1;
            }
        }
        let detail = || serde_json::from_str::<Value>(&e.detail).ok();
        match e.kind.as_str() {
            "member.provisioned" => summary.members_provisioned += 1,
            "member.deprovisioned" => summary.members_deprovisioned += 1,
            "device.enrolled" => summary.devices_enrolled += 1,
            "change.applied" => {
                summary.admin_changes_applied += 1;
                // Applied on a second admin's approval: the event names who proposed it.
                if detail().is_some_and(|d| d.get("proposed_by").is_some()) {
                    summary.admin_changes_two_person += 1;
                }
            }
            "policy.activated" => summary.policy_activations += 1,
            "decision.auto" => summary.decisions_auto += 1,
            "decision.approved" => summary.decisions_approved += 1,
            "decision.denied" => summary.decisions_denied += 1,
            "review.opened" => summary.reviews_opened += 1,
            "review.expired" => summary.reviews_expired += 1,
            "request.executed" => summary.executions += 1,
            "lease.issued" => summary.leases_issued += 1,
            "lease.revoked" => summary.leases_revoked += 1,
            _ => {}
        }
        for rule in EXCEPTIONS.iter().filter(|r| r.kind == e.kind) {
            if rule.flag.is_none_or(|key| flag(detail().as_ref(), key)) {
                exceptions.push(Exception {
                    seq: e.seq,
                    kind: e.kind.clone(),
                    control: match framework {
                        Framework::Soc2 => rule.soc2,
                        Framework::Iso27001 => rule.iso27001,
                    },
                    why: rule.why,
                });
            }
        }
    }
    let controls = catalogue
        .iter()
        .zip(seqs)
        .zip(counts)
        .map(|((control, seqs), counts)| ControlEvidence {
            id: control.id,
            title: control.title,
            total: seqs.len() as u64,
            counts,
            samples: samples(&seqs),
        })
        .collect();
    Evidence {
        framework,
        org: report.org.clone(),
        period,
        export: ExportSummary {
            first_seq: report.first_seq,
            last_seq: report.last_seq,
            events: report.events,
            head_hash: report.head_hash.clone(),
            checkpoints: report.checkpoints,
        },
        summary,
        controls,
        exceptions,
    }
}

const DAY: i64 = 86_400;

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 {
        y - 1
    } else {
        y
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 {
        mp + 3
    } else {
        mp - 9
    };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// The unix time of 00:00 UTC on a `YYYY-MM-DD` date.
#[must_use]
pub fn day_start(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |s: &str| -> Option<i64> { s.bytes().all(|c| c.is_ascii_digit()).then(|| s.parse().ok())? };
    let (y, m, d) = (num(&date[..4])?, num(&date[5..7])?, num(&date[8..])?);
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    // The last day of the month: the day before the 1st of the next.
    let (ny, nm) = if m == 12 {
        (y + 1, 1)
    } else {
        (y, m + 1)
    };
    if d > days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1) {
        return None;
    }
    Some(days_from_civil(y, m, d) * DAY)
}

/// The UTC date (`YYYY-MM-DD`) of a unix time.
#[must_use]
pub fn date_of(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(DAY));
    format!("{y:04}-{m:02}-{d:02}")
}

fn period_text(p: Period) -> String {
    match (p.from, p.to) {
        (None, None) => "the whole export".to_owned(),
        (Some(f), Some(t)) => format!("{} to {} (UTC)", date_of(f), date_of(t - 1)),
        (Some(f), None) => format!("from {} (UTC)", date_of(f)),
        (None, Some(t)) => format!("up to {} (UTC)", date_of(t - 1)),
    }
}

/// Text safe in a Markdown table cell.
fn cell(s: &str) -> String {
    s.replace('\\', "\\\\").replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// `1 event`, `2 events`.
fn count(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn seq_list(seqs: &[i64]) -> String {
    seqs.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
}

/// The report as Markdown, for an auditor to read or attach.
#[must_use]
pub fn render_markdown(e: &Evidence) -> String {
    let mut md = String::new();
    let x = &e.export;
    let _ = writeln!(md, "# {} evidence: {}\n", e.framework.short(), cell(&e.org));
    let _ = writeln!(
        md,
        "Made by reins-audit {} from an audit export that verified. The same export gives the same report.\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(md, "| | |\n|---|---|");
    let _ = writeln!(md, "| Organization | {} |", cell(&e.org));
    let _ = writeln!(md, "| Framework | {} |", e.framework.title());
    let _ = writeln!(md, "| Period | {} |", period_text(e.period));
    let _ = writeln!(
        md,
        "| Verified export | events {} to {} ({}, {}) |",
        x.first_seq,
        x.last_seq,
        count(x.events, "event"),
        count(x.checkpoints, "checkpoint")
    );
    let _ = writeln!(md, "| Head hash | `{}` |\n", cell(&x.head_hash));

    let s = &e.summary;
    let _ = writeln!(md, "## Summary\n\n| | Count |\n|---|---:|");
    for (label, n) in [
        ("Events in the period", s.events),
        ("Members provisioned", s.members_provisioned),
        ("Members deprovisioned", s.members_deprovisioned),
        ("Devices enrolled", s.devices_enrolled),
        ("Admin changes applied", s.admin_changes_applied),
        ("Admin changes approved by a second admin", s.admin_changes_two_person),
        ("Policy activations", s.policy_activations),
        ("Decisions, automatic", s.decisions_auto),
        ("Decisions, approved", s.decisions_approved),
        ("Decisions, denied", s.decisions_denied),
        ("Reviews opened", s.reviews_opened),
        ("Reviews expired", s.reviews_expired),
        ("Executions", s.executions),
        ("Leases issued", s.leases_issued),
        ("Leases revoked", s.leases_revoked),
    ] {
        let _ = writeln!(md, "| {label} | {n} |");
    }
    md.push('\n');

    for c in &e.controls {
        let _ = writeln!(md, "## {} {}\n", c.id, c.title);
        if c.total == 0 {
            let _ = writeln!(md, "No events in the period.\n");
            continue;
        }
        let _ = writeln!(md, "{}.\n\n| Kind | Count |\n|---|---:|", count(c.total, "event"));
        for (kind, n) in &c.counts {
            let _ = writeln!(md, "| {} | {n} |", cell(kind));
        }
        let _ = writeln!(md, "\nSample events: {}.\n", seq_list(&c.samples));
    }

    let _ = writeln!(md, "## Exceptions\n");
    if e.exceptions.is_empty() {
        let _ = writeln!(md, "None.");
    } else {
        let _ = writeln!(md, "| Event | Kind | Control | Why |\n|---:|---|---|---|");
        for x in &e.exceptions {
            let _ = writeln!(md, "| {} | {} | {} | {} |", x.seq, cell(&x.kind), x.control, x.why);
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use reins_proto::managed::AuditEvent;

    use super::*;
    use crate::tests::{Signer, chain, export, rehash};
    use crate::{Report, verify_collect};

    /// A verified export of events with these kinds (and details), at `1000 + seq`.
    fn verified(kinds: &[(&str, &str)]) -> (Report, Vec<AuditEvent>) {
        let s = Signer::new("dk1");
        let mut events = chain(i64::try_from(kinds.len()).unwrap());
        for (e, (kind, detail)) in events.iter_mut().zip(kinds) {
            e.kind = (*kind).to_owned();
            e.detail = (*detail).to_owned();
            e.at = 1000 + e.seq;
        }
        rehash(&mut events);
        verify_collect(export(&events, &s).as_bytes(), &[s.trusted()], None).unwrap()
    }

    fn sample_export() -> (Report, Vec<AuditEvent>) {
        verified(&[
            ("member.provisioned", "{}"),
            ("member.provisioned", "{}"),
            ("member.deprovisioned", "{}"),
            ("member.provisioned", "{}"),
            ("member.deprovisioned", "{\"kept_as_last_owner\":false}"),
            ("change.applied", "{\"action\":{},\"proposed_by\":\"m1\"}"),
        ])
    }

    fn control<'a>(e: &'a Evidence, id: &str) -> &'a ControlEvidence {
        e.controls.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn soc2_counts_and_samples_per_control() {
        let (report, events) = sample_export();
        let e = build(Framework::Soc2, &report, &events, Period::default());
        assert_eq!(control(&e, "CC6.1").total, 3);
        assert_eq!(control(&e, "CC6.3").counts["member.deprovisioned"], 2);
        assert_eq!(control(&e, "CC8.1").counts["change.applied"], 1);
        let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
        for c in &e.controls {
            assert!(c.samples.len() <= 5, "{}", c.id);
            assert!(c.samples.iter().all(|s| seqs.contains(s)), "{}", c.id);
        }
        assert_eq!(control(&e, "CC6.1").samples, [1, 2, 4]);
        assert_eq!((e.summary.members_provisioned, e.summary.members_deprovisioned), (3, 2));
        assert_eq!((e.summary.admin_changes_applied, e.summary.admin_changes_two_person), (1, 1));
        assert!(e.exceptions.is_empty());
    }

    #[test]
    fn samples_are_first_last_and_evenly_between() {
        let kinds = vec![("member.provisioned", "{}"); 9];
        let (report, events) = verified(&kinds);
        let e = build(Framework::Soc2, &report, &events, Period::default());
        assert_eq!(control(&e, "CC6.1").samples, [1, 3, 5, 7, 9]);
    }

    #[test]
    fn the_period_bounds_the_events() {
        let (report, events) = sample_export();
        // `at` is 1000 + seq: [1002, 1005) keeps events 2, 3 and 4.
        let period = Period {
            from: Some(1002),
            to: Some(1005),
        };
        let e = build(Framework::Soc2, &report, &events, period);
        assert_eq!(control(&e, "CC6.1").total, 2);
        assert_eq!(control(&e, "CC6.1").samples, [2, 4]);
        assert_eq!(control(&e, "CC6.3").total, 1);
        assert_eq!(control(&e, "CC8.1").total, 0);
        assert_eq!(e.summary.events, 3);
        assert_eq!(e.period, period);
        // The export is still the whole verified range.
        assert_eq!((e.export.first_seq, e.export.last_seq), (1, 6));
        assert_eq!(e.export.head_hash, report.head_hash);
        assert_eq!(e.export.checkpoints, 1);
    }

    #[test]
    fn a_sole_holder_change_is_an_exception() {
        let (report, events) = verified(&[
            ("member.joined", "{}"),
            ("change.applied", "{\"action\":{},\"sole_holder\":true}"),
            ("change.applied", "{\"action\":{},\"sole_holder\":false}"),
        ]);
        let e = build(Framework::Soc2, &report, &events, Period::default());
        assert_eq!(
            e.exceptions,
            [Exception {
                seq: 2,
                kind: "change.applied".into(),
                control: "CC8.1",
                why: "applied without a second person",
            }]
        );
        let iso = build(Framework::Iso27001, &report, &events, Period::default());
        assert_eq!(iso.exceptions.len(), 1);
        assert_eq!(iso.exceptions[0].control, "A.8.32");
    }

    #[test]
    fn a_kept_last_owner_and_a_body_read_are_exceptions() {
        let (report, events) =
            verified(&[("member.deprovisioned", "{\"kept_as_last_owner\":true}"), ("audit.read", "{}")]);
        let e = build(Framework::Soc2, &report, &events, Period::default());
        let got: Vec<(i64, &str)> = e.exceptions.iter().map(|x| (x.seq, x.control)).collect();
        assert_eq!(got, [(1, "CC6.3"), (2, "C1.2")]);
        let iso = build(Framework::Iso27001, &report, &events, Period::default());
        let got: Vec<(i64, &str)> = iso.exceptions.iter().map(|x| (x.seq, x.control)).collect();
        assert_eq!(got, [(1, "A.5.18"), (2, "A.8.15")]);
    }

    #[test]
    fn iso_controls_use_annex_a_ids() {
        let (report, events) = sample_export();
        let e = build(Framework::Iso27001, &report, &events, Period::default());
        assert_eq!(e.controls.len(), 10);
        assert!(e.controls.iter().all(|c| c.id.starts_with("A.")));
        assert_eq!(control(&e, "A.5.16").total, 3);
        assert_eq!(control(&e, "A.5.18").total, 2);
        let soc2 = build(Framework::Soc2, &report, &events, Period::default());
        assert!(
            soc2.controls.iter().all(|c| c.id.starts_with("CC") || c.id.starts_with("A1") || c.id.starts_with("C1"))
        );
    }

    #[test]
    fn prefixes_match_on_the_dot() {
        let (report, events) = verified(&[("lease.revoked_upstream", "{}"), ("policy_repo.bound", "{}")]);
        let e = build(Framework::Soc2, &report, &events, Period::default());
        assert_eq!(control(&e, "CC7.3").counts["lease.revoked_upstream"], 1);
        assert_eq!(control(&e, "CC8.1").counts["policy_repo.bound"], 1);
        // An event counts once per control, whichever patterns match it.
        assert_eq!(control(&e, "CC8.1").total, 1);
        // `policy.*` stops at the dot: ISO's A.8.32 has no `policy_repo.*`.
        let iso = build(Framework::Iso27001, &report, &events, Period::default());
        assert_eq!(control(&iso, "A.8.32").total, 0);
    }

    #[test]
    fn markdown_names_the_head_and_every_control() {
        let (report, events) =
            verified(&[("member.provisioned", "{}"), ("change.applied", "{\"action\":{},\"sole_holder\":true}")]);
        for framework in [Framework::Soc2, Framework::Iso27001] {
            let e = build(framework, &report, &events, Period::default());
            let md = render_markdown(&e);
            assert!(md.contains(&report.head_hash), "{md}");
            for c in &e.controls {
                assert!(md.contains(&format!("## {} ", c.id)), "{}: {md}", c.id);
            }
            assert!(md.contains("## Exceptions"), "{md}");
            assert!(md.contains("applied without a second person"), "{md}");
            assert!(md.contains(framework.title()), "{md}");
            // The same input renders the same bytes.
            assert_eq!(md, render_markdown(&build(framework, &report, &events, Period::default())));
        }
        let (report, events) = verified(&[("member.provisioned", "{}"), ("member.joined", "{}")]);
        let md = render_markdown(&build(Framework::Soc2, &report, &events, Period::default()));
        assert!(md.contains("## Exceptions\n\nNone."), "{md}");
    }

    #[test]
    fn markdown_shows_the_period_as_dates() {
        let (report, events) = sample_export();
        let period = Period {
            from: day_start("2026-10-01"),
            to: day_start("2026-11-01"),
        };
        let md = render_markdown(&build(Framework::Soc2, &report, &events, period));
        assert!(md.contains("2026-10-01 to 2026-10-31"), "{md}");
        assert!(md.contains("(6 events, 1 checkpoint)"), "{md}");
    }

    #[test]
    fn dates_are_utc_days() {
        assert_eq!(day_start("1970-01-01"), Some(0));
        assert_eq!(day_start("2026-10-27"), Some(1_793_059_200));
        assert_eq!(day_start("2024-02-29"), Some(1_709_164_800));
        assert_eq!(day_start("2023-02-29"), None);
        assert_eq!(day_start("2026-13-01"), None);
        assert_eq!(day_start("2026-1-01"), None);
        assert_eq!(day_start("yesterday"), None);
        assert_eq!(date_of(1_793_059_200 + 86_399), "2026-10-27");
        assert_eq!(date_of(1_709_164_800), "2024-02-29");
        assert_eq!(date_of(-1), "1969-12-31");
    }

    #[test]
    fn frameworks_parse() {
        assert_eq!(Framework::parse("soc2"), Some(Framework::Soc2));
        assert_eq!(Framework::parse("iso27001"), Some(Framework::Iso27001));
        assert_eq!(Framework::parse("pci"), None);
    }

    #[test]
    fn verify_collect_returns_the_events_in_order() {
        let s = Signer::new("dk1");
        let events = chain(5);
        let text = export(&events, &s);
        let (report, got) = verify_collect(text.as_bytes(), &[s.trusted()], None).unwrap();
        assert_eq!(got, events);
        assert_eq!(report, crate::verify(text.as_bytes(), &[s.trusted()], None).unwrap());
        // A failure returns no events.
        let mut edited = events;
        edited[1].detail = "{\"n\":99}".into();
        assert_eq!(
            verify_collect(export(&edited, &s).as_bytes(), &[s.trusted()], None).unwrap_err(),
            crate::VerifyError::BadHash {
                seq: 2
            }
        );
    }
}
