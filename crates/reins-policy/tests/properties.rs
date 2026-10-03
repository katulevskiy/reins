//! Randomized checks of the security properties the phone relies on.

use proptest::collection::{btree_set, vec};
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;
use rewarden_policy::{
    AddrRule, Grant, MessageFacts, Pattern, ReadScope, Scope, SendDecision, SendScope, evaluate_read, evaluate_send,
};
use rewarden_proto::gmail::OutgoingEmail;
use rewarden_proto::ids::{ConnectionId, GrantId};
use rewarden_proto::normalize_address;

const LOCALS: [&str; 3] = ["alice", "bob", "eve"];
const DOMAINS: [&str; 4] = ["bank.com", "evil.com", "sub.bank.com", "bank.com.evil.com"];
const CONNECTIONS: [&str; 2] = ["c1", "c2"];

fn addr() -> impl Strategy<Value = String> {
    (select(LOCALS.to_vec()), select(DOMAINS.to_vec())).prop_map(|(l, d)| format!("{l}@{d}"))
}

fn addr_rule() -> impl Strategy<Value = AddrRule> {
    prop_oneof![
        addr().prop_map(|a| AddrRule::exact(&a).unwrap()),
        select(DOMAINS.to_vec()).prop_map(|d| AddrRule::domain(d).unwrap()),
        select(vec![r".*@bank\.com", "alice@.*", "b.*"]).prop_map(|r| AddrRule::regex(r).unwrap()),
    ]
}

fn pattern() -> impl Strategy<Value = Pattern> {
    select(vec!["invoice", "^your", "statement|otp"]).prop_map(|p| Pattern::find(p).unwrap())
}

fn read_scope() -> impl Strategy<Value = ReadScope> {
    (
        option::of(btree_set(select(vec!["m1", "m2", "m3"]).prop_map(str::to_owned), 1..3)),
        vec(addr_rule(), 0..2),
        vec(addr_rule(), 0..2),
        option::of(pattern()),
        option::of(pattern()),
        vec(select(vec!["INBOX", "Label_1"]).prop_map(str::to_owned), 0..2),
        option::of(0i64..100),
        option::of(50i64..150),
    )
        .prop_filter_map("valid read scope", |(message_ids, from, to, subject, body, labels, after, before)| {
            let s = ReadScope {
                any: false,
                message_ids,
                from,
                to,
                subject,
                body,
                labels,
                after,
                before,
            };
            s.validate().is_ok().then_some(s)
        })
}

fn send_scope() -> impl Strategy<Value = SendScope> {
    (vec(addr_rule(), 1..3), option::of(pattern()), option::of(pattern())).prop_map(|(recipients, subject, body)| {
        SendScope {
            recipients,
            subject,
            body,
        }
    })
}

fn grant() -> impl Strategy<Value = Grant> {
    (
        select(CONNECTIONS.to_vec()),
        prop_oneof![read_scope().prop_map(Scope::Read), send_scope().prop_map(Scope::Send)],
        option::of(0i64..200),
        option::of(1u32..3),
        0u32..4,
        any::<bool>(),
    )
        .prop_map(|(conn, scope, expires_at, max_uses, uses, revoked)| Grant {
            id: GrantId::from("unset"),
            connection_id: ConnectionId::from(conn),
            scope,
            created_at: 0,
            expires_at,
            max_uses,
            uses,
            revoked,
            account: None,
        })
}

fn grants() -> impl Strategy<Value = Vec<Grant>> {
    vec(grant(), 0..6).prop_map(|mut gs| {
        for (i, g) in gs.iter_mut().enumerate() {
            g.id = GrantId(format!("g{i}"));
        }
        gs
    })
}

/// A grant plus a `now` that often sits on its lifetime boundaries: one second
/// before, at, and after `expires_at`, and `uses` just below, at, and above `max_uses`.
fn grant_at_boundary() -> impl Strategy<Value = (Grant, i64)> {
    (grant(), -1i64..=1, 0i64..200, 0u32..=2, option::of(0u32..4)).prop_map(
        |(mut g, delta, any_now, uses_offset, max_uses)| {
            g.max_uses = max_uses;
            g.uses = max_uses.map_or(uses_offset, |m| m.saturating_sub(1) + uses_offset);
            let now = g.expires_at.map_or(any_now, |e| e + delta);
            (g, now)
        },
    )
}

