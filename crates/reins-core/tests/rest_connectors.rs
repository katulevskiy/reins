//! Google Calendar, Google Contacts and GitHub against fake servers: what they ask for, what they make of the answers,
//! and what they do when told to write.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FakeGoogle, FakeKeys};
use reins_core::connector::Connector;
use reins_core::connector::calendar::{GoogleCalendar, GoogleContacts};
use reins_core::connector::github::GitHub;
use reins_core::store::Store;
use reins_core::{CoreError, GmailStatus, GoogleTokenProvider};
use reins_proto::connector::{ConnectorCall, spec_for_tool};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool).unwrap().parse(args).unwrap().0
}

fn google() -> Arc<dyn GoogleTokenProvider> {
    Arc::new(FakeGoogle::new())
}

fn client() -> reqwest::Client {
    reins_core::http::client().unwrap()
}

// ---- Google Calendar ---------------------------------------------------------------------------------------------

fn calendar(server: &MockServer) -> GoogleCalendar {
    GoogleCalendar::new(client(), &server.uri(), google(), Duration::from_millis(1))
}

#[tokio::test]
async fn calendars_and_events_are_listed_with_their_names_and_times() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me/calendarList"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"id": "me@gmail.com", "summary": "me@gmail.com", "primary": true, "accessRole": "owner"},
            {"id": "team@group.calendar.google.com", "summary": "Team", "summaryOverride": "Our team", "accessRole": "reader"}]})))
        .mount(&server)
        .await;
    let calendars =
        calendar(&server).fetch("me@gmail.com", &call("calendar_list_calendars", &json!({}))).await.unwrap();
    assert_eq!(
        calendars.iter().map(|c| (c.id.as_str(), c.title.as_str(), c.from.as_str())).collect::<Vec<_>>(),
        [("primary", "me@gmail.com", "owner"), ("team@group.calendar.google.com", "Our team", "reader")]
    );

    Mock::given(method("GET"))
        .and(path("/calendars/primary/events"))
        .and(query_param("timeMin", "2026-10-05T00:00:00Z"))
        .and(query_param("timeMax", "2026-10-06T00:00:00Z"))
        .and(query_param("q", "dentist"))
        .and(query_param("singleEvents", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"id": "e1", "summary": "Dentist", "location": "Main St 1", "description": "Bring the card",
             "start": {"dateTime": "2026-10-05T14:00:00+02:00"}, "end": {"dateTime": "2026-10-05T15:00:00+02:00"},
             "organizer": {"email": "dr@clinic.example"}, "attendees": [{"email": "me@gmail.com"}], "htmlLink": "https://cal/e1"},
            {"id": "e2", "status": "cancelled", "summary": "Gone"},
            {"id": "e3", "summary": "Holiday", "start": {"date": "2026-10-05"}, "end": {"date": "2026-10-06"}}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/calendarList/primary"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let events = calendar(&server)
        .fetch(
            "me@gmail.com",
            &call("calendar_list_events", &json!({"from": "2026-10-05", "to": "2026-10-06", "query": "dentist"})),
        )
        .await
        .unwrap();
    assert_eq!(events.len(), 2, "a cancelled event is not listed");
    let dentist = &events[0];
    assert_eq!((dentist.id.as_str(), dentist.resource.as_str(), dentist.title.as_str()), ("e1", "primary", "Dentist"));
    assert_eq!(dentist.date, 1_791_201_600, "14:00 at +02:00 is 12:00 UTC");
    assert_eq!(dentist.body.as_deref(), Some("Bring the card"));
    assert_eq!(dentist.extra["location"], "Main St 1");
    assert_eq!(dentist.extra["attendees"], json!(["me@gmail.com"]));
    assert_eq!(events[1].snippet, "2026-10-05", "an all-day event shows its date");
}

