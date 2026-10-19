use super::*;
use crate::store::audit_push::tests::{push_call, summary};
use crate::store::tests::{audit_record, open};
use crate::store::{AuditEmail, AuditInfo, AuditMessage, AuditPush, AuditRequest};

const SHA: &str = "3f9c2d1e8b7a6f5e4d3c2b1a0f9e8d7c6b5a4f3e";

fn query(text: &str) -> ActivityQuery {
    ActivityQuery {
        text: text.to_owned(),
        ..ActivityQuery::default()
    }
}

fn ids(store: &Store, q: &ActivityQuery) -> Vec<i64> {
    store.search_activity(q, None, PAGE_MAX).unwrap().found.iter().map(|f| f.record.seq).collect()
}

fn parked(call: Value) -> ParkedRequest {
    ParkedRequest {
        request: reins_proto::relay::RelayRequest {
            v: 1,
            id: "r1".into(),
            connection_id: "c1".into(),
            connection_label: "laptop".to_owned(),
            created_at: 9,
            wait_until: None,
            account: None,
            call: ToolCall::Connector(serde_json::from_value(call).unwrap()),
        },
        messages: Vec::new(),
        covered: std::collections::BTreeMap::default(),
        account: None,
        accounts: Vec::new(),
        shared_accounts: Vec::new(),
        connector: None,
    }
}

fn email(at: i64, to: &str, subject: &str, body: &str) -> AuditRecord {
    AuditRecord {
        at,
        action: "send".to_owned(),
        outcome: "sent".to_owned(),
        detail: String::new(),
        info: AuditInfo {
            email: Some(AuditEmail {
                to: vec![to.to_owned()],
                cc: vec![],
                subject: subject.to_owned(),
                body: body.to_owned(),
            }),
            ..AuditInfo::default()
        },
        ..audit_record()
    }
}

fn push(at: i64) -> AuditRecord {
    let mut s = summary(2, 2, 3);
    s.updates[0].commits[0].sha = SHA.to_owned();
    s.updates[0].commits[0].subject = "Teach the parser about tabs".to_owned();
    s.updates[0].commits[1].files[1].path = "crates/core/src/search_index.rs".to_owned();
    s.updates[0].commits[1].files[1].patch = Some("@@ -1,1 +1,2 @@\n line\n+let quokka = 7;".to_owned());
    AuditRecord {
        at,
        service: "github".to_owned(),
        action: "write".to_owned(),
        op: "git_push".to_owned(),
        outcome: "released".to_owned(),
        detail: String::new(),
        info: AuditInfo {
            git: AuditPush::of_call(&push_call(&s)).map(Box::new),
            ..AuditInfo::default()
        },
        ..audit_record()
    }
}

fn command(at: i64, full: &str) -> AuditRecord {
    AuditRecord {
        at,
        service: "desktop".to_owned(),
        action: "write".to_owned(),
        op: "ask".to_owned(),
        connection_id: "c2".to_owned(),
        connection_label: "ThinkPad".to_owned(),
        detail: String::new(),
        info: AuditInfo {
            request: Some(Box::new(AuditRequest {
                agent: "Claude Code".to_owned(),
                full: full.to_owned(),
                cwd: "/home/ann/projects/web".to_owned(),
                ..AuditRequest::default()
            })),
            ..AuditInfo::default()
        },
        ..audit_record()
    }
}

/// The text of a snippet, and its set-apart pieces.
fn shown(s: &ActivitySnippet) -> (String, Vec<String>) {
    (
        s.spans.iter().map(|h| h.text.as_str()).collect(),
        s.spans.iter().filter(|h| h.hit).map(|h| h.text.clone()).collect(),
    )
}

