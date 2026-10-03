use std::collections::BTreeSet;

use rewarden_proto::gmail::OutgoingEmail;
use serde::{Deserialize, Serialize};

use crate::{AddrRule, Pattern, PolicyError};

/// What the policy engine knows about one real message, taken from Gmail on
/// the phone. Never derived from anything the AI supplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageFacts {
    pub id: String,
    /// Normalized sender address.
    pub from: String,
    /// Normalized To and Cc addresses.
    pub to: Vec<String>,
    pub subject: String,
    /// Plain-text body, `None` when it was not fetched.
    pub body: Option<String>,
    /// Gmail label ids (e.g. `INBOX`, `Label_12`).
    pub labels: Vec<String>,
    /// Unix seconds.
    pub date: i64,
}

/// Which messages a read grant covers. Every set field must match (AND);
/// within a list field any entry may match (OR).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadScope {
    /// Every message, whatever its sender or subject. Only valid on its own (no other constraint) and, in a
    /// [`crate::Grant`], only with an expiry. This is how "let this AI read my mail for the next hour" is stored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub any: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_ids: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<AddrRule>,
    /// Matches if any To/Cc address matches any rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<AddrRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Inclusive lower bound, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    /// Exclusive upper bound, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
}

/// Which emails a send grant covers. Every recipient must be covered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendScope {
    pub recipients: Vec<AddrRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Pattern>,
}

/// The right to be told which accounts an integration has (not what is in them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountsScope {
    /// The integration, e.g. "gmail".
    pub service: String,
    /// Which of its accounts may be shown. Empty means all of them (how the first grants of this kind were made).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
}

impl AccountsScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        let ok = !self.service.is_empty()
            && self.service.len() <= 32
            && self.service.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if ok {
            Ok(())
        } else {
            Err(PolicyError::InvalidScope("an accounts scope names an integration in lower case".to_owned()))
        }
    }
}

/// What a grant gives on any integration besides Gmail: one kind of access, to named things or to everything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceScope {
    /// "telegram", "gcalendar", ...
    pub service: String,
    /// "list", "read" or "write". Reading does not imply listing, and neither implies writing.
    pub access: String,
    /// The chats, calendars, repositories, ... it covers (their ids). Empty together with `any`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    /// Their names, in the same order, for showing the grant (not used to decide anything).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Everything of the integration. Only for a limited time, never for writing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub any: bool,
    /// For a write: the kinds of change it allows (issues, code, releases, ...). Empty = every kind the integration
    /// lets a permission cover.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
}

/// Whether a permission for `granted` covers `thing`: the same thing, or a part of it. Parts are written after a `@`
/// (a branch: `owner/repo` covers `owner/repo@main`) or a `/` (`owner` covers `owner/repo`, a folder covers what is
/// in it). A prefix that is not a whole part covers nothing (`owner/re` does not cover `owner/repo`).
#[must_use]
pub fn resource_covers(granted: &str, thing: &str) -> bool {
    thing == granted || thing.strip_prefix(granted).is_some_and(|rest| rest.starts_with('@') || rest.starts_with('/'))
}

/// The longest a grant that covers everything of an integration may live.
pub const MAX_ANY_RESOURCE_SECS: i64 = 7 * 86_400;