fn message() -> impl Strategy<Value = MessageFacts> {
    (
        select(vec!["m1", "m2", "m3", "m4"]),
        addr(),
        vec(addr(), 0..3),
        select(vec!["Invoice 42", "hello", "Your statement"]),
        option::of(select(vec!["your otp is 1", "lunch?"])),
        vec(select(vec!["INBOX", "Label_1", "SPAM"]).prop_map(str::to_owned), 0..3),
        0i64..200,
    )
        .prop_map(|(id, from, to, subject, body, labels, date)| MessageFacts {
            id: id.to_owned(),
            from,
            to,
            subject: subject.to_owned(),
            body: body.map(str::to_owned),
            labels,
            date,
        })
}

fn outgoing() -> impl Strategy<Value = OutgoingEmail> {
    (vec(addr(), 1..3), vec(addr(), 0..2), select(vec!["Invoice 42", "hello"]), select(vec!["your otp", "hi"]))
        .prop_map(|(to, cc, subject, body)| OutgoingEmail {
            to,
            cc,
            subject: subject.to_owned(),
            body: body.to_owned(),
            reply_to_message_id: None,
        })
}

/// Strings that embed or decorate real addresses but are not themselves a bare,
/// normalized address: upper case, padding, address lists, header injection
/// and display names.
fn unnormalized() -> impl Strategy<Value = String> {
    (addr(), addr(), 0usize..7)
        .prop_map(|(a, b, kind)| match kind {
            0 => a.to_uppercase(),
            1 => format!(" {a} "),
            2 => format!("{a}, {b}"),
            3 => format!("{b}\r\nBcc: {a}"),
            4 => format!("\"Bank\" <{b}>, {a}"),
            5 => format!("Bob <{a}>"),
            _ => format!("{a}\t{b}"),
        })
        .prop_filter("must differ from its normalized form", |v| normalize_address(v).as_deref() != Ok(v.as_str()))
}

/// Independent restatement of the lifetime rule (deliberately not `Grant::is_active`).
fn active_ref(g: &Grant, now: i64) -> bool {
    !g.revoked && g.expires_at.is_none_or(|e| now < e) && g.max_uses.is_none_or(|m| g.uses < m)
}

