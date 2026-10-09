//! Purchases through `reins mcp` (`docs/payments.md`): the bridge asks the phone to seal the card details of an
//! approved purchase to this app's key, so that the Reins server relays them without being able to read them, and
//! opens them for the local agent. The two arguments that ask for it are filled in here and hidden from the harness's
//! tool list.
//!
//! Everything fails closed: without this app's key a purchase is not sent; card details that come back unsealed are
//! withheld; a sealed answer is opened only when it carries a nonce this bridge issued and is signed by the phone's
//! payment key, which the user confirmed once with `reins payments-trust` (the server could otherwise seal a forged
//! card to the app's public key, signed by a key of its own). Answers fetched later with `reins_get_result` are opened
//! the same way.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use data_encoding::BASE64URL_NOPAD;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde_json::{Map, Value, json};

use crate::config::Paths;
use crate::identity::Identity;

const PURCHASE_TOOL: &str = "payments_purchase_request";
const SEALING_ARGS: [&str; 2] = ["client_key", "nonce"];
/// The `typ` of the phone's signed card details (`reins-core`'s `mandate::SEALED_JWS_TYPE`).
const SEALED_JWS_TYPE: &str = "reins-sealed-payment+jws";
/// Card fields that never pass the bridge unsealed.
const CARD_FIELDS: [&str; 2] = ["number", "code"];

/// What the bridge needs to open purchases.
pub struct Sealing {
    /// This app's key, or why it could not be read.
    identity: Result<Identity, String>,
    /// The nonces of purchases sent and not answered yet.
    issued: Mutex<HashSet<String>>,
    /// Where the phone's payment key is kept once the user trusted it (its RFC 7638 thumbprint).
    pin_file: PathBuf,
}

