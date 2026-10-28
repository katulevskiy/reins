//! The folders the phone signs for each paired computer (`reins_policy::desktop_folders`): the computer proposes its
//! `[workspace]` roots and scratch folders, canonicalized there (`POST /reins/desktop/folders`); the phone fetches the
//! proposals with its syncs (never a push, never a notification) and lists what isn't signed yet as question cards.
//!
//! - **Confirming** a folder loosens (the computer decides more without the phone): it needs the owner check, and only
//!   a folder the computer itself proposed, exactly as it resolved it, can be confirmed.
//! - **Removing** one only tightens: it applies at once (the computer is told to stop deciding until the new snapshot
//!   arrives, `desktop_policy::tighten_of`).
//! - **Dismissing** a card ("Not now") keeps it under Settings › Computers › the computer › Folders, where it can still
//!   be confirmed.
//!
//! What a computer gets signed ([`Engine::ap_signed_folders`]): the standard roots (`~/Documents/dev`, `~/src`,
//! `~/code`, `~/projects`, as before), the roots the owner confirmed, and the scratch folders the owner confirmed;
//! every one refused again by `refusal` (too broad, keys, Reins' own state) is left out.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use reins_policy::desktop_folders::{self as folders, refusal};
use reins_policy::effects::Workspace;
use reins_proto::desktop::{Folder, FolderKind, FolderProposals, MAX_FOLDERS};
use serde::{Deserialize, Serialize};

use super::types::{ComputerFolders, FolderKindChoice, FolderQuestion, FolderView};
use crate::CoreError;
use crate::OwnerCheck;
use crate::engine::Engine;
use crate::session::{Session, api_call};
use crate::store::unix_now;

/// The folders of every paired computer (sealed with the data key: they name the owner's folders).
const BOOK_KEY: &str = "autopilot.desktop_folders";
/// The proposals are fetched at most this often (with the syncs).
const FETCH_EVERY_SECS: i64 = 60;
/// ... and each time the cards or a computer's Folders are shown (a screen loading: a proposal just made shows).
const VIEW_FETCH_EVERY_SECS: i64 = 0;

/// One computer's folders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Computer {
    /// The computer's home folder, as it last said (what its `~` is).
    #[serde(default)]
    home: String,
    /// What it proposes now (refused ones never kept).
    #[serde(default)]
    proposed: Vec<Folder>,
    /// Unix seconds: when it last proposed.
    #[serde(default)]
    at: i64,
    /// Signed into its snapshot.
    #[serde(default)]
    confirmed: Vec<Folder>,
    /// Not asked about again as a card.
    #[serde(default)]
    dismissed: Vec<Folder>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    computers: BTreeMap<String, Computer>,
}

fn kind_of(k: FolderKindChoice) -> FolderKind {
    match k {
        FolderKindChoice::Root => FolderKind::Root,
        FolderKindChoice::Scratch => FolderKind::Scratch,
    }
}

fn choice_of(k: FolderKind) -> FolderKindChoice {
    match k {
        FolderKind::Root => FolderKindChoice::Root,
        FolderKind::Scratch => FolderKindChoice::Scratch,
    }
}

fn view(f: &Folder) -> FolderView {
    FolderView {
        path: f.path.clone(),
        kind: choice_of(f.kind),
    }
}

/// The standard roots every computer is signed (as before folders were confirmed).
fn standard_roots() -> Vec<String> {
    Workspace::standard("").roots
}

impl Computer {
    /// The roots and scratch folders signed for it.
    fn signed(&self) -> (Vec<String>, Vec<String>) {
        let mut roots = standard_roots();
        let mut scratch = Vec::new();
        for f in self.confirmed.iter().filter(|f| refusal(&f.path, &self.home).is_none()) {
            let list = match f.kind {
                FolderKind::Root => &mut roots,
                FolderKind::Scratch => &mut scratch,
            };
            if !list.contains(&f.path) {
                list.push(f.path.clone());
            }
        }
        (roots, scratch)
    }

