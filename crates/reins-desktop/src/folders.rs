//! This computer's `[workspace]` folders, proposed to the phone (`reins_policy::desktop_folders`).
//!
//! Hook requests decided here with the phone-signed policy count a local root or scratch folder only when the phone
//! signed it ([`crate::policy::effective`]). So the daemon proposes its own: each root and scratch folder of the
//! configuration, canonicalized here ([`canonical`]: `~` expanded, symlinks resolved, `*` parts kept below the folder
//! resolved), refused ones left out ([`reins_policy::desktop_folders::refusal`]: `/`, `~`, `/home/<user>`, keys,
//! Reins' own state). The owner sees each exactly as resolved, as a question card on the phone, and confirms it there
//! (the owner check); only then is it signed for this computer. Nothing here makes anything count.
//!
//! With them go the scratch folders this computer only suggests ([`crate::suggest`], [`with_suggestions`]): folders in
//! no configuration, learned from what the owner approved here. They are cards too, and count only once confirmed.
//!
//! The proposal goes to the server (`POST /reins/desktop/folders`) when it changed, and again every
//! [`FOLDERS_RESEND_SECS`] (the server keeps it in memory only).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use reins_policy::desktop_folders::refusal;
use reins_proto::desktop::{FOLDERS_RESEND_SECS, Folder, FolderKind, FolderProposal, MAX_FOLDERS};

use crate::config::{Config, Paths, WorkspaceConfig};
use crate::server::LinkError;
use crate::server::client::DesktopClient;

/// How often the daemon looks at its configuration again.
const LOOK_EVERY: Duration = Duration::from_mins(1);
/// A server too old to take proposals is tried again this much later.
const TOO_OLD_RETRY: Duration = Duration::from_mins(15);

fn is_glob(part: &str) -> bool {
    part.contains(['*', '?', '['])
}

/// The home folder, resolved (empty when not known).
#[must_use]
pub fn home() -> String {
    crate::config::home_dir()
        .ok()
        .map(|h| std::fs::canonicalize(&h).unwrap_or(h))
        .and_then(|h| h.to_str().map(str::to_owned))
        .unwrap_or_default()
}

/// `entry` (a `[workspace]` folder: `~/x`, `/x`, maybe with `*` parts) as this computer resolves it now: `~` read as
/// the home folder (`home_dir`, as written), the folders before the first `*` part resolved by the file system (they
/// must exist), the rest kept. `None`: relative, missing, `..` below a `*`, or not UTF-8.
#[must_use]
pub fn canonical(entry: &str, home_dir: &str) -> Option<String> {
    let entry = entry.trim();
    let expanded = if entry == "~" || entry.starts_with("~/") {
        if home_dir.is_empty() {
            return None;
        }
        format!("{home_dir}{}", &entry[1..])
    } else {
        entry.to_owned()
    };
    if !expanded.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = expanded.split('/').filter(|p| !p.is_empty()).collect();
    let first_glob = parts.iter().position(|p| is_glob(p)).unwrap_or(parts.len());
    let (literal, rest) = parts.split_at(first_glob);
    if rest.iter().any(|p| matches!(*p, "." | "..")) {
        return None;
    }
    let resolved = std::fs::canonicalize(Path::new(&format!("/{}", literal.join("/")))).ok()?;
    let mut out = resolved.to_str()?.trim_end_matches('/').to_owned();
    for p in rest {
        out.push('/');
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('/');
    }
    Some(out)
}

/// What `entries` resolve to here (those that resolve), for [`reins_policy::desktop_policy::Local::canonical`].
#[must_use]
pub fn resolved<'a>(entries: impl IntoIterator<Item = &'a String>) -> BTreeMap<String, String> {
    let home_dir = crate::config::home_dir().ok().and_then(|h| h.to_str().map(str::to_owned)).unwrap_or_default();
    entries.into_iter().filter_map(|e| Some((e.clone(), canonical(e, &home_dir)?))).collect()
}

