//! Client for the phone-facing server API (contracts §A), base
//! `{server}/reins/api`, authenticated with the Vaultwarden access token.

use std::time::Duration;

use reins_proto::device::{
    AccountDeletion, Connections, DEVICE_KEY_HEADER, DeviceRegistered, DeviceRegistration, PairingResult, Pending,
    codes,
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

    /// [`Self::into_core`] for `GET /connections`: a 403 saying this phone is neither the approval device
    /// (`not_approval_device`: an account without phone roles) nor a current member of the roster (`not_a_member`)
    /// is [`CoreError::NotAViewer`], so the app never shows the refusal as an empty list.
    pub fn into_connections_core(self) -> CoreError {
        match &self {
            Self::Status {
                status: 403,
                code,
                ..
            } if code == codes::NOT_A_MEMBER || code == codes::NOT_APPROVAL_DEVICE => CoreError::NotAViewer,
            _ => self.into_core(),
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

    pub async fn logout_browser_url(&self) -> Result<Option<String>, ApiFailure> {
        #[derive(serde::Deserialize)]
        struct Answer {
            browser_url: Option<String>,
        }
        let answer: Answer = Self::json(self.request(Method::POST, "/logout")).await?;
        if let Some(raw) = &answer.browser_url {
            let url = url::Url::parse(raw).map_err(|_| CoreError::invalid("invalid browser logout URL"))?;
            let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if !(url.scheme() == "https" || (url.scheme() == "http" && local))
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/user_management/sessions/logout"
            {
                return Err(CoreError::invalid("invalid browser logout URL").into());
            }
        }
        Ok(answer.browser_url)
    }

    /// `POST /account/delete`: deletes the signed-in account on the server (204).
    pub async fn delete_account(&self, deletion: &AccountDeletion) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/account/delete").json(deletion)).await.map(drop)
    }

    /// `PUT /modes`: the phone's Autopilot modes now, for the desktop apps (an older server answers 404).
    pub async fn put_modes(&self, report: &reins_proto::autopilot::ModesReport) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/modes").json(report)).await.map(drop)
    }

    /// `POST /requests/{id}/suggestion`: what Autopilot would do with a request that waits for the user (an older
    /// server answers 404).
    pub async fn post_suggestion(
        &self,
        id: &str,
        suggestion: &reins_proto::autopilot::Suggestion,
    ) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/requests/{id}/suggestion")).json(suggestion)).await.map(drop)
    }

    /// `PUT /services`: which integrations have an account on this phone.
    pub async fn put_services(&self, report: &reins_proto::device::ServicesReport) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/services").json(report)).await.map(drop)
    }

    /// `POST /desktop/revocations`: "drop the secrets you hold", sealed to each paired computer (204).
    pub async fn post_revocations(&self, post: &reins_proto::desktop::RevocationPost) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/desktop/revocations").json(post)).await.map(drop)
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

    /// `GET /tickets`: every open or approved ticket of the account, sealed as stored.
    pub async fn tickets(&self) -> Result<Vec<reins_proto::ticket::TicketItem>, ApiFailure> {
        #[derive(serde::Deserialize)]
        struct Tickets {
            tickets: Vec<reins_proto::ticket::TicketItem>,
        }
        let all: Tickets = Self::json(self.request(Method::GET, "/tickets")).await?;
        Ok(all.tickets)
    }

    /// `POST /tickets/{id}/events`: this phone's signed transitions of a ticket (Later, "Why?", an approval to
    /// collect later, an expiry it confirms).
    pub async fn post_ticket_events(
        &self,
        id: &str,
        events: &reins_proto::ticket::TicketEvents,
    ) -> Result<reins_proto::ticket::TicketStatus, ApiFailure> {
        check_id(id)?;
        Self::json(self.request(Method::POST, &format!("/tickets/{id}/events")).json(events)).await
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

    /// A11 `PUT /connections/{id}/name`: a computer's name, sealed to its desktop app's key (`None`: it has none).
    pub async fn put_connection_name(
        &self,
        id: &str,
        name: &reins_proto::device::SealedName,
    ) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::PUT, &format!("/connections/{id}/name")).json(name)).await.map(drop)
    }

    /// A9 `GET /devices`.
    pub async fn devices(&self) -> Result<reins_proto::device::Devices, ApiFailure> {
        Self::json(self.request(Method::GET, "/devices")).await
    }

    /// O1 `GET /org`: the work account's organization; `None` for any account that isn't an org's member (404).
    pub async fn org_me(&self) -> Result<Option<reins_proto::org::OrgMe>, ApiFailure> {
        match Self::json(self.request(Method::GET, "/org")).await {
            Ok(me) => Ok(Some(me)),
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// O1 `POST /org/enroll/challenge`.
    pub async fn org_challenge(&self) -> Result<reins_proto::org::EnrollmentChallenge, ApiFailure> {
        Self::json(self.request(Method::POST, "/org/enroll/challenge")).await
    }

    /// O1 `POST /org/devices`.
    pub async fn org_enroll(&self, request: &reins_proto::org::EnrollmentRequest) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::POST, "/org/devices").json(request)).await.map(drop)
    }

    /// O1 `PUT /org/posture`.
    pub async fn org_posture(&self, posture: &reins_proto::org::Posture) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/org/posture").json(posture)).await.map(drop)
    }

    /// O2 `GET /org/reviews`: what waits for this member's review; none for an account that isn't an org's member
    /// (404).
    pub async fn org_reviews(&self) -> Result<Vec<reins_proto::managed::ReviewItem>, ApiFailure> {
        match Self::json(self.request(Method::GET, "/org/reviews")).await {
            Ok(items) => Ok(items),
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// O2 `POST /org/reviews/{id}`: this device's signed answer.
    pub async fn org_answer(&self, id: &str, answer: &reins_proto::managed::ReviewAnswer) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::POST, &format!("/org/reviews/{id}")).json(answer)).await.map(drop)
    }

    /// O3 `GET /org/passkeys`: the web passkeys this member registered; none for an account that isn't an org's
    /// member (404).
    pub async fn org_passkeys(&self) -> Result<Vec<crate::org::PasskeyItem>, ApiFailure> {
        match Self::json(self.request(Method::GET, "/org/passkeys")).await {
            Ok(items) => Ok(items),
            Err(ApiFailure::Status {
                status: 404,
                ..
            }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// O3 `POST /org/passkeys/{id}/confirm`: this device's signed confirmation (or refusal) of a web passkey.
    pub async fn org_confirm_passkey(&self, id: &str, answer: &crate::org::PasskeyConfirm) -> Result<(), ApiFailure> {
        crate::org::check_credential_id(id)?;
        Self::send(self.request(Method::POST, &format!("/org/passkeys/{id}/confirm")).json(answer)).await.map(drop)
    }

    /// O3 `DELETE /org/passkeys/<id>`: revokes one of this member's web passkeys (no signature: it only narrows).
    pub async fn org_revoke_passkey(&self, id: &str) -> Result<(), ApiFailure> {
        crate::org::check_credential_id(id)?;
        Self::send(self.request(Method::DELETE, &format!("/org/passkeys/{id}"))).await.map(drop)
    }

    /// O2 `PUT /org/member-policy`: the member's own stricter policy, signed by this device.
    pub async fn org_member_policy(&self, policy: &reins_proto::managed::SignedMemberPolicy) -> Result<(), ApiFailure> {
        Self::send(self.request(Method::PUT, "/org/member-policy").json(policy)).await.map(drop)
    }

    /// A10 `DELETE /devices/{id}`: signs another device of the account out, with proof typed now.
    pub async fn delete_device(&self, id: &str, proof: &reins_proto::device::DeviceSignOut) -> Result<(), ApiFailure> {
        check_id(id)?;
        Self::send(self.request(Method::DELETE, &format!("/devices/{id}")).json(proof)).await.map(drop)
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
            sign_pub: None,
            box_pub: None,
        };
        assert!(api.register_device(&reg).await.unwrap().replaced_previous);
        assert_eq!(api.connections().await.unwrap().connections[0].client_host, "chatgpt.com");
        api.delete_connection("c1").await.unwrap();
    }

    #[test]
    fn a_refused_connections_list_is_not_a_viewer_and_other_failures_keep_their_meaning() {
        let status = |status: u16, code: &str| ApiFailure::Status {
            status,
            code: code.to_owned(),
            message: "m".to_owned(),
        };
        // Neither the approval device nor a member: the list is not this phone's to read.
        assert_eq!(status(403, codes::NOT_A_MEMBER).into_connections_core(), CoreError::NotAViewer);
        assert_eq!(status(403, codes::NOT_APPROVAL_DEVICE).into_connections_core(), CoreError::NotAViewer);
        // Anything else stays what it was.
        assert_eq!(
            status(403, codes::RATE_LIMITED).into_connections_core(),
            CoreError::Server {
                status: 403,
                reason: codes::RATE_LIMITED.to_owned()
            }
        );
        assert_eq!(
            status(500, codes::NOT_A_MEMBER).into_connections_core(),
            CoreError::Server {
                status: 500,
                reason: codes::NOT_A_MEMBER.to_owned()
            }
        );
        assert_eq!(ApiFailure::Unauthorized.into_connections_core(), CoreError::NotLoggedIn);
        assert_eq!(status(404, "").into_connections_core(), CoreError::NotFound);
        // Elsewhere a 403 keeps its code (the "replaced" banner reads it from the poll).
        assert_eq!(
            status(403, codes::NOT_APPROVAL_DEVICE).into_core(),
            CoreError::Server {
                status: 403,
                reason: codes::NOT_APPROVAL_DEVICE.to_owned()
            }
        );
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
            decided: None,
            events: vec![],
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

    #[tokio::test]
    async fn browser_logout_never_returns_an_insecure_or_unrelated_navigation() {
        let server = MockServer::start().await;
        let (http, url) = setup(&server);
        let api = PhoneApi::new(&http, &url, "TOKEN");
        for raw in [
            "javascript:alert(1)",
            "http://example.com/user_management/sessions/logout",
            "https://user:password@example.com/user_management/sessions/logout",
            "https://api.workos.com/other",
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .and(path("/reins/api/logout"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"browser_url":raw})))
                .mount(&server)
                .await;
            assert!(matches!(api.logout_browser_url().await, Err(ApiFailure::Core(CoreError::Invalid { .. }))));
        }
    }
}