impl ServiceScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        let service_ok = !self.service.is_empty()
            && self.service.len() <= 32
            && self.service.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !service_ok {
            return Err(PolicyError::InvalidScope("a scope names an integration in lower case".to_owned()));
        }
        if !matches!(self.access.as_str(), "list" | "read" | "write") {
            return Err(PolicyError::InvalidScope("access must be list, read or write".to_owned()));
        }
        if self.any != self.resources.is_empty() {
            return Err(PolicyError::InvalidScope(
                "name the things it covers, or choose everything, not both".to_owned(),
            ));
        }
        if self.any && self.access == "write" {
            return Err(PolicyError::InvalidScope("writing cannot be allowed everywhere".to_owned()));
        }
        if !self.labels.is_empty() && self.labels.len() != self.resources.len() {
            return Err(PolicyError::InvalidScope("labels must match the resources".to_owned()));
        }
        if self.resources.iter().any(|r| r.trim().is_empty() || r.len() > 200) {
            return Err(PolicyError::InvalidScope("a resource id is empty or too long".to_owned()));
        }
        if !self.classes.is_empty() && self.access != "write" {
            return Err(PolicyError::InvalidScope(
                "only a permission to change things names kinds of change".to_owned(),
            ));
        }
        let class_ok =
            |c: &String| !c.is_empty() && c.len() <= 32 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
        if !self.classes.iter().all(class_ok)
            || self.classes.iter().collect::<BTreeSet<_>>().len() != self.classes.len()
        {
            return Err(PolicyError::InvalidScope("a kind of change is named in lower case, once".to_owned()));
        }
        Ok(())
    }

    /// Whether the scope covers this access to this thing. `class` is the kind of change a write is ("" when the
    /// operation has none): a scope that names kinds covers only those.
    #[must_use]
    pub fn covers(&self, service: &str, access: &str, class: &str, resource: &str) -> bool {
        self.service == service
            && self.access == access
            && (self.classes.is_empty() || class.is_empty() || self.classes.iter().any(|c| c == class))
            && (self.any || self.resources.iter().any(|r| resource_covers(r, resource)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Scope {
    Read(ReadScope),
    Send(SendScope),
    /// See which accounts one integration has.
    Accounts(AccountsScope),
    /// Access to another integration.
    Service(ServiceScope),
}

impl ReadScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        if !self.is_constrained() {
            return Err(PolicyError::InvalidScope("a read scope needs at least one constraint".to_owned()));
        }
        if self.any && self.has_narrowing_constraint() {
            return Err(PolicyError::InvalidScope("`any` cannot be combined with other constraints".to_owned()));
        }
        if self.message_ids.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err(PolicyError::InvalidScope("message_ids must not be empty".to_owned()));
        }
        if self.labels.iter().any(|l| l.trim().is_empty()) {
            return Err(PolicyError::InvalidScope("labels must not be blank".to_owned()));
        }
        if let (Some(after), Some(before)) = (self.after, self.before)
            && after >= before
        {
            return Err(PolicyError::InvalidScope("after must be earlier than before".to_owned()));
        }
        Ok(())
    }

    /// False for an unconstrained scope, so a damaged stored grant matches nothing.
    #[must_use]
    pub fn matches(&self, m: &MessageFacts) -> bool {
        self.is_constrained()
            && self.message_ids.as_ref().is_none_or(|ids| ids.contains(&m.id))
            && (self.from.is_empty() || self.from.iter().any(|r| r.matches(&m.from)))
            && (self.to.is_empty() || m.to.iter().any(|a| self.to.iter().any(|r| r.matches(a))))
            && self.subject.as_ref().is_none_or(|p| p.is_match(&m.subject))
            && self.body.as_ref().is_none_or(|p| m.body.as_deref().is_some_and(|b| p.is_match(b)))
            && (self.labels.is_empty() || m.labels.iter().any(|l| self.labels.contains(l)))
            && self.after.is_none_or(|t| m.date >= t)
            && self.before.is_none_or(|t| m.date < t)
    }

    #[must_use]
    pub fn needs_body(&self) -> bool {
        self.body.is_some()
    }

    fn is_constrained(&self) -> bool {
        self.any || self.has_narrowing_constraint()
    }

    fn has_narrowing_constraint(&self) -> bool {
        self.message_ids.is_some()
            || !self.from.is_empty()
            || !self.to.is_empty()
            || self.subject.is_some()
            || self.body.is_some()
            || !self.labels.is_empty()
            || self.after.is_some()
            || self.before.is_some()
    }
}

impl SendScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.recipients.is_empty() {
            return Err(PolicyError::InvalidScope("a send scope needs at least one recipient rule".to_owned()));
        }
        Ok(())
    }

    /// `email` must be normalized (`OutgoingEmail::normalized`).
    #[must_use]
    pub fn matches(&self, email: &OutgoingEmail) -> bool {
        !self.recipients.is_empty()
            && email.recipients().next().is_some()
            && email.recipients().all(|a| self.recipients.iter().any(|r| r.matches(a)))
            && self.subject.as_ref().is_none_or(|p| p.is_match(&email.subject))
            && self.body.as_ref().is_none_or(|p| p.is_match(&email.body))
    }
}

impl Scope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        match self {
            Self::Read(s) => s.validate(),
            Self::Send(s) => s.validate(),
            Self::Accounts(s) => s.validate(),
            Self::Service(s) => s.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn msg(id: &str, from: &str, to: &[&str], subject: &str) -> MessageFacts {
        MessageFacts {
            id: id.to_owned(),
            from: from.to_owned(),
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            subject: subject.to_owned(),
            body: None,
            labels: vec!["INBOX".to_owned()],
            date: 100,
        }
    }