#[tokio::test]
async fn an_event_is_previewed_and_then_created_with_the_times_normalised() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/calendars/primary/events"))
        .and(query_param("sendUpdates", "all"))
        .and(body_json(json!({
            "summary": "Lunch", "start": {"dateTime": "2026-10-05T10:00:00Z"}, "end": {"dateTime": "2026-10-05T11:30:00Z"},
            "location": "Cafe", "attendees": [{"email": "ann@corp.com"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "new1", "htmlLink": "https://cal/new1"})))
        .expect(1)
        .mount(&server)
        .await;
    let create = call(
        "calendar_create_event",
        &json!({"title": "Lunch", "start": "2026-10-05T12:00:00+02:00", "end": "2026-10-05T13:30:00+02:00", "location": "Cafe", "attendees": ["Ann@Corp.com"]}),
    );
    let c = calendar(&server);
    let preview = c.preview("me@gmail.com", &create).await.unwrap();
    assert_eq!((preview.resource.as_str(), preview.resource_label.as_str()), ("primary", "Main calendar"));
    assert!(
        preview.lines[0].contains("Lunch") && preview.lines.iter().any(|l| l.contains("ann@corp.com")),
        "{:?}",
        preview.lines
    );
    let done = c.perform("me@gmail.com", &create).await.unwrap();
    assert_eq!(done["event_id"], "new1");
}

#[tokio::test]
async fn event_times_are_refused_when_they_could_mean_two_things() {
    let server = MockServer::start().await;
    let c = calendar(&server);
    for (args, expect) in [
        (json!({"title": "T", "start": "2026-10-05T14:00:00"}), "time zone"),
        (json!({"title": "T", "start": "2026-10-05", "end": "2026-10-05T14:00:00Z"}), "both be dates"),
        (json!({"title": "T", "start": "2026-10-05T14:00:00Z", "end": "2026-10-05T13:00:00Z"}), "end after"),
        (json!({"title": "T", "start": "soon"}), "not a date"),
    ] {
        let err = c.preview("me@gmail.com", &call("calendar_create_event", &args)).await.unwrap_err();
        assert!(err.to_string().contains(expect), "{args}: {err}");
    }
    // A bare date is an all-day event, ending the next day by default.
    Mock::given(method("POST"))
        .and(path("/calendars/primary/events"))
        .and(body_json(json!({"summary": "Off", "start": {"date": "2026-10-05"}, "end": {"date": "2026-10-06"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "d1"})))
        .expect(1)
        .mount(&server)
        .await;
    c.perform("me@gmail.com", &call("calendar_create_event", &json!({"title": "Off", "start": "2026-10-05"})))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_event_can_be_deleted_after_showing_which_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendars/team%40group.calendar.google.com/events/e%2F1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "e/1", "summary": "Standup", "start": {"dateTime": "2026-10-05T09:00:00Z"}, "end": {"dateTime": "2026-10-05T09:15:00Z"}})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me/calendarList/team%40group.calendar.google.com"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"summary": "Team"})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/calendars/team%40group.calendar.google.com/events/e%2F1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let delete =
        call("calendar_delete_event", &json!({"event_id": "e/1", "calendar": "team@group.calendar.google.com"}));
    let c = calendar(&server);
    let preview = c.preview("me@gmail.com", &delete).await.unwrap();
    assert_eq!((preview.resource_label.as_str(), preview.lines[0].as_str()), ("Team", "Delete from Team: Standup"));
    assert_eq!(c.perform("me@gmail.com", &delete).await.unwrap(), json!({"deleted": true}));
}

#[tokio::test]
async fn the_calendar_account_is_the_address_of_its_primary_calendar_and_a_lost_permission_is_reported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendars/primary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "Me@Gmail.com"})))
        .mount(&server)
        .await;
    assert_eq!(calendar(&server).identify("me@gmail.com").await.unwrap(), "me@gmail.com");
    // The engine reaches it through the trait, which used to refuse ("not added that way").
    assert_eq!(Connector::identify(&calendar(&server), "me@gmail.com").await.unwrap(), "me@gmail.com");
    Mock::given(method("GET"))
        .and(path("/users/me/calendarList"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    assert_eq!(calendar(&server).status("me@gmail.com").await, GmailStatus::NeedsConsent);
    let err = calendar(&server).fetch("me@gmail.com", &call("calendar_list_calendars", &json!({}))).await.unwrap_err();
    assert_eq!(err, CoreError::GmailNeedsConsent);
}

#[tokio::test]
async fn a_google_api_that_is_switched_off_is_explained_without_project_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendars/primary"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": {"code": 403, "status": "PERMISSION_DENIED",
            "message": "Google Calendar API has not been used in project 123456 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/x?project=123456",
            "errors": [{"reason": "accessNotConfigured"}]}})))
        .mount(&server)
        .await;
    let message = Connector::identify(&calendar(&server), "me@gmail.com").await.unwrap_err().to_string();
    assert!(message.contains("not switched on"), "{message}");
    assert!(!message.contains("123456") && !message.contains("https://"), "{message}");
}

