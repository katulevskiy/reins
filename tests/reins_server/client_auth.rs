//! RFC 7523 `private_key_jwt` client authentication, as ChatGPT uses it: a client ID metadata document with a
//! `jwks_uri`, served here by a mock server (the test mode lets the server fetch from `http://127.0.0.1`).

use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use data_encoding::BASE64URL_NOPAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use openssl::{pkey::PKey, rsa::Rsa};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::{
    harness::{Options, Phone, REDIRECT, Server, VERIFIER, between, client, query_param},
    mock::{MockServer, Reply},
};

const JWT_BEARER: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

fn now() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs()).expect("time")
}

fn b64(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

/// An RSA signing key and its public JWK.
struct Key {
    kid: String,
    jwk: Value,
    signing: EncodingKey,
}

impl Key {
    fn new(kid: &str) -> Self {
        let rsa = Rsa::generate(2048).expect("rsa");
        let jwk = json!({"kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256",
            "n": b64(&rsa.n().to_vec()), "e": b64(&rsa.e().to_vec())});
        let pem = PKey::from_rsa(rsa).expect("pkey").private_key_to_pem_pkcs8().expect("pem");
        Self {
            kid: kid.to_owned(),
            jwk,
            signing: EncodingKey::from_rsa_pem(&pem).expect("encoding key"),
        }
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        jsonwebtoken::encode(&header, claims, &self.signing).expect("sign")
    }
}

/// A `private_key_jwt` client: its metadata document and key set on a mock server, its keys to sign with.
struct JwtClient {
    mock: MockServer,
    id: String,
    key: Key,
    /// What `/jwks.json` serves; tests rotate it.
    served: Arc<Mutex<Vec<Value>>>,
}

impl JwtClient {
    async fn start() -> Self {
        let key = Key::new("key-1");
        let served = Arc::new(Mutex::new(vec![key.jwk.clone()]));
        let keys = Arc::clone(&served);
        let mock = MockServer::start(move |req| {
            let origin = format!("http://{}", req.header("host").unwrap_or_default());
            match req.path.as_str() {
                // Shaped like ChatGPT's document (src/api/reins/oauth.rs tests hold the real one).
                "/oauth/client.json" => Reply::json(
                    200,
                    &json!({
                        "client_id": format!("{origin}/oauth/client.json"),
                        "client_name": "ChatGPT",
                        "redirect_uris": [REDIRECT],
                        "token_endpoint_auth_method": "private_key_jwt",
                        "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
                        "grant_types": ["authorization_code", "refresh_token"],
                        "response_types": ["code"],
                        "token_endpoint_auth_signing_alg": "RS256",
                        "jwks_uri": format!("{origin}/oauth/jwks.json")
                    }),
                ),
                "/oauth/jwks.json" => Reply::json(200, &json!({"keys": *keys.lock().unwrap()})),
                _ => Reply::json(404, &json!({})),
            }
        })
        .await;
        let id = mock.url("/oauth/client.json");
        Self {
            mock,
            id,
            key,
            served,
        }
    }

    fn jwks_fetches(&self) -> usize {
        self.mock.requests().iter().filter(|r| r.path == "/oauth/jwks.json").count()
    }

    fn claims(&self, server: &Server, jti: &str) -> Value {
        json!({"iss": self.id, "sub": self.id, "aud": server.url("/reins/oauth/token"), "jti": jti,
            "iat": now(), "exp": now() + 120})
    }

    fn assertion(&self, server: &Server, jti: &str) -> String {
        self.key.sign(&self.claims(server, jti))
    }
}

/// Authorizes `client_id` for `email` through the browser pages and the phone; returns the authorization code.
async fn authorize_code(server: &Server, phone: &Phone, email: &str, client_id: &str) -> String {
    let wait_url = server.start_authorization(client_id, email).await;
    let html = client().get(&wait_url).send().await.expect("wait").text().await.expect("wait html");
    let code: u8 = between(&html, "aria-label=\"code\">", "<").parse().expect("code");
    let (_, pending) = phone.get("/pending?wait=5").await;
    let id = pending["pairings"][0]["id"].as_str().expect("pairing id").to_owned();
    let (status, body) = phone
        .post(
            &format!("/pairings/{id}/response"),
            &json!({"v": 1, "approved": true, "chosen_code": code, "label": null}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let done = client().get(&wait_url).send().await.expect("wait again");
    let location = done.headers()["location"].to_str().expect("location").to_owned();
    query_param(&location, "code").expect("authorization code")
}

fn code_form<'a>(code: &'a str, client_id: Option<&'a str>, assertion: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT),
        ("code_verifier", VERIFIER),
    ];
    if let Some(id) = client_id {
        form.push(("client_id", id));
    }
    if let Some(assertion) = assertion {
        form.extend([("client_assertion_type", JWT_BEARER), ("client_assertion", assertion)]);
    }
    form
}

fn refresh_form<'a>(refresh: &'a str, assertion: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", refresh)];
    if let Some(assertion) = assertion {
        form.extend([("client_assertion_type", JWT_BEARER), ("client_assertion", assertion)]);
    }
    form
}