    fn email(to: &[&str], subject: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: subject.to_owned(),
            body: "body".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn empty_read_scope_is_invalid_and_matches_nothing() {
        let s = ReadScope::default();
        assert!(s.validate().is_err());
        assert!(!s.matches(&msg("m1", "a@b.com", &[], "x")));
        let damaged: ReadScope = serde_json::from_value(json!({})).unwrap();
        assert!(!damaged.matches(&msg("m1", "a@b.com", &[], "x")));
    }

    #[test]
    fn any_covers_every_message_but_only_when_explicit() {
        let all = ReadScope {
            any: true,
            ..ReadScope::default()
        };
        assert!(all.validate().is_ok());
        assert!(all.matches(&msg("m1", "a@b.com", &[], "x")));
        assert!(all.matches(&msg("m2", "z@y.org", &["INBOX"], "anything")));
        // Round trip keeps it; a stored scope without the flag stays unconstrained.
        let back: ReadScope = serde_json::from_value(serde_json::to_value(&all).unwrap()).unwrap();
        assert_eq!(back, all);
        assert!(!serde_json::from_value::<ReadScope>(json!({})).unwrap().matches(&msg("m1", "a@b.com", &[], "x")));
        let narrowed = ReadScope {
            any: true,
            labels: vec!["INBOX".to_owned()],
            ..ReadScope::default()
        };
        assert!(narrowed.validate().is_err());
    }

    #[test]
    fn message_ids_restrict() {
        let s = ReadScope {
            message_ids: Some(["m1".to_owned()].into()),
            ..ReadScope::default()
        };
        assert!(s.validate().is_ok());
        assert!(s.matches(&msg("m1", "a@b.com", &[], "x")));
        assert!(!s.matches(&msg("m2", "a@b.com", &[], "x")));
        let empty = ReadScope {
            message_ids: Some(BTreeSet::new()),
            ..ReadScope::default()
        };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn fields_are_anded_lists_are_ored() {
        let s = ReadScope {
            from: vec![AddrRule::domain("bank.com").unwrap(), AddrRule::exact("boss@work.com").unwrap()],
            subject: Some(Pattern::find("statement").unwrap()),
            ..ReadScope::default()
        };
        assert!(s.matches(&msg("1", "a@bank.com", &[], "Your Statement")));
        assert!(s.matches(&msg("2", "boss@work.com", &[], "statement")));
        assert!(!s.matches(&msg("3", "a@bank.com", &[], "hello")));
        assert!(!s.matches(&msg("4", "a@evil.com", &[], "statement")));
    }

    #[test]
    fn to_matches_any_recipient() {
        let s = ReadScope {
            to: vec![AddrRule::exact("me@work.com").unwrap()],
            ..ReadScope::default()
        };
        assert!(s.matches(&msg("1", "x@y.com", &["a@b.com", "me@work.com"], "s")));
        assert!(!s.matches(&msg("2", "x@y.com", &["a@b.com"], "s")));
    }

    #[test]
    fn body_pattern_requires_fetched_body() {
        let s = ReadScope {
            body: Some(Pattern::find("otp").unwrap()),
            ..ReadScope::default()
        };
        assert!(s.needs_body());
        let mut m = msg("1", "a@b.com", &[], "s");
        assert!(!s.matches(&m), "missing body must not match");
        m.body = Some("your OTP is 1234".to_owned());
        assert!(s.matches(&m));
    }

    #[test]
    fn labels_and_date_bounds() {
        let s = ReadScope {
            labels: vec!["INBOX".to_owned()],
            after: Some(100),
            before: Some(200),
            ..ReadScope::default()
        };
        let mut m = msg("1", "a@b.com", &[], "s");
        assert!(s.matches(&m), "after is inclusive");
        m.date = 200;
        assert!(!s.matches(&m), "before is exclusive");
        m.date = 150;
        m.labels = vec!["SPAM".to_owned()];
        assert!(!s.matches(&m));
        let inverted = ReadScope {
            after: Some(5),
            before: Some(5),
            ..ReadScope::default()
        };
        assert!(inverted.validate().is_err());
        let blank_label = ReadScope {
            labels: vec![" ".to_owned()],
            ..ReadScope::default()
        };
        assert!(blank_label.validate().is_err());
    }

    #[test]
    fn send_requires_every_recipient_covered() {
        let s = SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        };
        assert!(s.validate().is_ok());
        assert!(s.matches(&email(&["a@work.com", "b@work.com"], "x")));
        let mut mixed = email(&["a@work.com"], "x");
        mixed.cc = vec!["eve@evil.com".to_owned()];
        assert!(!s.matches(&mixed));
        assert!(!s.matches(&email(&[], "x")), "no recipients never matches");
    }

    #[test]
    fn send_subject_and_body_patterns() {
        let s = SendScope {
            recipients: vec![AddrRule::exact("a@work.com").unwrap()],
            subject: Some(Pattern::find("^weekly report").unwrap()),
            body: Some(Pattern::find("regards").unwrap()),
        };
        let mut e = email(&["a@work.com"], "Weekly report 12");
        e.body = "Kind regards".to_owned();
        assert!(s.matches(&e));
        e.subject = "Other".to_owned();
        assert!(!s.matches(&e));
    }

    #[test]
    fn empty_send_scope_invalid_and_matches_nothing() {
        let s = SendScope {
            recipients: vec![],
            subject: None,
            body: None,
        };
        assert!(s.validate().is_err());
        assert!(!s.matches(&email(&["a@b.com"], "x")));
    }

    #[test]
    fn scope_wire_format() {
        let s = Scope::Send(SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        });
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v, json!({"action": "send", "recipients": [{"kind": "domain", "value": "work.com"}]}));
        assert_eq!(serde_json::from_value::<Scope>(v).unwrap(), s);
    }

    #[test]
    fn unknown_fields_rejected() {
        let read = json!({"action": "read", "from": [{"kind": "domain", "value": "bank.com"}], "subjct": "x"});
        assert!(serde_json::from_value::<Scope>(read).is_err());
        let send = json!({"action": "send", "recipients": [{"kind": "domain", "value": "work.com"}], "bcc": "x"});
        assert!(serde_json::from_value::<Scope>(send).is_err());
        let ok = json!({"action": "read", "from": [{"kind": "domain", "value": "bank.com"}]});
        assert!(serde_json::from_value::<Scope>(ok).is_ok());
    }
}
