//! The executor's side of the executor HTTP contract (`/reins/org-exec`, the O2 plan's "executor HTTP contract"). Every
//! call is signed with the executor's Ed25519 key over [`exec_request_message`]; the server's answers are only data
//! here, checked by [`crate::verify`] against pinned keys before anything acts on them.

use std::path::Path;
use std::sync::Arc;

use reins_proto::managed::{
    Envelope, EnvelopeKind, Job, Receipt, canonical_json, envelope_message, exec_request_message,
};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::connectors::http_client;

/// The base path of every executor route.
pub const BASE_PATH: &str = "/reins/org-exec";

/// `GET /keys`: the org's public keys, (key id, base64url). Shown for pinning only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgKeys {
    pub issuer: (String, String),
    pub decision: (String, String),
}

/// `GET /revocations`: a lease to revoke upstream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revocation {
    pub lease: String,
    /// `revoked` or `expired`.
    pub reason: String,
}

/// Makes a new executor key at `out` (PKCS#8, base64url, owner-only) and returns its public key (base64url). Never
/// overwrites a key.
pub fn generate_key(out: &Path) -> Result<String, String> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| "no randomness".to_owned())?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "key generation failed".to_owned())?;
    let text = Zeroizing::new(data_encoding::BASE64URL_NOPAD.encode(pkcs8.as_ref()));
    if out.exists() {
        return Err(format!("{} exists; not replacing a key", out.display()));
    }
    crate::leases::write_private(out, text.as_bytes())?;
    Ok(data_encoding::BASE64URL_NOPAD.encode(pair.public_key().as_ref()))
}

/// Reads the key [`generate_key`] wrote.
pub fn load_key(path: &Path) -> Result<Ed25519KeyPair, String> {
    let text = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?);
    let der = Zeroizing::new(
        data_encoding::BASE64URL_NOPAD
            .decode(text.trim().as_bytes())
            .map_err(|_| format!("{} is not an executor key", path.display()))?,
    );
    Ed25519KeyPair::from_pkcs8(&der).map_err(|_| format!("{} is not an executor key", path.display()))
}

pub struct Client {
    server: String,
    org: String,
    executor_id: String,
    key: Arc<Ed25519KeyPair>,
    http: reqwest::Client,
}

fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && s != "."
        && s != ".."
}

impl Client {
    /// `server`: the Reins server's origin.
    pub fn new(server: &str, org: &str, executor_id: &str, key: Arc<Ed25519KeyPair>) -> Result<Self, String> {
        Ok(Self {
            server: server.trim_end_matches('/').to_owned(),
            org: org.to_owned(),
            executor_id: executor_id.to_owned(),
            key,
            http: http_client()?,
        })
    }

    #[must_use]
    pub fn org(&self) -> &str {
        &self.org
    }

    #[must_use]
    pub fn executor_id(&self) -> &str {
        &self.executor_id
    }

    /// The executor's public key (base64url).
    #[must_use]
    pub fn public_key(&self) -> String {
        data_encoding::BASE64URL_NOPAD.encode(self.key.public_key().as_ref())
    }

    /// `receipt` as an envelope signed by this executor (`key_id` = the executor id).
    #[must_use]
    pub fn sign_receipt(&self, receipt: &Receipt) -> Envelope {
        let body = canonical_json(&serde_json::to_value(receipt).unwrap_or_default());
        let sig = self.key.sign(&envelope_message(EnvelopeKind::Receipt, &self.org, &self.executor_id, &body));
        Envelope {
            kind: EnvelopeKind::Receipt,
            org: self.org.clone(),
            body,
            key_id: self.executor_id.clone(),
            sig: data_encoding::BASE64URL_NOPAD.encode(sig.as_ref()),
        }
    }

