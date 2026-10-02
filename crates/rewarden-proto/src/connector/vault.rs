//! The vault tools, by area. A permission for a write names the kinds of change it allows (`CLASSES`) and the folder
//! (or item) it applies to.

mod desktop;
mod items;
mod sends;

use super::{ClassInfo, ToolSpec};

/// The kinds of change a vault write falls into, for permissions.
pub(super) const CLASSES: &[ClassInfo] = &[
    ClassInfo {
        id: "items",
        label: "Create and edit items",
    },
    ClassInfo {
        id: "organize",
        label: "Folders, favorites, trash and archive",
    },
    ClassInfo {
        id: "sends",
        label: "Sends",
    },
    ClassInfo {
        id: "secrets",
        label: "Secrets used on the computer",
    },
    ClassInfo {
        id: "ssh",
        label: "SSH sign-ins",
    },
];

pub(super) fn tools() -> Vec<ToolSpec> {
    let mut all = Vec::new();
    all.extend(items::tools());
    all.extend(sends::tools());
    all.extend(desktop::tools());
    all
}