/// This computer's proposal: its roots, then its scratch folders, each canonicalized, refused and unresolvable ones
/// left out, once each.
#[must_use]
pub fn proposal(workspace: &WorkspaceConfig) -> FolderProposal {
    let home_dir = crate::config::home_dir().ok().and_then(|h| h.to_str().map(str::to_owned)).unwrap_or_default();
    let home = home();
    let mut folders: Vec<Folder> = Vec::new();
    let all = workspace
        .roots
        .iter()
        .map(|r| (r, FolderKind::Root))
        .chain(workspace.scratch.iter().map(|s| (s, FolderKind::Scratch)));
    for (entry, kind) in all {
        let Some(path) = canonical(entry, &home_dir) else {
            continue;
        };
        if let Some(why) = refusal(&path, &home) {
            log::info!("folders: {path} is not proposed to the phone ({})", why.label());
            continue;
        }
        let f = Folder {
            path,
            kind,
        };
        if !folders.contains(&f) && folders.len() < MAX_FOLDERS {
            folders.push(f);
        }
    }
    FolderProposal {
        home,
        folders,
        suggested_scratch: Vec::new(),
    }
}

/// `proposal` with the scratch folders this computer only suggests ([`crate::suggest`]): those the phone can sign as
/// they are, after the configured folders and within [`MAX_FOLDERS`] in all (the configured ones win). One the
/// configuration names already, or `refusal` refuses, is left out. A suggestion makes nothing count: the phone shows it
/// as a card, and the folder counts here once the owner confirmed it there.
#[must_use]
pub fn with_suggestions(mut proposal: FolderProposal, suggested: &[crate::suggest::Suggestion]) -> FolderProposal {
    for s in suggested.iter().filter(|s| s.proposable) {
        let configured = proposal.folders.iter().any(|f| f.kind == FolderKind::Scratch && f.path == s.path);
        if proposal.folders.len() + proposal.suggested_scratch.len() < MAX_FOLDERS
            && !configured
            && !proposal.suggested_scratch.contains(&s.path)
            && refusal(&s.path, &proposal.home).is_none()
        {
            proposal.suggested_scratch.push(s.path.clone());
        }
    }
    proposal
}

/// Sends the proposal (logged in only).
pub async fn propose(paths: &Paths, proposal: &FolderProposal) -> Result<(), LinkError> {
    DesktopClient::new(paths).map_err(LinkError::Failed)?.post_folders(proposal).await
}

