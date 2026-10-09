use std::collections::BTreeSet;

use reins_proto::gmail::OutgoingEmail;
use reins_proto::ids::{ConnectionId, GrantId};

use crate::{Grant, MessageFacts, ReadScope, Scope};

/// Per-message outcome of a read or search.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReadDecision {
    /// `(index into the evaluated messages, grant that covers it)`.
    pub allowed: Vec<(usize, GrantId)>,
    /// Indices of messages no active grant covers.
    pub needs_approval: Vec<usize>,
}

impl ReadDecision {
    #[must_use]
    pub fn fully_allowed(&self) -> bool {
        self.needs_approval.is_empty()
    }

    #[must_use]
    pub fn grants_used(&self) -> BTreeSet<GrantId> {
        self.allowed.iter().map(|(_, g)| g.clone()).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendDecision {
    Allowed(GrantId),
    NeedsApproval,
}

/// Active grants of `connection`, unlimited ones first, then by id.
fn candidates<'g>(grants: &'g [Grant], connection: &ConnectionId, now: i64) -> Vec<&'g Grant> {
    let mut active: Vec<&Grant> = grants.iter().filter(|g| g.applies_to(connection, now)).collect();
    active.sort_by(|a, b| (a.max_uses.is_some(), &a.id).cmp(&(b.max_uses.is_some(), &b.id)));
    active
}

#[must_use]
pub fn evaluate_read(grants: &[Grant], connection: &ConnectionId, messages: &[MessageFacts], now: i64) -> ReadDecision {
    let reads: Vec<(&Grant, &ReadScope)> = candidates(grants, connection, now)
        .into_iter()
        .filter_map(|g| match &g.scope {
            Scope::Read(s) => Some((g, s)),
            Scope::Send(_) | Scope::Accounts(_) | Scope::Service(_) => None,
        })
        .collect();
    let mut decision = ReadDecision::default();
    for (index, message) in messages.iter().enumerate() {
        match reads.iter().find(|(_, scope)| scope.matches(message)) {
            Some((grant, _)) => decision.allowed.push((index, grant.id.clone())),
            None => decision.needs_approval.push(index),
        }
    }
    decision
}

/// `email` must be normalized (`OutgoingEmail::normalized`).
#[must_use]
pub fn evaluate_send(grants: &[Grant], connection: &ConnectionId, email: &OutgoingEmail, now: i64) -> SendDecision {
    candidates(grants, connection, now)
        .into_iter()
        .find(|g| matches!(&g.scope, Scope::Send(s) if s.matches(email)))
        .map_or(SendDecision::NeedsApproval, |g| SendDecision::Allowed(g.id.clone()))
}

/// The active grant that lets `connection`, acting as `account`, do `access` ("list", "read" or "write") to `resource` of
/// `service` without asking, if there is one. Unlimited grants are preferred, so a capped grant is not spent needlessly.
#[must_use]
#[allow(clippy::too_many_arguments, reason = "a lookup is described by this many independent facts")]
pub fn service_allows(
    grants: &[Grant],
    connection: &ConnectionId,
    account: Option<&str>,
    service: &str,
    access: &str,
    class: &str,
    resource: &str,
    now: i64,
) -> Option<GrantId> {
    candidates(grants, connection, now)
        .into_iter()
        .find(|g| {
            g.covers_account(account)
                && matches!(&g.scope, Scope::Service(s) if s.covers(service, access, class, resource))
        })
        .map(|g| g.id.clone())
}

/// Which accounts of one integration `connection` may be shown without asking, and by which grants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountCoverage {
    /// A grant covers every account, whichever they are.
    pub all: bool,
    /// The accounts named by grants.
    pub accounts: BTreeSet<String>,
    /// The grants that contribute, each with the accounts it names (empty = all).
    pub grants: Vec<(GrantId, Vec<String>)>,
}

impl AccountCoverage {
    #[must_use]
    pub fn covers(&self, account: &str) -> bool {
        self.all || self.accounts.contains(account)
    }
}

/// The accounts of `service` that active grants let `connection` see.
#[must_use]
pub fn account_coverage(grants: &[Grant], connection: &ConnectionId, service: &str, now: i64) -> AccountCoverage {
    let mut coverage = AccountCoverage::default();
    for grant in candidates(grants, connection, now) {
        if let Scope::Accounts(s) = &grant.scope
            && s.service == service
        {
            if s.accounts.is_empty() {
                coverage.all = true;
            }
            coverage.accounts.extend(s.accounts.iter().cloned());
            coverage.grants.push((grant.id.clone(), s.accounts.clone()));
        }
    }
    coverage
}

