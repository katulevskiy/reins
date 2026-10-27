//! Circle obligations (`docs/design/decisions-v2/04-shared-control.md` §1, 02 §3 rule 2, 08 R3): which of the owner's
//! arrangements cover a request, and whether the votes collected so far approve it, deny it or still wait.
//!
//! The owner's own review (its **slot**) follows the owner's usual policy: a tap, Auto or Bypass fills it. A
//! **delegate**'s approval fills it too. **Co-signers** and **guardians** add slots only they fill; no permit, mode or
//! other person fills them (hard obligations, 08 R3). A **watcher** adds nothing to decide.
//!
//! Pure: no IO, clock or signatures. The phone verifies every vote before it reaches [`settle`].

use reins_proto::circle::{Arrangement, CircleKind, Class, OnExpiry, Scope, Vote};

use crate::{
    context::Zone,
    effects::{Certainty, EffectKind, Level},
};

/// What a scope is matched against: the request as the consequence engine and the context graph read it.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub kinds: &'a [EffectKind],
    pub zone: Zone,
    pub level: Level,
    pub certainty: Certainty,
    pub connection: &'a str,
    pub account: Option<&'a str>,
}

fn kind_name(kind: EffectKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn zone_name(zone: Zone) -> String {
    serde_json::to_value(zone).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Whether `scope` covers the request. Each non-empty list must hold the request's value.
///
/// Unknown always widens an obligation's reach, never narrows it: a request whose effects couldn't be read
/// (certainty `None`, or an `unknown` effect) has a guessed kind, zone and level, so it is inside every effect, zone and
/// level filter (it counts as L3 anywhere). The connection and the account are known facts and still filter.
#[must_use]
pub fn scope_matches(scope: &Scope, input: &Input<'_>) -> bool {
    let unreadable = input.certainty == Certainty::None || input.kinds.contains(&EffectKind::Unknown);
    if !unreadable {
        if !scope.effects.is_empty() && !input.kinds.iter().any(|k| scope.effects.contains(&kind_name(*k))) {
            return false;
        }
        if !scope.zones.is_empty() && !scope.zones.contains(&zone_name(input.zone)) {
            return false;
        }
        if let Some(min) = scope.min_level
            && (input.level as u8) < min
        {
            return false;
        }
    }
    if !scope.connections.is_empty() && !scope.connections.iter().any(|c| c == input.connection) {
        return false;
    }
    if !scope.accounts.is_empty() {
        let Some(account) = input.account else {
            return false;
        };
        if !scope.accounts.iter().any(|a| a.eq_ignore_ascii_case(account)) {
            return false;
        }
    }
    true
}

/// One person an obligation names, through one arrangement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub arrangement: String,
    pub account: String,
    pub kind: CircleKind,
    pub class: Class,
    pub window_secs: u64,
    pub on_expiry: OnExpiry,
}

impl Person {
    fn of(a: &Arrangement) -> Self {
        Self {
            arrangement: a.id.clone(),
            account: a.helper.account.clone(),
            kind: a.terms.kind,
            class: a.terms.class,
            window_secs: a.terms.window_secs,
            on_expiry: a.terms.on_expiry,
        }
    }
}

/// What a request needs from the owner's Circle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Obligations {
    /// May fill the owner's slot.
    pub delegates: Vec<Person>,
    /// Co-signers and guardians: each must approve.
    pub required: Vec<Person>,
    /// Told, never asked.
    pub watchers: Vec<Person>,
}

impl Obligations {
    /// No one but the owner decides.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.delegates.is_empty() && self.required.is_empty()
    }

    /// Deferred when any reviewer's arrangement is.
    #[must_use]
    pub fn class(&self) -> Class {
        if self.reviewers().any(|p| p.class == Class::Deferred) {
            Class::Deferred
        } else {
            Class::Instant
        }
    }

    /// The shortest window among the reviewers (the request's deadline), if any.
    #[must_use]
    pub fn window_secs(&self) -> Option<u64> {
        self.reviewers().map(|p| p.window_secs).min()
    }

    pub fn reviewers(&self) -> impl Iterator<Item = &Person> {
        self.delegates.iter().chain(&self.required)
    }
}

