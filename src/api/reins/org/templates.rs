//! Org policy templates (05 §7.2), as [`OrgRule`] builders an author fills in and drafts (O2's `policy::save_draft`);
//! O4's source format renders them. Activation stays O2's two-person change.
//!
//! | Template | Rules |
//! |---|---|
//! | Developer baseline | O2's baseline, plus L3 pushes and branch deletions reviewed by the repository's code owners |
//! | Production change control | Infra or pushes on production: on-call and an engineer, Deferred 4 h, urgent; outside change windows also Security |
//! | Customer data | reads need a reason (else the cell asks); exports and sends: Security; Bypass never covers them |
//! | Finance | Money at or over a threshold: 2 of Finance, distinct people; a new payee: Finance and the requester's manager |
//! | Executive assistant | an exec's messages and writes: the exec or an assistant (04's Delegate, on org roles) |
//!
//! Ids are `tpl.<template>.<n>`: [`apply`] replaces a template's rules and keeps everything else.

use reins_policy::{
    effects::{EffectKind, Level},
    matrix::{Issuer, Rule, Then, When},
    org::{Hours, OrgPolicy, OrgRule, ReviewClass, ReviewSpec, Reviewers},
};
use serde::{Deserialize, Serialize};

/// A template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    DeveloperBaseline,
    ProductionChangeControl,
    CustomerData,
    Finance,
    ExecutiveAssistant,
}

impl Template {
    pub const ALL: [Self; 5] = [
        Self::DeveloperBaseline,
        Self::ProductionChangeControl,
        Self::CustomerData,
        Self::Finance,
        Self::ExecutiveAssistant,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeveloperBaseline => "developer_baseline",
            Self::ProductionChangeControl => "production_change_control",
            Self::CustomerData => "customer_data",
            Self::Finance => "finance",
            Self::ExecutiveAssistant => "executive_assistant",
        }
    }

    #[must_use]
    #[allow(dead_code, reason = "the console's templates page shows it")]
    pub fn label(self) -> &'static str {
        match self {
            Self::DeveloperBaseline => "Developer baseline",
            Self::ProductionChangeControl => "Production change control",
            Self::CustomerData => "Customer data",
            Self::Finance => "Finance",
            Self::ExecutiveAssistant => "Executive assistant",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    fn prefix(self) -> String {
        format!("tpl.{}.", self.as_str())
    }
}

/// What a template is filled in with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateParams {
    pub security_group: String,
    pub engineering_group: String,
    /// Resource globs (`acme/prod-*`).
    pub production: Vec<String>,
    pub change_windows: Vec<Hours>,
    pub customer_data: Vec<String>,
    pub finance_group: String,
    /// Minor units: 100 000 is $1,000.
    pub money_threshold_minor: u64,
    pub currency: String,
    pub finance_quorum: u8,
    /// User ids.
    pub exec_members: Vec<String>,
    pub assistant_group: String,
    pub oncall_alias: String,
}

impl Default for TemplateParams {
    fn default() -> Self {
        Self {
            security_group: "security".to_owned(),
            engineering_group: "engineering".to_owned(),
            production: vec!["prod-*".to_owned()],
            change_windows: Vec::new(),
            customer_data: Vec::new(),
            finance_group: "finance".to_owned(),
            money_threshold_minor: 100_000,
            currency: "USD".to_owned(),
            finance_quorum: 2,
            exec_members: Vec::new(),
            assistant_group: "assistants".to_owned(),
            oncall_alias: "production".to_owned(),
        }
    }
}

struct Builder {
    template: Template,
    rules: Vec<OrgRule>,
}

impl Builder {
    fn rule(&mut self, when: When, then: Then, review: Option<ReviewSpec>, explain: &str) -> &mut OrgRule {
        let id = format!("{}{}", self.template.prefix(), self.rules.len() + 1);
        self.rules.push(OrgRule::new(
            Rule {
                id,
                // `policy::parse_draft` sets the org.
                issuer: Issuer::Owner,
                when,
                then,
                not_after: None,
                accepted: false,
                explain: explain.to_owned(),
            },
            review,
        ));
        self.rules.last_mut().unwrap_or_else(|| unreachable!("just pushed"))
    }
}

