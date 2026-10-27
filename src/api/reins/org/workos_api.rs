//! Proving a tenant's WorkOS binding before it is saved. Every Reins org shares the server's WorkOS environment, so an
//! id typed into the console proves nothing: a tenant could name another customer's WorkOS organization (and get its
//! SSO sign-ins) or directory (and get its SCIM users and groups). A binding counts only when WorkOS itself says:
//!
//! - the WorkOS organization lists one of the tenant's **verified** domains, verified on the WorkOS side too;
//! - the directory belongs to that WorkOS organization.
//!
//! [`VerifiedBinding`] can only be made by [`verify_binding`] (or a test), and `admin::propose`/`admin::approve`
//! refuse an SSO identity without one that matches. The database adds the last word: one tenant per WorkOS
//! organization and per directory (unique columns).

use std::sync::{Arc, LazyLock, RwLock};

use serde::Deserialize;

use super::OrgError;

/// What WorkOS says about its organizations and directories.
#[rocket::async_trait]
pub trait WorkosDirectoryApi: Send + Sync {
    /// The organization's domains that WorkOS verified.
    async fn verified_domains(&self, workos_org_id: &str) -> Result<Vec<String>, String>;
    /// The organization a directory belongs to.
    async fn directory_organization(&self, directory_id: &str) -> Result<String, String>;
}

/// The WorkOS REST API with the server's API key.
pub struct WorkosRest;

#[derive(Deserialize)]
struct OrgDomain {
    domain: String,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
struct Organization {
    #[serde(default)]
    domains: Vec<OrgDomain>,
}

#[derive(Deserialize)]
struct Directory {
    organization_id: Option<String>,
}

async fn get_json<T: for<'de> Deserialize<'de>>(path: &[&str]) -> Result<T, String> {
    let base = crate::sso_workos::api_base().map_err(|e| format!("{e:?}"))?;
    let mut url = url::Url::parse(&base).map_err(|e| e.to_string())?;
    url.path_segments_mut().map_err(|()| "bad WorkOS URL".to_owned())?.extend(path);
    let client = crate::sso_workos::http_client().map_err(|e| format!("{e:?}"))?;
    let response = client
        .get(url)
        .bearer_auth(crate::sso_workos::api_key())
        .send()
        .await
        .map_err(|e| format!("could not reach WorkOS: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("WorkOS answered {status}"));
    }
    response.json::<T>().await.map_err(|e| format!("unexpected WorkOS answer: {e}"))
}

#[rocket::async_trait]
impl WorkosDirectoryApi for WorkosRest {
    async fn verified_domains(&self, workos_org_id: &str) -> Result<Vec<String>, String> {
        let org: Organization = get_json(&["organizations", workos_org_id]).await?;
        Ok(org
            .domains
            .into_iter()
            .filter(|d| d.state.as_deref() == Some("verified"))
            .filter_map(|d| reins_proto::org::normalize_domain(&d.domain))
            .collect())
    }

    async fn directory_organization(&self, directory_id: &str) -> Result<String, String> {
        let dir: Directory = get_json(&["directories", directory_id]).await?;
        dir.organization_id.ok_or_else(|| "the directory has no organization".to_owned())
    }
}

static API: LazyLock<RwLock<Arc<dyn WorkosDirectoryApi>>> = LazyLock::new(|| RwLock::new(Arc::new(WorkosRest)));

pub fn api() -> Arc<dyn WorkosDirectoryApi> {
    Arc::clone(&API.read().unwrap_or_else(std::sync::PoisonError::into_inner))
}

#[cfg(test)]
pub fn set_api(a: Arc<dyn WorkosDirectoryApi>) {
    *API.write().unwrap_or_else(std::sync::PoisonError::into_inner) = a;
}

/// A WorkOS organization (and directory) WorkOS confirmed for a tenant's verified domains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedBinding {
    pub workos_org_id: String,
    pub directory_id: Option<String>,
    _proof: (),
}

impl VerifiedBinding {
    #[cfg(test)]
    pub fn for_tests(workos_org_id: &str, directory_id: Option<&str>) -> Self {
        Self {
            workos_org_id: workos_org_id.to_owned(),
            directory_id: directory_id.map(str::to_owned),
            _proof: (),
        }
    }

