//! The GitHub tools, by area. Each area lists its own tools; a permission for a write names the kinds of change it
//! allows (`CLASSES`), the repository and, for changes to code, the branch.

mod actions;
mod api;
mod code;
mod git;
mod issues;
mod repos;

use super::{ClassInfo, ToolSpec};

/// The kinds of change a GitHub write falls into, for permissions.
pub(super) const CLASSES: &[ClassInfo] = &[
    ClassInfo {
        id: "issues",
        label: "Issues and comments",
    },
    ClassInfo {
        id: "pulls",
        label: "Pull requests and reviews",
    },
    ClassInfo {
        id: "code",
        label: "Commits, files and branches",
    },
    ClassInfo {
        id: "releases",
        label: "Releases and tags",
    },
    ClassInfo {
        id: "actions",
        label: "Workflow runs and variables",
    },
    ClassInfo {
        id: "settings",
        label: "Repository settings",
    },
    ClassInfo {
        id: "account",
        label: "Stars, gists and notifications",
    },
];

pub(super) fn tools() -> Vec<ToolSpec> {
    let mut all = Vec::new();
    all.extend(repos::tools());
    all.extend(code::tools());
    all.extend(issues::tools());
    all.extend(actions::tools());
    all.extend(git::tools());
    all.extend(api::tools());
    all
}
