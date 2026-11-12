use super::*;
use crate::autopilot::types::{RiskLevel, RiskView};
use crate::types::{ActivityExecution, ActivityInfo, ActivityMessage, ActivityRequest, EmailView, RequestLabel};
use serde_json::Value;

/// 2026-11-12T09:30:00Z
const NOW: i64 = 1_794_475_800;

fn entry_at(id: i64, at: i64) -> ActivityEntry {
    ActivityEntry {
        id,
        at,
        connection_id: "c1".to_owned(),
        connection_label: "Laptop".to_owned(),
        action: "send".to_owned(),
        outcome: "sent".to_owned(),
        detail: String::new(),
        grant_id: None,
        service: "gmail".to_owned(),
        account: Some("me@example.com".to_owned()),
        count: 1,
        info: ActivityInfo {
            query: None,
            messages: Vec::new(),
            email: None,
            note: None,
            grant_summary: None,
            accounts: Vec::new(),
        },
        op: String::new(),
        op_title: "Send an email".to_owned(),
        decided_by: String::new(),
        autopilot: None,
        label: None,
        request: None,
        hold: None,
        execution: None,
    }
}

fn parse(e: &ActivityExport) -> Value {
    serde_json::from_str(&e.contents).unwrap()
}

/// Parses RFC 4180 text back into rows, to check the escaping round-trips.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') if chars.peek() == Some(&'\n') => {
                chars.next();
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (_, c) => field.push(c),
        }
    }
    assert!(field.is_empty() && row.is_empty(), "the last row ends in CRLF");
    rows
}

#[test]
fn rfc3339_dates() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(NOW), "2026-11-12T09:30:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
}

#[test]
fn file_names_and_types() {
    let csv = export(ActivityExportFormat::Csv, &[], NOW);
    assert_eq!((csv.file_name.as_str(), csv.mime_type.as_str()), ("reins-decisions-2026-11-12.csv", "text/csv"));
    let json = export(ActivityExportFormat::Json, &[], NOW);
    assert_eq!(
        (json.file_name.as_str(), json.mime_type.as_str()),
        ("reins-decisions-2026-11-12.json", "application/json")
    );
}

#[test]
fn empty_history() {
    let csv = export(ActivityExportFormat::Csv, &[], NOW);
    assert_eq!(csv.entries, 0);
    assert_eq!(csv.contents, format!("{}\r\n", CSV_COLUMNS.join(",")));
    let json = export(ActivityExportFormat::Json, &[], NOW);
    assert_eq!(json.entries, 0);
    assert_eq!(parse(&json)["entries"], serde_json::json!([]));
}

#[test]
fn json_header_and_entry_fields() {
    let mut e = entry_at(7, NOW - 60);
    e.outcome = "denied".to_owned();
    e.decided_by = "autopilot".to_owned();
    e.op_title = "Read Telegram messages".to_owned();
    e.info.note = Some("looked wrong".to_owned());
    e.info.grant_summary = Some("for 1 hour".to_owned());
    e.label = Some(RequestLabel {
        provider: "telegram".to_owned(),
        action: "read".to_owned(),
        lead: "Read".to_owned(),
        subject: "secret chat".to_owned(),
        ..RequestLabel::default()
    });
    e.request = Some(ActivityRequest {
        agent: "Claude Code".to_owned(),
        cwd: "/work".to_owned(),
        why: "matched rule".to_owned(),
        purpose: "check inbox".to_owned(),
        asked_at: NOW - 90,
        ..ActivityRequest::default()
    });
    e.execution = Some(ActivityExecution {
        state: ExecutionState::Done,
        records: vec![8],
        started_at: NOW - 59,
    });
    let out = export(ActivityExportFormat::Json, &[e], NOW);
    assert_eq!(out.entries, 1);
    let v = parse(&out);
    assert_eq!(v["format"], "reins.decision-history");
    assert_eq!(v["version"], 1);
    assert_eq!(v["exported_at"], "2026-11-12T09:30:00Z");
    assert_eq!(v["scope"], "this-device");
    let x = &v["entries"][0];
    assert_eq!(x["id"], 7);
    assert_eq!(x["at"], "2026-11-12T09:29:00Z");
    assert_eq!(x["decision"], "denied");
    assert_eq!(x["decided_by"], "autopilot");
    assert_eq!(x["title"], "Read Telegram messages");
    assert_eq!(x["connection"], "Laptop");
    assert_eq!(x["label"], serde_json::json!({"provider": "telegram", "action": "read"}));
    assert_eq!(x["request"]["agent"], "Claude Code");
    assert_eq!(x["request"]["asked_at"], "2026-11-12T09:28:30Z");
    assert_eq!(x["note"], "looked wrong");
    assert_eq!(x["grant_summary"], "for 1 hour");
    assert_eq!(x["execution"], serde_json::json!({"state": "done", "started_at": "2026-11-12T09:29:01Z"}));
    assert!(!out.contents.contains("secret chat"), "a label's subject is not exported");
}