/// The obligations of the owner's `active` arrangements that cover the request. An arrangement of another owner, or
/// whose helper is the owner's own account, adds nothing.
#[must_use]
pub fn obligations(owner_account: &str, active: &[&Arrangement], input: &Input<'_>) -> Obligations {
    let mut ob = Obligations::default();
    for a in active {
        if a.owner.account != owner_account
            || a.helper.account == owner_account
            || !scope_matches(&a.terms.scope, input)
        {
            continue;
        }
        let p = Person::of(a);
        match a.terms.kind {
            CircleKind::Delegate => ob.delegates.push(p),
            CircleKind::Cosign | CircleKind::Guardian => ob.required.push(p),
            CircleKind::Watcher => ob.watchers.push(p),
        }
    }
    ob
}

/// One verified answer of a helper, for one arrangement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ballot {
    pub arrangement: String,
    pub account: String,
    pub vote: Vote,
}

/// What the owner's phone has collected for a request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Votes {
    /// The owner's slot: `Some(true)` from a tap, Auto or Bypass; `Some(false)` from a denial.
    pub owner: Option<bool>,
    pub people: Vec<Ballot>,
    /// Arrangements whose window ran out unanswered.
    pub lapsed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settle {
    Approved,
    Denied {
        /// The helper's account, `None` for the owner (or a lapsed window).
        by: Option<String>,
        lapsed: bool,
    },
    Waiting {
        /// The owner's slot is still open.
        owner: bool,
        /// Accounts still to answer.
        people: Vec<String>,
    },
}

/// The ballot `p` cast through its own arrangement.
fn ballot<'a>(votes: &'a Votes, owner_account: &str, p: &Person) -> Option<&'a Ballot> {
    votes.people.iter().find(|b| b.arrangement == p.arrangement && b.account == p.account && b.account != owner_account)
}