// ---- Google Contacts ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn contacts_are_found_by_name_with_their_addresses_and_numbers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/people:searchContacts"))
        .and(query_param("query", "ann"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [{"person": {
            "resourceName": "people/c1", "names": [{"displayName": "Ann Lee"}],
            "emailAddresses": [{"value": "ann@corp.com"}], "phoneNumbers": [{"value": "+1 555 0100"}],
            "organizations": [{"name": "Corp"}]}}]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/people/me/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": []})))
        .mount(&server)
        .await;
    let contacts = GoogleContacts::new(client(), &server.uri(), google(), Duration::from_millis(1));
    let found = contacts.fetch("me@gmail.com", &call("contacts_search", &json!({"query": "ann"}))).await.unwrap();
    assert_eq!(
        (found[0].title.as_str(), found[0].from.as_str(), found[0].snippet.as_str()),
        ("Ann Lee", "Corp", "ann@corp.com · +1 555 0100")
    );
    assert_eq!(found[0].extra["emails"], json!(["ann@corp.com"]));
    assert_eq!(found[0].resource, "contacts");
    // The account is the address the phone authorized; the contacts permission does not reach the profile.
    assert_eq!(Connector::identify(&contacts, " Me@Gmail.com ").await.unwrap(), "me@gmail.com");
    assert!(Connector::identify(&contacts, "").await.is_err());
    assert_eq!(contacts.status("me@gmail.com").await, GmailStatus::Ready);
}

#[tokio::test]
async fn the_first_contact_search_is_repeated_once_google_has_built_its_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/people:searchContacts"))
        .and(query_param("query", "ann"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/people:searchContacts"))
        .and(query_param("query", ""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/people:searchContacts"))
        .and(query_param("query", "ann"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [{"person": {
            "resourceName": "people/c1", "names": [{"displayName": "Ann Lee"}]}}]})))
        .mount(&server)
        .await;
    let contacts = GoogleContacts::new(client(), &server.uri(), google(), Duration::from_millis(1));
    let found = contacts.fetch("me@gmail.com", &call("contacts_search", &json!({"query": "ann"}))).await.unwrap();
    assert_eq!(found[0].title, "Ann Lee");
}

// ---- GitHub ---------------------------------------------------------------------------------------------------------

async fn github(server: &MockServer) -> (GitHub, tempfile::TempDir) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer ghp_secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "Octo-Cat"})))
        .mount(server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path(), &FakeKeys).unwrap());
    let github = GitHub::new(client(), &server.uri(), store, Duration::from_millis(1));
    assert_eq!(github.sign_in(" ghp_secret ").await.unwrap(), "octo-cat");
    (github, dir)
}

#[tokio::test]
async fn a_pasted_token_is_checked_kept_and_used() {
    let server = MockServer::start().await;
    let (github, _dir) = github(&server).await;
    assert_eq!(github.status("octo-cat").await, GmailStatus::Ready);
    assert_eq!(github.status("nobody").await, GmailStatus::NeedsConsent, "no token for that account");
    for bad in ["", "has space", "line\nbreak"] {
        assert!(matches!(github.sign_in(bad).await, Err(CoreError::Invalid { .. })), "{bad:?}");
    }
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&MockServer::start().await)
        .await;
    let refused = MockServer::start().await;
    Mock::given(method("GET")).and(path("/user")).respond_with(ResponseTemplate::new(401)).mount(&refused).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path(), &FakeKeys).unwrap());
    let other = GitHub::new(client(), &refused.uri(), store, Duration::from_millis(1));
    assert!(matches!(other.sign_in("ghp_wrong").await, Err(CoreError::ServiceNeedsAttention { .. })));
}

