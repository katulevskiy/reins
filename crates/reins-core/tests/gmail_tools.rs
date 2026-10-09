//! Gmail beyond search, read and send, against a fake Gmail: organizing, Trash, labels, drafts with a file,
//! attachments, filters and the vacation reply. What each asks Gmail for, what the user is shown, and what it refuses.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::FakeGoogle;
use data_encoding::{BASE64, BASE64URL};
use reins_core::connector::Connector;
use reins_core::connector::gmail::GmailTools;
use reins_proto::connector::{ConnectorCall, spec_for_tool};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const ME: &str = "me@gmail.com";

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool).unwrap().parse(args).unwrap().0
}

fn gmail(server: &MockServer) -> GmailTools {
    GmailTools::new(
        reins_core::http::client().unwrap(),
        &server.uri(),
        Arc::new(FakeGoogle::new()),
        Duration::from_millis(1),
    )
}

async fn labels(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/users/me/labels"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"labels": [
            {"id": "INBOX", "name": "INBOX", "type": "system"},
            {"id": "SPAM", "name": "SPAM", "type": "system"},
            {"id": "TRASH", "name": "TRASH", "type": "system"},
            {"id": "Label_7", "name": "Receipts", "type": "user"}]})))
        .mount(server)
        .await;
}

async fn message(server: &MockServer, id: &str, from: &str, subject: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/users/me/messages/{id}")))
        .and(query_param("format", "metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": id, "threadId": "t", "payload": {
            "headers": [{"name": "From", "value": from}, {"name": "Subject", "value": subject}]}})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn organizing_shows_who_and_how_many_then_changes_labels_in_one_batch() {
    let server = MockServer::start().await;
    labels(&server).await;
    message(&server, "m1", "Deals <deals@shop.example>", "50% off").await;
    message(&server, "m2", "deals@shop.example", "Last chance").await;
    message(&server, "m3", "news@paper.example", "Morning brief").await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/batchModify"))
        .and(body_json(json!({"ids": ["m1", "m2", "m3"], "addLabelIds": [], "removeLabelIds": ["INBOX"]})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let archive = call("gmail_organize", &json!({"message_ids": ["m1", "m2", "m3", "m1"], "action": "archive"}));
    let g = gmail(&server);
    let preview = g.preview(ME, &archive).await.unwrap();
    assert_eq!(preview.resource, "mailbox");
    assert_eq!(preview.lines[0], "Archive 3 messages");
    assert!(
        preview.lines.iter().any(|l| l == "From: deals@shop.example (2), news@paper.example (1)"),
        "{:?}",
        preview.lines
    );
    assert!(preview.lines.iter().any(|l| l == "• news@paper.example — Morning brief"), "{:?}", preview.lines);
    let done = g.perform(ME, &archive).await.unwrap();
    assert_eq!((done["changed"].as_u64(), done["undo_with"].as_str()), (Some(3), Some("move_to_inbox")));
}

#[tokio::test]
async fn labels_are_found_by_name_and_gmail_s_own_cannot_be_set_by_hand() {
    let server = MockServer::start().await;
    labels(&server).await;
    message(&server, "m1", "a@b.example", "Invoice").await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/batchModify"))
        .and(body_json(json!({"ids": ["m1"], "addLabelIds": ["Label_7"], "removeLabelIds": []})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let g = gmail(&server);
    let label = call("gmail_organize", &json!({"message_ids": ["m1"], "action": "add_label", "label": "receipts"}));
    assert_eq!(g.preview(ME, &label).await.unwrap().lines[0], "Label \"Receipts\": 1 message");
    g.perform(ME, &label).await.unwrap();
    for (args, expect) in [
        (json!({"message_ids": ["m1"], "action": "add_label", "label": "TRASH"}), "gmail_trash"),
        (json!({"message_ids": ["m1"], "action": "add_label", "label": "Nope"}), "no label"),
        (json!({"message_ids": ["m1"], "action": "add_label"}), "needs `label`"),
        (json!({"message_ids": ["../../x"], "action": "archive"}), "not a Gmail message id"),
    ] {
        let err = g.preview(ME, &call("gmail_organize", &args)).await.unwrap_err();
        assert!(err.to_string().contains(expect), "{args}: {err}");
    }
    let rename = call("gmail_rename_label", &json!({"label": "INBOX", "new_name": "x"}));
    assert!(g.preview(ME, &rename).await.unwrap_err().to_string().contains("Gmail's own"));
}

#[tokio::test]
async fn trash_moves_each_message_and_reports_the_ones_that_failed() {
    let server = MockServer::start().await;
    message(&server, "m1", "a@b.example", "One").await;
    message(&server, "m2", "a@b.example", "Two").await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/m1/trash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/me/messages/m2/trash"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let g = gmail(&server);
    let trash = call("gmail_trash", &json!({"message_ids": ["m1", "m2"]}));
    let preview = g.preview(ME, &trash).await.unwrap();
    assert_eq!(preview.lines[0], "Move 2 messages to Trash");
    assert!(preview.lines[1].contains("30 days"));
    let done = g.perform(ME, &trash).await.unwrap();
    assert_eq!((done["moved"].as_u64(), done["failed"].clone()), (Some(1), json!(["m2"])));
    Mock::given(method("POST"))
        .and(path("/users/me/messages/m1/untrash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m1"})))
        .expect(1)
        .mount(&server)
        .await;
    let restore = call("gmail_trash", &json!({"message_ids": ["m1"], "action": "restore"}));
    assert_eq!(g.preview(ME, &restore).await.unwrap().lines[0], "Move 1 message out of Trash");
    assert_eq!(g.perform(ME, &restore).await.unwrap()["undo_with"], "trash");
}

#[tokio::test]
async fn a_draft_with_a_file_is_saved_in_its_thread_and_nothing_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/orig1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "orig1", "threadId": "thr9", "payload": {
            "headers": [{"name": "Message-ID", "value": "<orig@mail.example>"}]}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/me/drafts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "d1", "message": {"id": "dm1"}})))
        .expect(1)
        .mount(&server)
        .await;
    let draft = call(
        "gmail_create_draft",
        &json!({"to": ["Ada@Example.com"], "subject": "Re: plan", "body": "Yes, see attached.",
                "reply_to_message_id": "orig1", "file_name": "plan.pdf", "content_base64": BASE64.encode(b"%PDF-1.4")}),
    );
    let g = gmail(&server);
    let preview = g.preview(ME, &draft).await.unwrap();
    assert_eq!(preview.resource, "drafts");
    assert_eq!(preview.lines[0], "Draft to: ada@example.com");
    assert!(preview.lines.iter().any(|l| l.starts_with("Attached: plan.pdf")), "{:?}", preview.lines);
    assert!(preview.lines.last().unwrap().contains("Nothing is sent"));
    let done = g.perform(ME, &draft).await.unwrap();
    assert_eq!(done["draft_id"], "d1");
    let sent: Vec<Request> = server.received_requests().await.unwrap();
    let saved = sent.iter().find(|r| r.url.path() == "/users/me/drafts").unwrap();
    let body: Value = serde_json::from_slice(&saved.body).unwrap();
    assert_eq!(body["message"]["threadId"], "thr9");
    let raw =
        String::from_utf8(BASE64URL.decode(body["message"]["raw"].as_str().unwrap().as_bytes()).unwrap()).unwrap();
    assert!(raw.contains("In-Reply-To: <orig@mail.example>") && raw.contains("multipart/mixed"), "{raw}");
    assert!(raw.contains("Content-Type: application/pdf; name=\"plan.pdf\""), "{raw}");
    assert!(!sent.iter().any(|r| r.url.path().ends_with("/send")), "a draft is never sent");
    let spec = spec_for_tool("gmail_create_draft").unwrap();
    assert!(spec.parse(&json!({"to": ["a@b.com\r\nBcc: e@x.com"], "subject": "s", "body": "b"})).is_err());
    let named = call("gmail_create_draft", &json!({"to": ["Eve <eve@x.com>"], "subject": "s", "body": "b"}));
    assert!(g.preview(ME, &named).await.is_err(), "only bare addresses reach a header");
}

