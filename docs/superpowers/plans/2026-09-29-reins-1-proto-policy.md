# Reins Plan 1: Wire Types and Grant Policy Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two pure Rust crates to the Vaultwarden workspace: `reins-proto` (wire types shared by server and phone, with input validation) and `reins-policy` (the grant model and matcher that decides what an AI may see or send).

**Architecture:** Both crates are IO-free libraries in `crates/`, members of the existing Vaultwarden Cargo workspace and subject to its strict lints. `reins-policy` depends on `reins-proto`. The policy engine matches grants against *facts about real messages* (never against the AI's query), fails closed on malformed data, and is covered by unit tests plus property tests.

**Tech Stack:** Rust 1.98.1 (edition 2024), serde 1.0.229, thiserror 2.0.20, regex 1.13.1, proptest 1 (dev), serde_json 1.0.151 (dev).

**Spec:** `docs/superpowers/specs/2026-09-28-reins-mvp-design.md` (§5.1, §5.2). Roadmap: `docs/superpowers/plans/2026-09-29-reins-roadmap.md`.

## Global Constraints

- Repo root: `<repo>`, branch `reins-mvp`. Use rustup's toolchain: `export PATH="$HOME/.cargo/bin:$PATH"` before any cargo command.
- New crates set `[lints] workspace = true`; code must pass `cargo clippy -p <crate> --all-targets -- -D warnings` (workspace denies all warnings, clippy `pedantic` included) and `cargo fmt --check`.
- Workspace forbids `unsafe_code`; denies `single_use_lifetimes`, `unused_qualifications`, `variant_size_differences`, `trivial_casts`, clippy `str_to_string` (use `to_owned()`), `redundant_clone`.
- No IO, no async, no global state in either crate.
- Tool limits (verbatim from spec §4.2): `gmail_search.max_results` 1..50 (default 10 is applied by the server, not here); `gmail_read.message_ids` 1..20.
- Patterns are case-insensitive; `subject`/`body` patterns search; address patterns match the whole address; `Domain(d)` matches exactly `d`, not subdomains.
- One-time approvals do not create grants; a grant with neither `expires_at` nor `max_uses` lasts until revoked.
- When several grants match, unlimited grants (`max_uses == None`) are preferred, then lowest `GrantId`.
- A grant is inactive when `revoked`, when `now >= expires_at`, or when `uses >= max_uses`.
- Do not modify any upstream Vaultwarden source file except root `Cargo.toml` (workspace members).

## Review Focus

- **Email header injection:** an address or subject containing CR/LF (e.g. `"a@b.com\r\nBcc: x@evil.com"`) must be rejected before it can reach an RFC 5322 header. Pinned in Task 1.
- **Look-alike addresses:** `Bob <eve@evil.com>`, `alice@bank.com.evil.com`, `alice@sub.bank.com` must not satisfy `Domain("bank.com")`; uppercase/whitespace variants must normalize to the same address. Pinned in Tasks 1 and 3.
- **Regex wrapper escape:** a full-match address pattern like `a)|(.*` must be rejected rather than compiled into `^(?:a)|(.*)$`, which matches everything. Pinned in Task 3.
- **Corrupt or empty stored scope:** a `ReadScope` with no constraints (e.g. deserialized from a damaged store) must match nothing. Pinned in Task 4.
- **Grant lifetime boundaries:** a grant is dead at exactly `now == expires_at`, at `uses == max_uses`, and when revoked; another connection's grant never applies. Pinned in Tasks 5 and 6.

---

## File Structure

```
Cargo.toml                                  modify: workspace members
crates/reins-proto/Cargo.toml            create
crates/reins-proto/src/lib.rs            create: module wiring, PROTOCOL_VERSION
crates/reins-proto/src/ids.rs            create: RequestId, ConnectionId, PairingId, GrantId
crates/reins-proto/src/validate.rs       create: ValidationError, normalize_address, invalid()
crates/reins-proto/src/gmail.rs          create: ToolCall, OutgoingEmail, MessageSummary, MessageFull, SentMessage
crates/reins-proto/src/relay.rs          create: RelayRequest, RelayResponse, RelayOutcome, ToolResult
crates/reins-proto/src/pairing.rs        create: PairingRequest, PairingResponse, PushMessage, PushKind
crates/reins-policy/Cargo.toml           create
crates/reins-policy/src/lib.rs           create: module wiring, PolicyError
crates/reins-policy/src/pattern.rs       create: Pattern, AddrRule
crates/reins-policy/src/scope.rs         create: MessageFacts, ReadScope, SendScope, Scope
crates/reins-policy/src/grant.rs         create: Grant, record_uses
crates/reins-policy/src/evaluate.rs      create: evaluate_read, evaluate_send, needs_body, decisions
crates/reins-policy/tests/properties.rs  create: proptest soundness/completeness
```

---

### Task 1: Workspace wiring + `reins-proto` ids, validation, Gmail types

**Files:**
- Modify: `Cargo.toml` (the `[workspace]` table, currently `members = ["macros"]`)
- Create: `crates/reins-proto/Cargo.toml`, `src/lib.rs`, `src/ids.rs`, `src/validate.rs`, `src/gmail.rs`

**Interfaces:**
- Produces:
  - `reins_proto::PROTOCOL_VERSION: u32 = 1`
  - `reins_proto::ids::{RequestId, ConnectionId, PairingId, GrantId}`: `pub struct X(pub String)`, serde-transparent, `Display`, `From<&str>`, `Clone + Debug + Eq + Ord + Hash`
  - `reins_proto::{ValidationError, normalize_address}`: `fn normalize_address(raw: &str) -> Result<String, ValidationError>`
  - `reins_proto::gmail::{ToolCall, OutgoingEmail, MessageSummary, MessageFull, SentMessage}` and constants `MAX_SEARCH_RESULTS: u32 = 50`, `MAX_READ_IDS: usize = 20`, `MAX_QUERY_LEN: usize = 1024`, `MAX_RECIPIENTS: usize = 50`, `MAX_SUBJECT_LEN: usize = 998`, `MAX_BODY_LEN: usize = 1 << 20`
  - `ToolCall::normalized(self) -> Result<ToolCall, ValidationError>`; `OutgoingEmail::normalized(self) -> Result<OutgoingEmail, ValidationError>`; `OutgoingEmail::recipients(&self) -> impl Iterator<Item = &str>`

- [ ] **Step 1: Register the crates in the workspace**

In root `Cargo.toml` change:

```toml
[workspace]
members = ["macros"]
```

to:

```toml
[workspace]
members = ["macros", "crates/reins-proto", "crates/reins-policy"]
```

(`reins-policy` is created in Task 3; until then create a placeholder so the workspace loads — Step 2 below.)

- [ ] **Step 2: Create both crate manifests and skeletons**

`crates/reins-proto/Cargo.toml`:

```toml
[package]
name = "reins-proto"
version = "0.1.0"
repository.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
serde = { version = "1.0.229", features = ["derive"] }
thiserror = "2.0.20"

[dev-dependencies]
serde_json = "1.0.151"

[lints]
workspace = true
```

`crates/reins-policy/Cargo.toml`:

```toml
[package]
name = "reins-policy"
version = "0.1.0"
repository.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
reins-proto = { path = "../reins-proto" }
regex = "1.13.1"
serde = { version = "1.0.229", features = ["derive"] }
thiserror = "2.0.20"

[dev-dependencies]
proptest = "1"
serde_json = "1.0.151"

[lints]
workspace = true
```

`crates/reins-policy/src/lib.rs` (placeholder, replaced in Task 3):

```rust
//! Reins grant model and policy engine.
```

`crates/reins-proto/src/lib.rs`:

```rust
//! Wire types shared by the Reins server and the Reins phone core.
//!
//! Pure data and validation: no IO, no async.

pub mod gmail;
pub mod ids;
mod validate;

pub use validate::{ValidationError, normalize_address};

/// Version stamped on every relay and pairing message (`v` field).
pub const PROTOCOL_VERSION: u32 = 1;
```

`crates/reins-proto/src/ids.rs`:

```rust
use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($($(#[$meta:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    )+};
}

id_type!(
    /// A relayed tool call waiting for the phone.
    RequestId,
    /// An AI client authorized by a user (one per OAuth authorization).
    ConnectionId,
    /// An in-progress AI-connection pairing.
    PairingId,
    /// A standing permission stored on the phone.
    GrantId,
);
```

- [ ] **Step 3: Write failing tests for address validation**

`crates/reins-proto/src/validate.rs`:

```rust
use thiserror::Error;

/// Input rejected at the protocol boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("{field}: {reason}")]
    Invalid { field: &'static str, reason: String },
}

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> ValidationError {
    ValidationError::Invalid {
        field,
        reason: reason.into(),
    }
}

/// Validates a bare email address (`local@domain`) and lower-cases it.
///
/// Display names, whitespace, control characters and header metacharacters are
/// rejected, so a normalized address can be written into an RFC 5322 header
/// without enabling header injection.
pub fn normalize_address(raw: &str) -> Result<String, ValidationError> {
    todo!("{raw}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_and_trims() {
        assert_eq!(normalize_address("  Alice@Example.COM ").unwrap(), "alice@example.com");
    }

    #[test]
    fn rejects_header_injection_and_display_names() {
        for bad in [
            "a@b.com\r\nBcc: x@evil.com",
            "a@b.com\nx",
            "Bob <bob@x.com>",
            "a@b.com,c@d.com",
            "a b@c.com",
            "\"a\"@b.com",
        ] {
            assert!(normalize_address(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "a@b", "a@@b.com", "@b.com", "a@.com", "a@b.com.", "a@b..com", "ab.com"] {
            assert!(normalize_address(bad).is_err(), "accepted {bad:?}");
        }
        let long = format!("{}@b.com", "a".repeat(250));
        assert!(normalize_address(&long).is_err());
    }
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p reins-proto validate`
Expected: FAIL — tests panic with `not yet implemented`.

- [ ] **Step 5: Implement `normalize_address`**

Replace the `todo!` body:

```rust
pub fn normalize_address(raw: &str) -> Result<String, ValidationError> {
    let bad = |reason: &str| invalid("address", format!("{reason}: {raw:?}"));
    let addr = raw.trim().to_lowercase();
    if addr.is_empty() || addr.len() > 254 {
        return Err(bad("length must be 1..=254"));
    }
    if addr
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "<>,;:\"()[]\\".contains(c))
    {
        return Err(bad("forbidden character"));
    }
    let Some((local, domain)) = addr.split_once('@') else {
        return Err(bad("missing @"));
    };
    let domain_ok = domain.contains('.')
        && !domain.contains('@')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..");
    if local.is_empty() || !domain_ok {
        return Err(bad("malformed"));
    }
    Ok(addr)
}
```

- [ ] **Step 6: Run to verify pass**

Run: `cargo test -p reins-proto validate`
Expected: 3 passed.

- [ ] **Step 7: Write failing tests for Gmail types**

`crates/reins-proto/src/gmail.rs`:

```rust
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::validate::{ValidationError, invalid, normalize_address};

pub const MAX_SEARCH_RESULTS: u32 = 50;
pub const MAX_READ_IDS: usize = 20;
pub const MAX_QUERY_LEN: usize = 1024;
pub const MAX_RECIPIENTS: usize = 50;
pub const MAX_SUBJECT_LEN: usize = 998;
pub const MAX_BODY_LEN: usize = 1 << 20;
const MAX_MESSAGE_ID_LEN: usize = 64;

/// A tool call made by an AI client, as relayed to the phone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolCall {
    GmailSearch { query: String, max_results: u32 },
    GmailRead { message_ids: Vec<String> },
    GmailSend { email: Box<OutgoingEmail> },
}

/// An email an AI asks to send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingEmail {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
}

/// Header-level view of a message, as released by `gmail_search`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: String,
    pub thread_id: String,
    /// Bare, lower-cased sender address.
    pub from: String,
    /// Bare, lower-cased To and Cc addresses.
    pub to: Vec<String>,
    pub subject: String,
    /// Unix seconds (Gmail `internalDate`).
    pub date: i64,
    pub snippet: String,
}

/// A full message, as released by `gmail_read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFull {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub body_text: String,
}

/// Result of a successful `gmail_send`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentMessage {
    pub id: String,
    pub thread_id: String,
}

