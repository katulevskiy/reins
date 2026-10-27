//! A Team org that signs in with GitHub (08 R11): a member must be an active member of the org's GitHub
//! organization, read with the token GitHub gave WorkOS for this sign-in (WorkOS returns it as `oauth_tokens` when the
//! GitHub connection asks for `read:org`).

use std::sync::{Arc, LazyLock, RwLock};

use serde::Deserialize;

#[rocket::async_trait]
pub trait GithubMembership: Send + Sync {
    /// Whether the token's user is an active member of `org`.
    async fn is_member(&self, token: &str, org: &str) -> Result<bool, String>;
}

/// `GET https://api.github.com/user/memberships/orgs/{org}` as the user.
pub struct GithubApi;

#[derive(Deserialize)]
struct Membership {
    state: String,
}

#[rocket::async_trait]
impl GithubMembership for GithubApi {
    async fn is_member(&self, token: &str, org: &str) -> Result<bool, String> {
        let client = crate::http_client::get_reqwest_client_builder(false)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let mut url = url::Url::parse("https://api.github.com/user/memberships/orgs").map_err(|e| e.to_string())?;
        url.path_segments_mut().map_err(|()| "bad URL".to_owned())?.push(org);
        let response = client
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "Reins")
            .send()
            .await
            .map_err(|e| format!("could not reach GitHub: {e}"))?;
        match response.status().as_u16() {
            200 => Ok(response.json::<Membership>().await.map_err(|e| e.to_string())?.state == "active"),
            // Not a member, or the token may not see the org's memberships.
            403 | 404 => Ok(false),
            status => Err(format!("GitHub answered {status}")),
        }
    }
}

static CHECKER: LazyLock<RwLock<Arc<dyn GithubMembership>>> = LazyLock::new(|| RwLock::new(Arc::new(GithubApi)));

pub fn checker() -> Arc<dyn GithubMembership> {
    Arc::clone(&CHECKER.read().unwrap_or_else(std::sync::PoisonError::into_inner))
}