fn covers_read(g: &Grant, conn: &ConnectionId, now: i64, m: &MessageFacts) -> bool {
    &g.connection_id == conn && active_ref(g, now) && matches!(&g.scope, Scope::Read(s) if s.matches(m))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn read_decisions_are_sound_and_complete(
        grants in grants(),
        msgs in vec(message(), 0..6),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let d = evaluate_read(&grants, &conn, &msgs, now);
        let mut indices: Vec<usize> = d.allowed.iter().map(|(i, _)| *i).chain(d.needs_approval.iter().copied()).collect();
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..msgs.len()).collect::<Vec<_>>(), "allowed and needs_approval must partition the messages");
        for (i, gid) in &d.allowed {
            let g = grants.iter().find(|g| &g.id == gid).expect("allowed by unknown grant");
            prop_assert!(covers_read(g, &conn, now, &msgs[*i]), "grant {} does not cover message {}", gid, i);
        }
        for i in &d.needs_approval {
            prop_assert!(!grants.iter().any(|g| covers_read(g, &conn, now, &msgs[*i])), "message {} was coverable", i);
        }
    }

    #[test]
    fn send_decisions_are_sound_and_complete(
        grants in grants(),
        email in outgoing(),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let covers = |g: &Grant| {
            g.connection_id == conn && active_ref(g, now) && matches!(&g.scope, Scope::Send(s) if s.matches(&email))
        };
        match evaluate_send(&grants, &conn, &email, now) {
            SendDecision::Allowed(gid) => {
                let g = grants.iter().find(|g| g.id == gid).expect("allowed by unknown grant");
                prop_assert!(covers(g));
                if let Scope::Send(s) = &g.scope {
                    for r in email.recipients() {
                        prop_assert!(s.recipients.iter().any(|rule| rule.matches(r)), "recipient {} uncovered", r);
                    }
                }
            }
            SendDecision::NeedsApproval => prop_assert!(!grants.iter().any(covers)),
        }
    }

    #[test]
    fn is_active_matches_reference((g, now) in grant_at_boundary()) {
        prop_assert_eq!(g.is_active(now), active_ref(&g, now), "now={} grant={:?}", now, g);
    }

    #[test]
    fn covered_reads_are_allowed(
        grants in grants(),
        msgs in vec(message(), 1..6),
        mask in vec(any::<bool>(), 6),
        at in any::<prop::sample::Index>(),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let covered_ids: std::collections::BTreeSet<String> =
            msgs.iter().zip(&mask).filter(|(_, on)| **on).map(|(m, _)| m.id.clone()).collect();
        let mut grants = grants;
        if !covered_ids.is_empty() {
            let cover = Grant {
                id: GrantId::from("cover"),
                connection_id: conn.clone(),
                scope: Scope::Read(ReadScope { message_ids: Some(covered_ids.clone()), ..ReadScope::default() }),
                created_at: 0,
                expires_at: None,
                max_uses: None,
                uses: 0,
                revoked: false,
                account: None,
            };
            let pos = at.index(grants.len() + 1);
            grants.insert(pos, cover);
        }
        let d = evaluate_read(&grants, &conn, &msgs, now);
        for (i, m) in msgs.iter().enumerate() {
            if covered_ids.contains(&m.id) {
                let gid = d.allowed.iter().find(|(j, _)| *j == i).map(|(_, g)| g);
                prop_assert!(gid.is_some(), "covered message {} ({}) was not allowed", i, m.id);
                let g = grants.iter().find(|g| Some(&g.id) == gid).expect("allowed by unknown grant");
                prop_assert!(covers_read(g, &conn, now, m));
            }
        }
    }

    #[test]
    fn covered_sends_are_allowed(
        grants in grants(),
        email in outgoing(),
        at in any::<prop::sample::Index>(),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let cover = Grant {
            id: GrantId::from("cover"),
            connection_id: conn.clone(),
            scope: Scope::Send(SendScope {
                recipients: email.recipients().map(|r| AddrRule::exact(r).unwrap()).collect(),
                subject: None,
                body: None,
            }),
            created_at: 0,
            expires_at: None,
            max_uses: None,
            uses: 0,
            revoked: false,
            account: None,
        };
        let mut grants = grants;
        let pos = at.index(grants.len() + 1);
        grants.insert(pos, cover);
        match evaluate_send(&grants, &conn, &email, now) {
            SendDecision::Allowed(gid) => {
                let g = grants.iter().find(|g| g.id == gid).expect("allowed by unknown grant");
                prop_assert!(g.connection_id == conn && active_ref(g, now));
                prop_assert!(matches!(&g.scope, Scope::Send(s) if s.matches(&email)));
            }
            SendDecision::NeedsApproval => prop_assert!(false, "an active covering grant existed"),
        }
    }

    #[test]
    fn unnormalized_addresses_never_match_any_rule(rule in addr_rule(), v in unnormalized()) {
        prop_assert!(!rule.matches(&v), "{:?} matched {:?}", rule, v);
    }

    #[test]
    fn unnormalized_recipients_need_approval(
        rules in vec(addr_rule(), 1..3),
        v in unnormalized(),
        mut email in outgoing(),
        as_cc in any::<bool>(),
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        if as_cc { email.cc.push(v); } else { email.to.push(v); }
        let grants = vec![Grant {
            id: GrantId::from("g"),
            connection_id: conn.clone(),
            scope: Scope::Send(SendScope { recipients: rules, subject: None, body: None }),
            created_at: 0,
            expires_at: None,
            max_uses: None,
            uses: 0,
            revoked: false,
            account: None,
        }];
        prop_assert_eq!(evaluate_send(&grants, &conn, &email, 10), SendDecision::NeedsApproval);
    }

    #[test]
    fn unnormalized_senders_and_recipients_need_approval(
        from_rules in vec(addr_rule(), 1..3),
        to_rules in vec(addr_rule(), 1..3),
        v in unnormalized(),
        msg in message(),
        conn in select(CONNECTIONS.to_vec()),
    ) {
        // Only from/to-constrained scopes: a scope constrained by message id
        // alone may legitimately match whatever the sender string is.
        let conn = ConnectionId::from(conn);
        let mk = |id: &str, scope: ReadScope| Grant {
            id: GrantId::from(id),
            connection_id: conn.clone(),
            scope: Scope::Read(scope),
            created_at: 0,
            expires_at: None,
            max_uses: None,
            uses: 0,
            revoked: false,
            account: None,
        };
        let grants = vec![
            mk("from", ReadScope { from: from_rules, ..ReadScope::default() }),
            mk("to", ReadScope { to: to_rules, ..ReadScope::default() }),
        ];
        let msg = MessageFacts { from: v.clone(), to: vec![v], ..msg };
        let d = evaluate_read(&grants, &conn, std::slice::from_ref(&msg), 10);
        prop_assert!(d.allowed.is_empty(), "allowed {:?} for {:?}", d.allowed, msg);
        prop_assert_eq!(d.needs_approval, vec![0]);
    }

    #[test]
    fn serde_round_trip_preserves_decisions(
        grants in grants(),
        msgs in vec(message(), 0..6),
        now in 0i64..200,
    ) {
        let json = serde_json::to_string(&grants).unwrap();
        let restored: Vec<Grant> = serde_json::from_str(&json).unwrap();
        let conn = ConnectionId::from("c1");
        prop_assert_eq!(evaluate_read(&grants, &conn, &msgs, now), evaluate_read(&restored, &conn, &msgs, now));
    }
}