#[tokio::test]
async fn repositories_issues_and_searches_come_back_as_items() {
    let server = MockServer::start().await;
    let (github, _dir) = github(&server).await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"full_name": "octo/cat", "private": true, "description": "The cat", "pushed_at": "2026-10-01T00:00:00Z"},
            {"full_name": "octo/dog", "private": false, "description": null, "pushed_at": "2026-09-01T00:00:00Z"}])))
        .mount(&server)
        .await;
    let repos = github.fetch("octo-cat", &call("github_list_repos", &json!({"query": "CAT"}))).await.unwrap();
    assert_eq!(
        repos.iter().map(|r| (r.id.as_str(), r.from.as_str(), r.snippet.as_str())).collect::<Vec<_>>(),
        [("octo/cat", "private", "The cat")]
    );

    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues"))
        .and(query_param("state", "closed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"number": 7, "title": "Bug", "state": "closed", "user": {"login": "ann"}, "labels": [{"name": "bug"}], "updated_at": "2026-10-02T10:00:00Z", "html_url": "https://gh/7"},
            {"number": 8, "title": "Fix", "state": "closed", "user": {"login": "bob"}, "labels": [], "pull_request": {}, "updated_at": "2026-10-03T10:00:00Z"}])))
        .mount(&server)
        .await;
    let issues = github
        .fetch("octo-cat", &call("github_list_issues", &json!({"repo": "octo/cat", "state": "closed"})))
        .await
        .unwrap();
    assert_eq!(
        (issues[0].id.as_str(), issues[0].snippet.as_str(), issues[0].extra["number"].clone()),
        ("octo/cat#7", "#7 · closed · bug", json!(7))
    );
    assert_eq!(issues[1].extra["pull_request"], true);

    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 7, "title": "Bug", "state": "open", "body": "It crashes", "user": {"login": "ann"}, "labels": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues/7/comments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!([{"user": {"login": "bob"}, "created_at": "2026-10-02T11:00:00Z", "body": "Me too"}]),
            ),
        )
        .mount(&server)
        .await;
    let one =
        github.fetch("octo-cat", &call("github_get_issue", &json!({"repo": "octo/cat", "number": 7}))).await.unwrap();
    let body = one[0].body.clone().unwrap();
    assert!(body.starts_with("It crashes") && body.contains("@bob (2026-10-02T11:00:00Z):\nMe too"), "{body}");

    Mock::given(method("GET"))
        .and(path("/search/issues"))
        .and(query_param("q", "crash repo:octo/cat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [
            {"number": 7, "title": "Bug", "state": "open", "user": {"login": "ann"}, "repository_url": "https://api.github.com/repos/octo/cat"}]})))
        .mount(&server)
        .await;
    let found =
        github.fetch("octo-cat", &call("github_search", &json!({"query": "crash", "repo": "octo/cat"}))).await.unwrap();
    assert_eq!((found[0].resource.as_str(), found[0].id.as_str()), ("octo/cat", "octo/cat#7"));
}

#[tokio::test]
async fn comments_and_issues_are_previewed_then_posted() {
    let server = MockServer::start().await;
    let (github, _dir) = github(&server).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/cat/issues/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 7, "title": "Bug"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/issues/7/comments"))
        .and(body_json(json!({"body": "Thanks!"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"html_url": "https://gh/7#c1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/octo/cat/issues"))
        .and(body_json(json!({"title": "New", "body": "Details"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 9, "html_url": "https://gh/9"})))
        .expect(1)
        .mount(&server)
        .await;
    let comment = call("github_comment", &json!({"repo": "octo/cat", "number": 7, "body": "Thanks!"}));
    let preview = github.preview("octo-cat", &comment).await.unwrap();
    assert_eq!(
        (preview.resource.as_str(), preview.lines[0].as_str(), preview.lines[1].as_str()),
        ("octo/cat", "Comment on octo/cat#7: Bug", "Thanks!")
    );
    assert_eq!(github.perform("octo-cat", &comment).await.unwrap()["commented"], true);
    let issue = call("github_create_issue", &json!({"repo": "octo/cat", "title": "New", "body": "Details"}));
    assert_eq!(github.preview("octo-cat", &issue).await.unwrap().lines[0], "New issue in octo/cat: New");
    assert_eq!(github.perform("octo-cat", &issue).await.unwrap()["number"], 9);
}

#[tokio::test]
async fn github_failures_are_explained_without_leaking_the_token_and_bad_repos_never_reach_the_network() {
    let server = MockServer::start().await;
    let (github, _dir) = github(&server).await;
    Mock::given(method("GET"))
        .and(path("/repos/octo/secret/issues"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let err = github.fetch("octo-cat", &call("github_list_issues", &json!({"repo": "octo/secret"}))).await.unwrap_err();
    assert!(err.to_string().contains("does not exist") && !err.to_string().contains("ghp_"), "{err}");
    for repo in ["../../user", "a/b/c", "a b/c"] {
        let err = github.fetch("octo-cat", &call("github_list_issues", &json!({"repo": repo}))).await.unwrap_err();
        assert!(err.to_string().contains("owner/name"), "{repo}: {err}");
    }
    Mock::given(method("GET"))
        .and(path("/repos/octo/flaky/issues"))
        .respond_with(ResponseTemplate::new(503))
        .expect(3)
        .mount(&server)
        .await;
    assert!(github.fetch("octo-cat", &call("github_list_issues", &json!({"repo": "octo/flaky"}))).await.is_err());
}
