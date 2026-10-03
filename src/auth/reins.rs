//! Rewarden MCP tokens (spec §4.5): RS256 access JWTs signed with Vaultwarden's RSA key,
//! issuer `{domain_origin}|mcp`, audience = canonical MCP URL; opaque refresh tokens that
//! are stored only as SHA-256 hex.

use std::sync::LazyLock;

use data_encoding::BASE64URL_NOPAD;
use jsonwebtoken::{DecodingKey, EncodingKey, Validation};

use super::{JWT_ALGORITHM, JWT_HEADER, PRIVATE_RSA_KEY, PUBLIC_RSA_KEY};
use crate::{
    CONFIG,
    api::rewarden::ACCESS_TOKEN_SECS,
    crypto::{encode_random_bytes, sha256_hex},
};

pub static JWT_MCP_ISSUER: LazyLock<String> = LazyLock::new(|| format!("{}|mcp", CONFIG.domain_origin()));

/// Clock skew tolerated on `exp`/`nbf` (same as `auth::decode_jwt`).
const LEEWAY_SECS: u64 = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpClaims {
    pub nbf: i64,
    pub exp: i64,
    pub iss: String,
    /// The canonical MCP URL this token is for (RFC 8707 audience).
    pub aud: String,
    /// User uuid.
    pub sub: String,
    /// Connection uuid; every MCP request checks it still exists.
    pub cid: String,
    pub client_id: String,
    pub scope: String,
}

pub fn mcp_claims(
    issuer: &str,
    audience: &str,
    user_uuid: &str,
    connection_uuid: &str,
    client_id: &str,
    now: i64,
) -> McpClaims {
    McpClaims {
        nbf: now,
        exp: now + ACCESS_TOKEN_SECS,
        iss: issuer.to_owned(),
        aud: audience.to_owned(),
        sub: user_uuid.to_owned(),
        cid: connection_uuid.to_owned(),
        client_id: client_id.to_owned(),
        scope: "mcp".to_owned(),
    }
}

pub fn encode_with(key: &EncodingKey, claims: &McpClaims) -> Result<String, String> {
    jsonwebtoken::encode(&JWT_HEADER, claims, key).map_err(|e| e.to_string())
}

pub fn decode_with(key: &DecodingKey, token: &str, issuer: &str, audience: &str) -> Result<McpClaims, String> {
    let mut validation = Validation::new(JWT_ALGORITHM);
    validation.leeway = LEEWAY_SECS;
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
    jsonwebtoken::decode::<McpClaims>(token.trim(), key, &validation).map(|d| d.claims).map_err(|e| e.to_string())
}

/// A 1 h access token for `connection_uuid`, audience `mcp_url`.
pub fn issue_access_token(user_uuid: &str, connection_uuid: &str, client_id: &str, mcp_url: &str) -> String {
    let claims =
        mcp_claims(&JWT_MCP_ISSUER, mcp_url, user_uuid, connection_uuid, client_id, chrono::Utc::now().timestamp());
    encode_with(PRIVATE_RSA_KEY.wait(), &claims).expect("signing with the server key cannot fail")
}

pub fn decode_access_token(token: &str, mcp_url: &str) -> Result<McpClaims, String> {
    decode_with(PUBLIC_RSA_KEY.wait(), token, &JWT_MCP_ISSUER, mcp_url)
}

/// 256-bit random opaque token (authorization codes, refresh tokens, session ids).
pub fn random_token() -> String {
    encode_random_bytes::<32>(&BASE64URL_NOPAD)
}

/// How opaque tokens are stored and looked up.
pub fn hash_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

#[cfg(test)]
mod tests {
    use openssl::rsa::Rsa;

    use super::*;

    const ISS: &str = "https://rw.example.com|mcp";
    const AUD: &str = "https://rw.example.com/mcp";

    fn keys() -> (EncodingKey, DecodingKey) {
        let rsa = Rsa::generate(2048).unwrap();
        (
            EncodingKey::from_rsa_pem(&rsa.private_key_to_pem().unwrap()).unwrap(),
            DecodingKey::from_rsa_pem(&rsa.public_key_to_pem().unwrap()).unwrap(),
        )
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    #[test]
    fn round_trips_with_issuer_and_audience() {
        let (enc, dec) = keys();
        let claims = mcp_claims(ISS, AUD, "user-1", "conn-1", "client-1", now());
        assert_eq!(claims.exp - claims.nbf, 3600);
        assert_eq!(claims.scope, "mcp");
        let token = encode_with(&enc, &claims).unwrap();
        assert_eq!(decode_with(&dec, &token, ISS, AUD).unwrap(), claims);
    }

    #[test]
    fn rejects_wrong_audience_issuer_key_and_tampering() {
        let (enc, dec) = keys();
        let token = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now())).unwrap();
        assert!(decode_with(&dec, &token, ISS, "https://other.example.com/mcp").is_err());
        assert!(decode_with(&dec, &token, "https://rw.example.com|login", AUD).is_err());
        let (_, other_dec) = keys();
        assert!(decode_with(&other_dec, &token, ISS, AUD).is_err());
        let mut tampered = token;
        tampered.insert(tampered.len() - 5, 'A');
        assert!(decode_with(&dec, &tampered, ISS, AUD).is_err());
        assert!(decode_with(&dec, "not-a-jwt", ISS, AUD).is_err());
    }

    #[test]
    fn rejects_expired_and_not_yet_valid_tokens() {
        let (enc, dec) = keys();
        let expired = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now() - 3600 - 31)).unwrap();
        assert!(decode_with(&dec, &expired, ISS, AUD).is_err());
        let future = encode_with(&enc, &mcp_claims(ISS, AUD, "u", "c", "k", now() + 120)).unwrap();
        assert!(decode_with(&dec, &future, ISS, AUD).is_err());
    }

    #[test]
    fn opaque_tokens_are_random_urlsafe_and_hashed() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
