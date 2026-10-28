//! Bring your own KMS for the org master key (`docs/design/decisions-v2/06-trust-and-compliance.md` §3–§4, `08` R12).
//!
//! With `REINS_ORG_KMS_PROVIDER` set to `aws`, `gcp` or `azure`, the key file (`REINS_ORG_KMS_KEY_FILE`) holds the
//! 32-byte master key *wrapped* by the customer's key (`REINS_ORG_KMS_KEY_ID`), as the text
//! `reins-wrapped-v1:<provider>:<base64>`. [`init`] runs once at launch: it unwraps the master (or, with no file yet,
//! makes one, wraps it, checks the KMS unwraps it back, and writes the file 0600) and installs it with
//! [`kms::install_kms`]. When it fails the server exits, and [`kms::kms`] never falls back to a local key.
//!
//! - **AWS KMS:** `Encrypt`/`Decrypt` (JSON 1.1, SigV4) with the encryption context `{"reins": "org-kms-master/v1"}`;
//!   credentials from `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`, else the instance role (IMDSv2).
//! - **GCP Cloud KMS:** `:encrypt`/`:decrypt` with additional authenticated data; a token from the service-account key
//!   file at `GOOGLE_APPLICATION_CREDENTIALS` (an RS256 assertion), else the metadata server.
//! - **Azure Key Vault:** `wrapkey`/`unwrapkey` with `RSA-OAEP-256`; a token from `AZURE_TENANT_ID`/`AZURE_CLIENT_ID`/
//!   `AZURE_CLIENT_SECRET`, else the managed identity (`AZURE_CLIENT_ID` alone picks a user-assigned one).
//!
//! These are operator-configured cloud endpoints and metadata services, so they use a plain client (10 s, no
//! redirects), not `api::reins::outbound` (which refuses metadata addresses by design). Tests point them at a mock
//! with `REINS_TEST_ALLOW_LOOPBACK` and `REINS_TEST_KMS_ENDPOINT` (the KMS API's origin) and
//! `REINS_TEST_KMS_TOKEN_ENDPOINT` (the token or metadata service's origin). Errors name the provider, the step and
//! the HTTP status, never a key, token or secret.

use std::{io::Write, path::Path, sync::Arc};

use data_encoding::{BASE64, BASE64URL_NOPAD};
use futures::future::BoxFuture;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use url::Url;
use zeroize::Zeroizing;

use super::{
    channels::{api_base, http},
    kms::{self, LocalKms},
    sigv4::{self, AwsCreds},
};
use crate::CONFIG;

/// What a wrapped key file starts with.
const FILE_PREFIX: &str = "reins-wrapped-v1";
/// AWS's encryption context value (`{"reins": …}`).
const AWS_CONTEXT: &str = "org-kms-master/v1";
/// GCP's additional authenticated data.
const GCP_AAD: &[u8] = b"reins/org-kms-master/v1";
const GCP_SCOPE: &str = "https://www.googleapis.com/auth/cloudkms";
const AZURE_RESOURCE: &str = "https://vault.azure.net";
/// The KMS API's origin, in tests.
pub const ENDPOINT_ENV: &str = "REINS_TEST_KMS_ENDPOINT";
/// The token (or metadata) service's origin, in tests.
pub const TOKEN_ENDPOINT_ENV: &str = "REINS_TEST_KMS_TOKEN_ENDPOINT";
const LINK_LOCAL_METADATA: &str = "http://169.254.169.254";
const GCP_METADATA: &str = "http://metadata.google.internal";

/// Wraps and unwraps the master key with a customer's KMS key.
pub trait KeyWrapper: Send + Sync {
    fn wrap<'a>(&'a self, plain: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>, String>>;
    fn unwrap<'a>(&'a self, wrapped: &'a [u8]) -> BoxFuture<'a, Result<Zeroizing<Vec<u8>>, String>>;
    /// The KMS key (ARN, key name or key URL); not secret.
    fn id(&self) -> String;
}

/// A BYOK provider (`REINS_ORG_KMS_PROVIDER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Aws,
    Gcp,
    Azure,
}

impl Provider {
    /// `None` for `local`; an error for anything unknown.
    pub fn parse(s: &str) -> Result<Option<Self>, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(None),
            "aws" => Ok(Some(Self::Aws)),
            "gcp" => Ok(Some(Self::Gcp)),
            "azure" => Ok(Some(Self::Azure)),
            _ => Err("REINS_ORG_KMS_PROVIDER must be local, aws, gcp or azure".to_owned()),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Azure => "azure",
        }
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Sends `req` and reads a JSON answer. Errors name the provider, the step and the status only.
async fn send_json(provider: &str, step: &str, req: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            format!("{provider} KMS: {step} timed out")
        } else {
            format!("{provider} KMS: {step} could not be reached")
        }
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("{provider} KMS: {step} answered HTTP {}", status.as_u16()));
    }
    resp.json::<Value>().await.map_err(|_| format!("{provider} KMS: {step} did not answer JSON"))
}

async fn send_text(provider: &str, step: &str, req: reqwest::RequestBuilder) -> Result<Zeroizing<String>, String> {
    let resp = req.send().await.map_err(|_| format!("{provider} KMS: {step} could not be reached"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("{provider} KMS: {step} answered HTTP {}", status.as_u16()));
    }
    resp.text().await.map(Zeroizing::new).map_err(|_| format!("{provider} KMS: {step} broke off"))
}

fn field<'a>(v: &'a Value, name: &str, provider: &str, step: &str) -> Result<&'a str, String> {
    v.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{provider} KMS: {step} answered without {name}"))
}

