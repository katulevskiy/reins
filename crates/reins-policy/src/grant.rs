use std::collections::BTreeSet;

use reins_proto::ids::{ConnectionId, GrantId};
use serde::{Deserialize, Serialize};

use crate::{PolicyError, Scope};

/// A standing permission for one AI connection, created on the phone.
/// No `expires_at` and no `max_uses` means it lasts until revoked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: GrantId,
    pub connection_id: ConnectionId,
    pub scope: Scope,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; the grant is dead at and after this instant.
    pub expires_at: Option<i64>,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub revoked: bool,
    /// The connected account (e.g. a Gmail address) the grant is for. `None` only for grants made before accounts
    /// existed and not yet bound to one; those cover the phone's first account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// The longest a grant that covers all of a user's mail may live.
pub const MAX_ANY_MAIL_SECS: i64 = 30 * 86_400;

impl Grant {
    pub fn new(
        id: GrantId,
        connection_id: ConnectionId,
        scope: Scope,
        created_at: i64,
        expires_at: Option<i64>,
        max_uses: Option<u32>,
    ) -> Result<Self, PolicyError> {
        scope.validate()?;
        if let Scope::Service(s) = &scope
            && s.any
            && !matches!(expires_at, Some(t) if t.saturating_sub(created_at) <= crate::scope::MAX_ANY_RESOURCE_SECS)
        {
            return Err(PolicyError::InvalidGrant(format!(
                "a grant for everything must expire within {} days",
                crate::scope::MAX_ANY_RESOURCE_SECS / 86_400
            )));
        }
        if matches!(&scope, Scope::Read(r) if r.any) {
            match expires_at {
                Some(t) if t.saturating_sub(created_at) <= MAX_ANY_MAIL_SECS => {}
                _ => {
                    return Err(PolicyError::InvalidGrant(format!(
                        "a grant for all mail must expire within {} days",
                        MAX_ANY_MAIL_SECS / 86_400
                    )));
                }
            }
        }
        if expires_at.is_some_and(|t| t <= created_at) {
            return Err(PolicyError::InvalidGrant("expires_at must be after created_at".to_owned()));
        }
        if max_uses == Some(0) {
            return Err(PolicyError::InvalidGrant("max_uses must be at least 1".to_owned()));
        }
        Ok(Self {
            id,
            connection_id,
            scope,
            created_at,
            expires_at,
            max_uses,
            uses: 0,
            revoked: false,
            account: None,
        })
    }

    /// The same grant, limited to one account.
    #[must_use]
    pub fn for_account(mut self, account: Option<String>) -> Self {
        self.account = account;
        self
    }

    /// Whether the grant covers requests about `account` (a grant not bound to an account covers every one).
    #[must_use]
    pub fn covers_account(&self, account: Option<&str>) -> bool {
        self.account.as_deref().is_none_or(|own| account == Some(own))
    }

    #[must_use]
    pub fn is_active(&self, now: i64) -> bool {
        !self.revoked && self.expires_at.is_none_or(|t| now < t) && self.max_uses.is_none_or(|m| self.uses < m)
    }

    pub fn record_use(&mut self) {
        self.uses = self.uses.saturating_add(1);
    }

    pub(crate) fn applies_to(&self, connection: &ConnectionId, now: i64) -> bool {
        &self.connection_id == connection && self.is_active(now)
    }
}

/// Consumes one use of each grant a completed tool call relied on.
pub fn record_uses(grants: &mut [Grant], used: &BTreeSet<GrantId>) {
    for grant in grants.iter_mut().filter(|g| used.contains(&g.id)) {
        grant.record_use();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddrRule, SendScope};

    fn scope() -> Scope {
        Scope::Send(SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        })
    }

    fn grant(expires_at: Option<i64>, max_uses: Option<u32>) -> Grant {
        Grant::new("g1".into(), "c1".into(), scope(), 100, expires_at, max_uses).unwrap()
    }

    fn all_mail() -> Scope {
        Scope::Read(crate::ReadScope {
            any: true,
            ..crate::ReadScope::default()
        })
    }

    #[test]
    fn a_grant_for_all_mail_must_expire_soon() {
        let day = 86_400;
        assert!(Grant::new("g".into(), "c".into(), all_mail(), 100, Some(100 + day), None).is_ok());
        assert!(Grant::new("g".into(), "c".into(), all_mail(), 100, Some(100 + MAX_ANY_MAIL_SECS), None).is_ok());
        assert!(Grant::new("g".into(), "c".into(), all_mail(), 100, None, None).is_err(), "no expiry");
        assert!(
            Grant::new("g".into(), "c".into(), all_mail(), 100, None, Some(5)).is_err(),
            "uses alone are not a time box"
        );
        assert!(
            Grant::new("g".into(), "c".into(), all_mail(), 100, Some(101 + MAX_ANY_MAIL_SECS), None).is_err(),
            "too long"
        );
    }

    #[test]
    fn all_mail_cannot_be_combined_with_other_constraints() {
        let narrowed = Scope::Read(crate::ReadScope {
            any: true,
            from: vec![AddrRule::domain("work.com").unwrap()],
            ..crate::ReadScope::default()
        });
        assert!(Grant::new("g".into(), "c".into(), narrowed, 100, Some(200), None).is_err());
    }

    #[test]
    fn new_validates() {
        assert!(Grant::new("g".into(), "c".into(), scope(), 100, Some(100), None).is_err());
        assert!(Grant::new("g".into(), "c".into(), scope(), 100, None, Some(0)).is_err());
        let empty = Scope::Send(SendScope {
            recipients: vec![],
            subject: None,
            body: None,
        });
        assert!(Grant::new("g".into(), "c".into(), empty, 100, None, None).is_err());
        let g = grant(None, None);
        assert_eq!((g.uses, g.revoked), (0, false));
    }

    #[test]
    fn unknown_fields_rejected() {
        let mut v = serde_json::to_value(grant(None, None)).unwrap();
        assert!(serde_json::from_value::<Grant>(v.clone()).is_ok());
        v["not_before"] = serde_json::json!(500);
        assert!(serde_json::from_value::<Grant>(v).is_err());
    }

    #[test]
    fn expiry_boundary() {
        let g = grant(Some(200), None);
        assert!(g.is_active(199));
        assert!(!g.is_active(200));
        assert!(!g.is_active(201));
    }

    #[test]
    fn use_limit() {
        let mut g = grant(None, Some(2));
        assert!(g.is_active(150));
        g.record_use();
        assert!(g.is_active(150));
        g.record_use();
        assert!(!g.is_active(150));
    }

    #[test]
    fn revocation_and_until_revoked() {
        let mut g = grant(None, None);
        assert!(g.is_active(i64::MAX));
        g.revoked = true;
        assert!(!g.is_active(150));
    }

    #[test]
    fn record_uses_only_touches_used() {
        let mut gs = vec![
            grant(None, Some(5)),
            Grant {
                id: "g2".into(),
                ..grant(None, Some(5))
            },
        ];
        record_uses(&mut gs, &["g2".into()].into());
        assert_eq!((gs[0].uses, gs[1].uses), (0, 1));
    }
}