/// Settles a request's obligations with the votes so far (02 §3 rule 2, 04 §1, §7 "either deny wins").
#[must_use]
pub fn settle(owner_account: &str, ob: &Obligations, votes: &Votes) -> Settle {
    // 1. Any deny of a person who reviews through that arrangement, or of the owner.
    for p in ob.reviewers() {
        if let Some(b) = ballot(votes, owner_account, p)
            && b.vote == Vote::Deny
        {
            return Settle::Denied {
                by: Some(p.account.clone()),
                lapsed: false,
            };
        }
    }
    if votes.owner == Some(false) {
        return Settle::Denied {
            by: None,
            lapsed: false,
        };
    }
    // 2. Windows that ran out.
    let lapsed = |p: &Person| votes.lapsed.contains(&p.arrangement) && ballot(votes, owner_account, p).is_none();
    if ob.reviewers().any(|p| lapsed(p) && p.on_expiry == OnExpiry::Deny) {
        return Settle::Denied {
            by: None,
            lapsed: true,
        };
    }
    // 3. The owner's slot: the owner, or a delegate who is not also a required reviewer (`distinct_persons`).
    let required: Vec<&str> = ob.required.iter().map(|p| p.account.as_str()).collect();
    let owner_filled = votes.owner == Some(true)
        || ob.delegates.iter().any(|p| {
            !lapsed(p)
                && !required.contains(&p.account.as_str())
                && ballot(votes, owner_account, p).is_some_and(|b| b.vote == Vote::Approve)
        });
    // 4. Every required person, through their own arrangement.
    let mut missing: Vec<String> = ob
        .required
        .iter()
        .filter(|p| ballot(votes, owner_account, p).is_none_or(|b| b.vote != Vote::Approve))
        .map(|p| p.account.clone())
        .collect();
    missing.dedup();
    if owner_filled && missing.is_empty() {
        Settle::Approved
    } else {
        Settle::Waiting {
            owner: !owner_filled,
            people: missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::circle::{Party, Redaction, Terms};

    use super::*;

    fn arrangement(id: u8, helper: &str, kind: CircleKind) -> Arrangement {
        let party = |account: &str, n: u8| Party {
            account: account.to_owned(),
            label: account.to_owned(),
            owner_pub: format!("{n}"),
            epoch: 1,
            entry_hash: String::new(),
        };
        Arrangement {
            v: 1,
            id: format!("{id:032x}"),
            revision: 1,
            owner: party("dana", 1),
            helper: party(helper, id),
            terms: Terms::default_for(kind),
            created_at: 0,
        }
    }

    fn input(kinds: &[EffectKind]) -> Input<'_> {
        Input {
            kinds,
            zone: Zone::Outside,
            level: Level::L2,
            certainty: Certainty::Full,
            connection: "c1",
            account: Some("dana@example.com"),
        }
    }

    fn approve(a: &Arrangement) -> Ballot {
        Ballot {
            arrangement: a.id.clone(),
            account: a.helper.account.clone(),
            vote: Vote::Approve,
        }
    }

    fn deny(a: &Arrangement) -> Ballot {
        Ballot {
            vote: Vote::Deny,
            ..approve(a)
        }
    }

    #[test]
    fn scopes_match_by_effect_zone_level_connection_account() {
        let mut s = Scope::default();
        let msg = [EffectKind::Message];
        assert!(scope_matches(&s, &input(&msg)));
        s.effects = vec!["money".to_owned()];
        assert!(!scope_matches(&s, &input(&msg)));
        assert!(scope_matches(&s, &input(&[EffectKind::Money, EffectKind::DataRead])));
        let mut s = Scope {
            zones: vec!["work".to_owned()],
            ..Scope::default()
        };
        assert!(!scope_matches(&s, &input(&msg)));
        s.zones.push("outside".to_owned());
        assert!(scope_matches(&s, &input(&msg)));
        let s = Scope {
            min_level: Some(3),
            ..Scope::default()
        };
        assert!(!scope_matches(&s, &input(&msg)));
        let s = Scope {
            connections: vec!["c2".to_owned()],
            ..Scope::default()
        };
        assert!(!scope_matches(&s, &input(&msg)));
        let s = Scope {
            accounts: vec!["Dana@Example.com".to_owned()],
            ..Scope::default()
        };
        assert!(scope_matches(&s, &input(&msg)));
        let mut no_account = input(&msg);
        no_account.account = None;
        assert!(!scope_matches(&s, &no_account));
    }

    #[test]
    fn unknown_effects_stay_in_scope() {
        let s = Scope {
            effects: vec!["money".to_owned()],
            connections: vec!["c1".to_owned()],
            ..Scope::default()
        };
        let mut i = input(&[EffectKind::Message]);
        i.certainty = Certainty::None;
        assert!(scope_matches(&s, &i));
        assert!(scope_matches(&s, &input(&[EffectKind::Unknown])));
        // A level or zone guessed for an unreadable request never takes it out of scope: it counts as L3, anywhere.
        let high = Scope {
            min_level: Some(3),
            ..Scope::default()
        };
        let mut none = input(&[EffectKind::Message]);
        none.certainty = Certainty::None;
        none.level = Level::L0;
        assert!(scope_matches(&high, &none));
        let outside = Scope {
            zones: vec!["outside".to_owned()],
            ..Scope::default()
        };
        let mut unknown = input(&[EffectKind::Unknown]);
        unknown.zone = Zone::Mine;
        assert!(scope_matches(&outside, &unknown));
        // A readable request is still filtered by its level and zone.
        let mut known = input(&[EffectKind::Message]);
        known.zone = Zone::Mine;
        assert!(!scope_matches(&outside, &known));
        assert!(!scope_matches(&high, &known));
        // The known facts (connection, account) still apply.
        let mut other = input(&[EffectKind::Unknown]);
        other.connection = "c9";
        assert!(!scope_matches(&s, &other));
    }

    #[test]
    fn cosign_needs_both_and_either_deny_wins() {
        let alex = arrangement(2, "alex", CircleKind::Cosign);
        let ob = obligations("dana", &[&alex], &input(&[EffectKind::Message]));
        assert_eq!(ob.required.len(), 1);
        let mut v = Votes::default();
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: true,
                people: vec!["alex".to_owned()]
            }
        );
        v.owner = Some(true);
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: false,
                people: vec!["alex".to_owned()]
            }
        );
        v.people.push(approve(&alex));
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
        // The owner denies after the co-signer approved: denied.
        v.owner = Some(false);
        assert!(matches!(
            settle("dana", &ob, &v),
            Settle::Denied {
                by: None,
                ..
            }
        ));
        // The co-signer denies while the owner approved: denied.
        let v = Votes {
            owner: Some(true),
            people: vec![deny(&alex)],
            lapsed: vec![],
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Denied {
                by: Some("alex".to_owned()),
                lapsed: false
            }
        );
    }

    #[test]
    fn delegate_or_owner_is_enough() {
        let maria = arrangement(3, "maria", CircleKind::Delegate);
        let ob = obligations("dana", &[&maria], &input(&[EffectKind::Message]));
        assert_eq!(ob.delegates.len(), 1);
        let v = Votes {
            people: vec![approve(&maria)],
            ..Votes::default()
        };
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
        let v = Votes {
            owner: Some(true),
            ..Votes::default()
        };
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
        let v = Votes {
            people: vec![deny(&maria)],
            ..Votes::default()
        };
        assert!(matches!(settle("dana", &ob, &v), Settle::Denied { .. }));
    }

    #[test]
    fn guardian_needs_the_guardian_and_the_owner_slot() {
        let mom = arrangement(4, "mom", CircleKind::Guardian);
        let ob = obligations("dana", &[&mom], &input(&[EffectKind::Money]));
        let v = Votes {
            people: vec![approve(&mom)],
            ..Votes::default()
        };
        // The ward's own slot (Auto or a tap) is still open.
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: true,
                people: vec![]
            }
        );
        // Auto filling the ward's slot never fills the guardian's.
        let v = Votes {
            owner: Some(true),
            ..Votes::default()
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: false,
                people: vec!["mom".to_owned()]
            }
        );
    }

    #[test]
    fn a_cosigner_who_is_also_delegate_does_not_fill_the_owner_slot() {
        let deleg = arrangement(5, "alex", CircleKind::Delegate);
        let cosign = arrangement(6, "alex", CircleKind::Cosign);
        let ob = obligations("dana", &[&deleg, &cosign], &input(&[EffectKind::Message]));
        let v = Votes {
            people: vec![approve(&deleg), approve(&cosign)],
            ..Votes::default()
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: true,
                people: vec![]
            }
        );
        // Another delegate may.
        let maria = arrangement(7, "maria", CircleKind::Delegate);
        let ob = obligations("dana", &[&deleg, &cosign, &maria], &input(&[EffectKind::Message]));
        let v = Votes {
            people: vec![approve(&cosign), approve(&maria)],
            ..Votes::default()
        };
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
    }

    #[test]
    fn one_answer_counts_for_its_own_arrangement_only() {
        let a = arrangement(8, "alex", CircleKind::Cosign);
        let mut b = arrangement(9, "alex", CircleKind::Guardian);
        b.terms.redaction = Redaction::Summary;
        let ob = obligations("dana", &[&a, &b], &input(&[EffectKind::Money]));
        let v = Votes {
            owner: Some(true),
            people: vec![approve(&a)],
            lapsed: vec![],
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: false,
                people: vec!["alex".to_owned()]
            }
        );
        // An answer for an arrangement the request isn't under counts for nothing.
        let other = arrangement(10, "alex", CircleKind::Cosign);
        let v = Votes {
            owner: Some(true),
            people: vec![approve(&other), approve(&a)],
            lapsed: vec![],
        };
        assert!(matches!(settle("dana", &ob, &v), Settle::Waiting { .. }));
    }

    #[test]
    fn the_owner_never_fills_a_helper_slot() {
        let mut selfish = arrangement(11, "dana", CircleKind::Cosign);
        // An arrangement with oneself adds nothing (it is invalid on the wire too).
        assert!(obligations("dana", &[&selfish], &input(&[EffectKind::Message])).is_empty());
        selfish.helper.account = "alex".to_owned();
        let ob = obligations("dana", &[&selfish], &input(&[EffectKind::Message]));
        // A ballot claiming the owner's account for the helper's arrangement counts for nothing.
        let v = Votes {
            owner: Some(true),
            people: vec![Ballot {
                arrangement: selfish.id,
                account: "dana".to_owned(),
                vote: Vote::Approve,
            }],
            lapsed: vec![],
        };
        assert!(matches!(settle("dana", &ob, &v), Settle::Waiting { .. }));
        // Another owner's arrangement adds nothing.
        let mut theirs = arrangement(12, "alex", CircleKind::Cosign);
        theirs.owner.account = "sam".to_owned();
        assert!(obligations("dana", &[&theirs], &input(&[EffectKind::Message])).is_empty());
    }

    #[test]
    fn lapsed_windows_deny_or_fall_back_to_the_owner() {
        let alex = arrangement(13, "alex", CircleKind::Cosign);
        let ob = obligations("dana", &[&alex], &input(&[EffectKind::Message]));
        let v = Votes {
            owner: Some(true),
            lapsed: vec![alex.id],
            ..Votes::default()
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Denied {
                by: None,
                lapsed: true
            }
        );
        let mut maria = arrangement(14, "maria", CircleKind::Delegate);
        maria.terms.on_expiry = OnExpiry::Owner;
        let ob = obligations("dana", &[&maria], &input(&[EffectKind::Message]));
        let v = Votes {
            lapsed: vec![maria.id.clone()],
            ..Votes::default()
        };
        assert_eq!(
            settle("dana", &ob, &v),
            Settle::Waiting {
                owner: true,
                people: vec![]
            }
        );
        let v = Votes {
            owner: Some(true),
            lapsed: vec![maria.id],
            ..Votes::default()
        };
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
    }

    #[test]
    fn watchers_add_no_obligation() {
        let w = arrangement(15, "pat", CircleKind::Watcher);
        let ob = obligations("dana", &[&w], &input(&[EffectKind::Message]));
        assert!(ob.is_empty());
        assert_eq!(ob.watchers.len(), 1);
        let v = Votes {
            owner: Some(true),
            ..Votes::default()
        };
        assert_eq!(settle("dana", &ob, &v), Settle::Approved);
    }

    #[test]
    fn classes_and_windows() {
        let mut a = arrangement(16, "alex", CircleKind::Cosign);
        a.terms.class = Class::Instant;
        a.terms.window_secs = 120;
        let b = arrangement(17, "bea", CircleKind::Cosign);
        let ob = obligations("dana", &[&a], &input(&[EffectKind::Message]));
        assert_eq!(ob.class(), Class::Instant);
        assert_eq!(ob.window_secs(), Some(120));
        let ob = obligations("dana", &[&a, &b], &input(&[EffectKind::Message]));
        assert_eq!(ob.class(), Class::Deferred);
        assert_eq!(ob.window_secs(), Some(120));
    }

    #[test]
    fn scope_effect_names_match_effect_kinds() {
        let all = [
            EffectKind::FileRead,
            EffectKind::FileWrite,
            EffectKind::FileDelete,
            EffectKind::GitPush,
            EffectKind::GitRefDelete,
            EffectKind::Message,
            EffectKind::Publish,
            EffectKind::Permission,
            EffectKind::Money,
            EffectKind::SecretOut,
            EffectKind::SecretUse,
            EffectKind::Network,
            EffectKind::Infra,
            EffectKind::Exec,
            EffectKind::DataRead,
            EffectKind::DataWrite,
            EffectKind::Unknown,
        ];
        let names: Vec<String> = all.into_iter().map(kind_name).collect();
        assert_eq!(names, reins_proto::circle::SCOPE_EFFECTS);
        let zones: Vec<String> = Zone::ALL.into_iter().map(zone_name).collect();
        assert_eq!(zones, reins_proto::circle::SCOPE_ZONES);
    }
}