/// The daemon's proposer: the configuration's folders and the scratch folders it suggests, sent when they changed and
/// every [`FOLDERS_RESEND_SECS`]. The configuration is read again each time, so a folder added with `reins folders`
/// is proposed within [`LOOK_EVERY`] without a restart.
pub async fn proposer(paths: Paths, start: WorkspaceConfig) {
    let mut last: Option<(FolderProposal, i64)> = None;
    loop {
        if crate::server::oauth::logged_in_server(&paths).is_none() {
            last = None;
            tokio::time::sleep(LOOK_EVERY).await;
            continue;
        }
        let config = Config::load(&paths).ok();
        let workspace = config.as_ref().map_or_else(|| start.clone(), |c| c.workspace.clone());
        let now = crate::now_unix();
        let mut p = proposal(&workspace);
        if let Some(config) = config {
            // Reading the activity log again takes a moment: off the daemon's own threads.
            let state = paths.clone();
            let suggested =
                tokio::task::spawn_blocking(move || crate::suggest::suggestions(&state, &config, now)).await;
            p = with_suggestions(p, &suggested.unwrap_or_default());
        }
        let due = last.as_ref().is_none_or(|(sent, at)| *sent != p || now - at >= FOLDERS_RESEND_SECS);
        if due {
            match propose(&paths, &p).await {
                Ok(()) => {
                    log::info!("folders: {} proposed to the phone", p.folders.len());
                    last = Some((p, now));
                }
                Err(LinkError::NotFound) => {
                    tokio::time::sleep(TOO_OLD_RETRY).await;
                    continue;
                }
                Err(e) => log::info!("folders: {e}"),
            }
        }
        tokio::time::sleep(LOOK_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Folders are proposed from Linux and macOS only (Windows paths are not in the `/…` form the checks expect).
    #[cfg(unix)]
    #[test]
    fn folders_are_canonicalized_and_broad_ones_never_proposed() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        let home = real.to_str().unwrap().to_owned();
        std::fs::create_dir_all(real.join("dev/app")).unwrap();
        std::fs::create_dir_all(real.join("data/dev")).unwrap();
        std::fs::create_dir_all(real.join(".claude/projects")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.join("data/dev"), real.join("work")).unwrap();
        assert_eq!(canonical("~/dev", &home), Some(format!("{home}/dev")));
        assert_eq!(canonical("~/dev/app/../app", &home), Some(format!("{home}/dev/app")));
        #[cfg(unix)]
        assert_eq!(
            canonical("~/work", &home),
            Some(format!("{home}/data/dev")),
            "a link is shown as what it points to"
        );
        assert_eq!(
            canonical("~/.claude/projects/*/memory", &home),
            Some(format!("{home}/.claude/projects/*/memory")),
            "the parts below a `*` are kept"
        );
        assert_eq!(canonical("~/missing", &home), None);
        assert_eq!(canonical("relative/dev", &home), None);
        assert_eq!(canonical("~/.claude/projects/*/../..", &home), None);
        assert_eq!(canonical("~", &home), Some(home.clone()));
        assert_eq!(canonical("/", &home), Some("/".to_owned()));
    }

    #[test]
    fn suggestions_go_after_the_configured_folders_and_never_among_them() {
        use crate::suggest::{Suggestion, Why};
        let suggestion = |path: &str, proposable: bool| Suggestion {
            path: path.to_owned(),
            why: Why::Approved(3),
            proposable,
        };
        let folder = |path: &str, kind| Folder {
            path: path.to_owned(),
            kind,
        };
        let configured = FolderProposal {
            home: "/home/me".to_owned(),
            folders: vec![
                folder("/home/me/Documents/dev", FolderKind::Root),
                folder("/home/me/Documents/dev/tmp", FolderKind::Scratch),
            ],
            suggested_scratch: Vec::new(),
        };
        let p = with_suggestions(
            configured.clone(),
            &[
                suggestion("/home/me/Downloads/tmp", true),
                // In the configuration already; reached through a link; refused; twice.
                suggestion("/home/me/Documents/dev/tmp", true),
                suggestion("/home/me/linked", false),
                suggestion("/home/me/.ssh", true),
                suggestion("/home/me", true),
                suggestion("/home/me/Downloads/tmp", true),
            ],
        );
        assert_eq!(p.folders, configured.folders, "a suggestion is never a configured folder");
        assert_eq!(p.suggested_scratch, ["/home/me/Downloads/tmp"]);
        // The configured folders win the room there is.
        let mut full = configured;
        full.folders = (0..MAX_FOLDERS - 1).map(|i| folder(&format!("/home/me/work/{i}"), FolderKind::Root)).collect();
        let p = with_suggestions(full, &[suggestion("/home/me/a/tmp", true), suggestion("/home/me/b/tmp", true)]);
        assert_eq!(p.suggested_scratch, ["/home/me/a/tmp"]);
        assert!(p.fits());
    }

    #[test]
    fn the_proposal_leaves_out_what_is_refused() {
        // The real home: `~` and `/` are refused, whatever the configuration says.
        let workspace = WorkspaceConfig {
            roots: vec!["~".to_owned(), "/".to_owned(), "~/.ssh".to_owned()],
            scratch: vec!["/".to_owned()],
        };
        assert!(proposal(&workspace).folders.is_empty(), "{:?}", proposal(&workspace));
        let p = proposal(&WorkspaceConfig {
            roots: Vec::new(),
            scratch: vec![std::env::temp_dir().to_string_lossy().into_owned(), "/tmp/../tmp".to_owned()],
        });
        assert!(p.folders.len() <= 1, "once each: {p:?}");
        assert!(p.folders.iter().all(|f| refusal(&f.path, &p.home).is_none()));
    }
}
