//! A repository's code owners, from its default branch (Task 4).

use super::Resolution;
use crate::db::DbPool;

/// The code owners of `paths` in `repo` (the default owners when `None`).
pub async fn owners(_pool: &DbPool, _org: &str, _repo: &str, _paths: Option<&[String]>) -> Resolution {
    Resolution::nobody("codeowners", "code owners unavailable")
}

/// The files a pull request changes, through the org's reader.
pub async fn pr_files(_pool: &DbPool, _org: &str, _repo: &str, _pr: u64) -> Option<Vec<String>> {
    None
}