#[test]
fn json_omits_absent_values_and_normalises() {
    let mut failed = entry_at(2, 100);
    failed.outcome = "error".to_owned();
    failed.account = None;
    failed.op_title = String::new();
    let mut doubtful = entry_at(3, 200);
    doubtful.execution = Some(ActivityExecution {
        state: ExecutionState::Unknown,
        records: vec![],
        started_at: 0,
    });
    let v = parse(&export(ActivityExportFormat::Json, &[failed, doubtful, entry_at(4, 300)], NOW));
    let e = v["entries"].as_array().unwrap();
    assert_eq!(e[0]["decision"], "failed");
    assert_eq!(e[1]["decision"], "failed", "an unknown execution needs attention, however the row ended");
    assert_eq!(e[2]["decision"], "approved");
    assert_eq!(e[2]["decided_by"], "you");
    for x in e {
        for absent in ["request", "autopilot", "note", "grant_summary", "hold", "execution", "label"] {
            if x.get(absent).is_some() {
                assert!(x["id"] == 3 && absent == "execution", "{absent} should be absent");
            }
        }
        assert!(!x.as_object().unwrap().values().any(Value::is_null), "no nulls");
    }
    assert!(e[0].get("account").is_none() && e[0].get("title").is_none());
}

#[test]
fn json_autopilot_and_hold() {
    let mut e = entry_at(1, 10);
    e.decided_by = "autopilot".to_owned();
    e.autopilot = Some(AutopilotNote {
        mode: AutopilotMode::Auto,
        suggested: Verdict::Approve,
        p_approve: 0.9,
        p_deny: 0.1,
        confidence: 0.9,
        profile_id: "p".to_owned(),
        profile_name: "Default".to_owned(),
        neighbours: vec!["approved: secret thing".to_owned()],
        reason: "usual".to_owned(),
        correctable: true,
        risk: Some(RiskView {
            level: RiskLevel::Risky,
            p_catastrophic: 0.01,
            why: "deletes".to_owned(),
            held: false,
        }),
        matrix: None,
        request_id: "r-1".to_owned(),
    });
    e.hold = Some(crate::send_hold::ActivityHold {
        id: "h".to_owned(),
        release_at: NOW,
        state: HoldState::Waiting,
    });
    let out = export(ActivityExportFormat::Json, &[e], NOW);
    let x = &parse(&out)["entries"][0];
    assert_eq!(
        x["autopilot"],
        serde_json::json!({"mode": "auto", "suggested": "approve", "confidence": 0.9, "risk": "risky", "reason": "usual"})
    );
    assert_eq!(x["hold"], serde_json::json!({"state": "waiting", "release_at": "2026-11-12T09:30:00Z"}));
    assert!(!out.contents.contains("secret thing"), "similar past decisions are not exported");
}

#[test]
fn csv_columns_and_rows() {
    let mut e = entry_at(1, NOW);
    e.decided_by = "rule".to_owned();
    e.count = 3;
    e.request = Some(ActivityRequest {
        agent: "Claude Code".to_owned(),
        purpose: "p".to_owned(),
        why: "w".to_owned(),
        ..ActivityRequest::default()
    });
    e.autopilot = Some(AutopilotNote {
        mode: AutopilotMode::Manual,
        suggested: Verdict::Ask,
        p_approve: 0.0,
        p_deny: 0.0,
        confidence: 0.875,
        profile_id: String::new(),
        profile_name: String::new(),
        neighbours: vec![],
        reason: String::new(),
        correctable: false,
        risk: None,
        matrix: None,
        request_id: String::new(),
    });
    let out = export(ActivityExportFormat::Csv, &[e], NOW);
    assert!(out.contents.ends_with("\r\n") && !out.contents.replace("\r\n", "").contains('\n'));
    let rows = parse_csv(&out.contents);
    assert_eq!(rows[0], CSV_COLUMNS);
    assert_eq!(
        rows[1],
        [
            "2026-11-12T09:30:00Z",
            "approved",
            "rule",
            "gmail",
            "me@example.com",
            "send",
            "Send an email",
            "Laptop",
            "Claude Code",
            "p",
            "w",
            "",
            "3",
            "",
            "ask",
            "0.88"
        ]
    );
    assert_eq!(rows.len(), 2);
}