/// Calls `f` on the parameters of every purchase request in a message (alone or in a batch); how many there were.
fn each_purchase(msg: &mut Value, mut f: impl FnMut(&mut Map<String, Value>)) -> usize {
    let mut n = 0;
    let mut visit = |m: &mut Value| {
        if m.get("method").and_then(Value::as_str) != Some("tools/call") {
            return;
        }
        let Some(params) = m.get_mut("params").and_then(Value::as_object_mut) else {
            return;
        };
        if params.get("name").and_then(Value::as_str) == Some(PURCHASE_TOOL) {
            n += 1;
            f(params);
        }
    };
    match msg {
        Value::Array(items) => items.iter_mut().for_each(&mut visit),
        m => visit(m),
    }
    n
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Sealing {
    #[must_use]
    pub fn load(paths: &Paths) -> Self {
        Self {
            identity: Identity::load_or_create(&paths.identity_file()).map_err(|e| e.to_string()),
            issued: Mutex::new(HashSet::new()),
            pin_file: paths.phone_payment_key_file(),
        }
    }

    #[cfg(test)]
    fn with(identity: Identity, pin_file: PathBuf) -> Self {
        Self {
            identity: Ok(identity),
            issued: Mutex::new(HashSet::new()),
            pin_file,
        }
    }

    /// A message on its way to the server: every purchase request in it (alone or in a batch) gets this app's key and
    /// a fresh nonce. An error, and nothing is sent, when there is a purchase and this app's key cannot be read.
    pub fn prepare(&self, msg: &mut Value) -> Result<(), String> {
        if each_purchase(msg, |_| {}) == 0 {
            return Ok(());
        }
        let identity = self.identity.as_ref().map_err(|e| {
            format!("Reins: this app's key could not be read ({e}), so card details could not be sealed to it.")
        })?;
        each_purchase(msg, |params| {
            let args = params.entry("arguments").or_insert_with(|| Value::Object(Map::new()));
            if let Some(args) = args.as_object_mut() {
                let nonce = crate::server::random_token(16);
                args.insert("client_key".to_owned(), json!(identity.public_key()));
                args.insert("nonce".to_owned(), json!(nonce));
                lock(&self.issued).insert(nonce);
            }
        });
        Ok(())
    }

    /// An answer from the server: the sealing arguments are dropped from a tool list, a sealed payment is opened, and
    /// card details that came unsealed are withheld; either failure makes the answer an error that says why.
    pub fn open(&self, msg: &mut Value) {
        hide_sealing_args(msg);
        let Some(result) = msg.get_mut("result") else {
            return;
        };
        let Some(payment) = result.pointer("/structuredContent/payment").filter(|p| p.is_object()).cloned() else {
            return;
        };
        let opened = match payment.get("sealed").and_then(Value::as_str) {
            Some(sealed) => {
                let purchase =
                    result.pointer("/structuredContent/purchase_id").and_then(Value::as_str).unwrap_or_default();
                self.unseal(sealed, purchase)
            }
            None if CARD_FIELDS.iter().any(|f| payment.get(*f).is_some()) => {
                Err("Reins: the card details came back unsealed, so they were withheld. Deny it on the phone and \
                     ask again."
                    .to_owned())
            }
            None => return,
        };
        match opened {
            Ok(payment) => {
                result["structuredContent"]["payment"] = payment;
                let text = result["structuredContent"].to_string();
                result["content"] = json!([{"type": "text", "text": text}]);
            }
            Err(why) => *result = json!({"content": [{"type": "text", "text": why}], "isError": true}),
        }
    }

    fn unseal(&self, sealed: &str, purchase: &str) -> Result<Value, String> {
        let identity = self.identity.as_ref().map_err(|e| format!("Reins: this app's key could not be read ({e})."))?;
        let plain = identity
            .unseal(sealed)
            .map_err(|_| "The phone sealed the card details to another key; this purchase cannot be paid from here.")?;
        let outer: Value = serde_json::from_slice(&plain)
            .map_err(|_| "The phone's sealed card details could not be read.".to_owned())?;
        let (inner, thumbprint) = verify_jws(outer["jws"].as_str().unwrap_or_default())?;
        self.check_pin(&thumbprint)?;
        let nonce = inner["nonce"].as_str().unwrap_or_default();
        if inner["v"] != 2 || inner["purchase_id"] != purchase || !lock(&self.issued).remove(nonce) {
            return Err("The sealed card details are for another request.".to_owned());
        }
        Ok(inner["payment"].clone())
    }

    /// The phone's payment key must be the one the user confirmed (`reins payments-trust`): a server could otherwise
    /// seal and sign a card of its own with a key of its own.
    fn check_pin(&self, thumbprint: &str) -> Result<(), String> {
        match std::fs::read_to_string(&self.pin_file) {
            Ok(pinned) if pinned.trim() == thumbprint => Ok(()),
            Ok(_) => Err("The card details are signed by another key than the one you trusted for your phone: they \
                          were withheld. If you reset your account, compare the key under Integrations → Payments on \
                          the phone and run `reins payments-trust <key>` again."
                .to_owned()),
            Err(_) => Err(format!(
                "Reins: to pay from this computer, confirm your phone's payment key once. If Integrations → Payments \
                 on the phone shows {thumbprint}, run `reins payments-trust {thumbprint}` and ask for the purchase \
                 again. The card details of this one were withheld."
            )),
        }
    }
}

/// Trusts the phone's payment key (its RFC 7638 thumbprint, 43 base64url characters), for `reins payments-trust`.
pub fn trust(paths: &Paths, thumbprint: &str) -> Result<(), String> {
    let thumbprint = thumbprint.trim();
    if thumbprint.len() != 43 || BASE64URL_NOPAD.decode(thumbprint.as_bytes()).is_err() {
        return Err(
            "That is not a key thumbprint: copy the 43 characters shown under Integrations → Payments.".to_owned()
        );
    }
    paths.ensure().map_err(|e| e.to_string())?;
    crate::config::write_private(&paths.phone_payment_key_file(), thumbprint.as_bytes()).map_err(|e| e.to_string())
}

/// Checks a compact JWS signed with Ed25519 by the key in its header; the payload and the key's RFC 7638 thumbprint.
fn verify_jws(jws: &str) -> Result<(Value, String), String> {
    let bad = || "The phone's signature on the card details does not check out; they were withheld.".to_owned();
    let parts: Vec<&str> = jws.split('.').collect();
    let [header, payload, signature] = parts.as_slice() else {
        return Err(bad());
    };
    let decode = |s: &str| BASE64URL_NOPAD.decode(s.as_bytes()).map_err(|_| bad());
    let head: Value = serde_json::from_slice(&decode(header)?).map_err(|_| bad())?;
    if head["alg"] != "EdDSA" || head["typ"] != SEALED_JWS_TYPE || head["jwk"]["crv"] != "Ed25519" {
        return Err(bad());
    }
    let x = head["jwk"]["x"].as_str().unwrap_or_default();
    let public = decode(x)?;
    UnparsedPublicKey::new(&ED25519, &public)
        .verify(format!("{header}.{payload}").as_bytes(), &decode(signature)?)
        .map_err(|_| bad())?;
    let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, BASE64URL_NOPAD.encode(&public));
    let thumbprint = BASE64URL_NOPAD.encode(ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes()).as_ref());
    Ok((serde_json::from_slice(&decode(payload)?).map_err(|_| bad())?, thumbprint))
}