impl ToolCall {
    /// Validates limits and normalizes addresses and ids. Call on every call
    /// received from an AI before it is relayed, evaluated or executed.
    pub fn normalized(self) -> Result<Self, ValidationError> {
        todo!()
    }
}

impl OutgoingEmail {
    pub fn normalized(self) -> Result<Self, ValidationError> {
        todo!()
    }

    /// All To and Cc addresses.
    pub fn recipients(&self) -> impl Iterator<Item = &str> {
        self.to.iter().chain(&self.cc).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn email(to: &[&str], subject: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: subject.to_owned(),
            body: "hi".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn search_limits() {
        let ok = ToolCall::GmailSearch { query: " from:bank ".to_owned(), max_results: 50 };
        assert_eq!(
            ok.normalized().unwrap(),
            ToolCall::GmailSearch { query: "from:bank".to_owned(), max_results: 50 }
        );
        for max_results in [0, 51] {
            let c = ToolCall::GmailSearch { query: "x".to_owned(), max_results };
            assert!(c.normalized().is_err());
        }
        for query in [String::new(), "   ".to_owned(), "x".repeat(MAX_QUERY_LEN + 1)] {
            assert!(ToolCall::GmailSearch { query, max_results: 5 }.normalized().is_err());
        }
    }

    #[test]
    fn read_ids_validated_and_deduplicated() {
        let c = ToolCall::GmailRead { message_ids: vec!["a1".into(), "b2".into(), "a1".into()] };
        assert_eq!(
            c.normalized().unwrap(),
            ToolCall::GmailRead { message_ids: vec!["a1".into(), "b2".into()] }
        );
        assert!(ToolCall::GmailRead { message_ids: vec![] }.normalized().is_err());
        assert!(ToolCall::GmailRead { message_ids: vec!["../x".into()] }.normalized().is_err());
        let many = (0..=MAX_READ_IDS).map(|i| format!("m{i}")).collect();
        assert!(ToolCall::GmailRead { message_ids: many }.normalized().is_err());
    }

    #[test]
    fn send_normalizes_and_rejects_injection() {
        let e = email(&["Alice@Bank.com"], "Hello").normalized().unwrap();
        assert_eq!(e.to, vec!["alice@bank.com"]);
        assert!(email(&[], "x").normalized().is_err());
        assert!(email(&["a@b.com"], "Hi\r\nBcc: eve@evil.com").normalized().is_err());
        assert!(email(&["a@b.com\r\nBcc: eve@evil.com"], "x").normalized().is_err());
        let too_many: Vec<String> = (0..=MAX_RECIPIENTS).map(|i| format!("u{i}@b.com")).collect();
        let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert!(email(&refs, "x").normalized().is_err());
        let mut big = email(&["a@b.com"], "x");
        big.body = "x".repeat(MAX_BODY_LEN + 1);
        assert!(big.normalized().is_err());
        let mut reply = email(&["a@b.com"], "x");
        reply.reply_to_message_id = Some("bad id".into());
        assert!(reply.normalized().is_err());
    }

    #[test]
    fn send_via_toolcall_normalizes_boxed_email() {
        let c = ToolCall::GmailSend { email: Box::new(email(&["A@B.com"], "x")) };
        let ToolCall::GmailSend { email } = c.normalized().unwrap() else { panic!("variant changed") };
        assert_eq!(email.to, vec!["a@b.com"]);
    }

    #[test]
    fn recipients_chains_to_and_cc() {
        let mut e = email(&["a@b.com"], "x");
        e.cc = vec!["c@d.com".into()];
        assert_eq!(e.recipients().collect::<Vec<_>>(), vec!["a@b.com", "c@d.com"]);
    }

    #[test]
    fn wire_format() {
        let c = ToolCall::GmailSearch { query: "x".into(), max_results: 5 };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({"tool": "gmail_search", "query": "x", "max_results": 5})
        );
        let parsed: ToolCall = serde_json::from_value(json!({
            "tool": "gmail_send",
            "email": {"to": ["a@b.com"], "subject": "s", "body": "b"}
        }))
        .unwrap();
        let ToolCall::GmailSend { email } = parsed else { panic!("wrong variant") };
        assert!(email.cc.is_empty() && email.reply_to_message_id.is_none());
    }
}
```

Add `pub mod gmail;` is already in `lib.rs`.

- [ ] **Step 8: Run to verify failure**

Run: `cargo test -p reins-proto gmail`
Expected: FAIL — `not yet implemented` panics (`wire_format` and `recipients_chains_to_and_cc` pass).

- [ ] **Step 9: Implement normalization**

Replace the two `todo!()` bodies and add helpers below the `impl OutgoingEmail` block:

```rust
impl ToolCall {
    pub fn normalized(self) -> Result<Self, ValidationError> {
        match self {
            Self::GmailSearch { query, max_results } => {
                let query = query.trim().to_owned();
                if query.is_empty() || query.len() > MAX_QUERY_LEN {
                    return Err(invalid("query", format!("length must be 1..={MAX_QUERY_LEN}")));
                }
                if !(1..=MAX_SEARCH_RESULTS).contains(&max_results) {
                    return Err(invalid("max_results", format!("must be 1..={MAX_SEARCH_RESULTS}")));
                }
                Ok(Self::GmailSearch { query, max_results })
            }
            Self::GmailRead { message_ids } => Ok(Self::GmailRead {
                message_ids: normalize_ids(message_ids)?,
            }),
            Self::GmailSend { email } => Ok(Self::GmailSend {
                email: Box::new((*email).normalized()?),
            }),
        }
    }
}
```

```rust
    pub fn normalized(self) -> Result<Self, ValidationError> {
        let to = self.to.iter().map(String::as_str).map(normalize_address).collect::<Result<Vec<_>, _>>()?;
        let cc = self.cc.iter().map(String::as_str).map(normalize_address).collect::<Result<Vec<_>, _>>()?;
        if to.is_empty() {
            return Err(invalid("to", "at least one recipient is required"));
        }
        if to.len() + cc.len() > MAX_RECIPIENTS {
            return Err(invalid("to", format!("at most {MAX_RECIPIENTS} recipients")));
        }
        if self.subject.len() > MAX_SUBJECT_LEN || self.subject.chars().any(char::is_control) {
            return Err(invalid(
                "subject",
                format!("must be at most {MAX_SUBJECT_LEN} bytes without control characters"),
            ));
        }
        if self.body.len() > MAX_BODY_LEN {
            return Err(invalid("body", format!("must be at most {MAX_BODY_LEN} bytes")));
        }
        if let Some(id) = &self.reply_to_message_id {
            validate_message_id(id)?;
        }
        Ok(Self { to, cc, ..self })
    }
```

```rust
fn validate_message_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() || id.len() > MAX_MESSAGE_ID_LEN || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(invalid("message_id", format!("invalid id {id:?}")));
    }
    Ok(())
}

