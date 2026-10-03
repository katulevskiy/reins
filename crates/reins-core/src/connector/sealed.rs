//! Answers for the paired Rewarden desktop app, sealed to its key: the key and nonce arguments every desktop-only tool
//! carries, and the sealed box itself. The flow has already checked that the key is the one pinned at pairing.

use data_encoding::BASE64URL_NOPAD;
use rewarden_proto::connector::ConnectorCall;
use rewarden_proto::desktop;
use serde::Serialize;
use zeroize::Zeroizing;

use crate::CoreError;

fn bad(message: &str) -> CoreError {
    CoreError::service(message)
}

/// The desktop app's public key, from the call's `client_key`.
pub(crate) fn client_key(call: &ConnectorCall) -> Result<[u8; 32], CoreError> {
    call.str_arg("client_key")
        .and_then(desktop::decode_key)
        .ok_or_else(|| bad("`client_key` must be the desktop app's public key (32 bytes, base64url without padding)."))
}

/// The call's `nonce`, echoed inside the sealed answer.
pub(crate) fn nonce_arg(call: &ConnectorCall) -> Result<String, CoreError> {
    match call.str_arg("nonce") {
        Some(n) if !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_graphic()) => Ok(n.to_owned()),
        _ => Err(bad("`nonce` must be 1 to 64 visible ASCII characters.")),
    }
}

/// `value` as JSON in a sealed box to `key`, base64url without padding. The clear JSON is wiped afterwards.
pub(crate) fn seal<T: Serialize>(key: [u8; 32], value: &T) -> Result<String, CoreError> {
    let unsealable = || bad("The answer could not be sealed for the desktop app.");
    let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| unsealable())?);
    let sealed =
        crypto_box::PublicKey::from(key).seal(&mut crypto_box::aead::OsRng, &plain).map_err(|_| unsealable())?;
    Ok(BASE64URL_NOPAD.encode(&sealed))
}

#[cfg(test)]
mod tests {
    use rewarden_proto::desktop::AskAnswer;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_sealed_answer_opens_with_the_apps_secret_key_only() {
        let secret = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let answer = AskAnswer {
            v: 1,
            nonce: "n".to_owned(),
            approved: true,
        };
        let sealed = seal(*secret.public_key().as_bytes(), &answer).unwrap();
        let bytes = BASE64URL_NOPAD.decode(sealed.as_bytes()).unwrap();
        let opened: AskAnswer = serde_json::from_slice(&secret.unseal(&bytes).unwrap()).unwrap();
        assert_eq!(opened, answer);
        let other = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        assert!(other.unseal(&bytes).is_err());
    }

    #[test]
    fn keys_and_nonces_are_checked() {
        let call = |args: serde_json::Value| ConnectorCall {
            service: "desktop".to_owned(),
            op: "ask".to_owned(),
            args: args.as_object().unwrap().clone(),
        };
        assert!(client_key(&call(json!({"client_key": "short"}))).is_err());
        assert!(client_key(&call(json!({"client_key": desktop::encode_key(&[1; 32])}))).is_ok());
        assert!(nonce_arg(&call(json!({"nonce": "a b"}))).is_err());
        assert!(nonce_arg(&call(json!({"nonce": ""}))).is_err());
        assert_eq!(nonce_arg(&call(json!({"nonce": "n-1"}))).unwrap(), "n-1");
    }
}
