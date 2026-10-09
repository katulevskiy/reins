//! The Integrations screen's operations (catalogue, sign-in of every kind, removal) and the connectors that live on the
//! phone itself, with fake stand-ins for Telegram and for Android.

mod common;

use std::sync::{Arc, Mutex};

use common::{FakeGoogle, FakeKeys, RecordingNotifier};
use reins_core::ForeignError;
use reins_core::connector::device::{DeviceCalendar, DeviceContacts, Sms};
use reins_core::connector::{Connector, LoginProgress};
use reins_core::{
    CoreConfig, CoreError, DeviceBridge, DeviceContact, DeviceEvent, GmailStatus, GoogleTokenProvider, KeyWrapper,
    NewDeviceEvent, Notifier, ReinsCore, SmsMessage, SmsThread,
};
use reins_proto::connector::{ConnectorCall, spec_for_tool};
use serde_json::{Value, json};

fn call(tool: &str, args: &Value) -> ConnectorCall {
    spec_for_tool(tool).unwrap().parse(args).unwrap().0
}

/// A Telegram that signs in with code 12345 and, for `+15550101`, wants the password "pw" as well.
#[derive(Default)]
struct SignIn {
    phone: Mutex<String>,
    signed_out: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Connector for SignIn {
    fn service(&self) -> &'static str {
        "telegram"
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }

    async fn login_begin(&self, phone: &str) -> Result<(), CoreError> {
        phone.clone_into(&mut self.phone.lock().unwrap());
        Ok(())
    }

    async fn login_code(&self, code: &str) -> Result<LoginProgress, CoreError> {
        if code != "12345" {
            return Err(CoreError::invalid("That code is wrong or has expired. Ask for a new one."));
        }
        let phone = self.phone.lock().unwrap().clone();
        Ok(if phone == "+15550101" {
            LoginProgress::NeedsPassword {
                hint: Some("pet".to_owned()),
            }
        } else {
            LoginProgress::Done {
                account: phone,
            }
        })
    }

    async fn login_password(&self, password: &str) -> Result<String, CoreError> {
        if password == "pw" {
            Ok(self.phone.lock().unwrap().clone())
        } else {
            Err(CoreError::invalid("Wrong password"))
        }
    }

    async fn forget(&self, account: &str) {
        self.signed_out.lock().unwrap().push(account.to_owned());
    }
}

#[derive(Default)]
struct Phone {
    /// What this build of the app offers; `None` is everything (the APK), the Google Play build has no "sms".
    offered: Option<Vec<String>>,
    allowed: Mutex<bool>,
    created: Mutex<Vec<NewDeviceEvent>>,
    texted: Mutex<Vec<(String, String)>>,
}

impl DeviceBridge for Phone {
    fn services(&self) -> Vec<String> {
        self.offered.clone().unwrap_or_else(|| vec!["device_calendar".into(), "device_contacts".into(), "sms".into()])
    }

    fn permitted(&self, _service: String) -> bool {
        *self.allowed.lock().unwrap()
    }

    fn calendar_events(
        &self,
        from: i64,
        to: i64,
        query: Option<String>,
        _limit: u32,
    ) -> Result<Vec<DeviceEvent>, ForeignError> {
        assert!(from < to);
        let event = DeviceEvent {
            id: "e1".into(),
            calendar_id: "1".into(),
            calendar: "Personal".into(),
            title: "Dentist".into(),
            start: 1_791_201_600,
            end: 1_791_205_200,
            all_day: false,
            location: "Main St".into(),
            description: "Bring the forms".into(),
        };
        Ok(vec![event].into_iter().filter(|e| query.as_deref().is_none_or(|q| e.title.contains(q))).collect())
    }

    fn calendar_create(&self, event: NewDeviceEvent) -> Result<String, ForeignError> {
        self.created.lock().unwrap().push(event);
        Ok("e2".into())
    }

