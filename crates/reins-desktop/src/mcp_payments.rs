//! Purchases through `reins mcp` (`docs/payments.md`): the bridge asks the phone to seal the card details of an
//! approved purchase to this app's key, so that the Reins server relays them without being able to read them, and
//! opens them for the local agent. The two arguments that ask for it are filled in here and hidden from the harness's
//! tool list.

use serde_json::{Map, Value, json};

use crate::identity::Identity;

const PURCHASE_TOOL: &str = "payments_purchase_request";
const SEALING_ARGS: [&str; 2] = ["client_key", "nonce"];

/// A message on its way to the server: a purchase request gets this app's key and a fresh nonce, which is returned
/// for the answer.
pub fn seal_request(msg: &mut Value, identity: &Identity) -> Option<String> {
    if msg.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    let params = msg.get_mut("params")?.as_object_mut()?;
    if params.get("name").and_then(Value::as_str) != Some(PURCHASE_TOOL) {
        return None;
    }
    let args = params.entry("arguments").or_insert_with(|| Value::Object(Map::new())).as_object_mut()?;
    let nonce = crate::server::random_token(16);
    args.insert("client_key".to_owned(), json!(identity.public_key()));
    args.insert("nonce".to_owned(), json!(nonce));
    Some(nonce)
}

/// An answer from the server: the sealing arguments are dropped from a tool list, and a sealed payment is opened
/// (its nonce and purchase checked) or the answer becomes an error that says why.
pub fn open_answer(msg: &mut Value, identity: &Identity, nonce: Option<&str>) {
    hide_sealing_args(msg);
    let Some(nonce) = nonce else {
        return;
    };
    let Some(result) = msg.get_mut("result") else {
        return;
    };
    let Some(sealed) = result.pointer("/structuredContent/payment/sealed").and_then(Value::as_str) else {
        return;
    };
    let purchase = result.pointer("/structuredContent/purchase_id").and_then(Value::as_str).unwrap_or_default();
    match open(identity, sealed, nonce, purchase) {
        Ok(payment) => {
            result["structuredContent"]["payment"] = payment;
            let text = result["structuredContent"].to_string();
            result["content"] = json!([{"type": "text", "text": text}]);
        }
        Err(why) => *result = json!({"content": [{"type": "text", "text": why}], "isError": true}),
    }
}

fn open(identity: &Identity, sealed: &str, nonce: &str, purchase: &str) -> Result<Value, String> {
    let plain = identity
        .unseal(sealed)
        .map_err(|_| "The phone sealed the card details to another key; this purchase cannot be paid from here.")?;
    let opened: Value =
        serde_json::from_slice(&plain).map_err(|_| "The phone's sealed card details could not be read.".to_owned())?;
    if opened["v"] != 1 || opened["nonce"] != nonce || opened["purchase_id"] != purchase {
        return Err("The sealed card details are for another request.".to_owned());
    }
    Ok(opened["payment"].clone())
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
    use super::*;
    use crate::identity::seal_to;

    #[test]
    fn a_purchase_asks_for_sealing_and_its_answer_is_opened_for_the_agent() {
        let identity = Identity::generate();
        let mut call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": PURCHASE_TOOL, "arguments": {"merchant": "Shop"}}});
        let nonce = seal_request(&mut call, &identity).unwrap();
        assert_eq!(call["params"]["arguments"]["client_key"], identity.public_key());
        assert_eq!(call["params"]["arguments"]["nonce"], nonce);
        let mut other = json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {"name": "gmail_search"}});
        assert_eq!(seal_request(&mut other, &identity), None);

        let payment = json!({"kind": "card", "number": "4111111111111111", "code": "737"});
        let inner = json!({"v": 1, "nonce": nonce, "purchase_id": "p1", "payment": payment});
        let sealed = seal_to(&identity.public_key(), inner.to_string().as_bytes()).unwrap();
        let answer = |sealed: &str| {
            json!({"jsonrpc": "2.0", "id": 7, "result": {"isError": false,
                "content": [{"type": "text", "text": "{}"}],
                "structuredContent": {"purchase_id": "p1", "payment": {"kind": "card", "sealed": sealed}}}})
        };
        let mut opened = answer(&sealed);
        open_answer(&mut opened, &identity, Some(&nonce));
        assert_eq!(opened["result"]["structuredContent"]["payment"], payment);
        assert!(opened["result"]["content"][0]["text"].as_str().unwrap().contains("4111111111111111"));

        let mut replayed = answer(&sealed);
        open_answer(&mut replayed, &identity, Some("another-nonce"));
        assert_eq!(replayed["result"]["isError"], true);
        let mut foreign = answer(&seal_to(&Identity::generate().public_key(), inner.to_string().as_bytes()).unwrap());
        open_answer(&mut foreign, &identity, Some(&nonce));
        assert!(foreign["result"]["content"][0]["text"].as_str().unwrap().contains("another key"));
    }

    #[test]
    fn the_sealing_arguments_are_hidden_from_the_tool_list() {
        let identity = Identity::generate();
        let mut list = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
            {"name": PURCHASE_TOOL, "inputSchema": {"properties": {"merchant": {}, "client_key": {}, "nonce": {}}}},
            {"name": "desktop_x", "inputSchema": {"properties": {"nonce": {}}}}]}});
        open_answer(&mut list, &identity, None);
        assert_eq!(list["result"]["tools"][0]["inputSchema"]["properties"], json!({"merchant": {}}));
        assert_eq!(
            list["result"]["tools"][1]["inputSchema"]["properties"],
            json!({"nonce": {}}),
            "only the purchase tool"
        );
    }
}