#[test]
fn each_kind_of_field_is_found_with_where_it_matched() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let mail = store
        .append_audit(&email(
            10,
            "alice@example.com",
            "Quarterly numbers",
            "Hi Alice, the invoice for March is attached.",
        ))
        .unwrap();
    let mut read = audit_record();
    read.at = 11;
    read.info.messages = vec![AuditMessage {
        id: "m1".to_owned(),
        text: String::new(),
        from: "Stripe <billing@stripe.example>".to_owned(),
        subject: "Your receipt".to_owned(),
        date: 1,
    }];
    let read = store.append_audit(&read).unwrap();
    let pushed = store.append_audit(&push(12)).unwrap();
    let ran = store.append_audit(&command(13, "cargo test -p reins-core --features bindgen")).unwrap();
    let fetched = store.append_audit(&command(14, "curl -sS https://api.example.org/v2/status")).unwrap();
    // A tool of an MCP server, kept only in this device's copy of the approval.
    let mut tool = command(15, "");
    tool.service = "mcp".to_owned();
    tool.info.request = None;
    tool.info.snapshot = Some(Box::new(parked(serde_json::json!({"service": "mcp:linear", "op": "create_issue",
        "args": {"teamKey": "ENG", "title": "Flaky zeppelin test", "url": "https://linear.example/eng"}}))));
    let tool = store.append_audit(&tool).unwrap();

    let cases: [(&str, i64, Option<ActivityField>); 12] = [
        ("invoice", mail, Some(ActivityField::Body)),
        // The recipient is in the row's title already: found, without a snippet.
        ("alice@example.com", mail, None),
        ("quarterly", mail, Some(ActivityField::Subject)),
        ("billing@stripe", read, Some(ActivityField::Address)),
        ("search_index.rs", pushed, Some(ActivityField::File)),
        ("3f9c2d1", pushed, Some(ActivityField::Commit)),
        ("quokka", pushed, Some(ActivityField::Diff)),
        ("parser tabs", pushed, Some(ActivityField::Commit)),
        ("bindgen", ran, Some(ActivityField::Command)),
        ("api.example.org/v2", fetched, Some(ActivityField::Command)),
        ("zeppelin", tool, Some(ActivityField::Subject)),
        ("create_issue", tool, Some(ActivityField::Request)),
    ];
    for (text, want, field) in cases {
        let page = store.search_activity(&query(text), None, 50).unwrap();
        assert_eq!(page.found.iter().map(|f| f.record.seq).collect::<Vec<_>>(), [want], "{text}");
        let snippet = page.found[0].snippet.as_ref();
        assert_eq!(snippet.map(|s| s.field), field, "{text}");
        assert!(snippet.is_none_or(|s| !shown(s).1.is_empty()), "{text}: something set apart");
    }
    // A whole address is set apart as one piece; a URL argument of the tool is found too.
    let page = store.search_activity(&query("billing@stripe.example"), None, 50).unwrap();
    assert_eq!(shown(page.found[0].snippet.as_ref().unwrap()).1, ["Stripe", "billing@stripe.example"]);
    assert_eq!(ids(&store, &query("linear.example")), [tool]);
    // The agent and the connection: on the row already, so no snippet.
    let page = store.search_activity(&query("thinkpad"), None, 50).unwrap();
    assert_eq!(page.found.len(), 3);
    assert!(page.found.iter().all(|f| f.snippet.is_none()));
    assert_eq!(ids(&store, &query("claude code")).len(), 2, "the tool call kept no agent");
    // Every word must be found, as the start of a word.
    assert!(ids(&store, &query("invoice zeppelin")).is_empty());
    assert!(ids(&store, &query("nvoice")).is_empty());
    assert_eq!(ids(&store, &query("INV")), [mail], "case does not matter");
}