fn normalize_ids(ids: Vec<String>) -> Result<Vec<String>, ValidationError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        validate_message_id(&id)?;
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    if out.is_empty() || out.len() > MAX_READ_IDS {
        return Err(invalid("message_ids", format!("must contain 1..={MAX_READ_IDS} ids")));
    }
    Ok(out)
}
```

- [ ] **Step 10: Run tests, clippy, fmt**

Run: `cargo test -p reins-proto && cargo clippy -p reins-proto --all-targets -- -D warnings && cargo fmt -p reins-proto --check`
Expected: all tests pass; no clippy findings; fmt clean. Fix any pedantic finding clippy reports (typically adding `#[must_use]`) without changing behavior.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock crates/reins-proto crates/reins-policy
git commit -m "feat(proto): ids, address validation and Gmail tool types"
```

---

### Task 2: `reins-proto` relay and pairing messages

**Files:**
- Create: `crates/reins-proto/src/relay.rs`, `crates/reins-proto/src/pairing.rs`
- Modify: `crates/reins-proto/src/lib.rs` (add `pub mod pairing; pub mod relay;`)

**Interfaces:**
- Consumes: `ids::{RequestId, ConnectionId, PairingId}`, `gmail::{ToolCall, MessageSummary, MessageFull, SentMessage}` (Task 1)
- Produces:
  - `relay::RelayRequest { v: u32, id: RequestId, connection_id: ConnectionId, connection_label: String, created_at: i64, call: ToolCall }`
  - `relay::ToolResult` = `Search { messages: Vec<MessageSummary> } | Read { messages: Vec<MessageFull> } | Sent(SentMessage)` (tag `kind`)
  - `relay::RelayOutcome` = `Result { result: ToolResult } | Denied { reason: Option<String> } | Error { message: String }` (tag `outcome`)
  - `relay::RelayResponse { v: u32, #[serde(flatten)] outcome: RelayOutcome }`
  - `pairing::PairingRequest { v, id: PairingId, client_name, client_host, choices: [u8; 3], created_at: i64 }`
  - `pairing::PairingResponse { v, approved: bool, chosen_code: Option<u8>, label: Option<String> }`
  - `pairing::PushMessage { t: PushKind, id: String }`, `pairing::PushKind = Req | Pair` (serialized `"req"`/`"pair"`)

- [ ] **Step 1: Write the types with wire-format tests**

`crates/reins-proto/src/relay.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::gmail::{MessageFull, MessageSummary, SentMessage, ToolCall};
use crate::ids::{ConnectionId, RequestId};

/// A tool call forwarded by the server to the approval device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRequest {
    pub v: u32,
    pub id: RequestId,
    pub connection_id: ConnectionId,
    /// User-chosen label of the AI connection (e.g. "ChatGPT").
    pub connection_label: String,
    /// Unix seconds.
    pub created_at: i64,
    pub call: ToolCall,
}

/// What the phone released or did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResult {
    Search { messages: Vec<MessageSummary> },
    Read { messages: Vec<MessageFull> },
    Sent(SentMessage),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RelayOutcome {
    Result { result: ToolResult },
    Denied { reason: Option<String> },
    Error { message: String },
}

/// The phone's answer to a `RelayRequest`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayResponse {
    pub v: u32,
    #[serde(flatten)]
    pub outcome: RelayOutcome,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::PROTOCOL_VERSION;

    #[test]
    fn request_round_trips() {
        let req = RelayRequest {
            v: PROTOCOL_VERSION,
            id: "r1".into(),
            connection_id: "c1".into(),
            connection_label: "ChatGPT".into(),
            created_at: 1_700_000_000,
            call: ToolCall::GmailRead { message_ids: vec!["m1".into()] },
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["call"], json!({"tool": "gmail_read", "message_ids": ["m1"]}));
        assert_eq!(v["id"], json!("r1"));
        assert_eq!(serde_json::from_value::<RelayRequest>(v).unwrap(), req);
    }

    #[test]
    fn response_wire_format() {
        let denied = RelayResponse { v: 1, outcome: RelayOutcome::Denied { reason: None } };
        assert_eq!(
            serde_json::to_value(&denied).unwrap(),
            json!({"v": 1, "outcome": "denied", "reason": null})
        );
        let sent = RelayResponse {
            v: 1,
            outcome: RelayOutcome::Result {
                result: ToolResult::Sent(SentMessage { id: "m9".into(), thread_id: "t9".into() }),
            },
        };
        let v = serde_json::to_value(&sent).unwrap();
        assert_eq!(v, json!({"v": 1, "outcome": "result", "result": {"kind": "sent", "id": "m9", "thread_id": "t9"}}));
        assert_eq!(serde_json::from_value::<RelayResponse>(v).unwrap(), sent);
    }

    #[test]
    fn full_message_flattens_summary() {
        let m = MessageFull {
            summary: MessageSummary {
                id: "m1".into(),
                thread_id: "t1".into(),
                from: "a@b.com".into(),
                to: vec!["me@x.com".into()],
                subject: "s".into(),
                date: 5,
                snippet: "sn".into(),
            },
            body_text: "body".into(),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["id"], json!("m1"));
        assert_eq!(v["body_text"], json!("body"));
        assert_eq!(serde_json::from_value::<MessageFull>(v).unwrap(), m);
    }
}
```

`crates/reins-proto/src/pairing.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::ids::PairingId;

/// Shown on the phone when an AI client asks to connect to the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequest {
    pub v: u32,
    pub id: PairingId,
    /// Client-declared name (e.g. "ChatGPT"). Untrusted: always shown with `client_host`.
    pub client_name: String,
    /// Host of the OAuth redirect URI (e.g. "chatgpt.com"). Server-verified.
    pub client_host: String,
    /// Three two-digit codes; exactly one equals the code shown in the browser.
    pub choices: [u8; 3],
    /// Unix seconds.
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResponse {
    pub v: u32,
    pub approved: bool,
    /// The code the user tapped; required when `approved`.
    pub chosen_code: Option<u8>,
    /// Label for the new connection; defaults to `client_name`.
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushKind {
    Req,
    Pair,
}

/// FCM data payload. Carries only an id; content is fetched over HTTPS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushMessage {
    pub t: PushKind,
    pub id: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn push_wire_format() {
        let p = PushMessage { t: PushKind::Req, id: "r1".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"t": "req", "id": "r1"}));
    }

    #[test]
    fn pairing_round_trips() {
        let req = PairingRequest {
            v: 1,
            id: "p1".into(),
            client_name: "ChatGPT".into(),
            client_host: "chatgpt.com".into(),
            choices: [12, 47, 83],
            created_at: 1,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["choices"], json!([12, 47, 83]));
        assert_eq!(serde_json::from_value::<PairingRequest>(v).unwrap(), req);
    }
}
```

In `crates/reins-proto/src/lib.rs` replace the module list with:

```rust
pub mod gmail;
pub mod ids;
pub mod pairing;
pub mod relay;
mod validate;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p reins-proto`
Expected: all pass (these are data types; serde derives are the implementation).

- [ ] **Step 3: Clippy, fmt, commit**

Run: `cargo clippy -p reins-proto --all-targets -- -D warnings && cargo fmt -p reins-proto --check`

```bash
git add crates/reins-proto
git commit -m "feat(proto): relay and pairing messages"
```

---

### Task 3: `reins-policy` patterns and address rules

**Files:**
- Modify: `crates/reins-policy/src/lib.rs`
- Create: `crates/reins-policy/src/pattern.rs`

**Interfaces:**
- Consumes: `reins_proto::{normalize_address, ValidationError}`
- Produces:
  - `reins_policy::PolicyError` = `InvalidPattern { pattern: String, reason: String } | InvalidAddress(ValidationError) | InvalidScope(String) | InvalidGrant(String)`
  - `reins_policy::Pattern`: `find(&str) -> Result<Pattern, PolicyError>` (search), `full(&str) -> Result<Pattern, PolicyError>` (whole-input), `source(&self) -> &str`, `is_match(&self, &str) -> bool`; serde as a plain string using `find` semantics; `PartialEq`/`Eq` by source.
  - `reins_policy::AddrRule` = `Exact(String) | Domain(String) | Regex(Pattern)` with constructors `exact(&str)`, `domain(&str)`, `regex(&str)` (full-match), `matches(&self, addr: &str) -> bool`; serde `{"kind": "exact"|"domain"|"regex", "value": "<string>"}`.
  - `MAX_PATTERN_LEN: usize = 512`

- [ ] **Step 1: Write `lib.rs` and failing pattern tests**

`crates/reins-policy/src/lib.rs`:

```rust
//! Reins grant model and policy engine.
//!
//! Decides which messages an AI connection may receive and which emails it may
//! send, from grants the user created on their phone. Pure and IO-free.
//! Every check fails closed: malformed or unknown data never grants access.

mod pattern;

use reins_proto::ValidationError;
use thiserror::Error;

pub use pattern::{AddrRule, MAX_PATTERN_LEN, Pattern};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("invalid pattern {pattern:?}: {reason}")]
    InvalidPattern { pattern: String, reason: String },
    #[error("invalid address: {0}")]
    InvalidAddress(#[from] ValidationError),
    #[error("invalid scope: {0}")]
    InvalidScope(String),
    #[error("invalid grant: {0}")]
    InvalidGrant(String),
}
```

`crates/reins-policy/src/pattern.rs`:

```rust
use std::fmt;

use regex::{Regex, RegexBuilder};
use reins_proto::normalize_address;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PolicyError;

pub const MAX_PATTERN_LEN: usize = 512;
const COMPILED_SIZE_LIMIT: usize = 1 << 18;

/// A validated, case-insensitive regular expression.
///
/// Backed by the `regex` crate, whose matching time is linear in the input, so
/// neither a hostile pattern nor a hostile message body can cause catastrophic
/// backtracking.
#[derive(Clone)]
pub struct Pattern {
    source: String,
    regex: Regex,
}

impl Pattern {
    /// Search semantics: matches when the pattern occurs anywhere in the input.
    pub fn find(source: &str) -> Result<Self, PolicyError> {
        todo!("{source}")
    }

    /// Full-match semantics: the whole input must match.
    pub fn full(source: &str) -> Result<Self, PolicyError> {
        todo!("{source}")
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn is_match(&self, haystack: &str) -> bool {
        self.regex.is_match(haystack)
    }
}

impl fmt::Debug for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Pattern").field(&self.source).finish()
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Pattern {}

impl Serialize for Pattern {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Self::find(&source).map_err(D::Error::custom)
    }
}

/// Matches one normalized email address. Build with the constructors, which
/// normalize their input; a hand-built unnormalized variant simply never
/// matches (fails closed).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AddrRuleRepr", into = "AddrRuleRepr")]
pub enum AddrRule {
    /// Exactly this address.
    Exact(String),
    /// Any address at exactly this domain (subdomains excluded).
    Domain(String),
    /// Whole-address regular expression.
    Regex(Pattern),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum AddrRuleRepr {
    Exact(String),
    Domain(String),
    Regex(String),
}

impl AddrRule {
    pub fn exact(addr: &str) -> Result<Self, PolicyError> {
        Ok(Self::Exact(normalize_address(addr)?))
    }

    pub fn domain(domain: &str) -> Result<Self, PolicyError> {
        let domain = domain.trim().trim_start_matches('@').to_lowercase();
        normalize_address(&format!("x@{domain}"))?;
        Ok(Self::Domain(domain))
    }

    pub fn regex(source: &str) -> Result<Self, PolicyError> {
        Ok(Self::Regex(Pattern::full(source)?))
    }

    /// `addr` must already be normalized (see `reins_proto::normalize_address`).
    #[must_use]
    pub fn matches(&self, addr: &str) -> bool {
        match self {
            Self::Exact(a) => a == addr,
            Self::Domain(d) => addr.rsplit_once('@').is_some_and(|(_, domain)| domain == d),
            Self::Regex(p) => p.is_match(addr),
        }
    }
}

impl TryFrom<AddrRuleRepr> for AddrRule {
    type Error = PolicyError;

    fn try_from(repr: AddrRuleRepr) -> Result<Self, Self::Error> {
        match repr {
            AddrRuleRepr::Exact(a) => Self::exact(&a),
            AddrRuleRepr::Domain(d) => Self::domain(&d),
            AddrRuleRepr::Regex(r) => Self::regex(&r),
        }
    }
}

impl From<AddrRule> for AddrRuleRepr {
    fn from(rule: AddrRule) -> Self {
        match rule {
            AddrRule::Exact(a) => Self::Exact(a),
            AddrRule::Domain(d) => Self::Domain(d),
            AddrRule::Regex(p) => Self::Regex(p.source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;

    #[test]
    fn find_searches_case_insensitively() {
        let p = Pattern::find("invoice").unwrap();
        assert!(p.is_match("Your INVOICE #42"));
        assert!(!p.is_match("receipt"));
    }

    #[test]
    fn full_requires_whole_input() {
        let p = Pattern::full("bank").unwrap();
        assert!(p.is_match("BANK"));
        assert!(!p.is_match("notbank"));
        let alt = Pattern::full("a|ab").unwrap();
        assert!(alt.is_match("ab"), "alternation must be anchored as a group");
    }

    #[test]
    fn full_rejects_wrapper_escape() {
        // Would compile to ^(?:a)|(.*)$ and match everything if not validated alone.
        assert!(Pattern::full("a)|(.*").is_err());
        assert!(Pattern::full("a\\").is_err());
    }

    #[test]
    fn rejects_empty_long_and_invalid() {
        assert!(Pattern::find("").is_err());
        assert!(Pattern::find(&"a".repeat(MAX_PATTERN_LEN + 1)).is_err());
        assert!(Pattern::find("(").is_err());
        assert!(Pattern::find("a{100000}").is_err(), "compiled size limit");
    }

    #[test]
    fn pathological_pattern_is_linear() {
        let p = Pattern::full("(a+)+$").unwrap();
        let input = format!("{}b", "a".repeat(100_000));
        let start = Instant::now();
        assert!(!p.is_match(&input));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn pattern_serde_round_trip() {
        let p = Pattern::find("inv.*").unwrap();
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v, json!("inv.*"));
        assert_eq!(serde_json::from_value::<Pattern>(v).unwrap(), p);
        assert!(serde_json::from_value::<Pattern>(json!("(")).is_err());
    }

    #[test]
    fn exact_rule_normalizes() {
        let r = AddrRule::exact(" Alice@Bank.COM ").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(AddrRule::exact("Bob <bob@x.com>").is_err());
    }

    #[test]
    fn domain_rule_is_exact_domain() {
        let r = AddrRule::domain("@Bank.com").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(!r.matches("alice@sub.bank.com"));
        assert!(!r.matches("alice@bank.com.evil.com"));
        assert!(!r.matches("alice@notbank.com"));
        assert!(AddrRule::domain("bank").is_err());
        assert!(AddrRule::domain("a@bank.com").is_err());
    }

    #[test]
    fn regex_rule_is_full_match() {
        let r = AddrRule::regex(r".*@bank\.com").unwrap();
        assert!(r.matches("alice@bank.com"));
        assert!(!r.matches("alice@bank.com.evil.com"));
        assert!(!AddrRule::regex("bank").unwrap().matches("notbank@x.com"));
    }

    #[test]
    fn addr_rule_serde() {
        let r = AddrRule::domain("bank.com").unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"kind": "domain", "value": "bank.com"}));
        assert_eq!(serde_json::from_value::<AddrRule>(v).unwrap(), r);
        assert!(serde_json::from_value::<AddrRule>(json!({"kind": "exact", "value": "not an address"})).is_err());
        assert!(serde_json::from_value::<AddrRule>(json!({"kind": "regex", "value": "a)|(.*"})).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reins-policy pattern`
Expected: FAIL — `not yet implemented`.

- [ ] **Step 3: Implement `find`/`full`**

Replace the two `todo!` bodies and add `build`:

```rust
    pub fn find(source: &str) -> Result<Self, PolicyError> {
        Self::build(source, source)
    }

    pub fn full(source: &str) -> Result<Self, PolicyError> {
        // Validate the source on its own first: only a balanced, standalone
        // pattern can be safely wrapped in a group without escaping it.
        Self::build(source, source)?;
        Self::build(source, &format!("^(?:{source})$"))
    }

    fn build(source: &str, compiled: &str) -> Result<Self, PolicyError> {
        let error = |reason: String| PolicyError::InvalidPattern {
            pattern: source.to_owned(),
            reason,
        };
        if source.is_empty() || source.len() > MAX_PATTERN_LEN {
            return Err(error(format!("length must be 1..={MAX_PATTERN_LEN}")));
        }
        let regex = RegexBuilder::new(compiled)
            .case_insensitive(true)
            .size_limit(COMPILED_SIZE_LIMIT)
            .dfa_size_limit(COMPILED_SIZE_LIMIT)
            .build()
            .map_err(|e| error(e.to_string()))?;
        Ok(Self {
            source: source.to_owned(),
            regex,
        })
    }
```

- [ ] **Step 4: Run tests, clippy, fmt**

Run: `cargo test -p reins-policy && cargo clippy -p reins-policy --all-targets -- -D warnings && cargo fmt -p reins-policy --check`
Expected: all pass, clean.

- [ ] **Step 5: Commit**

```bash
git add crates/reins-policy
git commit -m "feat(policy): validated patterns and address rules"
```

---

### Task 4: Scopes and message facts

**Files:**
- Create: `crates/reins-policy/src/scope.rs`
- Modify: `crates/reins-policy/src/lib.rs` (add `mod scope;` and `pub use scope::{MessageFacts, ReadScope, Scope, SendScope};`)

**Interfaces:**
- Consumes: `Pattern`, `AddrRule`, `PolicyError` (Task 3); `reins_proto::gmail::OutgoingEmail` (Task 1)
- Produces:
  - `MessageFacts { id: String, from: String, to: Vec<String>, subject: String, body: Option<String>, labels: Vec<String>, date: i64 }` — addresses normalized; `to` holds To+Cc; `body` is `None` when not fetched.
  - `ReadScope { message_ids: Option<BTreeSet<String>>, from: Vec<AddrRule>, to: Vec<AddrRule>, subject: Option<Pattern>, body: Option<Pattern>, labels: Vec<String>, after: Option<i64>, before: Option<i64> }` (derives `Default`) with `validate(&self) -> Result<(), PolicyError>`, `matches(&self, &MessageFacts) -> bool`, `needs_body(&self) -> bool`
  - `SendScope { recipients: Vec<AddrRule>, subject: Option<Pattern>, body: Option<Pattern> }` with `validate`, `matches(&self, &OutgoingEmail) -> bool`
  - `Scope = Read(ReadScope) | Send(SendScope)`, serde tag `"action"` (`"read"`/`"send"`), `validate(&self)`

- [ ] **Step 1: Write types and failing tests**

`crates/reins-policy/src/scope.rs`:

```rust
use std::collections::BTreeSet;

use reins_proto::gmail::OutgoingEmail;
use serde::{Deserialize, Serialize};

use crate::{AddrRule, Pattern, PolicyError};

/// What the policy engine knows about one real message, taken from Gmail on
/// the phone. Never derived from anything the AI supplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageFacts {
    pub id: String,
    /// Normalized sender address.
    pub from: String,
    /// Normalized To and Cc addresses.
    pub to: Vec<String>,
    pub subject: String,
    /// Plain-text body, `None` when it was not fetched.
    pub body: Option<String>,
    /// Gmail label ids (e.g. `INBOX`, `Label_12`).
    pub labels: Vec<String>,
    /// Unix seconds.
    pub date: i64,
}

/// Which messages a read grant covers. Every set field must match (AND);
/// within a list field any entry may match (OR).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_ids: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<AddrRule>,
    /// Matches if any To/Cc address matches any rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<AddrRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Inclusive lower bound, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    /// Exclusive upper bound, unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
}

/// Which emails a send grant covers. Every recipient must be covered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendScope {
    pub recipients: Vec<AddrRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Pattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Pattern>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Scope {
    Read(ReadScope),
    Send(SendScope),
}

impl ReadScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        todo!()
    }

    /// False for an unconstrained scope, so a damaged stored grant matches nothing.
    #[must_use]
    pub fn matches(&self, m: &MessageFacts) -> bool {
        todo!("{m:?}")
    }

    #[must_use]
    pub fn needs_body(&self) -> bool {
        self.body.is_some()
    }

    fn is_constrained(&self) -> bool {
        self.message_ids.is_some()
            || !self.from.is_empty()
            || !self.to.is_empty()
            || self.subject.is_some()
            || self.body.is_some()
            || !self.labels.is_empty()
            || self.after.is_some()
            || self.before.is_some()
    }
}

impl SendScope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        todo!()
    }

    /// `email` must be normalized (`OutgoingEmail::normalized`).
    #[must_use]
    pub fn matches(&self, email: &OutgoingEmail) -> bool {
        todo!("{email:?}")
    }
}

impl Scope {
    pub fn validate(&self) -> Result<(), PolicyError> {
        match self {
            Self::Read(s) => s.validate(),
            Self::Send(s) => s.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn msg(id: &str, from: &str, to: &[&str], subject: &str) -> MessageFacts {
        MessageFacts {
            id: id.to_owned(),
            from: from.to_owned(),
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            subject: subject.to_owned(),
            body: None,
            labels: vec!["INBOX".to_owned()],
            date: 100,
        }
    }

    fn email(to: &[&str], subject: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: subject.to_owned(),
            body: "body".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn empty_read_scope_is_invalid_and_matches_nothing() {
        let s = ReadScope::default();
        assert!(s.validate().is_err());
        assert!(!s.matches(&msg("m1", "a@b.com", &[], "x")));
        let damaged: ReadScope = serde_json::from_value(json!({})).unwrap();
        assert!(!damaged.matches(&msg("m1", "a@b.com", &[], "x")));
    }

    #[test]
    fn message_ids_restrict() {
        let s = ReadScope { message_ids: Some(["m1".to_owned()].into()), ..ReadScope::default() };
        assert!(s.validate().is_ok());
        assert!(s.matches(&msg("m1", "a@b.com", &[], "x")));
        assert!(!s.matches(&msg("m2", "a@b.com", &[], "x")));
        let empty = ReadScope { message_ids: Some(BTreeSet::new()), ..ReadScope::default() };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn fields_are_anded_lists_are_ored() {
        let s = ReadScope {
            from: vec![AddrRule::domain("bank.com").unwrap(), AddrRule::exact("boss@work.com").unwrap()],
            subject: Some(Pattern::find("statement").unwrap()),
            ..ReadScope::default()
        };
        assert!(s.matches(&msg("1", "a@bank.com", &[], "Your Statement")));
        assert!(s.matches(&msg("2", "boss@work.com", &[], "statement")));
        assert!(!s.matches(&msg("3", "a@bank.com", &[], "hello")));
        assert!(!s.matches(&msg("4", "a@evil.com", &[], "statement")));
    }

    #[test]
    fn to_matches_any_recipient() {
        let s = ReadScope { to: vec![AddrRule::exact("me@work.com").unwrap()], ..ReadScope::default() };
        assert!(s.matches(&msg("1", "x@y.com", &["a@b.com", "me@work.com"], "s")));
        assert!(!s.matches(&msg("2", "x@y.com", &["a@b.com"], "s")));
    }

    #[test]
    fn body_pattern_requires_fetched_body() {
        let s = ReadScope { body: Some(Pattern::find("otp").unwrap()), ..ReadScope::default() };
        assert!(s.needs_body());
        let mut m = msg("1", "a@b.com", &[], "s");
        assert!(!s.matches(&m), "missing body must not match");
        m.body = Some("your OTP is 1234".to_owned());
        assert!(s.matches(&m));
    }

    #[test]
    fn labels_and_date_bounds() {
        let s = ReadScope {
            labels: vec!["INBOX".to_owned()],
            after: Some(100),
            before: Some(200),
            ..ReadScope::default()
        };
        let mut m = msg("1", "a@b.com", &[], "s");
        assert!(s.matches(&m), "after is inclusive");
        m.date = 200;
        assert!(!s.matches(&m), "before is exclusive");
        m.date = 150;
        m.labels = vec!["SPAM".to_owned()];
        assert!(!s.matches(&m));
        let inverted = ReadScope { after: Some(5), before: Some(5), ..ReadScope::default() };
        assert!(inverted.validate().is_err());
        let blank_label = ReadScope { labels: vec![" ".to_owned()], ..ReadScope::default() };
        assert!(blank_label.validate().is_err());
    }

    #[test]
    fn send_requires_every_recipient_covered() {
        let s = SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        };
        assert!(s.validate().is_ok());
        assert!(s.matches(&email(&["a@work.com", "b@work.com"], "x")));
        let mut mixed = email(&["a@work.com"], "x");
        mixed.cc = vec!["eve@evil.com".to_owned()];
        assert!(!s.matches(&mixed));
        assert!(!s.matches(&email(&[], "x")), "no recipients never matches");
    }

    #[test]
    fn send_subject_and_body_patterns() {
        let s = SendScope {
            recipients: vec![AddrRule::exact("a@work.com").unwrap()],
            subject: Some(Pattern::find("^weekly report").unwrap()),
            body: Some(Pattern::find("regards").unwrap()),
        };
        let mut e = email(&["a@work.com"], "Weekly report 12");
        e.body = "Kind regards".to_owned();
        assert!(s.matches(&e));
        e.subject = "Other".to_owned();
        assert!(!s.matches(&e));
    }

    #[test]
    fn empty_send_scope_invalid_and_matches_nothing() {
        let s = SendScope { recipients: vec![], subject: None, body: None };
        assert!(s.validate().is_err());
        assert!(!s.matches(&email(&["a@b.com"], "x")));
    }

    #[test]
    fn scope_wire_format() {
        let s = Scope::Send(SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        });
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v, json!({"action": "send", "recipients": [{"kind": "domain", "value": "work.com"}]}));
        assert_eq!(serde_json::from_value::<Scope>(v).unwrap(), s);
    }
}
```

In `lib.rs` add after `mod pattern;`:

```rust
mod scope;
```

and after the existing `pub use`:

```rust
pub use scope::{MessageFacts, ReadScope, Scope, SendScope};
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p reins-policy scope`
Expected: FAIL — `not yet implemented` (only `scope_wire_format` passes).

- [ ] **Step 3: Implement validation and matching**

Replace the four `todo!` bodies:

```rust
    // ReadScope
    pub fn validate(&self) -> Result<(), PolicyError> {
        if !self.is_constrained() {
            return Err(PolicyError::InvalidScope("a read scope needs at least one constraint".to_owned()));
        }
        if self.message_ids.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err(PolicyError::InvalidScope("message_ids must not be empty".to_owned()));
        }
        if self.labels.iter().any(|l| l.trim().is_empty()) {
            return Err(PolicyError::InvalidScope("labels must not be blank".to_owned()));
        }
        if let (Some(after), Some(before)) = (self.after, self.before)
            && after >= before
        {
            return Err(PolicyError::InvalidScope("after must be earlier than before".to_owned()));
        }
        Ok(())
    }

    pub fn matches(&self, m: &MessageFacts) -> bool {
        self.is_constrained()
            && self.message_ids.as_ref().is_none_or(|ids| ids.contains(&m.id))
            && (self.from.is_empty() || self.from.iter().any(|r| r.matches(&m.from)))
            && (self.to.is_empty() || m.to.iter().any(|a| self.to.iter().any(|r| r.matches(a))))
            && self.subject.as_ref().is_none_or(|p| p.is_match(&m.subject))
            && self.body.as_ref().is_none_or(|p| m.body.as_deref().is_some_and(|b| p.is_match(b)))
            && (self.labels.is_empty() || m.labels.iter().any(|l| self.labels.contains(l)))
            && self.after.is_none_or(|t| m.date >= t)
            && self.before.is_none_or(|t| m.date < t)
    }
```

```rust
    // SendScope
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.recipients.is_empty() {
            return Err(PolicyError::InvalidScope("a send scope needs at least one recipient rule".to_owned()));
        }
        Ok(())
    }

    pub fn matches(&self, email: &OutgoingEmail) -> bool {
        !self.recipients.is_empty()
            && email.recipients().next().is_some()
            && email.recipients().all(|a| self.recipients.iter().any(|r| r.matches(a)))
            && self.subject.as_ref().is_none_or(|p| p.is_match(&email.subject))
            && self.body.as_ref().is_none_or(|p| p.is_match(&email.body))
    }
```

(`if let … && …` let-chains are stable in edition 2024.)

- [ ] **Step 4: Run tests, clippy, fmt**

Run: `cargo test -p reins-policy && cargo clippy -p reins-policy --all-targets -- -D warnings && cargo fmt -p reins-policy --check`
Expected: all pass, clean.

- [ ] **Step 5: Commit**

```bash
git add crates/reins-policy
git commit -m "feat(policy): read/send scopes over real message facts"
```

---

### Task 5: Grants and evaluation

**Files:**
- Create: `crates/reins-policy/src/grant.rs`, `crates/reins-policy/src/evaluate.rs`
- Modify: `crates/reins-policy/src/lib.rs`

**Interfaces:**
- Consumes: `Scope`, `ReadScope`, `SendScope`, `MessageFacts` (Task 4); `reins_proto::ids::{GrantId, ConnectionId}`, `reins_proto::gmail::OutgoingEmail`
- Produces:
  - `Grant { id: GrantId, connection_id: ConnectionId, scope: Scope, created_at: i64, expires_at: Option<i64>, max_uses: Option<u32>, uses: u32, revoked: bool }` (serde)
  - `Grant::new(id, connection_id, scope, created_at: i64, expires_at: Option<i64>, max_uses: Option<u32>) -> Result<Grant, PolicyError>`
  - `Grant::is_active(&self, now: i64) -> bool`, `Grant::record_use(&mut self)`
  - `record_uses(grants: &mut [Grant], used: &BTreeSet<GrantId>)`
  - `ReadDecision { allowed: Vec<(usize, GrantId)>, needs_approval: Vec<usize> }` with `fully_allowed()`, `grants_used() -> BTreeSet<GrantId>`
  - `SendDecision = Allowed(GrantId) | NeedsApproval`
  - `evaluate_read(&[Grant], &ConnectionId, &[MessageFacts], now: i64) -> ReadDecision`
  - `evaluate_send(&[Grant], &ConnectionId, &OutgoingEmail, now: i64) -> SendDecision`
  - `needs_body(&[Grant], &ConnectionId, now: i64) -> bool`

- [ ] **Step 1: Write grant module with failing tests**

`crates/reins-policy/src/grant.rs`:

```rust
use std::collections::BTreeSet;

use reins_proto::ids::{ConnectionId, GrantId};
use serde::{Deserialize, Serialize};

use crate::{PolicyError, Scope};

/// A standing permission for one AI connection, created on the phone.
/// No `expires_at` and no `max_uses` means it lasts until revoked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: GrantId,
    pub connection_id: ConnectionId,
    pub scope: Scope,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; the grant is dead at and after this instant.
    pub expires_at: Option<i64>,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub revoked: bool,
}

impl Grant {
    pub fn new(
        id: GrantId,
        connection_id: ConnectionId,
        scope: Scope,
        created_at: i64,
        expires_at: Option<i64>,
        max_uses: Option<u32>,
    ) -> Result<Self, PolicyError> {
        todo!("{id}{connection_id}{scope:?}{created_at}{expires_at:?}{max_uses:?}")
    }

    #[must_use]
    pub fn is_active(&self, now: i64) -> bool {
        todo!("{now}")
    }

    pub fn record_use(&mut self) {
        self.uses = self.uses.saturating_add(1);
    }

    pub(crate) fn applies_to(&self, connection: &ConnectionId, now: i64) -> bool {
        &self.connection_id == connection && self.is_active(now)
    }
}

/// Consumes one use of each grant a completed tool call relied on.
pub fn record_uses(grants: &mut [Grant], used: &BTreeSet<GrantId>) {
    for grant in grants.iter_mut().filter(|g| used.contains(&g.id)) {
        grant.record_use();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddrRule, SendScope};

    fn scope() -> Scope {
        Scope::Send(SendScope {
            recipients: vec![AddrRule::domain("work.com").unwrap()],
            subject: None,
            body: None,
        })
    }

    fn grant(expires_at: Option<i64>, max_uses: Option<u32>) -> Grant {
        Grant::new("g1".into(), "c1".into(), scope(), 100, expires_at, max_uses).unwrap()
    }

    #[test]
    fn new_validates() {
        assert!(Grant::new("g".into(), "c".into(), scope(), 100, Some(100), None).is_err());
        assert!(Grant::new("g".into(), "c".into(), scope(), 100, None, Some(0)).is_err());
        let empty = Scope::Send(SendScope { recipients: vec![], subject: None, body: None });
        assert!(Grant::new("g".into(), "c".into(), empty, 100, None, None).is_err());
        let g = grant(None, None);
        assert_eq!((g.uses, g.revoked), (0, false));
    }

    #[test]
    fn expiry_boundary() {
        let g = grant(Some(200), None);
        assert!(g.is_active(199));
        assert!(!g.is_active(200));
        assert!(!g.is_active(201));
    }

    #[test]
    fn use_limit() {
        let mut g = grant(None, Some(2));
        assert!(g.is_active(150));
        g.record_use();
        assert!(g.is_active(150));
        g.record_use();
        assert!(!g.is_active(150));
    }

    #[test]
    fn revocation_and_until_revoked() {
        let mut g = grant(None, None);
        assert!(g.is_active(i64::MAX));
        g.revoked = true;
        assert!(!g.is_active(150));
    }

    #[test]
    fn record_uses_only_touches_used() {
        let mut gs = vec![grant(None, Some(5)), Grant { id: "g2".into(), ..grant(None, Some(5)) }];
        record_uses(&mut gs, &["g2".into()].into());
        assert_eq!((gs[0].uses, gs[1].uses), (0, 1));
    }
}
```

- [ ] **Step 2: Wire modules and run to verify failure**

In `lib.rs` add `mod grant;` and `pub use grant::{Grant, record_uses};`.

Run: `cargo test -p reins-policy grant`
Expected: FAIL — `not yet implemented`.

- [ ] **Step 3: Implement `Grant::new` and `is_active`**

```rust
    pub fn new(
        id: GrantId,
        connection_id: ConnectionId,
        scope: Scope,
        created_at: i64,
        expires_at: Option<i64>,
        max_uses: Option<u32>,
    ) -> Result<Self, PolicyError> {
        scope.validate()?;
        if expires_at.is_some_and(|t| t <= created_at) {
            return Err(PolicyError::InvalidGrant("expires_at must be after created_at".to_owned()));
        }
        if max_uses == Some(0) {
            return Err(PolicyError::InvalidGrant("max_uses must be at least 1".to_owned()));
        }
        Ok(Self {
            id,
            connection_id,
            scope,
            created_at,
            expires_at,
            max_uses,
            uses: 0,
            revoked: false,
        })
    }

    pub fn is_active(&self, now: i64) -> bool {
        !self.revoked
            && self.expires_at.is_none_or(|t| now < t)
            && self.max_uses.is_none_or(|m| self.uses < m)
    }
```

Run: `cargo test -p reins-policy grant` → PASS.

- [ ] **Step 4: Write evaluate module with failing tests**

`crates/reins-policy/src/evaluate.rs`:

```rust
use std::collections::BTreeSet;

use reins_proto::gmail::OutgoingEmail;
use reins_proto::ids::{ConnectionId, GrantId};

use crate::{Grant, MessageFacts, Scope};

/// Per-message outcome of a read or search.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReadDecision {
    /// `(index into the evaluated messages, grant that covers it)`.
    pub allowed: Vec<(usize, GrantId)>,
    /// Indices of messages no active grant covers.
    pub needs_approval: Vec<usize>,
}

impl ReadDecision {
    #[must_use]
    pub fn fully_allowed(&self) -> bool {
        self.needs_approval.is_empty()
    }

    #[must_use]
    pub fn grants_used(&self) -> BTreeSet<GrantId> {
        self.allowed.iter().map(|(_, g)| g.clone()).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendDecision {
    Allowed(GrantId),
    NeedsApproval,
}

/// Active grants of `connection`, unlimited ones first, then by id.
fn candidates<'g>(grants: &'g [Grant], connection: &ConnectionId, now: i64) -> Vec<&'g Grant> {
    let mut active: Vec<&Grant> = grants.iter().filter(|g| g.applies_to(connection, now)).collect();
    active.sort_by(|a, b| (a.max_uses.is_some(), &a.id).cmp(&(b.max_uses.is_some(), &b.id)));
    active
}

#[must_use]
pub fn evaluate_read(
    grants: &[Grant],
    connection: &ConnectionId,
    messages: &[MessageFacts],
    now: i64,
) -> ReadDecision {
    todo!("{grants:?}{connection}{messages:?}{now}")
}

/// `email` must be normalized (`OutgoingEmail::normalized`).
#[must_use]
pub fn evaluate_send(grants: &[Grant], connection: &ConnectionId, email: &OutgoingEmail, now: i64) -> SendDecision {
    todo!("{grants:?}{connection}{email:?}{now}")
}

/// Whether message bodies must be fetched before `evaluate_read` can decide.
#[must_use]
pub fn needs_body(grants: &[Grant], connection: &ConnectionId, now: i64) -> bool {
    candidates(grants, connection, now)
        .iter()
        .any(|g| matches!(&g.scope, Scope::Read(s) if s.needs_body()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddrRule, Pattern, ReadScope, SendScope};

    const NOW: i64 = 1_000;

    fn read_grant(id: &str, conn: &str, scope: ReadScope, max_uses: Option<u32>) -> Grant {
        Grant::new(id.into(), conn.into(), Scope::Read(scope), 0, None, max_uses).unwrap()
    }

    fn from_domain(d: &str) -> ReadScope {
        ReadScope { from: vec![AddrRule::domain(d).unwrap()], ..ReadScope::default() }
    }

    fn msg(id: &str, from: &str) -> MessageFacts {
        MessageFacts {
            id: id.to_owned(),
            from: from.to_owned(),
            to: vec![],
            subject: "s".to_owned(),
            body: None,
            labels: vec![],
            date: 10,
        }
    }

    fn email(to: &[&str]) -> OutgoingEmail {
        OutgoingEmail {
            to: to.iter().map(|s| (*s).to_owned()).collect(),
            cc: vec![],
            subject: "s".to_owned(),
            body: "b".to_owned(),
            reply_to_message_id: None,
        }
    }

    #[test]
    fn partial_read() {
        let grants = [read_grant("g1", "c1", from_domain("bank.com"), None)];
        let msgs = [msg("m1", "a@bank.com"), msg("m2", "eve@evil.com")];
        let d = evaluate_read(&grants, &"c1".into(), &msgs, NOW);
        assert_eq!(d.allowed, vec![(0, "g1".into())]);
        assert_eq!(d.needs_approval, vec![1]);
        assert!(!d.fully_allowed());
    }

    #[test]
    fn other_connection_never_applies() {
        let grants = [read_grant("g1", "c2", from_domain("bank.com"), None)];
        let d = evaluate_read(&grants, &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
        assert!(d.allowed.is_empty());
    }

    #[test]
    fn inactive_grants_ignored() {
        let mut expired = read_grant("g1", "c1", from_domain("bank.com"), None);
        expired.expires_at = Some(NOW);
        let mut exhausted = read_grant("g2", "c1", from_domain("bank.com"), Some(1));
        exhausted.uses = 1;
        let mut revoked = read_grant("g3", "c1", from_domain("bank.com"), None);
        revoked.revoked = true;
        let d = evaluate_read(&[expired, exhausted, revoked], &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
    }

    #[test]
    fn prefers_unlimited_grant() {
        let grants = [
            read_grant("a-limited", "c1", from_domain("bank.com"), Some(3)),
            read_grant("z-unlimited", "c1", from_domain("bank.com"), None),
        ];
        let d = evaluate_read(&grants, &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.grants_used(), ["z-unlimited".into()].into());
    }

    #[test]
    fn send_grant_does_not_cover_reads() {
        let send = Grant::new(
            "g1".into(),
            "c1".into(),
            Scope::Send(SendScope { recipients: vec![AddrRule::domain("bank.com").unwrap()], subject: None, body: None }),
            0,
            None,
            None,
        )
        .unwrap();
        let d = evaluate_read(&[send], &"c1".into(), &[msg("m1", "a@bank.com")], NOW);
        assert_eq!(d.needs_approval, vec![0]);
    }

    #[test]
    fn send_decisions() {
        let g = Grant::new(
            "g1".into(),
            "c1".into(),
            Scope::Send(SendScope { recipients: vec![AddrRule::domain("work.com").unwrap()], subject: None, body: None }),
            0,
            None,
            None,
        )
        .unwrap();
        let grants = [g, read_grant("g2", "c1", from_domain("work.com"), None)];
        assert_eq!(evaluate_send(&grants, &"c1".into(), &email(&["a@work.com"]), NOW), SendDecision::Allowed("g1".into()));
        assert_eq!(evaluate_send(&grants, &"c1".into(), &email(&["a@evil.com"]), NOW), SendDecision::NeedsApproval);
        assert_eq!(evaluate_send(&grants, &"c2".into(), &email(&["a@work.com"]), NOW), SendDecision::NeedsApproval);
    }

    #[test]
    fn needs_body_only_for_active_body_grants() {
        let body = ReadScope { body: Some(Pattern::find("otp").unwrap()), ..ReadScope::default() };
        let grants = [read_grant("g1", "c1", body, None)];
        assert!(needs_body(&grants, &"c1".into(), NOW));
        assert!(!needs_body(&grants, &"c2".into(), NOW));
    }
}
```

In `lib.rs` add `mod evaluate;` and `pub use evaluate::{ReadDecision, SendDecision, evaluate_read, evaluate_send, needs_body};`.

Run: `cargo test -p reins-policy evaluate`
Expected: FAIL — `not yet implemented` (only `needs_body_only_for_active_body_grants` passes).

- [ ] **Step 5: Implement evaluation**

```rust
pub fn evaluate_read(
    grants: &[Grant],
    connection: &ConnectionId,
    messages: &[MessageFacts],
    now: i64,
) -> ReadDecision {
    let reads: Vec<(&Grant, &crate::ReadScope)> = candidates(grants, connection, now)
        .into_iter()
        .filter_map(|g| match &g.scope {
            Scope::Read(s) => Some((g, s)),
            Scope::Send(_) => None,
        })
        .collect();
    let mut decision = ReadDecision::default();
    for (index, message) in messages.iter().enumerate() {
        match reads.iter().find(|(_, scope)| scope.matches(message)) {
            Some((grant, _)) => decision.allowed.push((index, grant.id.clone())),
            None => decision.needs_approval.push(index),
        }
    }
    decision
}

pub fn evaluate_send(grants: &[Grant], connection: &ConnectionId, email: &OutgoingEmail, now: i64) -> SendDecision {
    candidates(grants, connection, now)
        .into_iter()
        .find(|g| matches!(&g.scope, Scope::Send(s) if s.matches(email)))
        .map_or(SendDecision::NeedsApproval, |g| SendDecision::Allowed(g.id.clone()))
}
```

(If `unused_qualifications` flags `crate::ReadScope`, import `ReadScope` at the top instead.)

- [ ] **Step 6: Run tests, clippy, fmt**

Run: `cargo test -p reins-policy && cargo clippy -p reins-policy --all-targets -- -D warnings && cargo fmt -p reins-policy --check`
Expected: all pass, clean.

- [ ] **Step 7: Commit**

```bash
git add crates/reins-policy
git commit -m "feat(policy): grants and read/send evaluation"
```

---

### Task 6: Property tests for soundness and completeness

**Files:**
- Create: `crates/reins-policy/tests/properties.rs`

**Interfaces:**
- Consumes: the public API of `reins-policy` and `reins-proto` (Tasks 1–5). Nothing new is produced.

- [ ] **Step 1: Write the property tests**

`crates/reins-policy/tests/properties.rs`:

```rust
//! Randomized checks of the security properties the phone relies on.

use proptest::collection::{btree_set, vec};
use proptest::option;
use proptest::prelude::*;
use proptest::sample::select;
use reins_policy::{
    AddrRule, Grant, MessageFacts, Pattern, ReadScope, Scope, SendDecision, SendScope, evaluate_read, evaluate_send,
};
use reins_proto::gmail::OutgoingEmail;
use reins_proto::ids::{ConnectionId, GrantId};

const LOCALS: [&str; 3] = ["alice", "bob", "eve"];
const DOMAINS: [&str; 4] = ["bank.com", "evil.com", "sub.bank.com", "bank.com.evil.com"];
const CONNECTIONS: [&str; 2] = ["c1", "c2"];

fn addr() -> impl Strategy<Value = String> {
    (select(LOCALS.to_vec()), select(DOMAINS.to_vec())).prop_map(|(l, d)| format!("{l}@{d}"))
}

fn addr_rule() -> impl Strategy<Value = AddrRule> {
    prop_oneof![
        addr().prop_map(|a| AddrRule::exact(&a).unwrap()),
        select(DOMAINS.to_vec()).prop_map(|d| AddrRule::domain(d).unwrap()),
        select(vec![r".*@bank\.com", "alice@.*", "b.*"]).prop_map(|r| AddrRule::regex(r).unwrap()),
    ]
}

fn pattern() -> impl Strategy<Value = Pattern> {
    select(vec!["invoice", "^your", "statement|otp"]).prop_map(|p| Pattern::find(p).unwrap())
}

fn read_scope() -> impl Strategy<Value = ReadScope> {
    (
        option::of(btree_set(select(vec!["m1", "m2", "m3"]).prop_map(str::to_owned), 1..3)),
        vec(addr_rule(), 0..2),
        vec(addr_rule(), 0..2),
        option::of(pattern()),
        option::of(pattern()),
        vec(select(vec!["INBOX", "Label_1"]).prop_map(str::to_owned), 0..2),
        option::of(0i64..100),
        option::of(50i64..150),
    )
        .prop_filter_map("valid read scope", |(message_ids, from, to, subject, body, labels, after, before)| {
            let s = ReadScope { message_ids, from, to, subject, body, labels, after, before };
            s.validate().is_ok().then_some(s)
        })
}

fn send_scope() -> impl Strategy<Value = SendScope> {
    (vec(addr_rule(), 1..3), option::of(pattern()), option::of(pattern()))
        .prop_map(|(recipients, subject, body)| SendScope { recipients, subject, body })
}

fn grants() -> impl Strategy<Value = Vec<Grant>> {
    let one = (
        select(CONNECTIONS.to_vec()),
        prop_oneof![read_scope().prop_map(Scope::Read), send_scope().prop_map(Scope::Send)],
        option::of(0i64..200),
        option::of(1u32..3),
        0u32..4,
        any::<bool>(),
    )
        .prop_map(|(conn, scope, expires_at, max_uses, uses, revoked)| Grant {
            id: GrantId::from("unset"),
            connection_id: ConnectionId::from(conn),
            scope,
            created_at: 0,
            expires_at,
            max_uses,
            uses,
            revoked,
        });
    vec(one, 0..6).prop_map(|mut gs| {
        for (i, g) in gs.iter_mut().enumerate() {
            g.id = GrantId(format!("g{i}"));
        }
        gs
    })
}

fn message() -> impl Strategy<Value = MessageFacts> {
    (
        select(vec!["m1", "m2", "m3", "m4"]),
        addr(),
        vec(addr(), 0..3),
        select(vec!["Invoice 42", "hello", "Your statement"]),
        option::of(select(vec!["your otp is 1", "lunch?"])),
        vec(select(vec!["INBOX", "Label_1", "SPAM"]).prop_map(str::to_owned), 0..3),
        0i64..200,
    )
        .prop_map(|(id, from, to, subject, body, labels, date)| MessageFacts {
            id: id.to_owned(),
            from,
            to,
            subject: subject.to_owned(),
            body: body.map(str::to_owned),
            labels,
            date,
        })
}

fn outgoing() -> impl Strategy<Value = OutgoingEmail> {
    (vec(addr(), 1..3), vec(addr(), 0..2), select(vec!["Invoice 42", "hello"]), select(vec!["your otp", "hi"]))
        .prop_map(|(to, cc, subject, body)| OutgoingEmail {
            to,
            cc,
            subject: subject.to_owned(),
            body: body.to_owned(),
            reply_to_message_id: None,
        })
}

fn covers_read(g: &Grant, conn: &ConnectionId, now: i64, m: &MessageFacts) -> bool {
    &g.connection_id == conn && g.is_active(now) && matches!(&g.scope, Scope::Read(s) if s.matches(m))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn read_decisions_are_sound_and_complete(
        grants in grants(),
        msgs in vec(message(), 0..6),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let d = evaluate_read(&grants, &conn, &msgs, now);
        prop_assert_eq!(d.allowed.len() + d.needs_approval.len(), msgs.len());
        for (i, gid) in &d.allowed {
            let g = grants.iter().find(|g| &g.id == gid).expect("allowed by unknown grant");
            prop_assert!(covers_read(g, &conn, now, &msgs[*i]), "grant {} does not cover message {}", gid, i);
        }
        for i in &d.needs_approval {
            prop_assert!(!grants.iter().any(|g| covers_read(g, &conn, now, &msgs[*i])), "message {} was coverable", i);
        }
    }

    #[test]
    fn send_decisions_are_sound_and_complete(
        grants in grants(),
        email in outgoing(),
        now in 0i64..200,
        conn in select(CONNECTIONS.to_vec()),
    ) {
        let conn = ConnectionId::from(conn);
        let covers = |g: &Grant| {
            g.connection_id == conn && g.is_active(now) && matches!(&g.scope, Scope::Send(s) if s.matches(&email))
        };
        match evaluate_send(&grants, &conn, &email, now) {
            SendDecision::Allowed(gid) => {
                let g = grants.iter().find(|g| g.id == gid).expect("allowed by unknown grant");
                prop_assert!(covers(g));
                if let Scope::Send(s) = &g.scope {
                    for r in email.recipients() {
                        prop_assert!(s.recipients.iter().any(|rule| rule.matches(r)), "recipient {} uncovered", r);
                    }
                }
            }
            SendDecision::NeedsApproval => prop_assert!(!grants.iter().any(covers)),
        }
    }

    #[test]
    fn serde_round_trip_preserves_decisions(
        grants in grants(),
        msgs in vec(message(), 0..6),
        now in 0i64..200,
    ) {
        let json = serde_json::to_string(&grants).unwrap();
        let restored: Vec<Grant> = serde_json::from_str(&json).unwrap();
        let conn = ConnectionId::from("c1");
        prop_assert_eq!(evaluate_read(&grants, &conn, &msgs, now), evaluate_read(&restored, &conn, &msgs, now));
    }
}
```

- [ ] **Step 2: Run the property tests**

Run: `cargo test -p reins-policy --test properties`
Expected: 3 passed. A failure prints a minimized counterexample — treat it as a real policy bug: fix the engine, never loosen the property.

- [ ] **Step 3: Full gate for both crates**

Run:
```bash
cargo test -p reins-proto -p reins-policy
cargo clippy -p reins-proto -p reins-policy --all-targets -- -D warnings
cargo fmt -p reins-proto -p reins-policy --check
```
Expected: all pass, no findings.

- [ ] **Step 4: Confirm upstream build is unaffected**

Run: `cargo check --features sqlite`
Expected: Vaultwarden itself still compiles (only `Cargo.toml` members and `Cargo.lock` changed upstream-side).

- [ ] **Step 5: Commit**

```bash
git add crates/reins-policy/tests
git commit -m "test(policy): property tests for grant soundness and completeness"
```