fn b64_field(v: &Value, name: &str, provider: &str, step: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    BASE64
        .decode(field(v, name, provider, step)?.as_bytes())
        .map(Zeroizing::new)
        .map_err(|_| format!("{provider} KMS: {step} answered a {name} that is not base64"))
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    BASE64URL_NOPAD.decode(s.trim_end_matches('=').as_bytes()).ok()
}

fn http_client(provider: &str) -> Result<reqwest::Client, String> {
    http().map_err(|_| format!("{provider} KMS: no HTTP client"))
}

// ---------------------------------------------------------------------------------------------------------- AWS KMS

/// AWS KMS (`TrentService.Encrypt` / `Decrypt`), keyed by a key or alias ARN.
pub struct AwsKms {
    region: String,
    key_id: String,
    /// The KMS API's origin (`https://kms.<region>.amazonaws.com`).
    endpoint: String,
    /// The instance metadata service's origin, for role credentials.
    imds: String,
    /// Static credentials from the environment; `None`: the instance role.
    creds: Option<AwsCreds>,
}

/// The region and DNS suffix of a KMS key ARN (`arn:<partition>:kms:<region>:<account>:key/…` or `alias/…`).
fn aws_region(arn: &str) -> Result<(String, &'static str), String> {
    let parts: Vec<&str> = arn.split(':').collect();
    let ok_region =
        |r: &str| !r.is_empty() && r.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    match parts.as_slice() {
        ["arn", partition, "kms", region, _account, resource]
            if ok_region(region) && (resource.starts_with("key/") || resource.starts_with("alias/")) =>
        {
            let suffix = match *partition {
                "aws" | "aws-us-gov" => "amazonaws.com",
                "aws-cn" => "amazonaws.com.cn",
                _ => return Err("REINS_ORG_KMS_KEY_ID: an unknown AWS partition".to_owned()),
            };
            Ok(((*region).to_owned(), suffix))
        }
        _ => Err("REINS_ORG_KMS_KEY_ID must be an AWS KMS key or alias ARN (arn:aws:kms:<region>:…)".to_owned()),
    }
}

/// Static AWS credentials, when both the key id and the secret are set.
fn aws_env_creds(get: &dyn Fn(&str) -> Option<String>) -> Option<AwsCreds> {
    Some(AwsCreds {
        access_key_id: get("AWS_ACCESS_KEY_ID")?,
        secret_access_key: Zeroizing::new(get("AWS_SECRET_ACCESS_KEY")?),
        session_token: get("AWS_SESSION_TOKEN"),
    })
}

/// A role's temporary credentials from IMDSv2.
async fn aws_imds_creds(imds: &str) -> Result<AwsCreds, String> {
    let http = http_client("aws")?;
    let token = send_text(
        "aws",
        "the instance metadata token",
        http.put(format!("{imds}/latest/api/token")).header("X-aws-ec2-metadata-token-ttl-seconds", "21600"),
    )
    .await?;
    let token = token.trim();
    let roles = send_text(
        "aws",
        "the instance role",
        http.get(format!("{imds}/latest/meta-data/iam/security-credentials/"))
            .header("X-aws-ec2-metadata-token", token),
    )
    .await?;
    let role = roles.lines().map(str::trim).find(|l| !l.is_empty()).ok_or("aws KMS: the instance has no role")?;
    if !role.bytes().all(|b| b.is_ascii_alphanumeric() || b"+=,.@_-".contains(&b)) {
        return Err("aws KMS: the instance role name is not valid".to_owned());
    }
    let v = send_json(
        "aws",
        "the instance role credentials",
        http.get(format!("{imds}/latest/meta-data/iam/security-credentials/{role}"))
            .header("X-aws-ec2-metadata-token", token),
    )
    .await?;
    let step = "the instance role credentials";
    Ok(AwsCreds {
        access_key_id: field(&v, "AccessKeyId", "aws", step)?.to_owned(),
        secret_access_key: Zeroizing::new(field(&v, "SecretAccessKey", "aws", step)?.to_owned()),
        session_token: Some(field(&v, "Token", "aws", step)?.to_owned()),
    })
}

impl AwsKms {
    /// For the key `arn`, with credentials from `get` (the environment) or the instance role.
    pub fn new(arn: &str, get: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let (region, suffix) = aws_region(arn)?;
        Ok(Self {
            endpoint: api_base(&format!("https://kms.{region}.{suffix}"), ENDPOINT_ENV),
            imds: api_base(LINK_LOCAL_METADATA, TOKEN_ENDPOINT_ENV),
            creds: aws_env_creds(get),
            region,
            key_id: arn.to_owned(),
        })
    }

    async fn call(&self, op: &str, body: &Value) -> Result<Value, String> {
        let creds = match &self.creds {
            Some(c) => c.clone(),
            None => aws_imds_creds(&self.imds).await?,
        };
        let url = Url::parse(&format!("{}/", self.endpoint)).map_err(|_| "aws KMS: a bad endpoint".to_owned())?;
        let body = serde_json::to_vec(body).map_err(|_| "aws KMS: a bad request".to_owned())?;
        let mut headers = vec![
            ("content-type".to_owned(), "application/x-amz-json-1.1".to_owned()),
            ("x-amz-target".to_owned(), format!("TrentService.{op}")),
        ];
        sigv4::sign(
            "POST",
            &url,
            &mut headers,
            &sigv4::hex_sha256(&body),
            &self.region,
            "kms",
            &creds,
            &sigv4::amz_date(now()),
        );
        let mut req = http_client("aws")?.post(url).body(body);
        for (k, v) in headers {
            // reqwest sends the same host itself.
            if k != "host" {
                req = req.header(k, v);
            }
        }
        send_json("aws", op, req).await
    }
}