    async fn call(&self, method: reqwest::Method, route: &str, body: Vec<u8>) -> Result<reqwest::Response, String> {
        let path = format!("{BASE_PATH}{route}");
        let at = crate::now();
        let sig = self.key.sign(&exec_request_message(&self.org, &self.executor_id, method.as_str(), &path, at, &body));
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.server))
            .header("X-Reins-Executor", format!("{}/{}", self.org, self.executor_id))
            .header("X-Reins-Time", at.to_string())
            .header("X-Reins-Signature", data_encoding::BASE64URL_NOPAD.encode(sig.as_ref()));
        if !body.is_empty() {
            req = req.header("Content-Type", "application/json").body(body);
        }
        let resp = req.send().await.map_err(|e| format!("the server: {}", e.without_url()))?;
        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(format!("the server answered {} to {route}", resp.status()))
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, route: &str) -> Result<T, String> {
        self.call(reqwest::Method::GET, route, Vec::new())
            .await?
            .json()
            .await
            .map_err(|e| format!("the server's answer to {route} is not valid: {}", e.without_url()))
    }

    /// `POST /claim`: up to ten jobs, now executing for this executor.
    pub async fn claim(&self) -> Result<Vec<Job>, String> {
        self.call(reqwest::Method::POST, "/claim", b"{}".to_vec())
            .await?
            .json()
            .await
            .map_err(|e| format!("the server's jobs are not valid: {}", e.without_url()))
    }

    /// `GET /keys`.
    pub async fn keys(&self) -> Result<OrgKeys, String> {
        self.get_json("/keys").await
    }

    /// `GET /policy/<hash>`.
    pub async fn policy(&self, hash: &str) -> Result<Envelope, String> {
        if !segment_ok(hash) {
            return Err("not a policy hash".to_owned());
        }
        self.get_json(&format!("/policy/{hash}")).await
    }

    /// `GET /rollouts`: the issuer-signed steps of the org's open and recently finished policy rollouts (O4).
    pub async fn rollouts(&self) -> Result<Vec<Envelope>, String> {
        self.get_json("/rollouts").await
    }

    /// `GET /roster`.
    pub async fn roster(&self) -> Result<Envelope, String> {
        self.get_json("/roster").await
    }

    /// `POST /receipt`.
    pub async fn receipt(&self, env: &Envelope) -> Result<(), String> {
        let body = serde_json::to_vec(env).map_err(|e| e.to_string())?;
        self.call(reqwest::Method::POST, "/receipt", body).await.map(drop)
    }

    /// `POST /lease/<id>` with `{sealed, expires_at}` (Task 13's route; the run loop reports leases in their receipt,
    /// as the contract says).
    pub async fn lease_issued(&self, id: &str, sealed: &str, expires_at: i64) -> Result<(), String> {
        if !segment_ok(id) {
            return Err("not a lease id".to_owned());
        }
        let body = serde_json::to_vec(&serde_json::json!({"sealed": sealed, "expires_at": expires_at}))
            .map_err(|e| e.to_string())?;
        self.call(reqwest::Method::POST, &format!("/lease/{id}"), body).await.map(drop)
    }

    /// `GET /revocations`.
    pub async fn revocations(&self) -> Result<Vec<Revocation>, String> {
        self.get_json("/revocations").await
    }

    /// `POST /revoked/<id>`.
    pub async fn revoked(&self, id: &str) -> Result<(), String> {
        if !segment_ok(id) {
            return Err("not a lease id".to_owned());
        }
        self.call(reqwest::Method::POST, &format!("/revoked/{id}"), Vec::new()).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{ED25519, UnparsedPublicKey};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn requests_are_signed_as_the_contract_says() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/reins/org-exec/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/reins/org-exec/revocations"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{"lease": "ls_1", "reason": "revoked"}])),
            )
            .mount(&server)
            .await;
        let key = Arc::new(Ed25519KeyPair::from_seed_unchecked(&[5; 32]).unwrap());
        let public = key.public_key().as_ref().to_vec();
        let client = Client::new(&server.uri(), "o1", "ex-1", key).unwrap();
        assert!(client.claim().await.unwrap().is_empty());
        assert_eq!(client.revocations().await.unwrap()[0].lease, "ls_1");
        assert!(client.policy("../keys").await.is_err());
        for req in server.received_requests().await.unwrap() {
            let h = |k: &str| req.headers.get(k).unwrap().to_str().unwrap().to_owned();
            assert_eq!(h("x-reins-executor"), "o1/ex-1");
            let at: i64 = h("x-reins-time").parse().unwrap();
            assert!((crate::now() - at).abs() < 5);
            let sig = data_encoding::BASE64URL_NOPAD.decode(h("x-reins-signature").as_bytes()).unwrap();
            let msg = exec_request_message("o1", "ex-1", req.method.as_str(), req.url.path(), at, &req.body);
            UnparsedPublicKey::new(&ED25519, &public).verify(&msg, &sig).expect("signed over the contract's message");
        }
        let claim = &server.received_requests().await.unwrap()[0];
        assert_eq!(claim.body, b"{}");
    }

    #[test]
    fn keys_are_made_once_and_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("executor.key");
        let public = generate_key(&path).unwrap();
        assert!(generate_key(&path).is_err(), "never replaces a key");
        let pair = load_key(&path).unwrap();
        assert_eq!(data_encoding::BASE64URL_NOPAD.encode(pair.public_key().as_ref()), public);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
