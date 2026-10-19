use reins_proto::authorization::*;
use serde_json::{Value, json};

fn action_json() -> Value {
    json!({
        "v": 1,
        "request_id": "request-1",
        "request_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "actor": {
            "principal_id": "employee", "tenant_id": "corp", "connection_id": "cloud-agent", "device_id": null,
            "executor_principal_id": "credential-owner", "executor_device_id": "phone"
        },
        "operation": { "service": "gmail", "capability": "messages", "action": "send" },
        "resource": { "account_id": "stable-account-id", "resource_id": null },
        "facts": [{ "key": "recipients", "fact": {
            "state": "known", "value": { "type": "string_set", "value": ["friend@example.com"] },
            "evidence": { "id": "connector-evidence", "source": "connector", "observed_at": 10, "valid_until": 20 }
        }}]
    })
}

fn policy_json() -> Value {
    json!({
        "v": 1, "id": "friends", "revision": 1,
        "issuer": { "principal_id": "owner", "tenant_id": null },
        "kind": "permission", "lifecycle": "active", "valid_from": 10, "expires_at": 100,
        "scope": {
            "principal_ids": [], "connection_ids": ["cloud-agent"], "device_ids": [],
            "account_ids": ["stable-account-id"], "resource_ids": [],
            "operations": [{ "service": "gmail", "capability": "messages", "action": "send" }]
        },
        "conditions": [{ "operator": "set_within", "key": "recipients", "values": ["friend@example.com"], "require_nonempty": true }],
        "effect": { "type": "permit" }, "parent_policy_ids": [],
        "limits": { "max_uses": 10, "max_cost_microunits": null, "cost_currency": null }
    })
}

fn parse_action(value: &Value) -> Result<ActionContext, reins_proto::ValidationError> {
    ActionContext::from_json(&serde_json::to_vec(value).unwrap())
}

fn parse_policy(value: &Value) -> Result<PolicyRecord, reins_proto::ValidationError> {
    PolicyRecord::from_json(&serde_json::to_vec(value).unwrap())
}

#[test]
fn v1_wire_names_and_exact_values_round_trip_without_defaults_or_coercion() {
    let action = action_json();
    let policy = policy_json();
    assert_eq!(serde_json::to_value(parse_action(&action).unwrap()).unwrap(), action);
    assert_eq!(serde_json::to_value(parse_policy(&policy).unwrap()).unwrap(), policy);
    let mut unknown = action_json();
    unknown["facts"][0]["fact"] = json!({ "state": "unknown", "reason": "untrusted" });
    assert_eq!(serde_json::to_value(parse_action(&unknown).unwrap()).unwrap(), unknown);
}