impl KeyWrapper for AwsKms {
    fn wrap<'a>(&'a self, plain: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move {
            let body = json!({
                "KeyId": self.key_id,
                "Plaintext": BASE64.encode(plain),
                "EncryptionContext": {"reins": AWS_CONTEXT},
            });
            let v = self.call("Encrypt", &body).await?;
            Ok(b64_field(&v, "CiphertextBlob", "aws", "Encrypt")?.to_vec())
        })
    }

    fn unwrap<'a>(&'a self, wrapped: &'a [u8]) -> BoxFuture<'a, Result<Zeroizing<Vec<u8>>, String>> {
        Box::pin(async move {
            let body = json!({
                "CiphertextBlob": BASE64.encode(wrapped),
                "KeyId": self.key_id,
                "EncryptionContext": {"reins": AWS_CONTEXT},
            });
            let v = self.call("Decrypt", &body).await?;
            b64_field(&v, "Plaintext", "aws", "Decrypt")
        })
    }

    fn id(&self) -> String {
        self.key_id.clone()
    }
}

// ----------------------------------------------------------------------------------------------------- GCP Cloud KMS

/// The fields of a service-account key file a token needs.
struct GcpServiceAccount {
    client_email: String,
    private_key: Zeroizing<String>,
    token_uri: String,
}

enum GcpAuth {
    ServiceAccount(GcpServiceAccount),
    /// The metadata server's origin.
    Metadata(String),
}

/// GCP Cloud KMS (`:encrypt` / `:decrypt`), keyed by `projects/…/locations/…/keyRings/…/cryptoKeys/…`.
pub struct GcpKms {
    key_name: String,
    /// The KMS API's origin (`https://cloudkms.googleapis.com`).
    endpoint: String,
    auth: GcpAuth,
}

fn gcp_key_name_ok(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() == 8
        && [(0, "projects"), (2, "locations"), (4, "keyRings"), (6, "cryptoKeys")].iter().all(|(i, s)| parts[*i] == *s)
        && [1, 3, 5, 7].iter().all(|i| {
            !parts[*i].is_empty() && parts[*i].bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
}

fn gcp_service_account(json: &str) -> Result<GcpServiceAccount, String> {
    let v: Value = serde_json::from_str(json).map_err(|_| "gcp KMS: the service-account key file is not JSON")?;
    let get = |n: &str| v.get(n).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
    let (Some(client_email), Some(private_key)) = (get("client_email"), get("private_key")) else {
        return Err("gcp KMS: the service-account key file lacks client_email or private_key".to_owned());
    };
    let token_uri = get("token_uri").unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_owned());
    if !token_uri.starts_with("https://") {
        return Err("gcp KMS: the service-account token_uri must be https".to_owned());
    }
    Ok(GcpServiceAccount {
        client_email,
        private_key: Zeroizing::new(private_key),
        token_uri,
    })
}

#[derive(Serialize)]
struct GcpAssertion<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// The RFC 7523 JWT-bearer assertion of `sa` for Cloud KMS.
fn gcp_assertion(sa: &GcpServiceAccount, now: i64) -> Result<String, String> {
    // Google ships PKCS#8 keys; re-encoded as PKCS#1, the form `EncodingKey::from_rsa_pem` accepts (as in `fcm`).
    let bad = || "gcp KMS: the service-account private key is not valid".to_owned();
    let rsa = openssl::rsa::Rsa::private_key_from_pem(sa.private_key.as_bytes()).map_err(|_| bad())?;
    let pkcs1 = Zeroizing::new(rsa.private_key_to_pem().map_err(|_| bad())?);
    let key = EncodingKey::from_rsa_pem(&pkcs1).map_err(|_| bad())?;
    let claims = GcpAssertion {
        iss: &sa.client_email,
        scope: GCP_SCOPE,
        aud: &sa.token_uri,
        iat: now,
        exp: now + 3600,
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|_| bad())
}

impl GcpKms {
    /// For `key_name`, with the service account at `GOOGLE_APPLICATION_CREDENTIALS` (from `get`) or the metadata
    /// server.
    pub fn new(key_name: &str, get: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        if !gcp_key_name_ok(key_name) {
            return Err("REINS_ORG_KMS_KEY_ID must be a GCP key name (projects/…/locations/…/keyRings/…/cryptoKeys/…)"
                .to_owned());
        }
        let token_override = api_base("", TOKEN_ENDPOINT_ENV);
        let auth = match get("GOOGLE_APPLICATION_CREDENTIALS") {
            Some(path) => {
                let json = Zeroizing::new(
                    std::fs::read_to_string(path.trim())
                        .map_err(|_| "gcp KMS: GOOGLE_APPLICATION_CREDENTIALS cannot be read".to_owned())?,
                );
                let mut sa = gcp_service_account(&json)?;
                if !token_override.is_empty() {
                    sa.token_uri = format!("{token_override}/token");
                }
                GcpAuth::ServiceAccount(sa)
            }
            None => GcpAuth::Metadata(api_base(GCP_METADATA, TOKEN_ENDPOINT_ENV)),
        };
        Ok(Self {
            key_name: key_name.to_owned(),
            endpoint: api_base("https://cloudkms.googleapis.com", ENDPOINT_ENV),
            auth,
        })
    }

    async fn token(&self) -> Result<Zeroizing<String>, String> {
        let http = http_client("gcp")?;
        let v = match &self.auth {
            GcpAuth::ServiceAccount(sa) => {
                let assertion = Zeroizing::new(gcp_assertion(sa, now())?);
                send_json(
                    "gcp",
                    "the token",
                    http.post(&sa.token_uri).form(&[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                        ("assertion", assertion.as_str()),
                    ]),
                )
                .await?
            }
            GcpAuth::Metadata(base) => {
                send_json(
                    "gcp",
                    "the metadata token",
                    http.get(format!("{base}/computeMetadata/v1/instance/service-accounts/default/token"))
                        .header("Metadata-Flavor", "Google"),
                )
                .await?
            }
        };
        Ok(Zeroizing::new(field(&v, "access_token", "gcp", "the token")?.to_owned()))
    }

    async fn call(&self, op: &str, body: &Value) -> Result<Value, String> {
        let token = self.token().await?;
        let req = http_client("gcp")?
            .post(format!("{}/v1/{}:{op}", self.endpoint, self.key_name))
            .bearer_auth(token.as_str())
            .json(body);
        send_json("gcp", op, req).await
    }
}

impl KeyWrapper for GcpKms {
    fn wrap<'a>(&'a self, plain: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move {
            let body = json!({
                "plaintext": BASE64.encode(plain),
                "additionalAuthenticatedData": BASE64.encode(GCP_AAD),
            });
            let v = self.call("encrypt", &body).await?;
            Ok(b64_field(&v, "ciphertext", "gcp", "encrypt")?.to_vec())
        })
    }

    fn unwrap<'a>(&'a self, wrapped: &'a [u8]) -> BoxFuture<'a, Result<Zeroizing<Vec<u8>>, String>> {
        Box::pin(async move {
            let body = json!({
                "ciphertext": BASE64.encode(wrapped),
                "additionalAuthenticatedData": BASE64.encode(GCP_AAD),
            });
            let v = self.call("decrypt", &body).await?;
            b64_field(&v, "plaintext", "gcp", "decrypt")
        })
    }

    fn id(&self) -> String {
        self.key_name.clone()
    }
}

