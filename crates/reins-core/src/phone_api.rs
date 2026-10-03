//! Client for the phone-facing server API (contracts §A), base
//! `{server}/reins/api`, authenticated with the Vaultwarden access token.

use std::time::Duration;

use reins_proto::device::{
    Connections, DEVICE_KEY_HEADER, DeviceRegistered, DeviceRegistration, PairingResult, Pending,
};
use reins_proto::pairing::{PairingClaim, PairingRequest, PairingResponse};
use reins_proto::relay::{RelayRequest, RelayResponse};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use crate::CoreError;
use crate::http::{ServerUrl, error_text};

/// Longest long-poll the server honours (contracts §A2).
pub const MAX_WAIT_SECS: u32 = 25;
const MAX_ID_LEN: usize = 64;

/// Why a phone-API call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiFailure {
    /// 401: the access token is missing, expired or revoked.
    Unauthorized,
    /// Any other non-success status with the body's `error` code and message.
    Status {
        status: u16,
        code: String,
        message: String,
    },
    /// Transport or decoding failure.
    Core(CoreError),
}

impl From<CoreError> for ApiFailure {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<reqwest::Error> for ApiFailure {
    fn from(e: reqwest::Error) -> Self {
        Self::Core(e.into())
    }
}

impl ApiFailure {
    /// The error Kotlin sees: 401 → `NotLoggedIn`, 404 → `NotFound`, other
    /// statuses → `Server { status, message: <error code, or message> }`.
    pub fn into_core(self) -> CoreError {
        match self {
            Self::Unauthorized => CoreError::NotLoggedIn,
            Self::Status {
                status: 404,
                ..
            } => CoreError::NotFound,
            Self::Status {
                status,
                code,
                message,
            } => CoreError::Server {
                status,
                reason: if code.is_empty() {
                    message
                } else {
                    code
                },
            },
            Self::Core(e) => e,
        }
    }
}

/// Adds the device key header to a phone-API request (never to a request to anything but the Reins server).
pub(crate) fn with_device_key(builder: RequestBuilder, device_key: Option<&str>) -> RequestBuilder {
    match device_key {
        Some(key) => builder.header(DEVICE_KEY_HEADER, key),
        None => builder,
    }
}

/// Ids are interpolated into URL paths: only `[A-Za-z0-9_-]{1,64}` is accepted.
pub fn check_id(id: &str) -> Result<(), CoreError> {
    if !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err(CoreError::invalid("malformed id"))
    }
}

pub struct PhoneApi<'a> {
    http: &'a reqwest::Client,
    server: &'a ServerUrl,
    token: &'a str,
    device_key: Option<&'a str>,
}

impl<'a> PhoneApi<'a> {
    pub fn new(http: &'a reqwest::Client, server: &'a ServerUrl, token: &'a str) -> Self {
        Self {
            http,
            server,
            token,
            device_key: None,
        }
    }

    /// Sends this phone's device key with every call ([`DEVICE_KEY_HEADER`]).
    #[must_use]
    pub fn with_device_key(mut self, device_key: Option<&'a str>) -> Self {
        self.device_key = device_key;
        self
    }

    pub(crate) fn request(&self, method: Method, path: &str) -> RequestBuilder {
        with_device_key(
            self.http.request(method, self.server.join(&format!("/reins/api{path}"))).bearer_auth(self.token),
            self.device_key,
        )
    }