    /// What it proposes that isn't signed yet (dismissed ones too).
    fn open(&self) -> Vec<Folder> {
        let (roots, scratch) = self.signed();
        folders::open(&self.proposed, &roots, &scratch, &self.home)
    }
}

impl Engine {
    fn ap_folder_book(&self) -> Result<Book, CoreError> {
        match self.store.meta_get_sealed(BOOK_KEY)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| CoreError::storage("corrupt computer folders")),
            None => Ok(Book::default()),
        }
    }

    /// Changes the book with `f` (read again under the store's lock).
    fn ap_update_folder_book<R>(&self, f: impl FnOnce(&mut Book) -> Result<R, CoreError>) -> Result<R, CoreError> {
        self.store.meta_update_sealed(BOOK_KEY, |current| {
            let mut book: Book = current.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let r = f(&mut book)?;
            let json = serde_json::to_vec(&book).map_err(|e| CoreError::storage(e.to_string()))?;
            Ok((Some(json), r))
        })
    }

    /// The roots and scratch folders signed into `connection_id`'s snapshot.
    pub(crate) fn ap_signed_folders(&self, connection_id: &str) -> Result<(Vec<String>, Vec<String>), CoreError> {
        Ok(self
            .ap_folder_book()?
            .computers
            .get(connection_id)
            .map_or_else(|| (standard_roots(), Vec::new()), Computer::signed))
    }

    /// Fetches what the paired computers propose (at most every [`FETCH_EVERY_SECS`] with the syncs), and keeps it for
    /// the cards. Only the phone that decides fetches; a failure (offline, an older server) waits for the next sync.
    pub(crate) async fn fetch_desktop_folders(&self, session: &Session) {
        self.fetch_desktop_folders_every(session, FETCH_EVERY_SECS).await;
    }

    /// [`Engine::fetch_desktop_folders`] when the last fetch is `every` seconds old or more (the cards and a computer's
    /// Folders fetch each time they are shown).
    pub(crate) async fn fetch_desktop_folders_every(&self, session: &Session, every: i64) {
        if self.require_decider().is_err() || self.not_approval.load(Ordering::Acquire) {
            return;
        }
        let now = unix_now();
        let last = self.autopilot.folders_fetched.load(Ordering::Acquire);
        if now - last < every {
            return;
        }
        self.autopilot.folders_fetched.store(now, Ordering::Release);
        match api_call!(session, |api| api.desktop_folders()) {
            Ok(proposals) => {
                if let Err(e) = self.ap_keep_folder_proposals(&proposals) {
                    log::warn!("could not keep the computers' folders: {e}");
                }
            }
            Err(e) => {
                self.refused_as_approval(&e);
                log::info!("could not fetch the computers' folders: {}", e.into_core());
            }
        }
    }

    /// [`Engine::fetch_desktop_folders_every`] when signed in, for a screen about to show them.
    pub(crate) async fn fetch_desktop_folders_for_view(&self) {
        if let Ok(session) = self.session() {
            self.fetch_desktop_folders_every(&session, VIEW_FETCH_EVERY_SECS).await;
        }
    }

    /// Keeps each paired computer's latest proposal (refused folders dropped), and forgets computers no longer paired.
    pub(crate) fn ap_keep_folder_proposals(&self, proposals: &FolderProposals) -> Result<(), CoreError> {
        let paired = self.store.desktop_connections()?;
        self.ap_update_folder_book(|book| {
            for item in &proposals.items {
                if !paired.contains(&item.connection_id) || !item.proposal.fits() {
                    continue;
                }
                let home = item.proposal.home.trim_end_matches('/').to_owned();
                let c = book.computers.entry(item.connection_id.clone()).or_default();
                if item.at < c.at {
                    continue;
                }
                let mut proposed: Vec<Folder> = Vec::new();
                for f in item.proposal.folders.iter().take(MAX_FOLDERS) {
                    if refusal(&f.path, &home).is_none() && !proposed.contains(f) {
                        proposed.push(f.clone());
                    }
                }
                c.home = home;
                c.proposed = proposed;
                c.at = item.at;
            }
            book.computers.retain(|id, _| paired.contains(id));
            Ok(())
        })
    }

    /// The folders the paired computers propose that aren't signed yet nor dismissed, newest first. Listed with the
    /// other question cards, never notified.
    pub fn folder_questions(&self) -> Result<Vec<FolderQuestion>, CoreError> {
        let book = self.ap_folder_book()?;
        let paired = self.store.desktop_connections()?;
        let names = self.store.connection_names()?;
        let mut out = Vec::new();
        for (id, c) in book.computers.iter().filter(|(id, _)| paired.contains(id)) {
            let computer = names
                .get(id)
                .cloned()
                .or_else(|| self.store.desktop_pairing(id).ok().flatten().map(|(label, _)| label))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Computer".to_owned());
            for f in c.open().into_iter().filter(|f| !c.dismissed.contains(f)) {
                out.push(FolderQuestion {
                    connection_id: id.clone(),
                    computer: computer.clone(),
                    path: f.path,
                    kind: choice_of(f.kind),
                    at: c.at,
                });
            }
        }
        out.sort_by_key(|q| std::cmp::Reverse(q.at));
        Ok(out)
    }

    /// A computer's folders: those signed (confirmed), and those it proposes that aren't.
    pub fn computer_folders(&self, connection_id: &str) -> Result<ComputerFolders, CoreError> {
        let book = self.ap_folder_book()?;
        let c = book.computers.get(connection_id).cloned().unwrap_or_default();
        Ok(ComputerFolders {
            connection_id: connection_id.to_owned(),
            confirmed: c.confirmed.iter().filter(|f| refusal(&f.path, &c.home).is_none()).map(view).collect(),
            suggested: c.open().iter().map(view).collect(),
        })
    }

    /// Confirms a folder the computer proposed, after the owner check (it loosens): it is signed into the computer's
    /// snapshot from the next publish. Only a folder the computer proposes now, exactly as it named it.
    pub async fn confirm_computer_folder(
        &self,
        connection_id: &str,
        path: &str,
        kind: FolderKindChoice,
        owner: &dyn OwnerCheck,
    ) -> Result<(), CoreError> {
        let folder = Folder {
            path: path.to_owned(),
            kind: kind_of(kind),
        };
        let proposed = |book: &Book| book.computers.get(connection_id).is_some_and(|c| c.open().contains(&folder));
        if !proposed(&self.ap_folder_book()?) {
            return Err(CoreError::NotFound);
        }
        let what = match folder.kind {
            FolderKind::Root => "projects",
            FolderKind::Scratch => "scratch",
        };
        super::policy::confirm(owner, &format!("Trust {path} as {what}")).await?;
        self.ap_update_folder_book(|book| {
            // Read again after the check: the computer may have withdrawn it meanwhile.
            if !proposed(book) {
                return Err(CoreError::NotFound);
            }
            let c = book.computers.entry(connection_id.to_owned()).or_default();
            c.dismissed.retain(|f| *f != folder);
            if !c.confirmed.contains(&folder) {
                c.confirmed.push(folder.clone());
            }
            Ok(())
        })
    }

    /// "Not now" on a card: it stays under the computer's Folders, unconfirmed. Changes nothing signed.
    pub fn dismiss_computer_folder(
        &self,
        connection_id: &str,
        path: &str,
        kind: FolderKindChoice,
    ) -> Result<(), CoreError> {
        let folder = Folder {
            path: path.to_owned(),
            kind: kind_of(kind),
        };
        self.ap_update_folder_book(|book| {
            let c = book.computers.get_mut(connection_id).ok_or(CoreError::NotFound)?;
            if !c.dismissed.contains(&folder) {
                c.dismissed.push(folder);
            }
            Ok(())
        })
    }

    /// Removes a confirmed folder: only tightens, so it applies at once (no owner check). It isn't asked about again as
    /// a card; it can be confirmed again under the computer's Folders while the computer proposes it.
    pub fn remove_computer_folder(
        &self,
        connection_id: &str,
        path: &str,
        kind: FolderKindChoice,
    ) -> Result<(), CoreError> {
        let folder = Folder {
            path: path.to_owned(),
            kind: kind_of(kind),
        };
        self.ap_update_folder_book(|book| {
            let c = book.computers.get_mut(connection_id).ok_or(CoreError::NotFound)?;
            let before = c.confirmed.len();
            c.confirmed.retain(|f| *f != folder);
            if c.confirmed.len() == before {
                return Err(CoreError::NotFound);
            }
            if !c.dismissed.contains(&folder) {
                c.dismissed.push(folder);
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use reins_proto::desktop::{FolderProposal, ProposedFolders};

    use super::*;
    use crate::api::ReinsCore;
    use crate::autopilot::testing::FakeOwner;
    use crate::engine::CoreConfig;
    use crate::store::tests::FakeKeys;
    use crate::{ForeignError, GoogleTokenProvider, Notifier, PendingItem};

    const DESK: &str = "desk1";

    struct Quiet;

    impl Notifier for Quiet {
        fn item_pending(&self, _: PendingItem) {
            panic!("a folder proposal never notifies");
        }
        fn item_resolved(&self, _: String) {}
        fn auto_decided(&self, _: crate::autopilot::AutoDecisionView) {}
        fn autopilot_changed(&self, _: crate::autopilot::AutopilotEvent) {}
        fn circle_changed(&self, _: crate::circles::CircleEvent) {}
    }

    struct NoGoogle;

    #[async_trait::async_trait]
    impl GoogleTokenProvider for NoGoogle {
        async fn access_token(&self, _: String, _: String) -> Result<String, ForeignError> {
            Err(ForeignError::Failed {
                reason: "no Google here".to_owned(),
            })
        }
    }

    fn core() -> (tempfile::TempDir, Arc<ReinsCore>) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CoreConfig {
            backoff_base: Duration::from_millis(1),
            ..CoreConfig::default()
        };
        let core = ReinsCore::with_config(
            dir.path().to_str().unwrap(),
            &FakeKeys::default(),
            Arc::new(NoGoogle),
            Arc::new(Quiet),
            cfg,
        )
        .unwrap();
        (dir, core)
    }

    fn folder(path: &str, kind: FolderKind) -> Folder {
        Folder {
            path: path.to_owned(),
            kind,
        }
    }

    fn proposal(at: i64, folders: Vec<Folder>) -> FolderProposals {
        FolderProposals {
            items: vec![ProposedFolders {
                connection_id: DESK.to_owned(),
                proposal: FolderProposal {
                    home: "/home/me".to_owned(),
                    folders,
                },
                at,
            }],
        }
    }

    const MEMORY: &str = "/home/me/.claude/projects/*/memory";

    #[tokio::test]
    async fn proposals_become_cards_and_confirmed_folders_are_signed() {
        let (_d, core) = core();
        let engine = core.engine();
        engine.store.pin_desktop_key(DESK, "key", "Laptop", 1).unwrap();
        engine
            .ap_keep_folder_proposals(&proposal(
                100,
                vec![
                    // Already signed (a standard root), refused (too broad, keys), and two new ones.
                    folder("/home/me/Documents/dev", FolderKind::Root),
                    folder("/home/me", FolderKind::Root),
                    folder("/", FolderKind::Scratch),
                    folder("/home/me/.ssh", FolderKind::Scratch),
                    folder(MEMORY, FolderKind::Scratch),
                    folder("/mnt/data/dev", FolderKind::Root),
                ],
            ))
            .unwrap();
        // Another computer's proposal, not paired here: ignored.
        let mut other = proposal(100, vec![folder("/mnt/x/dev", FolderKind::Root)]);
        other.items[0].connection_id = "stranger".to_owned();
        engine.ap_keep_folder_proposals(&other).unwrap();
        let qs = engine.folder_questions().unwrap();
        assert_eq!(
            qs.iter().map(|q| (q.path.as_str(), q.kind, q.computer.as_str())).collect::<Vec<_>>(),
            [(MEMORY, FolderKindChoice::Scratch, "Laptop"), ("/mnt/data/dev", FolderKindChoice::Root, "Laptop")]
        );
        // Confirming needs the owner: refused, nothing changes.
        let no = FakeOwner::new(Ok(false));
        assert!(engine.confirm_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch, no.as_ref()).await.is_err());
        assert_eq!(engine.ap_signed_folders(DESK).unwrap().1, Vec::<String>::new());
        // Only what the computer proposed, exactly.
        let yes = FakeOwner::new(Ok(true));
        for (path, kind) in [
            ("/home/me", FolderKindChoice::Root),
            ("/home/me/.claude/projects", FolderKindChoice::Scratch),
            (MEMORY, FolderKindChoice::Root),
        ] {
            assert!(engine.confirm_computer_folder(DESK, path, kind, yes.as_ref()).await.is_err(), "{path}");
        }
        engine.confirm_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch, yes.as_ref()).await.unwrap();
        assert_eq!(yes.asked.lock().unwrap().len(), 1, "asked once, for the one confirmed");
        let (roots, scratch) = engine.ap_signed_folders(DESK).unwrap();
        assert_eq!(roots, ["~/Documents/dev", "~/src", "~/code", "~/projects"]);
        assert_eq!(scratch, [MEMORY]);
        // The card is gone; the folder shows as confirmed.
        assert_eq!(engine.folder_questions().unwrap().len(), 1);
        let f = engine.computer_folders(DESK).unwrap();
        assert_eq!(f.confirmed.iter().map(|v| v.path.as_str()).collect::<Vec<_>>(), [MEMORY]);
        assert_eq!(f.suggested.iter().map(|v| v.path.as_str()).collect::<Vec<_>>(), ["/mnt/data/dev"]);
        // The snapshot the computer is sent carries it.
        let body = engine.ap_desktop_policy(DESK, "key", 1_000).unwrap();
        assert_eq!(body.scratch, [MEMORY]);
    }

    #[tokio::test]
    async fn dismissing_and_removing() {
        let (_d, core) = core();
        let engine = core.engine();
        engine.store.pin_desktop_key(DESK, "key", "Laptop", 1).unwrap();
        engine.ap_keep_folder_proposals(&proposal(100, vec![folder(MEMORY, FolderKind::Scratch)])).unwrap();
        engine.dismiss_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch).unwrap();
        assert!(engine.folder_questions().unwrap().is_empty(), "not asked again");
        assert_eq!(engine.computer_folders(DESK).unwrap().suggested.len(), 1, "still under the computer's Folders");
        // Still confirmable there.
        let yes = FakeOwner::new(Ok(true));
        engine.confirm_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch, yes.as_ref()).await.unwrap();
        // Removing applies at once, without the owner check.
        engine.remove_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch).unwrap();
        assert_eq!(yes.asked.lock().unwrap().len(), 1);
        assert!(engine.ap_signed_folders(DESK).unwrap().1.is_empty());
        assert!(engine.folder_questions().unwrap().is_empty(), "a removed folder isn't a card again");
        assert!(engine.remove_computer_folder(DESK, MEMORY, FolderKindChoice::Scratch).is_err());
        // An older proposal never replaces a newer one; a computer unpaired is forgotten.
        engine.ap_keep_folder_proposals(&proposal(50, Vec::new())).unwrap();
        assert_eq!(engine.computer_folders(DESK).unwrap().suggested.len(), 1);
        engine.store.remove_desktop_key(DESK).unwrap();
        engine.ap_keep_folder_proposals(&FolderProposals::default()).unwrap();
        assert!(engine.computer_folders(DESK).unwrap().suggested.is_empty());
    }
}