    fn contacts_search(&self, query: String, _limit: u32) -> Result<Vec<DeviceContact>, ForeignError> {
        if !*self.allowed.lock().unwrap() {
            return Err(ForeignError::NeedsUserInteraction);
        }
        Ok(vec![DeviceContact {
            id: "c1".into(),
            name: format!("Anna ({query})"),
            organization: "Acme".into(),
            emails: vec!["anna@example.com".into()],
            phones: vec!["+15550100".into()],
        }])
    }

    fn sms_threads(&self, _limit: u32) -> Result<Vec<SmsThread>, ForeignError> {
        Ok(vec![
            SmsThread {
                id: "t1".into(),
                address: "+15550100".into(),
                name: "Anna".into(),
                snippet: "See you at 8".into(),
                date: 10,
            },
            SmsThread {
                id: "t2".into(),
                address: "BANK".into(),
                name: String::new(),
                snippet: "Your code is 481516".into(),
                date: 20,
            },
        ])
    }

    fn sms_messages(&self, thread: String, _limit: u32) -> Result<Vec<SmsMessage>, ForeignError> {
        Ok(vec![
            SmsMessage {
                id: "1".into(),
                address: thread.clone(),
                body: "Your verification code is 481516".into(),
                date: 5,
                outgoing: false,
            },
            SmsMessage {
                id: "2".into(),
                address: thread,
                body: "ok, thanks".into(),
                date: 6,
                outgoing: true,
            },
        ])
    }

    fn sms_send(&self, to: String, text: String) -> Result<(), ForeignError> {
        if to.is_empty() {
            return Err(ForeignError::Failed {
                reason: "no number".into(),
            });
        }
        self.texted.lock().unwrap().push((to, text));
        Ok(())
    }
}

fn bridge(phone: &Arc<Phone>) -> Arc<dyn DeviceBridge> {
    Arc::<Phone>::clone(phone)
}

fn core(device: Option<Arc<Phone>>) -> (Arc<ReinsCore>, Arc<SignIn>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let sign_in = Arc::new(SignIn::default());
    let google: Arc<dyn GoogleTokenProvider> = Arc::new(FakeGoogle::new());
    let notifier: Arc<dyn Notifier> = Arc::new(RecordingNotifier::default());
    let mut extra: Vec<Arc<dyn Connector>> = vec![Arc::<SignIn>::clone(&sign_in)];
    if let Some(phone) = device {
        let bridge: Arc<dyn DeviceBridge> = phone;
        extra.push(Arc::new(DeviceCalendar::new(Arc::clone(&bridge))));
        extra.push(Arc::new(DeviceContacts::new(Arc::clone(&bridge))));
        extra.push(Arc::new(Sms::new(bridge)));
    }
    let core = ReinsCore::with_connectors(
        dir.path().to_str().unwrap(),
        &FakeKeys,
        google,
        notifier,
        CoreConfig::default(),
        extra,
    )
    .unwrap();
    (core, sign_in, dir)
}

#[tokio::test]
async fn the_catalogue_lists_every_integration_and_what_this_build_lacks() {
    let (plain, _, _dir) = core(None);
    let services = plain.services().await.unwrap();
    let names: Vec<_> = services.iter().map(|s| (s.service.as_str(), s.kind.as_str(), s.available)).collect();
    assert_eq!(
        names,
        [
            ("gmail", "google", true),
            ("gcalendar", "google", true),
            ("gcontacts", "google", true),
            ("telegram", "telegram", true),
            ("github", "token", true),
            ("gitlab", "token", true),
            ("codeberg", "token", true),
            ("bitbucket", "token", true),
            ("device_calendar", "device", false),
            ("device_contacts", "device", false),
            ("sms", "device", false),
            ("vault", "vault", true),
            ("payments", "payments", true),
        ]
    );
    assert!(services.iter().find(|s| s.service == "sms").unwrap().note.is_some());
    let (with_phone, _, _dir) = core(Some(Arc::default()));
    assert!(with_phone.services().await.unwrap().iter().find(|s| s.service == "sms").unwrap().available);
}