#[test]
fn filters_combine_with_each_other_and_with_words() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let mut denied_mail = email(100, "bob@example.com", "Plans", "invoice");
    denied_mail.outcome = "denied".to_owned();
    let denied_mail = store.append_audit(&denied_mail).unwrap();
    let sent_mail = store.append_audit(&email(200, "bob@example.com", "Plans", "invoice")).unwrap();
    let mut auto = push(300);
    auto.info.decided_by = "autopilot".to_owned();
    let auto = store.append_audit(&auto).unwrap();
    let mut overnight = command(400, "ls");
    overnight.info.decided_by = "overnight".to_owned();
    overnight.outcome = "error".to_owned();
    let overnight = store.append_audit(&overnight).unwrap();
    let mut bypass = command(500, "make deploy");
    bypass.info.decided_by = "bypass".to_owned();
    let bypass = store.append_audit(&bypass).unwrap();

    let q = |f: &dyn Fn(&mut ActivityQuery)| {
        let mut q = ActivityQuery::default();
        f(&mut q);
        ids(&store, &q)
    };
    assert_eq!(q(&|_| {}), [bypass, overnight, auto, sent_mail, denied_mail]);
    assert_eq!(q(&|q| q.outcomes = vec![ActivityOutcome::Denied]), [denied_mail]);
    assert_eq!(q(&|q| q.outcomes = vec![ActivityOutcome::Denied, ActivityOutcome::Failed]), [overnight, denied_mail]);
    assert_eq!(q(&|q| q.deciders = vec![ActivityDecider::Autopilot]), [overnight, auto], "Overnight is Autopilot");
    assert_eq!(q(&|q| q.deciders = vec![ActivityDecider::You]), [sent_mail, denied_mail]);
    assert_eq!(q(&|q| q.deciders = vec![ActivityDecider::Bypass, ActivityDecider::Lockdown]), [bypass]);
    assert_eq!(q(&|q| q.services = vec!["desktop".to_owned()]), [bypass, overnight]);
    assert_eq!(q(&|q| q.actions = vec!["send".to_owned()]), [sent_mail, denied_mail]);
    assert_eq!(q(&|q| q.connections = vec!["c2".to_owned()]), [bypass, overnight]);
    assert_eq!(
        q(&|q| {
            q.connections = vec!["c2".to_owned()];
            q.outcomes = vec![ActivityOutcome::Approved];
        }),
        [bypass]
    );
    assert_eq!(
        q(&|q| {
            q.since = Some(200);
            q.until = Some(400);
        }),
        [auto, sent_mail],
        "since is inclusive, until is not"
    );
    assert_eq!(
        q(&|q| {
            q.text = "invoice".to_owned();
            q.outcomes = vec![ActivityOutcome::Approved];
            q.deciders = vec![ActivityDecider::You];
            q.services = vec!["gmail".to_owned()];
            q.until = Some(300);
        }),
        [sent_mail]
    );
    assert!(
        q(&|q| {
            q.text = "invoice".to_owned();
            q.services = vec!["github".to_owned()];
        })
        .is_empty()
    );

    let stamps = store.activity_stamps(&ActivityQuery::default()).unwrap();
    assert_eq!(stamps.len(), 5);
    assert_eq!((stamps[1].outcome, stamps[1].decider), (ActivityOutcome::Failed, ActivityDecider::Autopilot));
    assert_eq!((stamps[4].outcome, stamps[4].decider), (ActivityOutcome::Denied, ActivityDecider::You));
    let facets = store.activity_facets().unwrap();
    assert_eq!(facets.services, ["desktop", "gmail", "github"]);
    assert_eq!(facets.connections.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["c2", "c1"]);
}

