//! Cart mandates: what the user approved, signed on the phone. A compact JWS (RFC 7515) with `EdDSA` (Ed25519, RFC
//! 8037) whose header carries the public key as a JWK, so that anyone can check it, and its RFC 7638 thumbprint as
//! `kid`, which the Payments screen shows. The key is made once per account and kept with its secrets.

use data_encoding::BASE64URL_NOPAD;
use reins_proto::payments::{CartMandate, MANDATE_JWS_TYPE, MANDATE_TYPE};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::CoreError;
use crate::store::Store;

/// Where the signing key's seed is kept (service, account).
const KEY: (&str, &str) = ("payments.mandate-key", "this");
/// How long a mandate is valid after it was signed.
pub const MANDATE_SECS: i64 = 3_600;

fn keypair(store: &Store) -> Result<Ed25519KeyPair, CoreError> {
    let (service, account) = KEY;
    let seed = match store.secret_get(service, account)? {
        Some(seed) if seed.len() == 32 => Zeroizing::new(seed),
        _ => {
            let fresh = Zeroizing::new(crate::crypto::random_bytes::<32>()?.to_vec());
            store.secret_put(service, account, &fresh)?;
            fresh
        }
    };
    Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| CoreError::storage("the mandate key cannot be used"))
}

fn jwk(public: &[u8]) -> Value {
    json!({"kty": "OKP", "crv": "Ed25519", "x": BASE64URL_NOPAD.encode(public)})
}

/// The RFC 7638 thumbprint of an Ed25519 public key: SHA-256 of its JWK's required members in order.
#[must_use]
pub fn thumbprint(public: &[u8]) -> String {
    let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, BASE64URL_NOPAD.encode(public));
    BASE64URL_NOPAD.encode(ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes()).as_ref())
}

/// The account's mandate key, as its thumbprint (made on first use).
pub fn key_thumbprint(store: &Store) -> Result<String, CoreError> {
    Ok(thumbprint(keypair(store)?.public_key().as_ref()))
}

/// Signs `mandate`.
pub fn sign(store: &Store, mandate: &CartMandate) -> Result<String, CoreError> {
    let pair = keypair(store)?;
    let public = pair.public_key().as_ref();
    let header = json!({"alg": "EdDSA", "typ": MANDATE_JWS_TYPE, "kid": thumbprint(public), "jwk": jwk(public)});
    let encode = |v: &[u8]| BASE64URL_NOPAD.encode(v);
    let payload = serde_json::to_vec(mandate).map_err(|_| CoreError::storage("cannot encode the mandate"))?;
    let signing_input = format!("{}.{}", encode(header.to_string().as_bytes()), encode(&payload));
    let signature = pair.sign(signing_input.as_bytes());
    Ok(format!("{signing_input}.{}", encode(signature.as_ref())))
}

/// Checks a mandate's signature against the key in its own header and returns it with that key's thumbprint. Whether
/// the key is the one you expect is up to you (compare the thumbprint).
pub fn verify(jws: &str) -> Result<(CartMandate, String), String> {
    let parts: Vec<&str> = jws.trim().split('.').collect();
    let [header, payload, signature] = parts.as_slice() else {
        return Err("not a compact JWS".to_owned());
    };
    let decode = |s: &str| BASE64URL_NOPAD.decode(s.as_bytes()).map_err(|_| "not base64url".to_owned());
    let head: Value = serde_json::from_slice(&decode(header)?).map_err(|_| "unreadable header".to_owned())?;
    if head["alg"] != "EdDSA" || head["jwk"]["kty"] != "OKP" || head["jwk"]["crv"] != "Ed25519" {
        return Err("not an Ed25519 signature".to_owned());
    }
    let public = decode(head["jwk"]["x"].as_str().unwrap_or_default())?;
    UnparsedPublicKey::new(&ED25519, &public)
        .verify(format!("{header}.{payload}").as_bytes(), &decode(signature)?)
        .map_err(|_| "the signature does not match".to_owned())?;
    let mandate: CartMandate =
        serde_json::from_slice(&decode(payload)?).map_err(|_| "unreadable payload".to_owned())?;
    if mandate.typ != MANDATE_TYPE {
        return Err("not a cart mandate".to_owned());
    }
    Ok((mandate, thumbprint(&public)))
}

#[cfg(test)]
mod tests {
    use reins_proto::payments::MandatePayment;

    use super::*;

    fn mandate() -> CartMandate {
        CartMandate {
            typ: MANDATE_TYPE.to_owned(),
            purchase_id: "p1".to_owned(),
            merchant: json!({"name": "Shop", "domain": "shop.example", "url": "https://shop.example/x"}),
            items: vec![json!({"name": "Mug", "quantity": 1, "unit_price": "8.00"})],
            amounts: json!({"total": "8.00"}),
            currency: "USD".to_owned(),
            ship_to: None,
            payment: MandatePayment {
                kind: "merchant_account".to_owned(),
                brand: None,
                last4: None,
            },
            agent: "Claude".to_owned(),
            approved_by: "you".to_owned(),
            iat: 1,
            exp: 2,
        }
    }

    #[test]
    fn a_mandate_verifies_with_the_key_in_its_header_and_tampering_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::tests::open(dir.path());
        let jws = sign(&store, &mandate()).unwrap();
        let (back, kid) = verify(&jws).unwrap();
        assert_eq!(back, mandate());
        assert_eq!(kid, key_thumbprint(&store).unwrap(), "the same key every time");
        assert_eq!(kid.len(), 43);

        let parts: Vec<&str> = jws.split('.').collect();
        let mut changed = mandate();
        changed.amounts = json!({"total": "800.00"});
        let forged_payload = BASE64URL_NOPAD.encode(&serde_json::to_vec(&changed).unwrap());
        assert!(verify(&format!("{}.{forged_payload}.{}", parts[0], parts[2])).is_err());
        assert!(verify("a.b").is_err());

        let other = tempfile::tempdir().unwrap();
        let other_store = crate::store::tests::open(other.path());
        let (_, other_kid) = verify(&sign(&other_store, &mandate()).unwrap()).unwrap();
        assert_ne!(kid, other_kid, "each account has its own key");
    }

    #[test]
    fn the_thumbprint_follows_rfc_7638() {
        // RFC 8037 appendix A.3: the thumbprint of its example Ed25519 key.
        let x = BASE64URL_NOPAD.decode(b"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo").unwrap();
        assert_eq!(thumbprint(&x), "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
    }
}