#[test]
fn all_layers_reject_unknown_fields_and_duplicate_json_keys() {
    for path in [
        "",
        "/actor",
        "/operation",
        "/resource",
        "/facts/0",
        "/facts/0/fact",
        "/facts/0/fact/value",
        "/facts/0/fact/evidence",
    ] {
        let mut value = action_json();
        value.pointer_mut(path).unwrap().as_object_mut().unwrap().insert("future_allow".into(), json!(true));
        assert!(parse_action(&value).is_err(), "accepted unknown field at {path}");
    }
    for path in ["", "/issuer", "/scope", "/scope/operations/0", "/conditions/0", "/effect", "/limits"] {
        let mut value = policy_json();
        value.pointer_mut(path).unwrap().as_object_mut().unwrap().insert("future_allow".into(), json!(true));
        assert!(parse_policy(&value).is_err(), "accepted unknown field at {path}");
    }
    assert!(ActionContext::from_json(br#"{"v":1,"v":2}"#).is_err());
    assert!(PolicyRecord::from_json(br#"{"v":1,"v":2}"#).is_err());
}

#[test]
fn version_unknown_operator_type_mismatch_and_coercions_are_rejected() {
    let mut a = action_json();
    a["v"] = json!(2);
    assert!(parse_action(&a).is_err());
    a = action_json();
    a["actor"]["tenant_id"] = json!("");
    assert!(parse_action(&a).is_err());
    a = action_json();
    a["operation"]["action"] = json!("send*");
    assert!(parse_action(&a).is_err());
    a = action_json();
    a["facts"][0]["fact"]["value"] = json!({"type":"string","value":"friend@example.com"});
    assert!(parse_action(&a).is_err());
    let mut p = policy_json();
    p["conditions"][0]["operator"] = json!("eval");
    assert!(parse_policy(&p).is_err());
    p = policy_json();
    p["effect"]["type"] = json!("approve_everything");
    assert!(parse_policy(&p).is_err());
    p = policy_json();
    p["conditions"][0]["require_nonempty"] = json!("true");
    assert!(parse_policy(&p).is_err());
}

#[test]
fn encoded_size_item_counts_text_lengths_and_duplicates_are_bounded() {
    assert!(ActionContext::from_json(&vec![b' '; MAX_AUTHORIZATION_BYTES + 1]).is_err());
    assert!(PolicyRecord::from_json(&vec![b' '; MAX_AUTHORIZATION_BYTES + 1]).is_err());
    let mut a = parse_action(&action_json()).unwrap();
    a.request_id = "x".repeat(MAX_AUTHORIZATION_TEXT + 1);
    assert!(a.validate().is_err());
    a = parse_action(&action_json()).unwrap();
    a.facts.push(a.facts[0].clone());
    assert!(a.validate().is_err());
    a = parse_action(&action_json()).unwrap();
    a.facts = vec![a.facts[0].clone(); MAX_AUTHORIZATION_ITEMS + 1];
    assert!(a.validate().is_err());
    let mut p = parse_policy(&policy_json()).unwrap();
    p.scope.connection_ids = vec!["same".into(), "same".into()];
    assert!(p.validate().is_err());
    p = parse_policy(&policy_json()).unwrap();
    p.scope.operations.push(p.scope.operations[0].clone());
    assert!(p.validate().is_err());
    p = parse_policy(&policy_json()).unwrap();
    p.conditions = (0..MAX_AUTHORIZATION_ITEMS)
        .map(|i| Condition::StringIn {
            key: AttributeKey::Repository,
            values: (0..MAX_AUTHORIZATION_ITEMS).map(|j| format!("{i}-{j}-{}", "x".repeat(256))).collect(),
        })
        .collect();
    // Each individual field is valid, but their total serialization exceeds the global bound.
    assert!(p.validate().is_err());
}

#[test]
fn malformed_request_digests_and_evidence_intervals_are_rejected() {
    for digest in ["a".repeat(63), "A".repeat(64), "g".repeat(64), format!("{}\n", "a".repeat(63))] {
        let mut a = action_json();
        a["request_digest"] = json!(digest);
        assert!(parse_action(&a).is_err());
    }
    for (observed, until) in [(-1, 20), (20, 20), (21, 20)] {
        let mut a = action_json();
        a["facts"][0]["fact"]["evidence"]["observed_at"] = json!(observed);
        a["facts"][0]["fact"]["evidence"]["valid_until"] = json!(until);
        assert!(parse_action(&a).is_err());
    }
}

#[test]
fn unsafe_authorizing_scopes_and_invalid_reviewer_requirements_are_rejected() {
    for key in ["connection_ids", "account_ids", "operations"] {
        let mut p = policy_json();
        p["scope"][key] = json!([]);
        assert!(parse_policy(&p).is_err());
    }
    let mut p = policy_json();
    p["kind"] = json!("mandatory");
    assert!(parse_policy(&p).is_err());
    p = policy_json();
    p["conditions"][0]["require_nonempty"] = json!(false);
    assert!(parse_policy(&p).is_err());
    for quorum in [0, 129] {
        let mut p = policy_json();
        p["effect"] = json!({ "type": "require_review", "requirements": [{
            "role_id": "legal", "quorum": quorum, "distinct_humans_group": "publication", "exclude_requester": true
        }] });
        p["limits"] = json!({ "max_uses": null, "max_cost_microunits": null, "cost_currency": null });
        assert!(parse_policy(&p).is_err());
    }
    p = policy_json();
    p["effect"] = json!({ "type": "require_review", "requirements": [] });
    assert!(parse_policy(&p).is_err());
}

#[test]
fn delegation_parents_expiry_and_explicit_currency_are_required() {
    let mut p = policy_json();
    p["kind"] = json!("delegation");
    assert!(parse_policy(&p).is_err());
    p["parent_policy_ids"] = json!(["authority"]);
    assert!(parse_policy(&p).is_ok());
    p["expires_at"] = Value::Null;
    assert!(parse_policy(&p).is_err());
    p = policy_json();
    p["limits"]["max_cost_microunits"] = json!(10);
    assert!(parse_policy(&p).is_err());
    p["limits"]["cost_currency"] = json!("USD");
    assert!(parse_policy(&p).is_ok());
    p["limits"]["cost_currency"] = json!("usd");
    assert!(parse_policy(&p).is_err());
    p["limits"]["cost_currency"] = json!("NEW_CURRENCY");
    assert!(parse_policy(&p).is_err());
}

#[test]
fn review_by_default_is_strictly_personal_non_authorizing_account_policy() {
    let mut value = policy_json();
    value["kind"] = json!("account_default");
    value["effect"] = json!({"type":"review_by_default"});
    value["limits"] = json!({"max_uses":null,"max_cost_microunits":null,"cost_currency":null});
    assert_eq!(serde_json::to_value(parse_policy(&value).unwrap()).unwrap(), value);
    for kind in ["mandatory", "permission", "delegation"] {
        let mut invalid = value.clone();
        invalid["kind"] = json!(kind);
        assert!(parse_policy(&invalid).is_err(), "{kind}");
    }
    let mut invalid = value.clone();
    invalid["issuer"]["tenant_id"] = json!("corporation");
    assert!(parse_policy(&invalid).is_err());
    invalid = value.clone();
    invalid["effect"]["requirements"] = json!([]);
    assert!(parse_policy(&invalid).is_err(), "unknown constraints must not disappear");
    invalid = value.clone();
    invalid["limits"]["max_uses"] = json!(1);
    assert!(parse_policy(&invalid).is_err(), "a default cannot consume execution authority");
    invalid = value;
    invalid["parent_policy_ids"] = json!(["authority"]);
    assert!(parse_policy(&invalid).is_err(), "a default cannot stand in for a delegation");
}