#[test]
fn pages_follow_each_other_without_gaps_or_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // Some entries share their second, and arrive out of order (synced from another phone).
    for i in 0..120_i64 {
        let at = 1_000 + (i * 7) % 50;
        store.append_audit(&email(at, "a@example.com", &format!("Report {i}"), "weekly report")).unwrap();
    }
    store.append_audit(&email(5_000, "z@example.com", "Other", "nothing")).unwrap();
    let mut seen = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let page = store.search_activity(&query("report"), after, 50).unwrap();
        assert_eq!(page.total, 120);
        pages += 1;
        seen.extend(page.found.iter().map(|f| (f.record.at, f.record.seq)));
        match page.next {
            Some(c) => after = Some(c),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), 120);
    let mut sorted = seen.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(seen, sorted, "newest first, each once");
    // A limit is bounded either way.
    assert_eq!(store.search_activity(&query("report"), None, 0).unwrap().found.len(), 1);
    assert_eq!(store.search_activity(&query("report"), None, 10_000).unwrap().found.len(), 120);
}

#[test]
fn hidden_characters_neither_hide_nor_fake_a_match() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // A soft hyphen inside "paypal", a right-to-left override, a zero-width space and a control character.
    let body =
        "Please pay\u{00AD}pal now. Sign at \u{202E}moc.elpmaxe\u{202C} or visit secure\u{200B}login today\u{0007}.";
    let seq = store.append_audit(&email(10, "x@example.com", "Hello", body)).unwrap();
    assert!(ids(&store, &query("paypal")).is_empty(), "a word split by a hidden character is two words, as shown");
    // Typed or pasted into the search, hidden characters are dropped.
    assert_eq!(ids(&store, &query("se\u{200B}cure")), [seq]);
    assert_eq!(ids(&store, &query("\u{202E}visit")), [seq]);
    for text in ["pay", "pal", "moc", "secure", "login", "today"] {
        let page = store.search_activity(&query(text), None, 10).unwrap();
        assert_eq!(page.found.len(), 1, "{text}");
        let snippet = page.found[0].snippet.as_ref().unwrap();
        let (whole, lit) = shown(snippet);
        assert!(!whole.chars().any(|c| text::is_invisible(c) || c.is_control()), "{text}: {whole:?}");
        assert!(whole.contains('\u{fffd}'), "hidden characters show: {whole:?}");
        assert_eq!(lit, [text], "{text}: only the word searched is set apart");
        // The pieces are the cleaned text: nothing added or reordered between them.
        assert!(clean(body).contains(whole.trim_matches('…')), "{whole:?}");
    }
    // A query that is only hidden characters or punctuation searches nothing: everything is listed, nothing set apart.
    let page = store.search_activity(&query("\u{200B}\u{202E} @@"), None, 10).unwrap();
    assert_eq!((page.found.len(), page.terms.len()), (1, 0));
    assert!(page.found[0].snippet.is_none());
    // FTS5 syntax in a query is only words.
    assert!(ids(&store, &query("pay OR \"x\" NEAR(a b) *")).is_empty());
    assert_eq!(ids(&store, &query("\"please\" pay*")), [seq]);
}

#[test]
fn highlights_cover_matches_only_and_join_across_punctuation() {
    let terms = parse("alice example com src/main").terms;
    let spans = highlight("Mail alice@example.com about src/main.rs and Alicea", &terms);
    let lit: Vec<&str> = spans.iter().filter(|s| s.hit).map(|s| s.text.as_str()).collect();
    assert_eq!(lit, ["alice@example.com", "src/main", "Alice"]);
    assert_eq!(
        spans.iter().map(|s| s.text.as_str()).collect::<String>(),
        "Mail alice@example.com about src/main.rs and Alicea"
    );
    // Not inside a word, and never across a space.
    assert!(hits("malice", &parse("alice").terms).is_empty());
    assert_eq!(hits("alice example", &parse("alice example").terms).len(), 2);
    // A long text: the window opens just before the match.
    let long = format!("{} the needle is here {}", "word ".repeat(200), "tail ".repeat(200));
    let spans = window(&long, &parse("needle").terms, long.find("needle").unwrap());
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert!(text.starts_with('…') && text.ends_with('…') && text.chars().count() < 160, "{text}");
    assert_eq!(spans.iter().filter(|s| s.hit).count(), 1);
}