fn on(effects: &[EffectKind], resource: Option<&str>) -> When {
    When {
        effects: effects.to_vec(),
        resource: resource.map(str::to_owned),
        ..When::default()
    }
}

fn deferred(r: Reviewers, hours: u64) -> ReviewSpec {
    ReviewSpec {
        window_secs: hours * 3600,
        ..ReviewSpec::by(r)
    }
}

/// The template's rules.
#[must_use]
pub fn build(t: Template, p: &TemplateParams) -> Vec<OrgRule> {
    let mut b = Builder {
        template: t,
        rules: Vec::new(),
    };
    match t {
        Template::DeveloperBaseline => {
            let when = When {
                levels: vec![Level::L3],
                ..on(&[EffectKind::GitPush, EffectKind::GitRefDelete], None)
            };
            b.rule(when, Then::Review, Some(deferred(Reviewers::CodeOwners, 4)), "Code owners review this.");
        }
        Template::ProductionChangeControl => {
            for glob in &p.production {
                let when = on(&[EffectKind::Infra, EffectKind::GitPush], Some(glob));
                let urgent = |r| ReviewSpec {
                    urgent: true,
                    ..deferred(r, 4)
                };
                b.rule(
                    when.clone(),
                    Then::Review,
                    Some(urgent(Reviewers::OnCall {
                        alias: p.oncall_alias.clone(),
                    })),
                    "Production: on-call approves.",
                );
                b.rule(
                    when.clone(),
                    Then::Review,
                    Some(urgent(Reviewers::Group(p.engineering_group.clone()))),
                    "Production: a second engineer approves.",
                );
                if !p.change_windows.is_empty() {
                    b.rule(
                        when,
                        Then::Review,
                        Some(deferred(Reviewers::Group(p.security_group.clone()), 4)),
                        "Outside change windows: Security approves too.",
                    )
                    .outside_hours
                    .clone_from(&p.change_windows);
                }
            }
        }
        Template::CustomerData => {
            for glob in &p.customer_data {
                b.rule(on(&[EffectKind::DataRead], Some(glob)), Then::Permit, None, "Customer data: say why.")
                    .require_reason = true;
                b.rule(
                    on(
                        &[EffectKind::SecretOut, EffectKind::Message, EffectKind::Publish, EffectKind::DataWrite],
                        Some(glob),
                    ),
                    Then::Review,
                    Some(deferred(Reviewers::Group(p.security_group.clone()), 4)),
                    "Customer data leaving: Security approves. Bypass never covers this.",
                );
            }
        }
        Template::Finance => {
            let mut over = ReviewSpec {
                quorum: p.finance_quorum.max(1),
                distinct_persons: true,
                ..deferred(Reviewers::Group(p.finance_group.clone()), 24)
            };
            over.class = ReviewClass::Deferred;
            let r =
                b.rule(on(&[EffectKind::Money], None), Then::Review, Some(over), "Large payments: Finance approves.");
            r.min_amount_minor = Some(p.money_threshold_minor);
            r.currency = Some(p.currency.clone());
            b.rule(
                on(&[EffectKind::Money], None),
                Then::Review,
                Some(deferred(Reviewers::Group(p.finance_group.clone()), 24)),
                "New payee: Finance approves.",
            )
            .new_party = true;
            b.rule(
                on(&[EffectKind::Money], None),
                Then::Review,
                Some(deferred(Reviewers::Manager, 24)),
                "New payee: your manager approves.",
            )
            .new_party = true;
        }
        Template::ExecutiveAssistant => {
            if !p.exec_members.is_empty() {
                let spec = ReviewSpec {
                    exclude_requester: false,
                    class: ReviewClass::Instant,
                    window_secs: 0,
                    ..ReviewSpec::by(Reviewers::AnyOf(vec![
                        Reviewers::Requester,
                        Reviewers::Group(p.assistant_group.clone()),
                    ]))
                };
                b.rule(
                    on(&[EffectKind::Message, EffectKind::DataWrite], None),
                    Then::Review,
                    Some(spec),
                    "The exec or an assistant decides.",
                )
                .members
                .clone_from(&p.exec_members);
            }
        }
    }
    b.rules
}