fn assert_invalid_client((status, body): &(StatusCode, Value), what: &str) {
    assert_eq!((*status, body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_client")), "{what}: {body}");
}

async fn start_server() -> Server {
    Server::start_with(Options {
        allow_loopback: true,
        ..Options::default()
    })
    .await
}

#[tokio::test]
async fn the_server_advertises_private_key_jwt() {
    let server = Server::start().await;
    let meta: Value =
        client().get(server.url("/.well-known/oauth-authorization-server")).send().await.unwrap().json().await.unwrap();
    assert_eq!(meta["token_endpoint_auth_methods_supported"], json!(["none", "private_key_jwt"]));
    assert_eq!(meta["token_endpoint_auth_signing_alg_values_supported"], json!(["RS256", "PS256", "ES256", "EdDSA"]));
    assert_eq!(meta["client_id_metadata_document_supported"], json!(true));
}

#[tokio::test]
async fn a_private_key_jwt_client_connects_refreshes_and_revokes() {
    let server = start_server().await;
    let phone = server.phone("ivy@example.com").await;
    phone.register_device().await;
    let chatgpt = JwtClient::start().await;

    // The authorize page accepts the client (this was the production failure).
    let page = client().get(server.authorize_url(&chatgpt.id)).send().await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.text().await.unwrap().contains("ChatGPT"));

    // Without an assertion the code is refused, but not burnt: only the client's key can use it.
    let code = authorize_code(&server, &phone, "ivy@example.com", &chatgpt.id).await;
    let missing = server.token(&code_form(&code, Some(&chatgpt.id), None)).await;
    assert_invalid_client(&missing, "no assertion");

    // With one, it is exchanged; client_id may be left out (the assertion names the client).
    let assertion = chatgpt.assertion(&server, "a1");
    let (status, tokens) = server.token(&code_form(&code, None, Some(&assertion))).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(chatgpt.jwks_fetches(), 1);
    let (_, again) = server.token(&code_form(&code, None, Some(&chatgpt.assertion(&server, "a0")))).await;
    assert_eq!(again["error"], "invalid_grant", "the code is single-use");

    // Refreshing needs the client's assertion too; failures do not burn the refresh token.
    assert_invalid_client(&server.token(&refresh_form(&refresh, None)).await, "refresh without an assertion");
    assert_invalid_client(&server.token(&refresh_form(&refresh, Some(&assertion))).await, "replayed jti");
    let (status, rotated) = server.token(&refresh_form(&refresh, Some(&chatgpt.assertion(&server, "a2")))).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let (_, old) = server.token(&refresh_form(&refresh, Some(&chatgpt.assertion(&server, "a3")))).await;
    assert_eq!(old["error"], "invalid_grant", "rotated away");
    let refresh = rotated["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(chatgpt.jwks_fetches(), 1, "the key set is cached");

    // Revoking its refresh token also needs the assertion.
    let revoke = |form: Vec<(&'static str, String)>| {
        let url = server.url("/reins/oauth/revoke");
        async move {
            let r = client().post(url).form(&form).send().await.unwrap();
            let status = r.status();
            (status, r.json::<Value>().await.unwrap_or(Value::Null))
        }
    };
    let denied = revoke(vec![("token", refresh.clone())]).await;
    assert_invalid_client(&denied, "revocation without an assertion");
    let count = async || phone.get("/connections").await.1["connections"].as_array().map_or(0, Vec::len);
    assert_eq!(count().await, 1, "a refused revocation revokes nothing");
    let (status, _) = revoke(vec![
        ("token", refresh.clone()),
        ("client_assertion_type", JWT_BEARER.to_owned()),
        ("client_assertion", chatgpt.assertion(&server, "a4")),
    ])
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count().await, 0);
    let (_, gone) = server.token(&refresh_form(&refresh, Some(&chatgpt.assertion(&server, "a5")))).await;
    assert_eq!(gone["error"], "invalid_grant");
}

#[tokio::test]
async fn bad_assertions_are_refused_at_refresh() {
    let server = start_server().await;
    let phone = server.phone("jade@example.com").await;
    phone.register_device().await;
    let chatgpt = JwtClient::start().await;
    let code = authorize_code(&server, &phone, "jade@example.com", &chatgpt.id).await;
    let (status, tokens) =
        server.token(&code_form(&code, Some(&chatgpt.id), Some(&chatgpt.assertion(&server, "first")))).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();

    let with = |change: &dyn Fn(&mut Value), jti: &str| {
        let mut claims = chatgpt.claims(&server, jti);
        change(&mut claims);
        chatgpt.key.sign(&claims)
    };
    let impostor = Key::new("key-1");
    let forged = impostor.sign(&chatgpt.claims(&server, "forged"));
    let header_none = b64(br#"{"alg":"none","kid":"key-1"}"#);
    let payload = b64(chatgpt.claims(&server, "none").to_string().as_bytes());
    let alg_none = format!("{header_none}.{payload}.");
    let mut hs_header = Header::new(Algorithm::HS256);
    hs_header.kid = Some("key-1".into());
    let n = chatgpt.key.jwk["n"].as_str().unwrap().as_bytes();
    let hs256 = jsonwebtoken::encode(&hs_header, &chatgpt.claims(&server, "hs"), &EncodingKey::from_secret(n)).unwrap();
    let cases = [
        (forged, "signed by another key"),
        (with(&|c| c["aud"] = json!("https://other.example/reins/oauth/token"), "aud"), "wrong aud"),
        (with(&|c| c["aud"] = json!(server.url("/mcp")), "aud2"), "the resource as aud"),
        (with(&|c| c["exp"] = json!(now() - 300), "exp"), "expired"),
        (with(&|c| c["exp"] = json!(now() + 3600), "far"), "expires an hour ahead"),
        (with(&|c| c["iss"] = json!("https://evil.example/client.json"), "iss"), "another iss"),
        (with(&|c| drop(c.as_object_mut().unwrap().remove("jti")), "unused"), "no jti"),
        (alg_none, "alg none"),
        (hs256, "HS256 with the public key as secret"),
        ("not-a-jwt".to_owned(), "garbage"),
    ];
    for (assertion, what) in &cases {
        assert_invalid_client(&server.token(&refresh_form(&refresh, Some(assertion))).await, what);
    }
    // The refresh token survived all of that.
    let (status, body) = server.token(&refresh_form(&refresh, Some(&chatgpt.assertion(&server, "last")))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_rotated_key_is_found_by_fetching_the_key_set_again_once() {
    let server = start_server().await;
    let phone = server.phone("kate@example.com").await;
    phone.register_device().await;
    let mut chatgpt = JwtClient::start().await;
    let code = authorize_code(&server, &phone, "kate@example.com", &chatgpt.id).await;
    let (_, tokens) = server.token(&code_form(&code, None, Some(&chatgpt.assertion(&server, "r1")))).await;
    let mut refresh = tokens["refresh_token"].as_str().expect("refresh").to_owned();
    assert_eq!(chatgpt.jwks_fetches(), 1);

    // The client rotates to a new key: the cached set lacks its kid, so the server fetches the set again.
    chatgpt.key = Key::new("key-2");
    chatgpt.served.lock().unwrap().push(chatgpt.key.jwk.clone());
    let (status, body) = server.token(&refresh_form(&refresh, Some(&chatgpt.assertion(&server, "r2")))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(chatgpt.jwks_fetches(), 2);
    refresh = body["refresh_token"].as_str().unwrap().to_owned();

    // Made-up kids do not make the server fetch again and again.
    let stranger = Key::new("key-3");
    for jti in ["s1", "s2", "s3"] {
        let assertion = stranger.sign(&chatgpt.claims(&server, jti));
        assert_invalid_client(&server.token(&refresh_form(&refresh, Some(&assertion))).await, "unknown kid");
    }
    assert_eq!(chatgpt.jwks_fetches(), 2, "refetched at most every 30 s per key set");
}

#[tokio::test]
async fn a_public_client_must_not_send_an_assertion() {
    let server = start_server().await;
    let phone = server.phone("liam@example.com").await;
    phone.register_device().await;
    let chatgpt = JwtClient::start().await;
    let public = server.register_client().await;
    // Its own (or anyone's) assertion at the code exchange: invalid_client, and the code stays usable.
    let code = authorize_code(&server, &phone, "liam@example.com", &public).await;
    let assertion = chatgpt.assertion(&server, "p1");
    assert_invalid_client(&server.token(&code_form(&code, Some(&public), Some(&assertion))).await, "code exchange");
    // And at refresh.
    let (status, tokens) = server.token(&code_form(&code, Some(&public), None)).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let assertion = chatgpt.assertion(&server, "p2");
    assert_invalid_client(&server.token(&refresh_form(refresh, Some(&assertion))).await, "refresh");
    let (status, _) = server.token(&refresh_form(refresh, None)).await;
    assert_eq!(status, StatusCode::OK, "still works as a public client");
    // A private_key_jwt client cannot use another client's code, even with its own valid assertion.
    let code = authorize_code(&server, &phone, "liam@example.com", &public).await;
    let theirs = chatgpt.assertion(&server, "p3");
    let (_, body) = server.token(&code_form(&code, Some(&chatgpt.id), Some(&theirs))).await;
    assert_eq!(body["error"], "invalid_grant");
}

/// Valid assertions that come without a valid grant of their own client (a made-up code or refresh token, another
/// client's token, an unknown token at revocation) record nothing: the same `jti` still works afterwards.
#[tokio::test]
async fn assertions_without_a_valid_grant_record_nothing() {
    let server = start_server().await;
    let phone = server.phone("mia@example.com").await;
    phone.register_device().await;
    let chatgpt = JwtClient::start().await;
    let public = server.register_client().await;
    let free = chatgpt.assertion(&server, "free");
    let bogus_code = code_form("made-up-code", None, Some(&free));
    assert_eq!(server.token(&bogus_code).await.1["error"], "invalid_grant");
    let bogus_refresh = refresh_form("made-up-refresh-token", Some(&free));
    assert_eq!(server.token(&bogus_refresh).await.1["error"], "invalid_grant");
    let r = client()
        .post(server.url("/reins/oauth/revoke"))
        .form(&[("token", "made-up-token"), ("client_assertion_type", JWT_BEARER), ("client_assertion", &free)])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // Another client's code, named as that client: the assertion (ChatGPT's) is refused, recording nothing.
    let other = authorize_code(&server, &phone, "mia@example.com", &public).await;
    let (status, body) = server.token(&code_form(&other, Some(&public), Some(&free))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(chatgpt.jwks_fetches(), 0, "nothing above even fetched ChatGPT's keys");
    // The same jti is still fresh for a real exchange.
    let code = authorize_code(&server, &phone, "mia@example.com", &chatgpt.id).await;
    let (status, tokens) = server.token(&code_form(&code, None, Some(&free))).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
}

/// A flood of valid assertions from another client (its own metadata document and keys) with made-up grants does not
/// stop ChatGPT from connecting.
#[tokio::test]
async fn another_clients_flood_does_not_block_chatgpt() {
    let server = Server::start_with(Options {
        allow_loopback: true,
        env: vec![("UNAUTHENTICATED_RATELIMIT_MAX_BURST", "1000".to_owned())],
        ..Options::default()
    })
    .await;
    let phone = server.phone("noah@example.com").await;
    phone.register_device().await;
    let chatgpt = JwtClient::start().await;
    let attacker = JwtClient::start().await;
    for i in 0..300 {
        let assertion = attacker.assertion(&server, &format!("flood-{i}"));
        let form = if i % 2 == 0 {
            code_form("made-up-code", Some(&attacker.id), Some(&assertion))
        } else {
            refresh_form("made-up-refresh-token", Some(&assertion))
        };
        assert_eq!(server.token(&form).await.1["error"], "invalid_grant");
    }
    let code = authorize_code(&server, &phone, "noah@example.com", &chatgpt.id).await;
    let (status, tokens) = server.token(&code_form(&code, None, Some(&chatgpt.assertion(&server, "c1")))).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let (status, body) = server.token(&refresh_form(refresh, Some(&chatgpt.assertion(&server, "c2")))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// `/token` and `/revoke` share the per-address limit for unauthenticated requests.
#[tokio::test]
async fn token_and_revocation_are_rate_limited_per_address() {
    let server = Server::start_with(Options {
        env: vec![("UNAUTHENTICATED_RATELIMIT_MAX_BURST", "5".to_owned())],
        ..Options::default()
    })
    .await;
    let mut statuses = Vec::new();
    for _ in 0..4 {
        statuses.push(server.token(&refresh_form("made-up", None)).await.0);
        let r = client().post(server.url("/reins/oauth/revoke")).form(&[("token", "made-up")]).send().await.unwrap();
        statuses.push(r.status());
    }
    assert!(statuses[..5].iter().all(|s| *s != StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
    assert!(statuses[5..].iter().all(|s| *s == StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
}

#[tokio::test]
async fn the_device_flow_is_refused_to_private_key_jwt_clients() {
    let server = start_server().await;
    let chatgpt = JwtClient::start().await;
    let r = client()
        .post(server.url("/reins/oauth/device_authorization"))
        .form(&[("client_id", chatgpt.id.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized_client");
}