#[test]
fn the_index_follows_the_entries_and_is_never_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    let first = store.append_audit(&email(1, "a@example.com", "One", "marmalade")).unwrap();
    assert_eq!(ids(&store, &query("marmalade")), [first]);
    let writes = store.write_count();
    let sealed = std::fs::read(dir.path().join("reins.sealed")).unwrap();
    // Searching writes nothing: the data is unchanged, the sealed copy too.
    for _ in 0..3 {
        store.search_activity(&query("marmalade"), None, 10).unwrap();
        store.activity_stamps(&query("marmalade")).unwrap();
    }
    assert_eq!(store.write_count(), writes);
    assert_eq!(std::fs::read(dir.path().join("reins.sealed")).unwrap(), sealed);
    {
        let conn = store.lock().unwrap();
        let main = conn.serialize("main").unwrap();
        assert!(!main.windows(9).any(|w| w == b"marmalade"), "no text in the database itself");
        assert!(!main.windows(12).any(|w| w == b"audit_search"), "no index in the database itself");
    }
    // A new entry is found at once; one replaced (an import, a merge) is indexed again; one gone is not found.
    let second = store.append_audit(&email(2, "b@example.com", "Two", "marmalade toast")).unwrap();
    assert_eq!(ids(&store, &query("marmalade")), [second, first]);
    let replacement = {
        let info = serde_json::to_vec(&email(1, "a@example.com", "One", "porridge").info).unwrap();
        store.seal(super::super::audit::INFO_AAD, &info).unwrap()
    };
    store
        .lock()
        .unwrap()
        .execute("UPDATE audit SET info = ?1 WHERE seq = ?2", rusqlite::params![replacement, first])
        .unwrap();
    assert_eq!(ids(&store, &query("marmalade")), [second]);
    assert_eq!(ids(&store, &query("porridge")), [first]);
    store.lock().unwrap().execute("DELETE FROM audit WHERE seq = ?1", [second]).unwrap();
    assert!(ids(&store, &query("marmalade")).is_empty());
    // An approval kept later (or pruned) changes what is found.
    let mut ran = command(3, "");
    ran.info.request = None;
    let ran = store.append_audit(&ran).unwrap();
    assert!(ids(&store, &query("frobnicate")).is_empty());
    let snap = store
        .seal(
            "audit.snapshot",
            &serde_json::to_vec(&parked(serde_json::json!({"service": "desktop", "op": "ask",
                "args": {"command": "frobnicate --all"}})))
            .unwrap(),
        )
        .unwrap();
    store
        .lock()
        .unwrap()
        .execute("INSERT INTO audit_snapshots (seq, data) VALUES (?1, ?2)", rusqlite::params![ran, snap])
        .unwrap();
    assert_eq!(ids(&store, &query("frobnicate")), [ran]);
    store.lock().unwrap().execute("DELETE FROM audit_snapshots WHERE seq = ?1", [ran]).unwrap();
    assert!(ids(&store, &query("frobnicate")).is_empty());
}

#[test]
fn a_day_is_from_its_midnight_to_the_next_in_any_time_zone() {
    // The phone asks for a day as [its midnight, the next midnight) in its own time zone: an entry one second before
    // midnight belongs to the day before, one at midnight to the day it starts.
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // 2026-03-08 in New York: clocks go forward at 02:00, so that day is 23 hours long (05:00Z to 04:00Z the next day).
    let midnight = 1_772_946_000; // 2026-03-08T00:00:00-05:00
    let next = midnight + 23 * 3_600; // 2026-03-09T00:00:00-04:00
    let before = store.append_audit(&email(midnight - 1, "a@example.com", "s", "late")).unwrap();
    let start = store.append_audit(&email(midnight, "a@example.com", "s", "early")).unwrap();
    let end = store.append_audit(&email(next - 1, "a@example.com", "s", "late")).unwrap();
    let after = store.append_audit(&email(next, "a@example.com", "s", "early")).unwrap();
    let day = |since: i64, until: i64| {
        ids(
            &store,
            &ActivityQuery {
                since: Some(since),
                until: Some(until),
                ..ActivityQuery::default()
            },
        )
    };
    assert_eq!(day(midnight, next), [end, start]);
    assert_eq!(day(midnight - 86_400, midnight), [before]);
    assert_eq!(day(next, next + 86_400), [after]);
    // The same instants in Kolkata (+05:30) group otherwise: the two around New York's first midnight on the 8th, the
    // two around its second on the 9th.
    let kolkata = 1_772_908_200; // 2026-03-08T00:00:00+05:30
    assert_eq!(day(kolkata, kolkata + 86_400), [start, before]);
    assert_eq!(day(kolkata + 86_400, kolkata + 2 * 86_400), [after, end]);
}