#[tokio::test]
async fn attachments_are_listed_by_part_and_fetched_with_a_fresh_attachment_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/m5"))
        .and(query_param("format", "full"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "m5", "threadId": "t", "payload": {
            "partId": "", "mimeType": "multipart/mixed", "headers": [{"name": "Subject", "value": "Your invoice"}],
            "parts": [
                {"partId": "0", "mimeType": "text/plain", "filename": "", "body": {"data": "SGk", "size": 2}},
                {"partId": "1", "mimeType": "application/pdf", "filename": "invoice.pdf",
                 "body": {"attachmentId": "ANGjdJ_fresh", "size": 5}}]}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/messages/m5/attachments/ANGjdJ_fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"size": 5, "data": BASE64URL.encode(b"%PDF!")})))
        .mount(&server)
        .await;
    let g = gmail(&server);
    let listed = g.fetch(ME, &call("gmail_list_attachments", &json!({"message_id": "m5"}))).await.unwrap();
    assert_eq!(listed.len(), 1, "the message text is not an attachment");
    assert_eq!((listed[0].id.as_str(), listed[0].title.as_str()), ("1", "invoice.pdf"));
    assert_eq!(listed[0].resource_label, "Your invoice");
    let got = g.fetch(ME, &call("gmail_get_attachment", &json!({"message_id": "m5", "part": "1"}))).await.unwrap();
    assert_eq!(got[0].body.as_deref(), Some(BASE64.encode(b"%PDF!").as_str()));
    assert!(got[0].secret, "file bytes are never shown in the approval or the activity");
    assert_eq!(got[0].text(), "invoice.pdf · application/pdf · 5 bytes");
    let missing = g.fetch(ME, &call("gmail_get_attachment", &json!({"message_id": "m5", "part": "9"}))).await;
    assert!(missing.unwrap_err().to_string().contains("no such attachment"));
}

