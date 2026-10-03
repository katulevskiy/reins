//! How a parked request keeps a `ToolCall::Mcp`. The call's own wire form is tagged by `tool` and `McpCall` has a
//! field `tool` too, so serde writes the key twice and cannot read it back. A parked MCP call is therefore kept as
//! `{"tool": "mcp", "server", "mcp_tool", "arguments"}`; every other call is kept as serde writes it.
//!
//! Use with `#[serde(with = "crate::mcp::wire::parked_request")]` on a `RelayRequest` field.

pub mod parked_request {
    use reins_proto::gmail::ToolCall;
    use reins_proto::relay::RelayRequest;
    use reins_proto::remote_mcp::McpCall;
    use serde::de::Error as _;
    use serde::ser::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{Value, json};

    pub fn serialize<S: Serializer>(request: &RelayRequest, s: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(request).map_err(S::Error::custom)?;
        if let ToolCall::Mcp(call) = &request.call {
            value["call"] =
                json!({"tool": "mcp", "server": call.server, "mcp_tool": call.tool, "arguments": call.arguments});
        }
        value.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<RelayRequest, D::Error> {
        let mut value = Value::deserialize(d)?;
        let call = &mut value["call"];
        if call["tool"] != "mcp" || call.get("mcp_tool").is_none() {
            return serde_json::from_value(value).map_err(D::Error::custom);
        }
        let mcp = McpCall {
            server: call["server"].as_str().ok_or_else(|| D::Error::custom("no server"))?.to_owned(),
            tool: call["mcp_tool"].as_str().ok_or_else(|| D::Error::custom("no tool"))?.to_owned(),
            arguments: call["arguments"].as_object().cloned().unwrap_or_default(),
        };
        *call = json!({"tool": "list_accounts"});
        let mut request: RelayRequest = serde_json::from_value(value).map_err(D::Error::custom)?;
        request.call = ToolCall::Mcp(mcp);
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::gmail::ToolCall;
    use reins_proto::relay::RelayRequest;
    use reins_proto::remote_mcp::McpCall;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super::parked_request")]
        request: RelayRequest,
    }

    fn holder(call: ToolCall) -> Holder {
        Holder {
            request: RelayRequest {
                v: 1,
                id: "r1".into(),
                connection_id: "c1".into(),
                connection_label: "Claude".into(),
                created_at: 5,
                wait_until: Some(9),
                account: Some("tracker".into()),
                call,
            },
        }
    }

    #[test]
    fn mcp_calls_and_others_round_trip() {
        let mcp = holder(ToolCall::Mcp(McpCall {
            server: "tracker".into(),
            tool: "create_issue".into(),
            arguments: json!({"title": "x", "tool": "not the tag"}).as_object().unwrap().clone(),
        }));
        let bytes = serde_json::to_vec(&mcp).unwrap();
        assert_eq!(serde_json::from_slice::<Holder>(&bytes).unwrap(), mcp);
        let gmail = holder(ToolCall::GmailRead {
            message_ids: vec!["m1".into()],
        });
        let bytes = serde_json::to_vec(&gmail).unwrap();
        assert_eq!(serde_json::from_slice::<Holder>(&bytes).unwrap(), gmail);
        assert!(String::from_utf8(bytes).unwrap().contains(r#""tool":"gmail_read""#), "others are kept as before");
    }
}