#[test]
fn ten_thousand_entries_stay_quick() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    // More than the retention: written as a merge would, without pruning.
    {
        let words = ["invoice", "deploy", "receipt", "standup", "migration", "keyboard", "lantern", "harbor"];
        let rows: Vec<(i64, Vec<u8>)> = (0..10_000_usize)
            .map(|i| {
                let mut r = email(i64::try_from(i).unwrap(), &format!("user{i}@example.com"), &format!("Note {i}"), "");
                r.info.email.as_mut().unwrap().body =
                    format!("{} and {} for ticket T{i}", words[i % 8], words[i * 3 % 8]);
                let info = serde_json::to_vec(&r.info).unwrap();
                (i64::try_from(i).unwrap(), store.seal(super::super::audit::INFO_AAD, &info).unwrap())
            })
            .collect();
        let mut conn = store.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for (at, info) in rows {
            tx.execute(
                "INSERT INTO audit (at, connection_id, connection_label, action, outcome, service, item_count, info, op) \
                 VALUES (?1, 'c1', 'ChatGPT', 'send', 'sent', 'gmail', 1, ?2, '')",
                rusqlite::params![at, info],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let started = std::time::Instant::now();
    let page = store.search_activity(&query("invoice"), None, 50).unwrap();
    let built = started.elapsed();
    assert_eq!(page.total, 1_250);
    assert_eq!(page.found.len(), 50);
    let started = std::time::Instant::now();
    let mut after = page.next;
    for _ in 0..5 {
        let page = store.search_activity(&query("lantern harbor"), after, 50).unwrap();
        after = page.next;
    }
    let one = store.search_activity(&query("t9999"), None, 50).unwrap();
    assert_eq!(one.found.len(), 1);
    let stamps = store.activity_stamps(&ActivityQuery::default()).unwrap();
    assert_eq!(stamps.len(), 10_000);
    let searched = started.elapsed();
    eprintln!("10k entries: index built and first page in {built:?}; 7 more searches in {searched:?}");
    // Generous bounds for a debug build on a busy machine; a release build is many times faster.
    assert!(built < std::time::Duration::from_secs(20), "{built:?}");
    assert!(searched < std::time::Duration::from_secs(5), "{searched:?}");
}

#[test]
fn ambiguous_and_withheld_execution_states_need_attention_and_are_not_approved() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    for (index, outcome) in ["error", "started", "unknown", "succeeded_withheld", "sent"].into_iter().enumerate() {
        let mut record = audit_record();
        record.at = i64::try_from(index).unwrap();
        record.outcome = outcome.to_owned();
        store.append_audit(&record).unwrap();
    }
    let attention = ActivityQuery {
        outcomes: vec![ActivityOutcome::Failed],
        ..ActivityQuery::default()
    };
    assert_eq!(ids(&store, &attention).len(), 4);
    let approved = ActivityQuery {
        outcomes: vec![ActivityOutcome::Approved],
        ..ActivityQuery::default()
    };
    assert_eq!(ids(&store, &approved).len(), 1);
}