// --------------------------------------------------------------------------------------------------- Azure Key Vault

enum AzureAuth {
    /// A service principal's client credentials, at the login service's origin.
    ClientSecret {
        base: String,
        tenant: String,
        client_id: String,
        secret: Zeroizing<String>,
    },
    /// The managed identity (a user-assigned one with `client_id`), at IMDS's origin.
    ManagedIdentity {
        base: String,
        client_id: Option<String>,
    },
}

/// Azure Key Vault (`wrapkey` / `unwrapkey`, `RSA-OAEP-256`), keyed by `https://<vault>.vault.azure.net/keys/<name>/<version>`.
pub struct AzureKeyVault {
    key_url: String,
    /// Where the calls go: `key_url`, or (tests) the same path on the test endpoint.
    call_url: String,
    auth: AzureAuth,
}

/// The path (`/keys/<name>/<version>`) of a valid key URL.
fn azure_key_path(key_url: &str) -> Result<String, String> {
    let bad = || {
        "REINS_ORG_KMS_KEY_ID must be an Azure key URL (https://<vault>.vault.azure.net/keys/<name>/<version>)"
            .to_owned()
    };
    let url = Url::parse(key_url).map_err(|_| bad())?;
    if url.scheme() != "https" || url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        return Err(bad());
    }
    let segs: Vec<&str> = url.path().trim_end_matches('/').split('/').skip(1).collect();
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    match segs.as_slice() {
        ["keys", name, version] if ok(name) && ok(version) => Ok(format!("/keys/{name}/{version}")),
        _ => Err(bad()),
    }
}

impl AzureKeyVault {
    /// For `key_url`, with a service principal from `get` (the environment) or the managed identity.
    pub fn new(key_url: &str, get: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let path = azure_key_path(key_url)?;
        let key_url = key_url.trim_end_matches('/').to_owned();
        let test = api_base("", ENDPOINT_ENV);
        let call_url = if test.is_empty() {
            key_url.clone()
        } else {
            format!("{test}{path}")
        };
        let auth = match (get("AZURE_TENANT_ID"), get("AZURE_CLIENT_ID"), get("AZURE_CLIENT_SECRET")) {
            (Some(tenant), Some(client_id), Some(secret)) => {
                if !tenant.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
                    return Err("azure KMS: AZURE_TENANT_ID is not valid".to_owned());
                }
                AzureAuth::ClientSecret {
                    base: api_base("https://login.microsoftonline.com", TOKEN_ENDPOINT_ENV),
                    tenant,
                    client_id,
                    secret: Zeroizing::new(secret),
                }
            }
            (_, client_id, _) => AzureAuth::ManagedIdentity {
                base: api_base(LINK_LOCAL_METADATA, TOKEN_ENDPOINT_ENV),
                client_id,
            },
        };
        Ok(Self {
            key_url,
            call_url,
            auth,
        })
    }

    async fn token(&self) -> Result<Zeroizing<String>, String> {
        let http = http_client("azure")?;
        let v = match &self.auth {
            AzureAuth::ClientSecret {
                base,
                tenant,
                client_id,
                secret,
            } => {
                send_json(
                    "azure",
                    "the token",
                    http.post(format!("{base}/{tenant}/oauth2/v2.0/token")).form(&[
                        ("grant_type", "client_credentials"),
                        ("client_id", client_id.as_str()),
                        ("client_secret", secret.as_str()),
                        ("scope", "https://vault.azure.net/.default"),
                    ]),
                )
                .await?
            }
            AzureAuth::ManagedIdentity {
                base,
                client_id,
            } => {
                let mut url = Url::parse(&format!("{base}/metadata/identity/oauth2/token"))
                    .map_err(|_| "azure KMS: a bad metadata endpoint".to_owned())?;
                url.query_pairs_mut().append_pair("api-version", "2018-02-01").append_pair("resource", AZURE_RESOURCE);
                if let Some(id) = client_id {
                    url.query_pairs_mut().append_pair("client_id", id);
                }
                send_json("azure", "the managed identity token", http.get(url).header("Metadata", "true")).await?
            }
        };
        Ok(Zeroizing::new(field(&v, "access_token", "azure", "the token")?.to_owned()))
    }

    async fn call(&self, op: &str, value: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        let token = self.token().await?;
        let body = json!({"alg": "RSA-OAEP-256", "value": BASE64URL_NOPAD.encode(value)});
        let req = http_client("azure")?
            .post(format!("{}/{op}?api-version=7.4", self.call_url))
            .bearer_auth(token.as_str())
            .json(&body);
        let v = send_json("azure", op, req).await?;
        b64url_decode(field(&v, "value", "azure", op)?)
            .map(Zeroizing::new)
            .ok_or_else(|| format!("azure KMS: {op} answered a value that is not base64url"))
    }
}