/// Whether message bodies must be fetched before `evaluate_read` can decide.
#[must_use]
pub fn needs_body(grants: &[Grant], connection: &ConnectionId, now: i64) -> bool {
    candidates(grants, connection, now).iter().any(|g| matches!(&g.scope, Scope::Read(s) if s.needs_body()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddrRule, Pattern, SendScope};

    const NOW: i64 = 1_000;

    fn read_grant(id: &str, conn: &str, scope: ReadScope, max_uses: Option<u32>) -> Grant {
        Grant::new(id.into(), conn.into(), Scope::Read(scope), 0, None, max_uses).unwrap()
    }

    fn from_domain(d: &str) -> ReadScope {
        ReadScope {
            from: vec![AddrRule::domain(d).unwrap()],
            ..ReadScope::default()
        }
    }

    fn msg(id: &str, from: &str) -> MessageFacts {
        MessageFacts {
            id: id.to_owned(),
            from: from.to_owned(),
            to: vec![],
            subject: "s".to_owned(),
            body: None,
            labels: vec![],
            date: 10,
        }
    }

    fn email(to: &[&str]) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: "s".to_owned(),
            body: "b".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn partial_read() {
        let grants = [read_grant("g1", "c1", from_domain("bank.com"), None)];
        let msgs = [msg("m1", "a@bank.com"), msg("m2", "eve@evil.com")];
        let d = evaluate_read(&grants, &"c1".into(), &msgs, NOW);
        assert_eq!(d.allowed, vec![(0, "g1".into())]);
        assert_eq!(d.needs_approval, vec![1]);
        assert!(!d.fully_allowed());
    }

    #[test]
    fn other_connection_never_applies() {
        let grants = [read_grant("g1", "c2", from_domain("bank.com"), None)];
        let d = evaluate_read(&grants, &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
        assert!(d.allowed.is_empty());
    }

    #[test]
    fn inactive_grants_ignored() {
        let mut expired = read_grant("g1", "c1", from_domain("bank.com"), None);
        expired.expires_at = Some(NOW);
        let mut exhausted = read_grant("g2", "c1", from_domain("bank.com"), Some(1));
        exhausted.uses = 1;
        let mut revoked = read_grant("g3", "c1", from_domain("bank.com"), None);
        revoked.revoked = true;
        let d = evaluate_read(&[expired, exhausted, revoked], &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
    }

    #[test]
    fn prefers_unlimited_grant() {
        let grants = [
            read_grant("a-limited", "c1", from_domain("bank.com"), Some(3)),
            read_grant("z-unlimited", "c1", from_domain("bank.com"), None),
        ];
        let d = evaluate_read(&grants, &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.grants_used(), ["z-unlimited".into()].into());
    }

    #[test]
    fn send_grant_does_not_cover_reads() {
        let send = Grant::new(
            "g1".into(),
            "c1".into(),
            Scope::Send(SendScope {
                recipients: vec![AddrRule::domain("bank.com").unwrap()],
                subject: None,
                body: None,
            }),
            0,
            None,
            None,
        )
        .unwrap();
        let d = evaluate_read(&[send], &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
    }

    #[test]
    fn send_decisions() {
        let g = Grant::new(
            "g1".into(),
            "c1".into(),
            Scope::Send(SendScope {
                recipients: vec![AddrRule::domain("work.com").unwrap()],
                subject: None,
                body: None,
            }),
            0,
            None,
            None,
        )
        .unwrap();
        let grants = [g, read_grant("g2", "c1", from_domain("work.com"), None)];
        assert_eq!(
            evaluate_send(&grants, &"c1".into(), &email(&["a@work.com"]), NOW),
            SendDecision::Allowed("g1".into())
        );
        assert_eq!(evaluate_send(&grants, &"c1".into(), &email(&["a@evil.com"]), NOW), SendDecision::NeedsApproval);
        assert_eq!(evaluate_send(&grants, &"c2".into(), &email(&["a@work.com"]), NOW), SendDecision::NeedsApproval);
    }

    #[test]
    fn needs_body_only_for_active_body_grants() {
        let body = ReadScope {
            body: Some(Pattern::find("otp").unwrap()),
            ..ReadScope::default()
        };
        let grants = [read_grant("g1", "c1", body, None)];
        assert!(needs_body(&grants, &"c1".into(), NOW));
        assert!(!needs_body(&grants, &"c2".into(), NOW));
    }

    fn accounts_grant(id: &str, service: &str, accounts: &[&str]) -> Grant {
        Grant::new(
            id.into(),
            "c1".into(),
            Scope::Accounts(crate::AccountsScope {
                service: service.to_owned(),
                accounts: accounts.iter().map(|a| (*a).to_owned()).collect(),
            }),
            0,
            Some(100),
            None,
        )
        .unwrap()
    }

    #[test]
    fn account_coverage_is_the_union_of_what_the_grants_name() {
        let grants =
            [accounts_grant("a1", "gmail", &["a@x.com", "b@x.com"]), accounts_grant("a2", "gmail", &["c@x.com"])];
        let c = account_coverage(&grants, &"c1".into(), "gmail", 50);
        assert!(!c.all);
        assert_eq!(c.accounts.iter().map(String::as_str).collect::<Vec<_>>(), ["a@x.com", "b@x.com", "c@x.com"]);
        assert!(c.covers("b@x.com") && !c.covers("d@x.com"));
        assert_eq!(c.grants.len(), 2);
        assert_eq!(
            account_coverage(&grants, &"c1".into(), "drive", 50),
            AccountCoverage::default(),
            "another integration"
        );
        assert_eq!(account_coverage(&grants, &"c2".into(), "gmail", 50), AccountCoverage::default(), "another AI");
        assert_eq!(account_coverage(&grants, &"c1".into(), "gmail", 100), AccountCoverage::default(), "expired");
        assert!(!needs_body(&grants, &"c1".into(), 50));
        assert!(evaluate_read(&grants, &"c1".into(), &[], 50).allowed.is_empty());
    }

    #[test]
    fn a_grant_without_named_accounts_covers_all_of_them() {
        let c = account_coverage(&[accounts_grant("a1", "gmail", &[])], &"c1".into(), "gmail", 50);
        assert!(c.all && c.covers("anyone@x.com"));
    }

    #[test]
    fn a_badly_named_integration_is_not_a_scope() {
        for bad in ["", "Gmail", "gm ail", &"x".repeat(33)] {
            let scope = Scope::Accounts(crate::AccountsScope {
                service: bad.to_owned(),
                accounts: Vec::new(),
            });
            assert!(scope.validate().is_err(), "{bad:?}");
        }
    }

    fn service_grant(
        id: &str,
        access: &str,
        resources: &[&str],
        any: bool,
        expires: Option<i64>,
        account: Option<&str>,
    ) -> Grant {
        Grant::new(
            id.into(),
            "c1".into(),
            Scope::Service(crate::ServiceScope {
                service: "telegram".to_owned(),
                access: access.to_owned(),
                resources: resources.iter().map(|r| (*r).to_owned()).collect(),
                labels: vec![],
                any,
                classes: vec![],
            }),
            0,
            expires,
            None,
        )
        .unwrap()
        .for_account(account.map(str::to_owned))
    }

    #[test]
    fn a_service_grant_covers_only_its_access_and_its_things() {
        let grants = [service_grant("s1", "read", &["chat-1", "chat-2"], false, Some(100), Some("+1555"))];
        let allows = |account: Option<&str>, service: &str, access: &str, resource: &str, now: i64| {
            service_allows(&grants, &"c1".into(), account, service, access, "", resource, now)
        };
        assert_eq!(allows(Some("+1555"), "telegram", "read", "chat-2", 50), Some("s1".into()));
        assert_eq!(allows(Some("+1555"), "telegram", "read", "chat-3", 50), None, "another chat");
        assert_eq!(allows(Some("+1555"), "telegram", "write", "chat-1", 50), None, "reading is not writing");
        assert_eq!(
            allows(Some("+1555"), "telegram", "list", "chat-1", 50),
            Some("s1".into()),
            "reading includes listing"
        );
        assert_eq!(allows(Some("+1555"), "telegram", "list", "chat-3", 50), None, "but only of the same things");
        assert_eq!(allows(Some("+1555"), "github", "read", "chat-1", 50), None, "another integration");
        assert_eq!(allows(Some("+1999"), "telegram", "read", "chat-1", 50), None, "another account");
        assert_eq!(allows(Some("+1555"), "telegram", "read", "chat-1", 100), None, "expired");
        assert!(!needs_body(&grants, &"c1".into(), 50));
    }

    #[test]
    fn a_grant_for_everything_needs_a_short_limit_and_is_never_for_writing() {
        assert!(
            Grant::new(
                "a".into(),
                "c1".into(),
                Scope::Service(crate::ServiceScope {
                    service: "telegram".into(),
                    access: "read".into(),
                    resources: vec![],
                    labels: vec![],
                    any: true,
                    classes: vec![],
                }),
                0,
                None,
                None
            )
            .is_err()
        );
        assert!(
            Grant::new(
                "a".into(),
                "c1".into(),
                Scope::Service(crate::ServiceScope {
                    service: "telegram".into(),
                    access: "read".into(),
                    resources: vec![],
                    labels: vec![],
                    any: true,
                    classes: vec![],
                }),
                0,
                Some(8 * 86_400),
                None
            )
            .is_err()
        );
        let all = service_grant("a", "read", &[], true, Some(86_400), None);
        assert!(
            service_allows(&[all], &"c1".into(), Some("x"), "telegram", "read", "", "anything", 5).is_some(),
            "everything, any account"
        );
        for scope in [
            crate::ServiceScope {
                service: "telegram".into(),
                access: "write".into(),
                resources: vec![],
                labels: vec![],
                any: true,
                classes: vec![],
            },
            crate::ServiceScope {
                service: "telegram".into(),
                access: "read".into(),
                resources: vec![],
                labels: vec![],
                any: false,
                classes: vec![],
            },
            crate::ServiceScope {
                service: "telegram".into(),
                access: "read".into(),
                resources: vec!["x".into()],
                labels: vec![],
                any: true,
                classes: vec![],
            },
            crate::ServiceScope {
                service: "Telegram".into(),
                access: "read".into(),
                resources: vec!["x".into()],
                labels: vec![],
                any: false,
                classes: vec![],
            },
            crate::ServiceScope {
                service: "telegram".into(),
                access: "delete".into(),
                resources: vec!["x".into()],
                labels: vec![],
                any: false,
                classes: vec![],
            },
            crate::ServiceScope {
                service: "telegram".into(),
                access: "read".into(),
                resources: vec![" ".into()],
                labels: vec![],
                any: false,
                classes: vec![],
            },
        ] {
            assert!(Scope::Service(scope.clone()).validate().is_err(), "{scope:?}");
        }
    }

    fn github_grant(access: &str, classes: &[&str], resources: &[&str]) -> Grant {
        Grant::new(
            "g".into(),
            "c1".into(),
            Scope::Service(crate::ServiceScope {
                service: "github".to_owned(),
                access: access.to_owned(),
                resources: resources.iter().map(|r| (*r).to_owned()).collect(),
                labels: vec![],
                any: false,
                classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            }),
            0,
            Some(100),
            None,
        )
        .unwrap()
    }

    #[test]
    fn a_permission_can_name_the_repository_the_branch_and_the_kind_of_change() {
        let allows = |g: &Grant, class: &str, resource: &str| {
            service_allows(std::slice::from_ref(g), &"c1".into(), None, "github", "write", class, resource, 5).is_some()
        };
        let repo = github_grant("write", &["issues", "code"], &["me/app"]);
        assert!(allows(&repo, "issues", "me/app"));
        assert!(allows(&repo, "code", "me/app@main"), "a repository covers its branches");
        assert!(!allows(&repo, "releases", "me/app"), "a kind of change that was not named");
        assert!(!allows(&repo, "issues", "me/other"));
        assert!(!allows(&repo, "issues", "me/application"), "a part is a whole name, not a prefix");
        let branch = github_grant("write", &["code"], &["me/app@dev"]);
        assert!(allows(&branch, "code", "me/app@dev"));
        assert!(allows(&branch, "code", "me/app@dev/topic"), "a branch name with slashes is under its prefix");
        assert!(!allows(&branch, "code", "me/app@main"));
        assert!(!allows(&branch, "code", "me/app"), "the repository as a whole is more than one branch");
        let owner = github_grant("write", &[], &["me"]);
        assert!(allows(&owner, "releases", "me/app"), "no kinds named: every kind");
        assert!(allows(&owner, "code", "me/other@main"));
        assert!(!allows(&owner, "code", "you/app"));
    }

    #[test]
    fn kinds_of_change_are_only_for_writes_and_named_cleanly() {
        let scope = |access: &str, classes: &[&str]| {
            Scope::Service(crate::ServiceScope {
                service: "github".into(),
                access: access.into(),
                resources: vec!["me/app".into()],
                labels: vec![],
                any: false,
                classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            })
        };
        assert!(scope("write", &["issues"]).validate().is_ok());
        assert!(scope("read", &["issues"]).validate().is_err());
        assert!(scope("write", &["Issues"]).validate().is_err());
        assert!(scope("write", &["issues", "issues"]).validate().is_err());
        assert!(scope("write", &[""]).validate().is_err());
    }

    #[test]
    fn hierarchy_needs_whole_parts() {
        use crate::resource_covers;
        assert!(resource_covers("a", "a"));
        assert!(resource_covers("a", "a/b"));
        assert!(resource_covers("a/b", "a/b@c"));
        assert!(resource_covers("f1", "f1/login/x"));
        assert!(!resource_covers("a", "ab"));
        assert!(!resource_covers("a/b@c", "a/b"));
        assert!(!resource_covers("", "a"), "nothing is covered by an empty name");
    }
}