/// Drops `client_key` and `nonce` from the purchase tool in a `tools/list` answer: the bridge sets them.
fn hide_sealing_args(msg: &mut Value) {
    let Some(tools) = msg.pointer_mut("/result/tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools.iter_mut().filter(|t| t["name"] == PURCHASE_TOOL) {
        if let Some(props) = tool.pointer_mut("/inputSchema/properties").and_then(Value::as_object_mut) {
            for arg in SEALING_ARGS {
                props.remove(arg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    use super::*;
    use crate::identity::seal_to;

    /// What the phone sends: the payment signed by its key, sealed to the app's.
    fn phone_answer(phone: &Ed25519KeyPair, app: &str, nonce: &str, purchase: &str, typ: &str) -> Value {
        let public = BASE64URL_NOPAD.encode(phone.public_key().as_ref());
        let header = json!({"alg": "EdDSA", "typ": typ, "jwk": {"kty": "OKP", "crv": "Ed25519", "x": public}});
        let payload = json!({"v": 2, "nonce": nonce, "purchase_id": purchase,
            "payment": {"kind": "card", "number": "4111111111111111", "code": "737"}});
        let input = format!(
            "{}.{}",
            BASE64URL_NOPAD.encode(header.to_string().as_bytes()),
            BASE64URL_NOPAD.encode(payload.to_string().as_bytes())
        );
        let jws = format!("{input}.{}", BASE64URL_NOPAD.encode(phone.sign(input.as_bytes()).as_ref()));
        let sealed = seal_to(app, json!({"jws": jws}).to_string().as_bytes()).unwrap();
        json!({"jsonrpc": "2.0", "id": 7, "result": {"isError": false, "content": [{"type": "text", "text": "{}"}],
            "structuredContent": {"purchase_id": purchase, "payment": {"kind": "card", "sealed": sealed}}}})
    }

    fn key(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }

    fn thumbprint_of(phone: &Ed25519KeyPair) -> String {
        let x = BASE64URL_NOPAD.encode(phone.public_key().as_ref());
        let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{x}"}}"#);
        BASE64URL_NOPAD.encode(ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes()).as_ref())
    }

    fn trust_in(sealing: &Sealing, phone: &Ed25519KeyPair) {
        std::fs::write(&sealing.pin_file, thumbprint_of(phone)).unwrap();
    }

    #[test]
    fn only_a_thumbprint_can_be_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(trust(&paths, "not a key").is_err());
        trust(&paths, &thumbprint_of(&key(1))).unwrap();
        assert_eq!(std::fs::read_to_string(paths.phone_payment_key_file()).unwrap(), thumbprint_of(&key(1)));
    }

    fn nonce_of(call: &Value) -> String {
        call["params"]["arguments"]["nonce"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_purchase_is_sealed_and_its_signed_answer_opened_once() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let app = identity.public_key();
        let sealing = Sealing::with(identity, dir.path().join("phone-payments.key"));
        trust_in(&sealing, &key(1));
        let mut call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": PURCHASE_TOOL, "arguments": {"merchant": "Shop"}}});
        sealing.prepare(&mut call).unwrap();
        assert_eq!(call["params"]["arguments"]["client_key"], app);
        let nonce = nonce_of(&call);
        let mut opened = phone_answer(&key(1), &app, &nonce, "p1", SEALED_JWS_TYPE);
        let replay = opened.clone();
        sealing.open(&mut opened);
        assert_eq!(opened["result"]["structuredContent"]["payment"]["number"], "4111111111111111");
        let mut again = replay;
        sealing.open(&mut again);
        assert_eq!(again["result"]["isError"], true, "a nonce opens one answer");
    }

    #[test]
    fn forged_unsigned_unsealed_and_foreign_answers_are_withheld() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let app = identity.public_key();
        let sealing = Sealing::with(identity, dir.path().join("phone-payments.key"));
        let issue = || {
            let mut call = json!({"method": "tools/call", "params": {"name": PURCHASE_TOOL, "arguments": {}}, "id": 1});
            sealing.prepare(&mut call).unwrap();
            nonce_of(&call)
        };
        // Nothing opens before the user trusted the phone's key; the answer says which key to compare.
        let mut untrusted = phone_answer(&key(1), &app, &issue(), "p0", SEALED_JWS_TYPE);
        sealing.open(&mut untrusted);
        let said = untrusted["result"]["content"][0]["text"].as_str().unwrap().to_owned();
        assert!(said.contains("reins payments-trust") && said.contains(&thumbprint_of(&key(1))), "{said}");
        // Trusted: the phone's answers open…
        trust_in(&sealing, &key(1));
        let mut first = phone_answer(&key(1), &app, &issue(), "p1", SEALED_JWS_TYPE);
        sealing.open(&mut first);
        assert_eq!(first["result"]["isError"], false);
        // …and a server sealing its own card to the app's public key, signed by its own key, is refused.
        let mut forged = phone_answer(&key(2), &app, &issue(), "p2", SEALED_JWS_TYPE);
        sealing.open(&mut forged);
        assert!(
            forged["result"]["content"][0]["text"].as_str().unwrap().contains("another key than the one you trusted")
        );
        let mut wrong_type = phone_answer(&key(1), &app, &issue(), "p3", "reins-cart-mandate+jws");
        sealing.open(&mut wrong_type);
        assert_eq!(wrong_type["result"]["isError"], true);
        let mut unknown_nonce = phone_answer(&key(1), &app, "made-up", "p4", SEALED_JWS_TYPE);
        sealing.open(&mut unknown_nonce);
        assert_eq!(unknown_nonce["result"]["isError"], true);
        // Card details that come back in the clear never reach the agent.
        let mut plain = json!({"result": {"structuredContent": {"purchase_id": "p5",
            "payment": {"kind": "card", "number": "4111111111111111"}}}});
        sealing.open(&mut plain);
        assert!(plain["result"]["content"][0]["text"].as_str().unwrap().contains("withheld"));
        let mut store = json!({"result": {"structuredContent": {"payment": {"kind": "merchant_account"}}}});
        sealing.open(&mut store);
        assert_eq!(store["result"]["structuredContent"]["payment"]["kind"], "merchant_account", "nothing to open");
    }

    #[test]
    fn batches_are_sealed_and_a_missing_key_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sealing = Sealing::with(Identity::generate(), dir.path().join("pin"));
        let mut batch = json!([
            {"method": "tools/call", "id": 1, "params": {"name": PURCHASE_TOOL, "arguments": {}}},
            {"method": "tools/call", "id": 2, "params": {"name": "gmail_search", "arguments": {}}},
        ]);
        sealing.prepare(&mut batch).unwrap();
        assert!(batch[0]["params"]["arguments"]["nonce"].is_string());
        assert!(batch[1]["params"]["arguments"].get("nonce").is_none());
        let broken = Sealing {
            identity: Err("unreadable".to_owned()),
            issued: Mutex::new(HashSet::new()),
            pin_file: dir.path().join("pin"),
        };
        let mut call = json!({"method": "tools/call", "id": 1, "params": {"name": PURCHASE_TOOL}});
        assert!(broken.prepare(&mut call).unwrap_err().contains("could not be read"));
        let mut other = json!({"method": "tools/call", "id": 1, "params": {"name": "gmail_search"}});
        assert!(broken.prepare(&mut other).is_ok(), "only purchases need the key");
    }

    #[test]
    fn the_sealing_arguments_are_hidden_from_the_tool_list() {
        let dir = tempfile::tempdir().unwrap();
        let sealing = Sealing::with(Identity::generate(), dir.path().join("pin"));
        let mut list = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
            {"name": PURCHASE_TOOL, "inputSchema": {"properties": {"merchant": {}, "client_key": {}, "nonce": {}}}},
            {"name": "desktop_x", "inputSchema": {"properties": {"nonce": {}}}}]}});
        sealing.open(&mut list);
        assert_eq!(list["result"]["tools"][0]["inputSchema"]["properties"], json!({"merchant": {}}));
        assert_eq!(
            list["result"]["tools"][1]["inputSchema"]["properties"],
            json!({"nonce": {}}),
            "only the purchase tool"
        );
    }
}