/// The core as the app builds it, over this phone.
fn app_core(phone: Phone) -> (Arc<ReinsCore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let keys: Arc<dyn KeyWrapper> = Arc::new(FakeKeys);
    let device: Arc<dyn DeviceBridge> = Arc::new(phone);
    let core = ReinsCore::new(
        dir.path().to_str().unwrap().to_owned(),
        keys,
        Arc::new(FakeGoogle::new()),
        Arc::new(RecordingNotifier::default()),
        Some(device),
        0,
        String::new(),
    )
    .unwrap();
    (core, dir)
}

/// Whether the phone's calendar, contacts and text messages can be used.
async fn on_phone(core: &ReinsCore) -> (bool, bool, bool) {
    let services = core.services().await.unwrap();
    let available = |id: &str| services.iter().find(|s| s.service == id).unwrap().available;
    (available("device_calendar"), available("device_contacts"), available("sms"))
}

#[tokio::test]
async fn the_phone_runs_only_the_integrations_this_build_offers() {
    let (full, _dir) = app_core(Phone::default());
    assert_eq!(on_phone(&full).await, (true, true, true));

    let play = Phone {
        allowed: Mutex::new(true),
        offered: Some(vec!["device_calendar".into(), "device_contacts".into()]),
        ..Phone::default()
    };
    let (core, _dir) = app_core(play);
    assert_eq!(on_phone(&core).await, (true, true, false));
    // Android would allow it, but there is no text-message integration to add an account to.
    assert!(matches!(core.add_service_account("sms".into(), String::new()).await, Err(CoreError::Invalid { .. })));
    assert!(core.add_service_account("device_contacts".into(), String::new()).await.is_ok());
}

#[tokio::test]
async fn a_phone_number_signs_in_with_a_code_and_a_password_when_there_is_one() {
    let (core, sign_in, _dir) = core(None);
    core.login_begin("telegram".into(), "+15550100".into()).await.unwrap();
    assert!(matches!(core.login_code("telegram".into(), "00000".into()).await, Err(CoreError::Invalid { .. })));
    assert_eq!(
        core.login_code("telegram".into(), " 12345 ".into()).await.unwrap(),
        LoginProgress::Done {
            account: "+15550100".into()
        }
    );
    core.login_begin("telegram".into(), "+15550101".into()).await.unwrap();
    assert_eq!(
        core.login_code("telegram".into(), "12345".into()).await.unwrap(),
        LoginProgress::NeedsPassword {
            hint: Some("pet".into())
        }
    );
    assert!(core.login_password("telegram".into(), "nope".into()).await.is_err());
    assert_eq!(core.login_password("telegram".into(), "pw".into()).await.unwrap().account, "+15550101");
    let telegram = core.services().await.unwrap().into_iter().find(|s| s.service == "telegram").unwrap();
    let accounts: Vec<_> = telegram.accounts.iter().map(|a| a.account.as_str()).collect();
    assert_eq!(accounts, ["+15550100", "+15550101"]);

    core.remove_service_account("telegram".into(), "+15550100".into()).await.unwrap();
    assert_eq!(*sign_in.signed_out.lock().unwrap(), ["+15550100"]);
    assert!(matches!(
        core.remove_service_account("telegram".into(), "+15550100".into()).await,
        Err(CoreError::NotFound)
    ));
    assert_eq!(core.accounts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn things_that_are_not_added_that_way_or_not_here_are_refused() {
    let (core, _, _dir) = core(None);
    assert!(matches!(core.add_service_account("vault".into(), String::new()).await, Err(CoreError::Invalid { .. })));
    assert!(matches!(core.add_token_account("nowhere".into(), "x".into()).await, Err(CoreError::Invalid { .. })));
    assert!(matches!(core.add_token_account("telegram".into(), "x".into()).await, Err(CoreError::Invalid { .. })));
    assert!(matches!(core.login_begin("github".into(), "+1555".into()).await, Err(CoreError::Invalid { .. })));
}

#[tokio::test]
async fn the_phones_own_services_are_added_only_once_android_allowed_them() {
    let phone = Arc::new(Phone::default());
    let (core, _, _dir) = core(Some(Arc::clone(&phone)));
    assert!(matches!(
        core.add_service_account("sms".into(), String::new()).await,
        Err(CoreError::ServiceNeedsAttention { .. })
    ));
    *phone.allowed.lock().unwrap() = true;
    let added = core.add_service_account("sms".into(), String::new()).await.unwrap();
    assert_eq!((added.service.as_str(), added.account.as_str()), ("sms", "this phone"));
    assert_eq!(core.service_account_status("sms".into(), "this phone".into()).await, GmailStatus::Ready);
    *phone.allowed.lock().unwrap() = false;
    assert_eq!(core.service_account_status("sms".into(), "this phone".into()).await, GmailStatus::NeedsConsent);
}

#[tokio::test]
async fn calendar_events_on_the_phone_are_listed_and_created_after_a_preview() {
    let phone = Arc::new(Phone::default());
    let calendar = DeviceCalendar::new(bridge(&phone));
    let list = call("device_calendar_list_events", &json!({"from": "2026-10-01", "to": "2026-11-01"}));
    let items = calendar.fetch("this phone", &list).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].title.as_str(), items[0].resource.as_str()), ("Dentist", "1"));
    assert!(items[0].snippet.contains("Main St"));

    let create = call("device_calendar_create_event", &json!({"title": "Lunch", "start": "2026-10-05T12:00:00Z"}));
    let preview = calendar.preview("this phone", &create).await.unwrap();
    assert!(preview.lines[0].contains("Lunch"), "{preview:?}");
    assert!(phone.created.lock().unwrap().is_empty(), "a preview changes nothing");
    calendar.perform("this phone", &create).await.unwrap();
    let made = phone.created.lock().unwrap().remove(0);
    assert_eq!((made.start, made.end, made.all_day), (1_791_201_600, 1_791_205_200, false));

    let backwards = call(
        "device_calendar_create_event",
        &json!({"title": "x", "start": "2026-10-05T12:00:00Z", "end": "2026-10-05T11:00:00Z"}),
    );
    assert!(calendar.preview("this phone", &backwards).await.is_err());
}

