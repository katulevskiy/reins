//! GitLab (gitlab.com). A personal access token with the scopes `read_api` (who it is, looking repositories up),
//! `read_repository` (fetch) and `write_repository` (push). The API takes it as a bearer token; git over HTTPS takes
//! any user name with it, `oauth2` by convention. Repositories may sit in nested groups (`group/subgroup/name`).

use rewarden_proto::connector::GITLAB;
use serde_json::Value;

use super::{Auth, Credentials, Forge, GitHost, plain_token};
use crate::CoreError;

pub struct Lab;

/// GitLab, as a connector.
pub type GitLab = GitHost<Lab>;

impl Forge for Lab {
    const SERVICE: &'static str = GITLAB;
    const NAME: &'static str = "GitLab";
    const NESTED: bool = true;

    fn parse(pasted: &str) -> Result<Credentials, CoreError> {
        plain_token(pasted, "GitLab")
    }

    fn auth(creds: &Credentials) -> Auth<'_> {
        Auth::Bearer(&creds.token)
    }

    fn login(me: &Value) -> Option<String> {
        me["username"].as_str().map(str::to_owned)
    }

    /// `/projects/group%2Fsubgroup%2Fname`: the path, URL-encoded, is the project's id.
    fn repo_path(repo: &str) -> String {
        format!("/projects/{}", repo.replace('/', "%2F"))
    }

    fn private(found: &Value) -> bool {
        found["visibility"].as_str() != Some("public")
    }

    fn git_username(_creds: &Credentials) -> String {
        "oauth2".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn projects_are_found_by_their_encoded_path() {
        assert_eq!(Lab::repo_path("g/s/app"), "/projects/g%2Fs%2Fapp");
        assert!(Lab::private(&json!({"visibility": "internal"})));
        assert!(!Lab::private(&json!({"visibility": "public"})));
    }
}