#[tokio::test]
async fn filters_are_explained_never_forward_and_need_a_condition_and_an_action() {
    let server = MockServer::start().await;
    labels(&server).await;
    Mock::given(method("POST"))
        .and(path("/users/me/settings/filters"))
        .and(body_json(json!({"criteria": {"from": "news@paper.example"},
            "action": {"addLabelIds": ["Label_7"], "removeLabelIds": ["INBOX"]}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "f1"})))
        .expect(1)
        .mount(&server)
        .await;
    let g = gmail(&server);
    let filter = call(
        "gmail_create_filter",
        &json!({"from": "news@paper.example", "skip_inbox": true, "add_label": "Receipts"}),
    );
    let preview = g.preview(ME, &filter).await.unwrap();
    assert_eq!(preview.lines[0], "New filter: Mail from news@paper.example");
    assert_eq!(preview.lines[1], "Then: skip the Inbox, label \"Receipts\"");
    assert_eq!(g.perform(ME, &filter).await.unwrap()["filter_id"], "f1");
    for (args, expect) in [
        (json!({"skip_inbox": true}), "needs a condition"),
        (json!({"from": "a@b.example"}), "needs an action"),
        (json!({"from": "a@b.example", "add_label": "SPAM"}), "cannot set"),
    ] {
        let err = g.preview(ME, &call("gmail_create_filter", &args)).await.unwrap_err();
        assert!(err.to_string().contains(expect), "{args}: {err}");
    }
    assert!(spec_for_tool("gmail_create_filter").unwrap().parse(&json!({"from": "a", "forward": "e@x.com"})).is_err());
}

#[tokio::test]
async fn the_vacation_reply_is_read_and_set_with_its_dates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/settings/vacation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"enableAutoReply": true,
            "responseSubject": "Away", "responseBodyPlainText": "Back Monday", "endTime": "1791504000000"})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/users/me/settings/vacation"))
        .and(body_json(json!({"enableAutoReply": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let g = gmail(&server);
    let read = g.fetch(ME, &call("gmail_get_vacation", &json!({}))).await.unwrap();
    assert_eq!((read[0].snippet.as_str(), read[0].body.as_deref()), ("On", Some("Back Monday")));
    assert_eq!(read[0].extra["end"], "2026-10-09T00:00:00Z");
    let off = call("gmail_set_vacation", &json!({"enabled": false}));
    assert_eq!(g.preview(ME, &off).await.unwrap().lines, ["Turn the vacation reply off"]);
    g.perform(ME, &off).await.unwrap();
}
