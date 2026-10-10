//! Vault items: logins, secure notes, cards, identities and SSH keys; folders, the trash and the archive.
//!
//! Reads (`read`) never hand a secret out except as a `sensitive` + `secret` item. Writes (`write`) are planned once
//! for the preview and again, from a fresh read of the vault, for the change itself; strings are encrypted here with
//! the item's key, and an edit keeps the item's own key and resends everything it did not change.

use reins_proto::connector::ConnectorCall;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::Vault;
use crate::CoreError;
use crate::connector::{Item, Preview};

pub(super) mod desktop;
pub(crate) mod editor;
mod model;
mod read;
pub(crate) mod wallet;
mod write;

/// The largest file an attachment may be, in either direction.
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// The vault as the server sent it, for the parts that read only the raw items.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Sync {
    #[serde(default)]
    ciphers: Vec<Value>,
    #[serde(default)]
    folders: Vec<Value>,
}

/// The host of an address, for a one-line list ("https://github.com/login" → "github.com").
fn host_of(uri: &str) -> String {
    Url::parse(uri).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| uri.to_owned())
}

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    Some(match call.op.as_str() {
        "search" => read::search(vault, account, call).await,
        "folders_list" => read::folders_list(vault, account).await,
        "item_view" => read::item_view(vault, account, call).await,
        "get" => read::get(vault, account, call).await,
        "attachment_get" => read::attachment_get(vault, account, call).await,
        _ => return None,
    })
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) async fn preview(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    Some(write::plan(vault, account, call).await?.map(|(_, plan)| plan.preview))
}

/// Does the write. `None` when the operation is not one of this area's.
pub(super) async fn perform(vault: &Vault, account: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    Some(match write::plan(vault, account, call).await? {
        Ok((session, plan)) => write::carry_out(vault, &session, plan).await,
        Err(e) => Err(e),
    })
}
