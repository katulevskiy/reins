//! Git on the user's computer, through the Rewarden desktop app (see `connector::git` for what every host shares).
//! GitHub looks the repository up with the token and expects the token with the user name `x-access-token`.

use reqwest::Method;
use rewarden_proto::connector::ConnectorCall;
use rewarden_proto::desktop::GIT_FETCH_OP;
use serde_json::Value;

use super::{GitHub, Preview, repo_arg};
use crate::CoreError;
use crate::connector::Item;
use crate::connector::git;

pub(crate) use crate::connector::git::summary_arg;

/// The HTTP basic user name GitHub expects with a token.
const USERNAME: &str = "x-access-token";

/// Lists and reads. `None` when the operation is not one of this area's.
pub(super) async fn fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Option<Result<Vec<Item>, CoreError>> {
    (call.op == GIT_FETCH_OP).then_some(())?;
    Some(git_fetch(gh, token, call).await)
}

/// What a write would do. `None` when the operation is not one of this area's.
pub(super) fn preview(call: &ConnectorCall) -> Option<Result<Preview, CoreError>> {
    git::is_push(call).then(|| git::preview_push(call, &repo_arg(call)?))
}

/// Does the write: hands out the push credential. `None` when the operation is not one of this area's.
pub(super) fn perform(token: &str, call: &ConnectorCall) -> Option<Result<Value, CoreError>> {
    git::is_push(call).then(|| git::perform_push(call, &repo_arg(call)?, USERNAME, token))
}

async fn git_fetch(gh: &GitHub, token: &str, call: &ConnectorCall) -> Result<Vec<Item>, CoreError> {
    let repo = repo_arg(call)?;
    git::check_fetch(call)?;
    // The token must reach the repository; otherwise the usual error says why.
    let found = gh.call(token, Method::GET, &format!("/repos/{repo}"), &[], None).await?;
    let private = found["private"].as_bool() == Some(true);
    Ok(vec![git::fetch_item(call, &repo, private, USERNAME, token)?])
}