    pub(crate) async fn send(builder: RequestBuilder) -> Result<(StatusCode, String), ApiFailure> {
        let resp = builder.send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if status.is_success() {
            return Ok((status, body));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiFailure::Unauthorized);
        }
        let (code, message) = error_text(status, &body);
        Err(ApiFailure::Status {
            status: status.as_u16(),
            code,
            message,
        })
    }

    pub(crate) async fn json<T: DeserializeOwned>(builder: RequestBuilder) -> Result<T, ApiFailure> {
        let (_, body) = Self::send(builder).await?;
        serde_json::from_str(&body).map_err(|_| {
            ApiFailure::Core(CoreError::Network {
                reason: "invalid response from the server".to_owned(),
            })
        })
    }

    /// A1 `PUT /device`.
    pub async fn register_device(&self, registration: &DeviceRegistration) -> Result<DeviceRegistered, ApiFailure> {
        Self::json(self.request(Method::PUT, "/device").json(registration)).await
    }

    /// `PUT /services`: which integrations have an account on this phone.
    pub async fn put_services(&self, report: &reins_proto::device::ServicesReport) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/services").json(report)).await.map(drop)
    }

    /// A2 `GET /pending?wait=` (long-poll, `wait` clamped to 25 s).
    pub async fn pending(&self, wait_secs: u32) -> Result<Pending, ApiFailure> {
        let wait = wait_secs.min(MAX_WAIT_SECS);
        let builder = self
            .request(Method::GET, "/pending")
            .query(&[("wait", wait)])
            .timeout(Duration::from_secs(u64::from(wait) + 15));
        Self::json(builder).await
    }

    /// A3 `GET /requests/{id}`.
    pub async fn get_request(&self, id: &str) -> Result<RelayRequest, ApiFailure> {
        check_id(id)?;
        let req: RelayRequest = Self::json(self.request(Method::GET, &format!("/requests/{id}"))).await?;
        if req.id.0 != id {
            return Err(CoreError::invalid("the server returned a different request").into());
        }
        Ok(req)
    }

    /// A4 `POST /requests/{id}/response`.
    pub async fn respond(&self, id: &str, response: &RelayResponse) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/requests/{id}/response")).json(response)).await.map(drop)
    }

    /// A5 `GET /pairings/{id}`.
    pub async fn get_pairing(&self, id: &str) -> Result<PairingRequest, ApiFailure> {
        check_id(id)?;
        let pairing: PairingRequest = Self::json(self.request(Method::GET, &format!("/pairings/{id}"))).await?;
        if pairing.id.0 != id {
            return Err(CoreError::invalid("the server returned a different pairing").into());
        }
        Ok(pairing)
    }

    /// A6 `POST /pairings/{id}/response`.
    pub async fn answer_pairing(&self, id: &str, response: &PairingResponse) -> Result<PairingResult, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::POST, &format!("/pairings/{id}/response")).json(response)).await
    }

    /// A6b `POST /pairings/claim`: the pairing a computer's code (already normalized) stands for.
    pub async fn claim_pairing(&self, user_code: &str) -> Result<PairingRequest, ApiFailure> {
        let claim = PairingClaim {
            v: reins_proto::PROTOCOL_VERSION,
            user_code: user_code.to_owned(),
        };
        let pairing: PairingRequest = Self::json(self.request(Method::POST, "/pairings/claim").json(&claim)).await?;
        check_id(&pairing.id.0)?;
        Ok(pairing)
    }

    /// A7 `GET /connections`.
    pub async fn connections(&self) -> Result<Connections, ApiFailure> {
        Self::json(self.request(Method::GET, "/connections")).await
    }

    /// A8 `DELETE /connections/{id}`.
    pub async fn delete_connection(&self, id: &str) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::DELETE, &format!("/connections/{id}"))).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::ToolCall;
    use reins_proto::relay::RelayOutcome;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::client;

    fn request_json(id: &str) -> serde_json::Value {
        json!({"v": 1, "id": id, "connection_id": "c1", "connection_label": "ChatGPT", "created_at": 5,
               "call": {"tool": "gmail_search", "query": "from:bank", "max_results": 10}})
    }

    fn setup(server: &MockServer) -> (reqwest::Client, ServerUrl) {
        (client().unwrap(), ServerUrl::parse(&server.uri()).unwrap())
    }

    #[tokio::test]
    async fn register_device_and_connections() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/reins/api/device"))
            .and(header("authorization", "Bearer TOKEN"))
            .and(header(DEVICE_KEY_HEADER, "KEY-1"))
            .and(body_json(json!({"fcm_token": "fcm-1"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"replaced_previous": true})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/connections"))
            .and(header(DEVICE_KEY_HEADER, "KEY-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connections": [
                {"id": "c1", "label": "ChatGPT", "client_name": "ChatGPT", "client_host": "chatgpt.com",
                 "created_at": 1, "last_used_at": null}]})))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/reins/api/connections/c1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN").with_device_key(Some("KEY-1"));
        let reg = DeviceRegistration {
            fcm_token: Some("fcm-1".to_owned()),
            master_password_hash: None,
        };
        assert!(api.register_device(&reg).await.unwrap().replaced_previous);
        assert_eq!(api.connections().await.unwrap().connections[0].client_host, "chatgpt.com");
        api.delete_connection("c1").await.unwrap();
    }

    #[tokio::test]
    async fn pending_clamps_wait() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .and(query_param("wait", "25"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "requests": [request_json("r1")],
                "pairings": [{"v": 1, "id": "p1", "client_name": "Claude", "client_host": "claude.ai",
                              "choices": [12, 47, 83], "created_at": 7}]})))
            .expect(1)
            .mount(&server)
            .await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        let p = api.pending(99).await.unwrap();
        assert_eq!(p.requests[0].id.0, "r1");
        assert!(matches!(p.requests[0].call, ToolCall::GmailSearch { .. }));
        assert_eq!(p.pairings[0].choices, [12, 47, 83]);
    }

    #[tokio::test]
    async fn status_mapping() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/reins/api/requests/gone"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "not_found", "message": "x"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/requests/r401"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/pending"))
            .respond_with(
                ResponseTemplate::new(403).set_body_json(json!({"error": "not_approval_device", "message": "m"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/api/requests/other"))
            .respond_with(ResponseTemplate::new(200).set_body_json(request_json("r1")))
            .mount(&server)
            .await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        assert_eq!(api.get_request("gone").await.unwrap_err().into_core(), CoreError::NotFound);
        assert_eq!(api.get_request("r401").await.unwrap_err(), ApiFailure::Unauthorized);
        assert_eq!(
            api.pending(0).await.unwrap_err().into_core(),
            CoreError::Server {
                status: 403,
                reason: "not_approval_device".to_owned()
            }
        );
        assert!(matches!(api.get_request("other").await, Err(ApiFailure::Core(CoreError::Invalid { .. }))));
    }

    #[tokio::test]
    async fn respond_and_pairing_answers() {
        let server = MockServer::start().await;
        let denied = RelayResponse {
            v: 1,
            outcome: RelayOutcome::Denied {
                reason: None,
            },
        };
        Mock::given(method("POST"))
            .and(path("/reins/api/requests/r1/response"))
            .and(body_json(serde_json::to_value(&denied).unwrap()))
            .respond_with(ResponseTemplate::new(204))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/api/requests/r1/response"))
            .respond_with(
                ResponseTemplate::new(409).set_body_json(json!({"error": "already_answered", "message": "m"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/api/pairings/p1/response"))
            .and(body_json(json!({"v": 1, "approved": true, "chosen_code": 47, "label": "Work Claude"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection_id": "c9"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/api/pairings/p2/response"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": "wrong_code", "message": "m"})))
            .mount(&server)
            .await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        api.respond("r1", &denied).await.unwrap();
        let again = api.respond("r1", &denied).await.unwrap_err();
        assert!(matches!(&again, ApiFailure::Status { status: 409, code, .. } if code == "already_answered"));
        let answer = PairingResponse {
            v: 1,
            approved: true,
            chosen_code: Some(47),
            label: Some("Work Claude".to_owned()),
        };
        assert_eq!(api.answer_pairing("p1", &answer).await.unwrap().connection_id.unwrap().0, "c9");
        let wrong = api.answer_pairing("p2", &answer).await.unwrap_err();
        assert!(matches!(&wrong, ApiFailure::Status { status: 409, code, .. } if code == "wrong_code"));
    }

    #[tokio::test]
    async fn claiming_a_code_returns_its_pairing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reins/api/pairings/claim"))
            .and(body_json(json!({"v": 1, "user_code": "BCDF-GHJK"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"v": 1, "id": "p1",
                "client_name": "Reins desktop app on mac", "client_host": "127.0.0.1", "choices": [12, 47, 83],
                "created_at": 7, "client_key": "k"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/reins/api/pairings/claim"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "not_found", "message": "m"})))
            .mount(&server)
            .await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        let pairing = api.claim_pairing("BCDF-GHJK").await.unwrap();
        assert_eq!((pairing.id.0.as_str(), pairing.client_key.as_deref()), ("p1", Some("k")));
        assert_eq!(api.claim_pairing("ZZZZ-ZZZZ").await.unwrap_err().into_core(), CoreError::NotFound);
    }

    #[tokio::test]
    async fn malformed_ids_never_reach_the_network() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any()).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        let long = "a".repeat(65);
        for bad in ["", "../connections", "a/b", "a?b", "a%2Fb", long.as_str()] {
            assert!(matches!(api.get_request(bad).await, Err(ApiFailure::Core(CoreError::Invalid { .. }))), "{bad}");
            assert!(api.get_pairing(bad).await.is_err());
            assert!(api.delete_connection(bad).await.is_err());
        }
    }
}