impl KeyWrapper for AzureKeyVault {
    fn wrap<'a>(&'a self, plain: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move { Ok(self.call("wrapkey", plain).await?.to_vec()) })
    }

    fn unwrap<'a>(&'a self, wrapped: &'a [u8]) -> BoxFuture<'a, Result<Zeroizing<Vec<u8>>, String>> {
        Box::pin(self.call("unwrapkey", wrapped))
    }

    fn id(&self) -> String {
        self.key_url.clone()
    }
}

// ------------------------------------------------------------------------------------------------------------- init

/// The wrapper for `provider` and its key, with credentials from the environment.
pub fn wrapper(provider: Provider, key_id: &str) -> Result<Box<dyn KeyWrapper>, String> {
    let key_id = key_id.trim();
    if key_id.is_empty() {
        return Err(format!("REINS_ORG_KMS_PROVIDER is {} but REINS_ORG_KMS_KEY_ID is empty", provider.as_str()));
    }
    Ok(match provider {
        Provider::Aws => Box::new(AwsKms::new(key_id, &env_var)?),
        Provider::Gcp => Box::new(GcpKms::new(key_id, &env_var)?),
        Provider::Azure => Box::new(AzureKeyVault::new(key_id, &env_var)?),
    })
}

/// Unlocks the org master key with the configured KMS and installs it. Nothing to do for `local`. On an error the
/// caller must not start the org features (`main` exits).
pub async fn init() -> Result<(), String> {
    let Some(provider) = Provider::parse(&CONFIG.reins_org_kms_provider())? else {
        return Ok(());
    };
    let wrapper = wrapper(provider, &CONFIG.reins_org_kms_key_id())?;
    let kms = init_with(provider, wrapper.as_ref(), Path::new(&CONFIG.reins_org_kms_key_file())).await?;
    kms::install_kms(Arc::new(kms));
    Ok(())
}

/// The wrapped bytes of a key file's text, which must be `reins-wrapped-v1:<provider>:<base64>`.
fn parse_key_file(provider: Provider, path: &Path, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let p = provider.as_str();
    let not_wrapped = || {
        format!(
            "{} is not a master key wrapped by {p} (a local key file can't be used with a cloud KMS)",
            path.display()
        )
    };
    let text = std::str::from_utf8(bytes).map_err(|_| not_wrapped())?.trim();
    let rest = text.strip_prefix(FILE_PREFIX).and_then(|r| r.strip_prefix(':')).ok_or_else(not_wrapped)?;
    let (by, b64) = rest.split_once(':').ok_or_else(not_wrapped)?;
    if by != p {
        return Err(format!("{} was wrapped by {by}, not {p}", path.display()));
    }
    BASE64.decode(b64.as_bytes()).map_err(|_| not_wrapped())
}

