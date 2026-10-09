//! The server's desktop API: `POST /reins/desktop/calls` submits a desktop tool call to the phone, `GET
//! /reins/desktop/calls/<id>` waits for its answer again. Both wait on the server side (like MCP calls) and answer
//! `answered` with the phone's outcome, or `pending` / `offline` to be polled again. `GET /reins/desktop/phone` says when
//! the approval phone last asked the server for work (for `reins doctor`).

use std::time::Duration;

use reins_proto::relay::RelayOutcome;
use serde::Deserialize;
use serde_json::Value;

use super::LinkError;
use super::oauth::{Access, SessionTokens};
use crate::config::Paths;

/// Longer than the server's own wait (at most 55 s), so its `pending` answer arrives before this gives up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(75);
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallStatus {
    /// The phone answered (a result, a denial or an error).
    Answered(RelayOutcome),
    /// The phone has the request; the user has not answered yet.
    Pending,
    /// The phone did not pick the request up (yet).
    Offline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallAnswer {
    pub request_id: String,
    pub status: CallStatus,
}

/// When the approval phone last polled the server, and the server's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct PhoneSeen {
    /// Unix seconds; `None`: not since the server started.
    pub last_seen: Option<i64>,
    pub server_time: i64,
}

pub struct DesktopClient {
    http: reqwest::Client,
    tokens: SessionTokens,
}

impl DesktopClient {
    /// Works with whatever session `paths` holds at the time of each call.
    pub fn new(paths: &Paths) -> Result<Self, String> {
        let http = crate::http::client(Some(REQUEST_TIMEOUT))?;
        Ok(Self {
            tokens: SessionTokens::new(paths, http.clone()),
            http,
        })
    }

    /// Asks the phone: `tool` (a desktop-only tool) with `arguments`, for `account` when the phone has several.
    pub async fn call(&self, tool: &str, arguments: &Value, account: Option<&str>) -> Result<CallAnswer, LinkError> {
        let body = serde_json::json!({"tool": tool, "arguments": arguments, "account": account});
        self.send(|server| self.http.post(format!("{server}/reins/desktop/calls")).json(&body)).await
    }

    /// Waits for the answer to an earlier call again.
    pub async fn poll(&self, request_id: &str) -> Result<CallAnswer, LinkError> {
        let id: String = url::form_urlencoded::byte_serialize(request_id.as_bytes()).collect();
        self.send(|server| self.http.get(format!("{server}/reins/desktop/calls/{id}"))).await
    }

    /// When the approval phone last asked the server for work. `LinkError::NotFound` from a server too old to say.
    pub async fn phone(&self) -> Result<PhoneSeen, LinkError> {
        let body = self.send_raw(|server| self.http.get(format!("{server}/reins/desktop/phone"))).await?;
        serde_json::from_slice(&body)
            .map_err(|e| LinkError::Failed(format!("unexpected answer from the Reins server: {e}")))
    }

    async fn send(&self, build: impl Fn(&str) -> reqwest::RequestBuilder) -> Result<CallAnswer, LinkError> {
        parse_answer(&self.send_raw(build).await?)
    }

    /// Sends with the access token; on 401 renews the token and tries once more. The body of a successful answer.
    async fn send_raw(&self, build: impl Fn(&str) -> reqwest::RequestBuilder) -> Result<Vec<u8>, LinkError> {
        let access = self.tokens.access().await?;
        let resp = Self::attempt(&build, &access).await?;
        let resp = if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            log::info!("the Reins server refused the access token; renewing it");
            let renewed = self.tokens.after_rejection(&access.token).await?;
            let again = Self::attempt(&build, &renewed).await?;
            if again.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(LinkError::LoggedOut(format!(
                    "{} does not accept this app's session; run `reins login {}` again",
                    renewed.server, renewed.server
                )));
            }
            again
        } else {
            resp
        };
        let status = resp.status();
        let body = super::read_limited(resp, MAX_ANSWER_BYTES).await.map_err(LinkError::Failed)?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(LinkError::NotFound);
        }
        if !status.is_success() {
            return Err(LinkError::Failed(format!("the Reins server answered {}", super::error_text(status, &body))));
        }
        Ok(body)
    }

    async fn attempt(
        build: &impl Fn(&str) -> reqwest::RequestBuilder,
        access: &Access,
    ) -> Result<reqwest::Response, LinkError> {
        build(&access.server)
            .bearer_auth(access.token.as_str())
            .send()
            .await
            .map_err(|e| LinkError::Failed(format!("cannot reach {}: {}", access.server, e.without_url())))
    }
}

fn parse_answer(body: &[u8]) -> Result<CallAnswer, LinkError> {
    #[derive(Deserialize)]
    struct Wire {
        request_id: String,
        status: String,
        #[serde(default)]
        outcome: Option<Value>,
    }
    let bad = |why: String| LinkError::Failed(format!("unexpected answer from the Reins server: {why}"));
    let w: Wire = serde_json::from_slice(body).map_err(|e| bad(e.to_string()))?;
    if w.request_id.is_empty() {
        return Err(bad("no request id".to_owned()));
    }
    let status = match (w.status.as_str(), w.outcome) {
        ("answered", Some(outcome)) => {
            CallStatus::Answered(serde_json::from_value(outcome).map_err(|e| bad(format!("outcome: {e}")))?)
        }
        ("pending", _) => CallStatus::Pending,
        ("offline", _) => CallStatus::Offline,
        (other, _) => return Err(bad(format!("status `{other}`"))),
    };
    Ok(CallAnswer {
        request_id: w.request_id,
        status,
    })
}

#[cfg(test)]
mod tests {
    use reins_proto::relay::ToolResult;

    use super::*;

    #[test]
    fn the_three_answer_shapes_parse() {
        let a = parse_answer(
            br#"{"request_id":"r1","status":"answered","outcome":{"outcome":"result","result":{"kind":"connector","data":{"sealed":"x"}}}}"#,
        )
        .unwrap();
        assert_eq!(a.request_id, "r1");
        assert_eq!(
            a.status,
            CallStatus::Answered(RelayOutcome::Result {
                result: ToolResult::Connector {
                    data: serde_json::json!({"sealed": "x"})
                }
            })
        );
        let d =
            parse_answer(br#"{"request_id":"r2","status":"answered","outcome":{"outcome":"denied","reason":null}}"#)
                .unwrap();
        assert_eq!(
            d.status,
            CallStatus::Answered(RelayOutcome::Denied {
                reason: None
            })
        );
        assert_eq!(parse_answer(br#"{"request_id":"r3","status":"pending"}"#).unwrap().status, CallStatus::Pending);
        assert_eq!(parse_answer(br#"{"request_id":"r4","status":"offline"}"#).unwrap().status, CallStatus::Offline);
        for bad in [
            &br#"{"request_id":"r","status":"answered"}"#[..],
            br#"{"request_id":"r","status":"weird"}"#,
            br#"{"request_id":"","status":"pending"}"#,
            br#"{"status":"pending"}"#,
            b"<html>",
        ] {
            assert!(matches!(parse_answer(bad), Err(LinkError::Failed(_))), "{}", String::from_utf8_lossy(bad));
        }
    }
}
