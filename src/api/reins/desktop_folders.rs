//! The folders each paired computer proposes the phone signs for it (Decisions v2, `reins_policy::desktop_folders`):
//! the computer posts its `[workspace]` roots and scratch folders, canonicalized there (`POST /reins/desktop/folders`);
//! the approval phone reads them with its syncs (`GET /reins/api/desktop/folders`) and shows each as a question card.
//! Nothing here decides anything: a folder counts on the computer only once the owner confirmed it on the phone and the
//! phone signed it into that computer's snapshot. No push wakes the phone for a proposal.
//!
//! Memory only: the latest proposal per connection, kept [`FOLDERS_TTL_SECS`]; the computer posts again every
//! [`reins_proto::desktop::FOLDERS_RESEND_SECS`], so a restart forgets nothing for long.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use reins_proto::desktop::{FOLDERS_TTL_SECS, FolderProposal, ProposedFolders};

/// The most connections kept in all (a backstop: a new connection's proposal is then refused, nothing is evicted).
pub const MAX_PROPOSALS: usize = 200_000;

/// Per desktop connection: its account and its latest proposal.
#[derive(Default)]
pub struct Folders {
    state: Mutex<HashMap<String, (String, ProposedFolders)>>,
}

impl Folders {
    /// Keeps `proposal` as `connection_id`'s (of account `user`), replacing the one before; `false` when it doesn't fit.
    pub fn post(&self, user: &str, connection_id: &str, proposal: FolderProposal, now: i64) -> bool {
        if !proposal.fits() {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.len() >= MAX_PROPOSALS && !state.contains_key(connection_id) {
            return false;
        }
        state.insert(
            connection_id.to_owned(),
            (
                user.to_owned(),
                ProposedFolders {
                    connection_id: connection_id.to_owned(),
                    proposal,
                    at: now,
                },
            ),
        );
        true
    }

    /// The proposals of `user`'s connections among `connections`.
    pub fn of(&self, user: &str, connections: &[String]) -> Vec<ProposedFolders> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out: Vec<ProposedFolders> = connections
            .iter()
            .filter_map(|c| state.get(c))
            .filter(|(owner, _)| owner == user)
            .map(|(_, p)| p.clone())
            .collect();
        out.sort_by(|a, b| a.connection_id.cmp(&b.connection_id));
        out
    }

    /// Forgets proposals older than [`FOLDERS_TTL_SECS`].
    pub fn purge(&self, now: i64) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).retain(|_, (_, p)| now - p.at < FOLDERS_TTL_SECS);
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::desktop::{Folder, FolderKind, MAX_FOLDERS};

    use super::*;

    fn proposal(paths: &[&str]) -> FolderProposal {
        FolderProposal {
            home: "/home/me".to_owned(),
            folders: paths
                .iter()
                .map(|p| Folder {
                    path: (*p).to_owned(),
                    kind: FolderKind::Scratch,
                })
                .collect(),
        }
    }

    #[test]
    fn the_latest_proposal_per_computer_for_its_account_only() {
        let f = Folders::default();
        assert!(f.post("u1", "c1", proposal(&["/home/me/a"]), 10));
        assert!(f.post("u1", "c1", proposal(&["/home/me/b"]), 20));
        assert!(f.post("u2", "c2", proposal(&["/home/me/c"]), 20));
        let mine = f.of("u1", &["c1".to_owned(), "c2".to_owned()]);
        assert_eq!(mine.len(), 1, "another account's computer is never shown");
        assert_eq!(mine[0].proposal.folders[0].path, "/home/me/b");
        assert_eq!(mine[0].at, 20);
        f.purge(20 + FOLDERS_TTL_SECS);
        assert!(f.of("u1", &["c1".to_owned()]).is_empty());
    }

    #[test]
    fn oversized_proposals_are_refused() {
        let f = Folders::default();
        let many: Vec<String> = (0..=MAX_FOLDERS).map(|i| format!("/home/me/{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(!f.post("u1", "c1", proposal(&many), 1));
        let long = format!("/home/me/{}", "a".repeat(600));
        assert!(!f.post("u1", "c1", proposal(&[&long]), 1));
        assert!(f.of("u1", &["c1".to_owned()]).is_empty());
    }
}