#[tokio::test]
async fn contacts_and_missing_permission() {
    let phone = Arc::new(Phone::default());
    let contacts = DeviceContacts::new(bridge(&phone));
    let search = call("device_contacts_search", &json!({"query": "ann"}));
    assert!(matches!(contacts.fetch("this phone", &search).await, Err(CoreError::ServiceNeedsAttention { .. })));
    *phone.allowed.lock().unwrap() = true;
    let found = contacts.fetch("this phone", &search).await.unwrap();
    assert_eq!(found[0].extra["emails"], json!(["anna@example.com"]));
}

#[tokio::test]
async fn login_codes_in_text_messages_are_hidden_from_lists_and_never_shared_by_default() {
    let phone = Arc::new(Phone::default());
    let sms = Sms::new(bridge(&phone));
    let threads = sms.fetch("this phone", &call("sms_list_threads", &json!({}))).await.unwrap();
    assert_eq!(threads[0].snippet, "See you at 8");
    assert_eq!(threads[1].snippet, "", "the last text of the bank thread is a code");
    assert!(!threads.iter().any(|t| t.snippet.contains("481516")));
    let messages = sms.fetch("this phone", &call("sms_read", &json!({"thread": "t2"}))).await.unwrap();
    assert!(messages[0].sensitive && !messages[1].sensitive);
    assert_eq!(messages[1].from, "me");

    let send = call("sms_send", &json!({"to": "+1 (555) 010-0100", "text": "on my way"}));
    assert!(sms.preview("this phone", &send).await.unwrap().lines[0].contains("+15550100100"));
    sms.perform("this phone", &send).await.unwrap();
    assert_eq!(phone.texted.lock().unwrap()[0], ("+15550100100".to_owned(), "on my way".to_owned()));
    let junk = call("sms_send", &json!({"to": "call me; rm", "text": "x"}));
    assert!(sms.preview("this phone", &junk).await.is_err());
}