    /// Whether this proves exactly `workos_org_id` and `directory_id`.
    #[must_use]
    pub fn proves(&self, workos_org_id: &str, directory_id: Option<&str>) -> bool {
        self.workos_org_id == workos_org_id && self.directory_id.as_deref() == directory_id
    }
}

/// Asks WorkOS whether `workos_org_id` (and `directory_id`) belong with a tenant whose verified domains are
/// `tenant_domains`.
pub async fn verify_binding(
    api: &dyn WorkosDirectoryApi,
    workos_org_id: &str,
    directory_id: Option<&str>,
    tenant_domains: &[String],
) -> Result<VerifiedBinding, OrgError> {
    if tenant_domains.is_empty() {
        return Err(OrgError::Invalid("Verify a domain before connecting SSO."));
    }
    let lookup = |e: String| {
        warn!("WorkOS binding check for {workos_org_id}: {e}");
        OrgError::Invalid("WorkOS didn't confirm that organization. Check the id and try again.")
    };
    let domains = api.verified_domains(workos_org_id).await.map_err(lookup)?;
    if !domains.iter().any(|d| tenant_domains.contains(d)) {
        return Err(OrgError::Invalid(
            "That WorkOS organization doesn't have one of this organization's verified domains.",
        ));
    }
    if let Some(dir) = directory_id {
        let owner = api.directory_organization(dir).await.map_err(lookup)?;
        if owner != workos_org_id {
            return Err(OrgError::Invalid("That directory belongs to another WorkOS organization."));
        }
    }
    Ok(VerifiedBinding {
        workos_org_id: workos_org_id.to_owned(),
        directory_id: directory_id.map(str::to_owned),
        _proof: (),
    })
}

#[cfg(test)]
pub mod fake {
    use std::collections::HashMap;

    use super::*;

    /// WorkOS as a test describes it: organizations' verified domains and directories' organizations.
    #[derive(Default)]
    pub struct FakeWorkos {
        pub domains: HashMap<String, Vec<String>>,
        pub directories: HashMap<String, String>,
    }

    #[rocket::async_trait]
    impl WorkosDirectoryApi for FakeWorkos {
        async fn verified_domains(&self, workos_org_id: &str) -> Result<Vec<String>, String> {
            self.domains.get(workos_org_id).cloned().ok_or_else(|| "404".to_owned())
        }

        async fn directory_organization(&self, directory_id: &str) -> Result<String, String> {
            self.directories.get(directory_id).cloned().ok_or_else(|| "404".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fake::FakeWorkos, *};

    fn workos() -> FakeWorkos {
        let mut w = FakeWorkos::default();
        w.domains.insert("org_ACME".to_owned(), vec!["acme.com".to_owned()]);
        w.domains.insert("org_EVIL".to_owned(), vec!["evil.com".to_owned()]);
        w.directories.insert("directory_ACME".to_owned(), "org_ACME".to_owned());
        w.directories.insert("directory_EVIL".to_owned(), "org_EVIL".to_owned());
        w
    }

    #[tokio::test]
    async fn a_binding_needs_a_shared_verified_domain_and_its_own_directory() {
        let w = workos();
        let acme = ["acme.com".to_owned()];
        let b = verify_binding(&w, "org_ACME", Some("directory_ACME"), &acme).await.unwrap();
        assert!(b.proves("org_ACME", Some("directory_ACME")));
        assert!(!b.proves("org_ACME", None));
        // Another customer's WorkOS organization, or its directory under ours.
        assert!(verify_binding(&w, "org_EVIL", None, &acme).await.is_err());
        assert!(verify_binding(&w, "org_ACME", Some("directory_EVIL"), &acme).await.is_err());
        assert!(verify_binding(&w, "org_NOPE", None, &acme).await.is_err());
        assert!(verify_binding(&w, "org_ACME", Some("directory_NOPE"), &acme).await.is_err());
        // A tenant without a verified domain binds nothing.
        assert!(verify_binding(&w, "org_ACME", None, &[]).await.is_err());
    }
}