#[test]
fn csv_quoting() {
    let mut e = entry_at(1, NOW);
    e.request = Some(ActivityRequest {
        agent: "a, b".to_owned(),
        why: "say \"hi\"\nthen\r\nbye".to_owned(),
        purpose: "plain".to_owned(),
        ..ActivityRequest::default()
    });
    let out = export(ActivityExportFormat::Csv, &[e], NOW);
    assert!(out.contents.contains("\"a, b\""));
    assert!(out.contents.contains("\"say \"\"hi\"\"\nthen\r\nbye\""));
    assert!(out.contents.contains(",plain,"), "a plain field is not quoted");
    let rows = parse_csv(&out.contents);
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[1][8].as_str(), rows[1][10].as_str()), ("a, b", "say \"hi\"\nthen\r\nbye"));
}

#[test]
fn csv_formula_injection() {
    for (cell, guarded) in [
        ("=SUM(A1)", "'=SUM(A1)"),
        ("+1+1", "'+1+1"),
        ("-2", "'-2"),
        ("@cmd", "'@cmd"),
        ("\tx", "'\tx"),
        ("\rx", "'\rx"),
        ("safe=ok", "safe=ok"),
    ] {
        let mut e = entry_at(1, NOW);
        e.op_title = cell.to_owned();
        e.connection_label = cell.to_owned();
        e.request = Some(ActivityRequest {
            agent: cell.to_owned(),
            ..ActivityRequest::default()
        });
        e.info.note = Some(cell.to_owned());
        let rows = parse_csv(&export(ActivityExportFormat::Csv, &[e], NOW).contents);
        for column in [6, 7, 8, 11] {
            assert_eq!(rows[1][column], guarded, "column {}", CSV_COLUMNS[column]);
        }
    }
    // Numbers are not touched, and the JSON keeps the text as it is (it is not a spreadsheet).
    let mut e = entry_at(1, NOW);
    e.op_title = "=1".to_owned();
    let out = export(ActivityExportFormat::Json, &[e], NOW);
    assert_eq!(parse(&out)["entries"][0]["title"], "=1");
}

#[test]
fn privacy_nothing_of_the_content_leaks() {
    let mut e = entry_at(1, NOW);
    e.detail = "DETAIL-LEAK".to_owned();
    e.info = ActivityInfo {
        query: Some("QUERY-LEAK".to_owned()),
        messages: vec![ActivityMessage {
            id: "m".to_owned(),
            text: "MESSAGE-LEAK".to_owned(),
            from: "FROM-LEAK@example.com".to_owned(),
            subject: "SUBJECT-LEAK".to_owned(),
            date: 1,
        }],
        email: Some(EmailView {
            to: vec!["TO-LEAK@example.com".to_owned()],
            cc: vec!["CC-LEAK@example.com".to_owned()],
            subject: "EMAIL-SUBJECT-LEAK".to_owned(),
            body: "BODY-LEAK".to_owned(),
        }),
        note: Some("kept note".to_owned()),
        grant_summary: None,
        accounts: vec!["ACCOUNTS-LEAK@example.com".to_owned()],
    };
    e.request = Some(ActivityRequest {
        agent: "Claude Code".to_owned(),
        full: "FULL-LEAK curl -H 'Authorization: Bearer SECRET-LEAK'".to_owned(),
        ..ActivityRequest::default()
    });
    e.label = Some(RequestLabel {
        provider: "gmail".to_owned(),
        action: "send".to_owned(),
        lead: "Send an email to".to_owned(),
        subject: "LABEL-SUBJECT-LEAK@example.com".to_owned(),
        tail: " and TAIL-LEAK".to_owned(),
        ..RequestLabel::default()
    });
    e.op_title = String::new();
    for format in [ActivityExportFormat::Csv, ActivityExportFormat::Json] {
        let out = export(format, &[e.clone()], NOW).contents;
        assert!(out.contains("kept note") && out.contains("Claude Code"));
        assert!(out.contains("Send an email to"));
        assert!(!out.contains("LEAK"), "{format:?} leaked: {out}");
    }
}

#[test]
fn order_is_kept() {
    let entries = [entry_at(9, 300), entry_at(5, 200), entry_at(2, 100)];
    let v = parse(&export(ActivityExportFormat::Json, &entries, NOW));
    let ids: Vec<_> = v["entries"].as_array().unwrap().iter().map(|e| e["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, [9, 5, 2]);
    let rows = parse_csv(&export(ActivityExportFormat::Csv, &entries, NOW).contents);
    assert_eq!(
        rows.iter().skip(1).map(|r| r[0].as_str()).collect::<Vec<_>>(),
        ["1970-01-01T00:05:00Z", "1970-01-01T00:03:20Z", "1970-01-01T00:01:40Z"]
    );
}
