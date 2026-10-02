//! The paired Rewarden desktop app itself: `desktop_ask`, a yes-or-no question (`rewarden ask`, a harness hook before a
//! command). The user reads it on the phone; approving answers yes with an [`AskAnswer`] sealed to the app's key and
//! bound to its nonce, denying is the ordinary denied answer. A standing permission can cover a topic
//! (`command:git push --force`), or questions without one (`ask`).
//!
//! There is no account to connect: the paired app is the account, and the flow checks its key.

use rewarden_proto::connector::{ConnectorCall, DESKTOP};
use rewarden_proto::desktop::{AskAnswer, SEALED_FIELD};
use serde_json::{Value, json};

use super::sealed::{client_key, nonce_arg, seal};
use super::{Connector, Preview};
use crate::types::{AskView, GmailStatus};
use crate::{CoreError, text};

/// The account every desktop call is made as.
pub const DESKTOP_ACCOUNT: &str = "desktop app";
/// The resource of a question without a topic.
const ASK: &str = "ask";
const MAX_QUESTION: usize = 300;
const MAX_DETAIL: usize = 8_000;
const MAX_TOPIC: usize = 100;

fn bad(message: &str) -> CoreError {
    CoreError::service(message)
}

fn question(call: &ConnectorCall) -> Result<String, CoreError> {
    let q = text::one_line(call.str_arg("question").unwrap_or_default());
    if q.is_empty() {
        return Err(bad("`question` is required."));
    }
    Ok(text::truncate_chars(&q, MAX_QUESTION))
}

/// The topic, when one was given: one line of visible text.
fn topic(call: &ConnectorCall) -> Result<Option<String>, CoreError> {
    match call.str_arg("topic").map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) if t.chars().count() <= MAX_TOPIC && !t.chars().any(char::is_control) && text::one_line(t) == t => {
            Ok(Some(t.to_owned()))
        }
        Some(_) => Err(bad("`topic` must be one line of at most 100 characters.")),
    }
}

/// The question as the approval shows it (`None` for anything but `desktop_ask`).
pub fn ask_view(call: &ConnectorCall) -> Option<AskView> {
    (call.service == DESKTOP && call.op == "ask").then(|| AskView {
        question: text::truncate_chars(&text::one_line(call.str_arg("question").unwrap_or_default()), MAX_QUESTION),
        detail: call
            .str_arg("detail")
            .map(|d| text::truncate_chars(&text::neutralize(d), MAX_DETAIL))
            .filter(|d| !d.trim().is_empty()),
        topic: topic(call).ok().flatten().map(|t| text::one_line(&t)),
    })
}

#[derive(Default)]
pub struct Desktop;

#[async_trait::async_trait]
impl Connector for Desktop {
    fn service(&self) -> &'static str {
        DESKTOP
    }

    async fn preview(&self, _account: &str, call: &ConnectorCall) -> Result<Preview, CoreError> {
        if call.op != "ask" {
            return Err(CoreError::service(format!("the desktop app cannot {}", call.op)));
        }
        client_key(call)?;
        nonce_arg(call)?;
        let question = question(call)?;
        let topic = topic(call)?;
        let mut lines = vec![question];
        if let Some(t) = &topic {
            lines.push(format!("About: {t}"));
        }
        let (resource, resource_label) = match topic {
            Some(t) => (t.clone(), format!("Questions about {t}")),
            None => (ASK.to_owned(), "Questions without a topic".to_owned()),
        };
        Ok(Preview {
            resource,
            resource_label,
            lines,
            parents: Vec::new(),
            once_only: false,
            ..Preview::default()
        })
    }

    async fn perform(&self, _account: &str, call: &ConnectorCall) -> Result<Value, CoreError> {
        let answer = AskAnswer {
            v: 1,
            nonce: nonce_arg(call)?,
            approved: true,
        };
        Ok(json!({ SEALED_FIELD: seal(client_key(call)?, &answer)? }))
    }

    async fn status(&self, _account: &str) -> GmailStatus {
        GmailStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &Value) -> ConnectorCall {
        ConnectorCall {
            service: DESKTOP.to_owned(),
            op: "ask".to_owned(),
            args: args.as_object().unwrap().clone(),
        }
    }

    #[test]
    fn the_view_shows_the_question_detail_and_topic_cleaned() {
        let v = ask_view(&call(&json!({"question": "Push\u{202e} to main?", "detail": "git push\n--force",
            "topic": "command:git push --force"})))
        .unwrap();
        assert_eq!(v.question, "Push to main?");
        assert_eq!(v.detail.as_deref(), Some("git push\n--force"));
        assert_eq!(v.topic.as_deref(), Some("command:git push --force"));
        let bare = ask_view(&call(&json!({"question": "Ok?", "detail": "  "}))).unwrap();
        assert_eq!((bare.detail, bare.topic), (None, None));
        let mut other = call(&json!({}));
        other.op = "git_fetch".to_owned();
        assert!(ask_view(&other).is_none());
    }

    #[test]
    fn topics_are_one_line() {
        assert!(topic(&call(&json!({"topic": "a\nb"}))).is_err());
        assert!(topic(&call(&json!({"topic": "x".repeat(101)}))).is_err());
        assert_eq!(topic(&call(&json!({"topic": " "}))).unwrap(), None);
    }
}
