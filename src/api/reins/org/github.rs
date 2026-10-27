//! A Team org that signs in with GitHub (08 R11), asked of GitHub with the token GitHub gave WorkOS for this sign-in
//! (WorkOS returns it as `oauth_tokens` when the GitHub connection asks for `read:org`).
//!
//! GitHub's answer has three outcomes, never two: a definite member (and whether an admin), a definite non-member
//! (404, or an `inactive` membership), and **unknown** (403, rate limits, 5xx, a `pending` invitation, no answer).
//! Unknown lets nobody in and removes nobody: removal is the directory's, an admin's, or a sync's that confirms twice.

use std::sync::{Arc, LazyLock, RwLock};

use serde::Deserialize;

/// What GitHub said about one user and one organization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubAnswer {
    Member {
        admin: bool,
    },
    NotMember,
    Unknown,
}

impl GithubAnswer {
    #[must_use]
    pub fn is_member(self) -> bool {
        matches!(self, Self::Member { .. })
    }
}

/// Reads `GET /user/memberships/orgs/{org}`: its status, and on 200 its `state` and `role`.
#[must_use]
pub fn classify(status: u16, state: Option<&str>, role: Option<&str>) -> GithubAnswer {
    match (status, state) {
        (200, Some("active")) => GithubAnswer::Member {
            admin: role == Some("admin"),
        },
        (200, Some("inactive")) | (404, _) => GithubAnswer::NotMember,
        // Pending invitations, 403 (no read:org, SSO enforcement), 429, 5xx, anything else.
        _ => GithubAnswer::Unknown,
    }
}

#[rocket::async_trait]
pub trait GithubMembership: Send + Sync {
    async fn membership(&self, token: &str, org: &str) -> GithubAnswer;
}

/// The GitHub REST API, as the user.
pub struct GithubApi;

#[derive(Deserialize)]
struct Membership {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    role: Option<String>,
}

#[rocket::async_trait]
impl GithubMembership for GithubApi {
    async fn membership(&self, token: &str, org: &str) -> GithubAnswer {
        let Ok(client) =
            crate::http_client::get_reqwest_client_builder(false).redirect(reqwest::redirect::Policy::none()).build()
        else {
            return GithubAnswer::Unknown;
        };
        let Ok(mut url) = url::Url::parse("https://api.github.com/user/memberships/orgs") else {
            return GithubAnswer::Unknown;
        };
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.push(org);
        }
        let response = match client
            .get(url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "Reins")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("GitHub membership check: {e}");
                return GithubAnswer::Unknown;
            }
        };
        let status = response.status().as_u16();
        let body = if status == 200 {
            response.json::<Membership>().await.ok()
        } else {
            None
        };
        classify(status, body.as_ref().and_then(|b| b.state.as_deref()), body.as_ref().and_then(|b| b.role.as_deref()))
    }
}

static CHECKER: LazyLock<RwLock<Arc<dyn GithubMembership>>> = LazyLock::new(|| RwLock::new(Arc::new(GithubApi)));

pub fn checker() -> Arc<dyn GithubMembership> {
    Arc::clone(&CHECKER.read().unwrap_or_else(std::sync::PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_definite_answers_count() {
        assert_eq!(
            classify(200, Some("active"), Some("member")),
            GithubAnswer::Member {
                admin: false
            }
        );
        assert_eq!(
            classify(200, Some("active"), Some("admin")),
            GithubAnswer::Member {
                admin: true
            }
        );
        assert_eq!(classify(404, None, None), GithubAnswer::NotMember);
        assert_eq!(classify(200, Some("inactive"), None), GithubAnswer::NotMember);
        for (status, state) in
            [(403, None), (429, None), (500, None), (502, None), (200, Some("pending")), (200, None), (301, None)]
        {
            assert_eq!(classify(status, state, None), GithubAnswer::Unknown, "{status} {state:?}");
        }
        assert!(!GithubAnswer::Unknown.is_member());
        assert!(!GithubAnswer::NotMember.is_member());
    }
}