/// `policy` with `rules` in place of the same templates' earlier rules (every other rule kept, in order).
#[must_use]
pub fn apply(policy: &OrgPolicy, rules: Vec<OrgRule>) -> OrgPolicy {
    let prefixes: Vec<String> = Template::ALL
        .iter()
        .map(|t| t.prefix())
        .filter(|p| rules.iter().any(|r| r.rule.id.starts_with(p.as_str())))
        .collect();
    let mut out = policy.clone();
    out.rules.retain(|r| !prefixes.iter().any(|p| r.rule.id.starts_with(p.as_str())));
    out.rules.extend(rules);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use reins_policy::org::{
        Approval, ManagedOutcome, ManagedRequest, Met, evaluate_managed, obligations_met_resolved,
    };
    use serde_json::{Value, json};

    use super::*;

    fn params() -> TemplateParams {
        TemplateParams {
            security_group: "sec".into(),
            engineering_group: "eng".into(),
            production: vec!["acme/prod-*".into()],
            change_windows: vec![Hours {
                days: vec![chrono::Weekday::Mon, chrono::Weekday::Tue, chrono::Weekday::Wed, chrono::Weekday::Thu],
                start: "09:00".into(),
                end: "17:00".into(),
                tz: "Europe/Berlin".into(),
            }],
            customer_data: vec!["acme/customers-*".into()],
            finance_group: "fin".into(),
            exec_members: vec!["ceo".into()],
            assistant_group: "asst".into(),
            oncall_alias: "payments".into(),
            ..TemplateParams::default()
        }
    }

    fn baseline() -> OrgPolicy {
        let mut p: OrgPolicy = serde_json::from_str(crate::api::reins::org::policy::DEVELOPER_BASELINE).unwrap();
        "o1".clone_into(&mut p.org);
        p.facts.set(reins_policy::context::FactKind::Owner, "acme", reins_policy::context::Zone::Work);
        p
    }

    /// The baseline with `templates` applied, every rule issued by o1.
    fn with(templates: &[Template]) -> OrgPolicy {
        let mut p = baseline();
        for t in templates {
            p = apply(&p, build(*t, &params()));
        }
        for r in &mut p.rules {
            r.rule.issuer = Issuer::Org("o1".into());
        }
        p
    }

    fn eval(
        p: &OrgPolicy,
        member: &str,
        tool: &str,
        args: &Value,
        now: i64,
        reason: Option<&str>,
    ) -> reins_policy::org::ManagedVerdict {
        evaluate_managed(
            &ManagedRequest {
                org: "o1",
                member,
                connection: "c1",
                tool,
                args,
                now,
                reason,
            },
            p,
            None,
        )
    }

    fn specs(v: &reins_policy::org::ManagedVerdict) -> Vec<(Reviewers, u8)> {
        v.obligations.iter().map(|o| (o.spec.reviewers.clone(), o.spec.quorum)).collect()
    }

    // A Monday, 10:00 in Berlin (2026-03-23T09:00Z), and the same day at 20:00.
    const IN_HOURS: i64 = 1_774_256_400;
    const AFTER_HOURS: i64 = IN_HOURS + 10 * 3600;

    #[test]
    fn templates_match_their_golden_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/reins/org/testdata/templates");
        for t in Template::ALL {
            let got = serde_json::to_string_pretty(&build(t, &params())).unwrap() + "\n";
            let file = dir.join(format!("{}.json", t.as_str()));
            if std::env::var("REINS_UPDATE_GOLDEN").is_ok() {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&file, &got).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&file).unwrap(), got, "{}", t.as_str());
        }
    }

    #[test]
    fn developer_baseline_equals_o2s() {
        let o2 = baseline();
        let p = apply(&o2, build(Template::DeveloperBaseline, &params()));
        // Everything O2's baseline says, unchanged, plus the code owners rule.
        assert_eq!(
            (p.matrix.clone(), p.cell_review.clone(), p.posture.clone()),
            (o2.matrix.clone(), o2.cell_review.clone(), o2.posture.clone())
        );
        assert_eq!(p.rules.len(), o2.rules.len() + 1);
        assert_eq!(p.rules.last().unwrap().review.as_ref().unwrap().reviewers, Reviewers::CodeOwners);
        // Applying again replaces, never duplicates.
        assert_eq!(apply(&p, build(Template::DeveloperBaseline, &params())).rules.len(), p.rules.len());
    }

    fn purchase(total: &str, url: &str) -> Value {
        json!({"merchant": "Vendor", "merchant_url": url, "currency": "USD", "total": total,
               "items": [{"name": "Thing", "quantity": 1, "unit_price": total}]})
    }

    #[test]
    fn finance_over_threshold_needs_two_of_finance() {
        let mut p = with(&[Template::Finance]);
        p.facts.set(reins_policy::context::FactKind::Domain, "vendor.example", reins_policy::context::Zone::Work);
        let v = eval(
            &p,
            "lee",
            "payments_purchase_request",
            &purchase("1500.00", "https://shop.vendor.example/x"),
            1,
            None,
        );
        assert!(specs(&v).contains(&(Reviewers::Group("fin".into()), 2)), "{:?}", specs(&v));
        let o = v.obligations.iter().find(|o| o.spec.quorum == 2).unwrap().clone();
        let fin = vec!["fin".to_owned()];
        let a = |m: &'static str| Approval {
            member: m,
            groups: &fin,
            roles: &[],
            approve: true,
        };
        let one = [o.clone()];
        assert!(matches!(obligations_met_resolved(&one, "lee", &[a("f1")], &BTreeMap::new()), Met::Waiting(_)));
        assert_eq!(obligations_met_resolved(&one, "lee", &[a("f1"), a("f2")], &BTreeMap::new()), Met::Yes);
        // Under the threshold, from a known payee: no Finance review.
        let v =
            eval(&p, "lee", "payments_purchase_request", &purchase("10.00", "https://shop.vendor.example/x"), 1, None);
        assert!(!specs(&v).iter().any(|(r, _)| *r == Reviewers::Group("fin".into())), "{:?}", specs(&v));
    }

    #[test]
    fn new_payee_needs_finance_and_manager() {
        let p = with(&[Template::Finance]);
        let v =
            eval(&p, "lee", "payments_purchase_request", &purchase("10.00", "https://never-seen.example/x"), 1, None);
        let s = specs(&v);
        assert!(s.contains(&(Reviewers::Group("fin".into()), 1)) && s.contains(&(Reviewers::Manager, 1)), "{s:?}");
        // Two obligations: the manager can't count for Finance, nor one person for both.
        let mgr = [v.obligations.iter().find(|o| o.spec.reviewers == Reviewers::Manager).unwrap().clone()];
        let resolved: BTreeMap<String, std::collections::BTreeSet<String>> =
            [(mgr[0].id.clone(), ["mia".to_owned()].into())].into();
        let none: Vec<String> = Vec::new();
        let mia = Approval {
            member: "mia",
            groups: &none,
            roles: &[],
            approve: true,
        };
        assert!(matches!(obligations_met_resolved(&v.obligations, "lee", &[mia], &resolved), Met::Waiting(_)));
    }

    fn push(repo: &str) -> Value {
        json!({"method": "PUT", "path": format!("/repos/{repo}/contents/deploy.yaml"), "body": {"message": "x"}})
    }

    #[test]
    fn prod_infra_needs_on_call_and_an_engineer_deferred_4h() {
        let p = with(&[Template::ProductionChangeControl]);
        let v = eval(&p, "lee", "github_api_write", &push("acme/prod-db"), IN_HOURS, None);
        assert_eq!(v.outcome, ManagedOutcome::Review);
        let oncall = v.obligations.iter().find(|o| matches!(o.spec.reviewers, Reviewers::OnCall { .. })).unwrap();
        let eng = v.obligations.iter().find(|o| o.spec.reviewers == Reviewers::Group("eng".into())).unwrap();
        for o in [oncall, eng] {
            assert_eq!((o.spec.class, o.spec.window(), o.spec.urgent), (ReviewClass::Deferred, 4 * 3600, true));
        }
        assert!(!specs(&v).iter().any(|(r, _)| *r == Reviewers::Group("sec".into())), "within change windows");
        // Not production: none of it.
        let v = eval(&p, "lee", "github_api_write", &push("acme/api"), IN_HOURS, None);
        assert!(!v.obligations.iter().any(|o| matches!(o.spec.reviewers, Reviewers::OnCall { .. })));
    }

    #[test]
    fn outside_change_windows_adds_security() {
        let p = with(&[Template::ProductionChangeControl]);
        let v = eval(&p, "lee", "github_api_write", &push("acme/prod-db"), AFTER_HOURS, None);
        assert!(specs(&v).iter().any(|(r, _)| *r == Reviewers::Group("sec".into())), "{:?}", specs(&v));
    }

    fn read_lease(repo: &str) -> Value {
        reins_proto::managed::lease_args(repo, reins_proto::managed::LeaseAccess::Read, "BOX")
    }

    #[test]
    fn customer_data_read_without_reason_is_reviewed() {
        let mut p = with(&[Template::CustomerData]);
        // Customer data is reviewed by the cell unless a reason is given (make the cell ask for Work L0).
        p.matrix.work[0] = reins_policy::matrix::Cell::Ask;
        let tool = reins_proto::managed::LEASE_TOOL;
        let v = eval(&p, "lee", tool, &read_lease("acme/customers-db"), 1, None);
        assert_eq!(v.outcome, ManagedOutcome::Review, "{v:?}");
        let v = eval(&p, "lee", tool, &read_lease("acme/customers-db"), 1, Some("Ticket 812: a refund"));
        assert_eq!(v.outcome, ManagedOutcome::Auto, "{v:?}");
    }

    #[test]
    fn exports_need_security() {
        let p = with(&[Template::CustomerData]);
        let v = eval(&p, "lee", "github_api_write", &push("acme/customers-db"), 1, Some("needed"));
        // Writing there needs Security, reason or not.
        let w = eval(
            &p,
            "lee",
            "git_lease",
            &reins_proto::managed::lease_args("acme/customers-db", reins_proto::managed::LeaseAccess::Push, "B"),
            1,
            Some("x"),
        );
        assert!(specs(&w).iter().any(|(r, _)| *r == Reviewers::Group("sec".into())), "{:?}", specs(&w));
        let _ = &v;
        assert!(p.rules.iter().any(|r| r.rule.explain.contains("Bypass never")));
    }

    #[test]
    fn assistants_decide_for_their_exec() {
        let p = with(&[Template::ExecutiveAssistant]);
        let send = json!({"to": "partner@else.example", "subject": "Lunch", "body": "x"});
        let v = eval(&p, "ceo", "gmail_send", &send, 1, None);
        let o = v.obligations.iter().find(|o| matches!(o.spec.reviewers, Reviewers::AnyOf(_))).unwrap().clone();
        let asst = vec!["asst".to_owned()];
        let none: Vec<String> = Vec::new();
        let ap = |m: &'static str, g: &'static [String]| Approval {
            member: m,
            groups: g,
            roles: &[],
            approve: true,
        };
        let asst: &'static [String] = Box::leak(asst.into_boxed_slice());
        let none: &'static [String] = Box::leak(none.into_boxed_slice());
        let one = [o];
        assert_eq!(obligations_met_resolved(&one, "ceo", &[ap("amy", asst)], &BTreeMap::new()), Met::Yes);
        assert_eq!(obligations_met_resolved(&one, "ceo", &[ap("ceo", none)], &BTreeMap::new()), Met::Yes);
        assert!(matches!(obligations_met_resolved(&one, "ceo", &[ap("bob", none)], &BTreeMap::new()), Met::Waiting(_)));
        // Someone else's mail isn't covered.
        let v = eval(&p, "lee", "gmail_send", &send, 1, None);
        assert!(!v.obligations.iter().any(|o| matches!(o.spec.reviewers, Reviewers::AnyOf(_))));
    }

    #[test]
    fn every_template_validates() {
        let p = with(&Template::ALL);
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        for t in Template::ALL {
            assert_eq!(Template::parse(t.as_str()), Some(t));
            assert!(build(t, &params()).iter().all(|r| r.rule.id.starts_with(&format!("tpl.{}.", t.as_str()))));
        }
    }
}