/// Opens (or, with no file, creates) the master key wrapped by `wrapper` at `path`.
pub async fn init_with(provider: Provider, wrapper: &dyn KeyWrapper, path: &Path) -> Result<LocalKms, String> {
    let p = provider.as_str();
    let mut master = Zeroizing::new([0u8; 32]);
    match std::fs::read(path) {
        Ok(bytes) => {
            let blob = parse_key_file(provider, path, &bytes)?;
            let plain = wrapper.unwrap(&blob).await?;
            if plain.len() != master.len() {
                return Err(format!("{p} KMS: the unwrapped master key is not 32 bytes"));
            }
            master.copy_from_slice(&plain);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            SystemRandom::new().fill(master.as_mut()).map_err(|_| "no randomness".to_owned())?;
            let blob = wrapper.wrap(master.as_ref()).await?;
            // A key the KMS wraps but won't unwrap (a policy without Decrypt) would lock every org out next launch.
            let back = wrapper.unwrap(&blob).await?;
            if back.as_slice() != master.as_ref() {
                return Err(format!("{p} KMS: the wrapped master key does not unwrap to itself"));
            }
            write_new(path, &format!("{FILE_PREFIX}:{p}:{}\n", BASE64.encode(&blob)))?;
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let id = format!("{p}:{}", &sigv4::hex_sha256(wrapper.id().as_bytes())[..8]);
    Ok(LocalKms::with_id(*master, id))
}

/// Writes a new file (never over an existing one), 0600; removed again if the write fails.
fn write_new(path: &Path, text: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut f = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Err(e) = f.write_all(text.as_bytes()).and_then(|()| f.sync_all()) {
        drop(f);
        std::fs::remove_file(path).ok();
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        RwLock,
        atomic::{AtomicUsize, Ordering},
    };

    use openssl::{pkey::PKey, rsa::Rsa};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, body_string_contains, header, method, path, query_param},
    };

    use super::{super::kms::Kms, *};

    const ARN: &str = "arn:aws:kms:us-east-2:111122223333:key/1234abcd-12ab-34cd-56ef-1234567890ab";

    async fn mock() -> MockServer {
        rustls::crypto::ring::default_provider().install_default().ok();
        MockServer::start().await
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reins-kms-byok-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn header_of(r: &wiremock::Request, name: &str) -> String {
        r.headers.get(name).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default()
    }

    fn aws(server: &MockServer, creds: Option<AwsCreds>) -> AwsKms {
        AwsKms {
            region: "us-east-2".into(),
            key_id: ARN.into(),
            endpoint: server.uri(),
            imds: server.uri(),
            creds,
        }
    }

    fn static_creds() -> AwsCreds {
        AwsCreds {
            access_key_id: "AKIDTEST".into(),
            secret_access_key: Zeroizing::new("secret".into()),
            session_token: Some("session".into()),
        }
    }

    async fn mock_aws(server: &MockServer, plain: &[u8], cipher: &[u8]) {
        let context = json!({"reins": "org-kms-master/v1"});
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("x-amz-target", "TrentService.Encrypt"))
            .and(header("content-type", "application/x-amz-json-1.1"))
            .and(body_partial_json(
                json!({"KeyId": ARN, "Plaintext": BASE64.encode(plain), "EncryptionContext": context}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"CiphertextBlob": BASE64.encode(cipher), "KeyId": ARN})),
            )
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("x-amz-target", "TrentService.Decrypt"))
            .and(body_partial_json(
                json!({"KeyId": ARN, "CiphertextBlob": BASE64.encode(cipher), "EncryptionContext": context}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"Plaintext": BASE64.encode(plain), "KeyId": ARN})),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn aws_wraps_and_unwraps_with_sigv4() {
        let server = mock().await;
        let plain = [9u8; 32];
        mock_aws(&server, &plain, b"aws-cipher").await;
        let kms = aws(&server, Some(static_creds()));
        let wrapped = kms.wrap(&plain).await.unwrap();
        assert_eq!(wrapped, b"aws-cipher");
        assert_eq!(&*kms.unwrap(&wrapped).await.unwrap(), &plain);
        assert_eq!(kms.id(), ARN);
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
        for r in &reqs {
            let auth = header_of(r, "authorization");
            assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDTEST/"), "{auth}");
            assert!(auth.contains("/us-east-2/kms/aws4_request"), "{auth}");
            assert!(
                auth.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token;x-amz-target,"),
                "{auth}"
            );
            assert_eq!(header_of(r, "x-amz-security-token"), "session");
        }
        assert_eq!(header_of(&reqs[0], "x-amz-target"), "TrentService.Encrypt");
        assert_eq!(header_of(&reqs[1], "x-amz-target"), "TrentService.Decrypt");

        // A refusal names the provider, the step and the status.
        let refusing = mock().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(400)).mount(&refusing).await;
        assert_eq!(
            aws(&refusing, Some(static_creds())).unwrap(b"x").await.unwrap_err(),
            "aws KMS: Decrypt answered HTTP 400"
        );
    }

    #[tokio::test]
    async fn aws_takes_role_credentials_from_imdsv2() {
        let server = mock().await;
        Mock::given(method("PUT"))
            .and(path("/latest/api/token"))
            .and(header("X-aws-ec2-metadata-token-ttl-seconds", "21600"))
            .respond_with(ResponseTemplate::new(200).set_body_string("imds-token"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/latest/meta-data/iam/security-credentials/"))
            .and(header("X-aws-ec2-metadata-token", "imds-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("reins-role\n"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/latest/meta-data/iam/security-credentials/reins-role"))
            .and(header("X-aws-ec2-metadata-token", "imds-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Code": "Success", "AccessKeyId": "ASIAROLE", "SecretAccessKey": "role-secret", "Token": "role-session"
            })))
            .mount(&server)
            .await;
        mock_aws(&server, &[1u8; 32], b"c").await;
        let kms = aws(&server, None);
        assert_eq!(kms.wrap(&[1u8; 32]).await.unwrap(), b"c");
        let reqs = server.received_requests().await.unwrap();
        let encrypt = reqs.iter().find(|r| header_of(r, "x-amz-target") == "TrentService.Encrypt").unwrap();
        assert!(header_of(encrypt, "authorization").starts_with("AWS4-HMAC-SHA256 Credential=ASIAROLE/"));
        assert_eq!(header_of(encrypt, "x-amz-security-token"), "role-session");
    }

    #[test]
    fn aws_reads_its_region_and_credentials() {
        assert_eq!(aws_region(ARN).unwrap(), ("us-east-2".to_owned(), "amazonaws.com"));
        assert_eq!(aws_region("arn:aws-cn:kms:cn-north-1:1:alias/reins").unwrap().1, "amazonaws.com.cn");
        assert!(aws_region("1234abcd-12ab").is_err());
        assert!(aws_region("arn:aws:kms:evil.example/x:1:key/k").is_err());
        assert!(aws_region("arn:aws:s3:us-east-1:1:key/k").is_err());
        let env = |n: &str| match n {
            "AWS_ACCESS_KEY_ID" => Some("AKID".to_owned()),
            "AWS_SECRET_ACCESS_KEY" => Some("s".to_owned()),
            _ => None,
        };
        let c = aws_env_creds(&env).unwrap();
        assert_eq!((c.access_key_id.as_str(), c.session_token), ("AKID", None));
        assert!(aws_env_creds(&|n: &str| (n == "AWS_ACCESS_KEY_ID").then(|| "AKID".to_owned())).is_none());
        // Credentials never show in Debug.
        assert!(!format!("{:?}", static_creds()).contains("secret"));
    }

    #[tokio::test]
    async fn gcp_uses_the_token_and_aad() {
        let server = mock().await;
        let pkey = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let private_pem = String::from_utf8(pkey.private_key_to_pem_pkcs8().unwrap()).unwrap();
        let sa = gcp_service_account(
            &json!({"type": "service_account", "client_email": "kms@p.iam.gserviceaccount.com", "private_key": private_pem,
                    "token_uri": "https://oauth2.googleapis.com/token"})
            .to_string(),
        )
        .unwrap();
        let sa = GcpServiceAccount {
            token_uri: format!("{}/token", server.uri()),
            ..sa
        };
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.test", "expires_in": 3599})),
            )
            .mount(&server)
            .await;
        let key = "projects/p/locations/global/keyRings/reins/cryptoKeys/master";
        let aad = BASE64.encode(b"reins/org-kms-master/v1");
        let plain = [4u8; 32];
        Mock::given(method("POST"))
            .and(path(format!("/v1/{key}:encrypt")))
            .and(header("authorization", "Bearer ya29.test"))
            .and(body_partial_json(json!({"plaintext": BASE64.encode(&plain), "additionalAuthenticatedData": aad})))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"name": key, "ciphertext": BASE64.encode(b"gcp-cipher")})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/{key}:decrypt")))
            .and(header("authorization", "Bearer ya29.test"))
            .and(body_partial_json(
                json!({"ciphertext": BASE64.encode(b"gcp-cipher"), "additionalAuthenticatedData": aad}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"plaintext": BASE64.encode(&plain)})))
            .mount(&server)
            .await;
        let kms = GcpKms {
            key_name: key.into(),
            endpoint: server.uri(),
            auth: GcpAuth::ServiceAccount(sa),
        };
        let wrapped = kms.wrap(&plain).await.unwrap();
        assert_eq!(wrapped, b"gcp-cipher");
        assert_eq!(&*kms.unwrap(&wrapped).await.unwrap(), &plain);
        // The assertion: RS256, for Cloud KMS, from the service account.
        let reqs = server.received_requests().await.unwrap();
        let form = String::from_utf8(reqs.iter().find(|r| r.url.path() == "/token").unwrap().body.clone()).unwrap();
        let jwt = form.split('&').find_map(|kv| kv.strip_prefix("assertion=")).unwrap();
        let mut parts = jwt.split('.');
        let head: Value = serde_json::from_slice(&b64url_decode(parts.next().unwrap()).unwrap()).unwrap();
        let claims: Value = serde_json::from_slice(&b64url_decode(parts.next().unwrap()).unwrap()).unwrap();
        assert_eq!(head["alg"], "RS256");
        assert_eq!(claims["scope"], GCP_SCOPE);
        assert_eq!(claims["iss"], "kms@p.iam.gserviceaccount.com");

        // The metadata server, when there is no key file.
        let meta = mock().await;
        Mock::given(method("GET"))
            .and(path("/computeMetadata/v1/instance/service-accounts/default/token"))
            .and(header("Metadata-Flavor", "Google"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "ya29.meta"})))
            .mount(&meta)
            .await;
        let kms = GcpKms {
            key_name: key.into(),
            endpoint: meta.uri(),
            auth: GcpAuth::Metadata(meta.uri()),
        };
        assert_eq!(kms.token().await.unwrap().as_str(), "ya29.meta");

        assert!(gcp_key_name_ok(key));
        assert!(!gcp_key_name_ok("projects/p/locations/global/keyRings/r"));
        assert!(!gcp_key_name_ok("projects/p/locations/../keyRings/r/cryptoKeys/k?x"));
        assert!(GcpKms::new("not-a-key", &|_: &str| None).is_err());
        assert!(
            gcp_service_account(&json!({"client_email": "e", "private_key": "k", "token_uri": "http://x"}).to_string())
                .is_err()
        );
    }

    #[tokio::test]
    async fn azure_wraps_with_rsa_oaep_256() {
        let server = mock().await;
        Mock::given(method("POST"))
            .and(path("/tenant-1/oauth2/v2.0/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("client_id=app-1"))
            .and(body_string_contains("scope=https%3A%2F%2Fvault.azure.net%2F.default"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"access_token": "az.test", "token_type": "Bearer"})),
            )
            .mount(&server)
            .await;
        let plain = [5u8; 32];
        for (op, input, output) in
            [("wrapkey", &plain[..], &b"azure-cipher"[..]), ("unwrapkey", b"azure-cipher", &plain[..])]
        {
            Mock::given(method("POST"))
                .and(path(format!("/keys/master/0123abcd/{op}")))
                .and(query_param("api-version", "7.4"))
                .and(header("authorization", "Bearer az.test"))
                .and(body_partial_json(json!({"alg": "RSA-OAEP-256", "value": BASE64URL_NOPAD.encode(input)})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "kid": "https://reins.vault.azure.net/keys/master/0123abcd", "value": BASE64URL_NOPAD.encode(output)
                })))
                .mount(&server)
                .await;
        }
        let key_url = "https://reins.vault.azure.net/keys/master/0123abcd";
        let kms = AzureKeyVault {
            key_url: key_url.into(),
            call_url: format!("{}/keys/master/0123abcd", server.uri()),
            auth: AzureAuth::ClientSecret {
                base: server.uri(),
                tenant: "tenant-1".into(),
                client_id: "app-1".into(),
                secret: Zeroizing::new("s3cret".into()),
            },
        };
        let wrapped = kms.wrap(&plain).await.unwrap();
        assert_eq!(wrapped, b"azure-cipher");
        assert_eq!(&*kms.unwrap(&wrapped).await.unwrap(), &plain);
        assert_eq!(kms.id(), key_url);

        // The managed identity.
        let mi = mock().await;
        Mock::given(method("GET"))
            .and(path("/metadata/identity/oauth2/token"))
            .and(query_param("resource", "https://vault.azure.net"))
            .and(query_param("api-version", "2018-02-01"))
            .and(header("Metadata", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "az.mi"})))
            .mount(&mi)
            .await;
        let kms = AzureKeyVault {
            key_url: key_url.into(),
            call_url: key_url.into(),
            auth: AzureAuth::ManagedIdentity {
                base: mi.uri(),
                client_id: None,
            },
        };
        assert_eq!(kms.token().await.unwrap().as_str(), "az.mi");

        assert_eq!(azure_key_path(key_url).unwrap(), "/keys/master/0123abcd");
        assert!(azure_key_path("http://reins.vault.azure.net/keys/master/0123abcd").is_err());
        assert!(azure_key_path("https://reins.vault.azure.net/keys/master").is_err());
        assert!(azure_key_path("https://reins.vault.azure.net/secrets/master/1").is_err());
    }

    /// An in-memory wrapper that counts its calls.
    #[derive(Default)]
    struct Fake {
        wraps: AtomicUsize,
        unwraps: AtomicUsize,
        fail_unwrap: bool,
        short: bool,
    }

    impl KeyWrapper for Fake {
        fn wrap<'a>(&'a self, plain: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>, String>> {
            self.wraps.fetch_add(1, Ordering::SeqCst);
            Box::pin(
                async move { Ok([b"fake".as_slice(), &plain.iter().map(|b| b ^ 0x5a).collect::<Vec<_>>()].concat()) },
            )
        }

        fn unwrap<'a>(&'a self, wrapped: &'a [u8]) -> BoxFuture<'a, Result<Zeroizing<Vec<u8>>, String>> {
            self.unwraps.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if self.fail_unwrap {
                    return Err("aws KMS: Decrypt answered HTTP 403".to_owned());
                }
                let body = wrapped.strip_prefix(b"fake").ok_or("not fake")?;
                let mut out: Vec<u8> = body.iter().map(|b| b ^ 0x5a).collect();
                if self.short {
                    out.pop();
                }
                Ok(Zeroizing::new(out))
            })
        }

        fn id(&self) -> String {
            ARN.to_owned()
        }
    }

    #[tokio::test]
    async fn init_creates_then_reopens_the_same_master() {
        let dir = test_dir("reopen");
        let path = dir.join("master.key");
        let fake = Fake::default();
        let first = init_with(Provider::Aws, &fake, &path).await.unwrap();
        assert_eq!((fake.wraps.load(Ordering::SeqCst), fake.unwraps.load(Ordering::SeqCst)), (1, 1), "checked");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("reins-wrapped-v1:aws:"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let id = first.id();
        assert_eq!(id, format!("aws:{}", &sigv4::hex_sha256(ARN.as_bytes())[..8]));
        let sealed = first.seal(b"aad", b"org key").unwrap();

        let second = init_with(Provider::Aws, &fake, &path).await.unwrap();
        assert_eq!(fake.wraps.load(Ordering::SeqCst), 1, "not made again");
        assert_eq!(second.id(), id);
        assert_eq!(&*second.open(b"aad", &sealed).unwrap(), b"org key");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "the file is unchanged");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_failed_unwrap_never_falls_back_to_a_local_key() {
        let dir = test_dir("failed");
        let path = dir.join("master.key");
        init_with(Provider::Aws, &Fake::default(), &path).await.unwrap();
        let before = std::fs::read(&path).unwrap();

        // The KMS refuses: an error naming the provider and the status, the file untouched.
        let server = mock().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
        let err = init_with(Provider::Aws, &aws(&server, Some(static_creds())), &path).await.err().unwrap();
        assert_eq!(err, "aws KMS: Decrypt answered HTTP 403");
        let failing = Fake {
            fail_unwrap: true,
            ..Fake::default()
        };
        assert!(init_with(Provider::Aws, &failing, &path).await.is_err());
        assert_eq!(failing.wraps.load(Ordering::SeqCst), 0, "no new master");
        let short = Fake {
            short: true,
            ..Fake::default()
        };
        assert!(init_with(Provider::Aws, &short, &path).await.err().unwrap().contains("not 32 bytes"));
        assert_eq!(std::fs::read(&path).unwrap(), before);

        // Nothing installed: kms() refuses, with the file present or not, and creates no local key.
        let slot = RwLock::new(None);
        let missing = dir.join("missing.key");
        for p in [&path, &missing] {
            let Err(super::super::OrgError::Internal(e)) = kms::resolve(&slot, "aws", p.to_str().unwrap()) else {
                panic!("kms() must fail closed");
            };
            assert_eq!(e, "the org KMS is not unlocked");
        }
        assert!(!missing.exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);

        // No file and a KMS that fails to wrap (or to unwrap what it wrapped): nothing is written.
        let refusing = Fake {
            fail_unwrap: true,
            ..Fake::default()
        };
        assert!(init_with(Provider::Aws, &refusing, &missing).await.is_err());
        assert!(init_with(Provider::Aws, &aws(&server, Some(static_creds())), &missing).await.is_err());
        assert!(!missing.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_raw_local_key_file_is_refused_for_a_cloud_provider() {
        let dir = test_dir("raw");
        let fake = Fake::default();
        // A local key (binary, or printable bytes).
        for (name, raw) in [("bin.key", vec![0xffu8; 32]), ("text.key", vec![b'a'; 32])] {
            let path = dir.join(name);
            std::fs::write(&path, &raw).unwrap();
            let err = init_with(Provider::Gcp, &fake, &path).await.err().unwrap();
            assert!(err.contains("is not a master key wrapped by gcp"), "{err}");
            assert_eq!(std::fs::read(&path).unwrap(), raw);
        }
        // Wrapped by another provider.
        let path = dir.join("aws.key");
        init_with(Provider::Aws, &fake, &path).await.unwrap();
        let err = init_with(Provider::Azure, &fake, &path).await.err().unwrap();
        assert!(err.ends_with("was wrapped by aws, not azure"), "{err}");
        assert_eq!((fake.wraps.load(Ordering::SeqCst), fake.unwraps.load(Ordering::SeqCst)), (1, 1), "never asked");

        assert_eq!(Provider::parse(" AWS ").unwrap(), Some(Provider::Aws));
        assert_eq!(Provider::parse("local").unwrap(), None);
        assert!(Provider::parse("vault").is_err());
        assert!(wrapper(Provider::Aws, " ").err().unwrap().contains("REINS_ORG_KMS_KEY_ID is empty"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
