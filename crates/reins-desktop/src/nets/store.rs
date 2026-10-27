//! Where nets keep what they saved: `nets` in the state folder (`~/.local/state/reins/nets`, next to the activity
//! log), one folder per plan.
//!
//! ```text
//! nets/<plan>/plan.json                 what was netted (the hook writes it)
//! nets/<plan>/items/<item>/item.json    a trashed path or a snapshot: where it was, when, what it was
//! nets/<plan>/items/<item>/data         the path itself (moved here), or the snapshot's copy
//! nets/<plan>/refs.jsonl                git backup refs (`refs/reins/backup/<plan>/<n>` in their repository)
//! nets/<plan>/restored                  once restored
//! nets/.failed/<key>                    a net that failed: the same command is planned without it for an hour
//! ```
//!
//! Every item has its own folder made with `create_dir`, and nothing is shared between items, so concurrent deletes
//! never collide. `item.json` is written before the path is moved, so a crash in between leaves a record, never a lost
//! path; when it can't be written (disk full), nothing is moved.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// How long a trashed path or a snapshot is kept (01 §5).
pub const TRASH_DAYS: i64 = 7;
/// How long a git backup ref is kept (01 §5).
pub const REF_DAYS: i64 = 30;
/// How long a failed net keeps the same command from being netted again.
pub const FAILED_SECS: i64 = 3_600;
const DAY: i64 = 86_400;
/// Below this much free space on the trash's disk no net is planned (a full disk can't keep a copy, or even record
/// that it failed).
pub const MIN_FREE: u64 = 64 << 20;

/// Why a net could not be applied. Nothing was deleted or changed when one is returned.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("it is on another disk than the Reins trash")]
    OtherDisk,
    #[error("{0}")]
    Unsafe(String),
    #[error("it is not there")]
    Missing,
    #[error("it is larger than the Reins trash keeps")]
    TooLarge,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// What kind of item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// The path itself, moved into the trash.
    Trash,
    /// A copy of a file, taken before it was changed.
    Snapshot,
}

/// A git worktree whose folder was trashed: what it takes to make it a worktree again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// The main repository's top folder.
    pub repo: String,
    pub head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// One trashed path or snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Its folder's name under `items/`.
    pub id: String,
    pub at: i64,
    pub kind: ItemKind,
    /// Where it was: the real (canonical) parent folder joined with its name.
    pub original: String,
    /// The parent's canonical path when it was saved: a restore goes back only there.
    pub parent: String,
    /// `file`, `dir` or `symlink`.
    pub what: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeInfo>,
    /// Its permission bits (a snapshot is put back with them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// How many bytes it keeps (the trash's quota counts them).
    #[serde(default)]
    pub bytes: u64,
}

/// A git backup ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefBackup {
    pub at: i64,
    /// The repository's top folder (or git dir) to run git in.
    pub repo: String,
    /// The backup ref (`refs/reins/backup/<plan>/<n>`).
    pub backup: String,
    pub oid: String,
    /// What it was: `branch`, `remote-branch`, `head` (before a hard reset), `stash` (the work tree before it), `remote`
    /// (a remote branch before a force push).
    pub what: String,
    /// The name it had (`feat/x`, `origin/feat/x`).
    pub name: String,
}

/// What the hook planned (`plan.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRecord {
    pub id: String,
    pub at: i64,
    /// The failure key: the same command in the same folder.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// The Activity entry it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The programs `reins net run` may run for it (`rm`, `git`).
    #[serde(default)]
    pub programs: Vec<String>,
    /// The folders it may run in (the request's, and the literal `cd` targets in it).
    #[serde(default)]
    pub cwds: Vec<String>,
    /// The home folder and the scratch roots as the hook read them: what `reins net run` deletes for real.
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub scratch: Vec<String>,
}

/// A plan's nets, for lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetSummary {
    pub id: String,
    pub at: i64,
    pub lines: Vec<String>,
    pub items: usize,
    pub refs: usize,
    /// When the last of it goes.
    pub until: i64,
    pub restored: bool,
}

/// A fault a test makes happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Every write of a record or a copy fails as if the disk were full.
    DiskFull,
}

#[cfg(unix)]
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn free_bytes(path: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn disk_bytes(path: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some(s.f_blocks.saturating_mul(s.f_frsize))
}

#[cfg(not(unix))]
fn disk_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(unix)]
fn mode_of(m: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_m: &std::fs::Metadata) -> Option<u32> {
    None
}

/// The bytes a path holds, its folders walked without following links (at most a million entries: past that, as
/// many bytes as the walk saw, and the quota is checked on that).
fn size_of(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(p) = stack.pop() {
        seen += 1;
        if seen > 1_000_000 {
            break;
        }
        let Ok(m) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        total = total.saturating_add(m.len());
        if m.is_dir() {
            stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()));
        }
    }
    total
}

/// Removes a folder even where its own permissions say no (a read-only folder in a trashed tree): the folders are made
/// writable on the way down, then removed. Whether it is gone.
pub(crate) fn remove_tree(path: &Path) -> bool {
    if std::fs::remove_dir_all(path).is_ok() || std::fs::symlink_metadata(path).is_err() {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut stack = vec![path.to_path_buf()];
        while let Some(p) = stack.pop() {
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                let _made = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700));
                stack.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()));
            }
        }
    }
    std::fs::remove_dir_all(path).is_ok()
}

/// The nets' store.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    device_of: fn(&Path) -> Option<u64>,
    fault: Option<Fault>,
    /// The most bytes the trash keeps; `None`: a tenth of its disk.
    quota: Option<u64>,
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `path` is absolute and has no `.` or `..` parts.
fn plain_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_) | Component::Prefix(_)))
}

fn what_of(m: &std::fs::Metadata) -> &'static str {
    if m.file_type().is_symlink() {
        "symlink"
    } else if m.is_dir() {
        "dir"
    } else {
        "file"
    }
}

impl Store {
    /// The store at `root`.
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            device_of: device,
            fault: None,
            quota: None,
        }
    }

    /// The store of this computer's Reins: `$REINS_NETS_DIR`, else `nets` in the state folder (next to the activity
    /// log: `~/.local/state/reins/nets`).
    #[must_use]
    pub fn open(paths: &crate::config::Paths) -> Self {
        Self::at(&env_dir("REINS_NETS_DIR").unwrap_or_else(|| paths.state_dir.join("nets")))
    }

    /// [`Self::open`] with the folders from the environment.
    pub fn open_default() -> Result<Self, String> {
        let paths = crate::config::Paths::from_env().map_err(|e| e.to_string())?;
        Ok(Self::open(&paths))
    }

    /// A store whose device check is `device_of` (tests: "another disk").
    #[must_use]
    pub fn with_devices(mut self, device_of: fn(&Path) -> Option<u64>) -> Self {
        self.device_of = device_of;
        self
    }

    /// A store that keeps at most `bytes`.
    #[must_use]
    pub fn with_quota(mut self, bytes: u64) -> Self {
        self.quota = Some(bytes);
        self
    }

    /// The most bytes the trash keeps: the configured quota, else a tenth of its disk.
    #[must_use]
    pub fn quota(&self) -> u64 {
        self.quota.unwrap_or_else(|| disk_bytes(&self.real_root()).map_or(u64::MAX, |b| b / 10))
    }

    /// Makes room for `bytes` more in the trash, dropping the oldest items of other plans first. `Err`: it can't fit
    /// (larger than the quota, or only `plan`'s own items would have to go).
    fn make_room(&self, plan: &str, bytes: u64) -> Result<(), NetError> {
        let quota = self.quota();
        if bytes > quota {
            return Err(NetError::TooLarge);
        }
        let mut all: Vec<(String, Item)> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| reins_policy::effects::nets::valid_plan_id(n))
            .flat_map(|p| self.items(&p).into_iter().map(move |i| (p.clone(), i)))
            .collect();
        let mut used: u64 = all.iter().map(|(_, i)| i.bytes).sum();
        all.sort_by_key(|(_, i)| i.at);
        for (p, i) in all {
            if used.saturating_add(bytes) <= quota {
                break;
            }
            if p == plan {
                continue;
            }
            if self.drop_item(&p, &i.id) {
                used = used.saturating_sub(i.bytes);
            }
        }
        if used.saturating_add(bytes) > quota {
            return Err(NetError::TooLarge);
        }
        Ok(())
    }

    /// Removes one item (claimed first, so a restore at the same moment finds it gone, not half removed).
    fn drop_item(&self, plan: &str, id: &str) -> bool {
        let items = self.root.join(plan).join("items");
        let claimed = items.join(format!(".{id}.pruning"));
        std::fs::rename(items.join(id), &claimed).is_ok() && remove_tree(&claimed)
    }

    /// A store where `fault` happens (tests).
    #[must_use]
    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = Some(fault);
        self
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Makes the store's folder (owner only) and returns where it really is.
    pub(crate) fn ensure_root(&self) -> io::Result<PathBuf> {
        std::fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::canonicalize(&self.root)
    }

    /// The root as it really is (canonical), or as configured when it does not exist yet.
    fn real_root(&self) -> PathBuf {
        std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone())
    }

    fn plan_dir(&self, plan: &str) -> Result<PathBuf, NetError> {
        if !reins_policy::effects::nets::valid_plan_id(plan) {
            return Err(NetError::Unsafe(format!("{plan:?} is not a plan id")));
        }
        Ok(self.root.join(plan))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.fault == Some(Fault::DiskFull) {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "No space left on device"));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::config::write_private(path, bytes)
    }

    /// Whether `abs` can be moved into the trash: the store's filesystem, and not the store, inside it or above it
    /// (planning: [`reins_policy::effects::nets::NetFacts`]). The nearest folder that exists decides the disk.
    #[must_use]
    pub fn trashable(&self, abs: &Path) -> bool {
        let Ok(root) = self.ensure_root() else {
            return false;
        };
        if self.low_space() {
            return false;
        }
        let Some(abs_s) = abs.to_str() else {
            return false;
        };
        if !reins_policy::effects::nets::trash_safe(abs_s, &root.to_string_lossy()) {
            return false;
        }
        // The real place too: a path that resolves into (or above) the trash is not trashable either.
        let existing = abs.ancestors().find(|a| a.exists()).unwrap_or(abs);
        let real = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
        if real.starts_with(&root) || (root.starts_with(&real) && existing == abs) {
            return false;
        }
        let dev = (self.device_of)(&real);
        dev.is_some() && dev == (self.device_of)(&root)
    }

    /// The trash's disk is (nearly) full.
    #[must_use]
    pub fn low_space(&self) -> bool {
        self.fault == Some(Fault::DiskFull) || free_bytes(&self.root).is_none_or(|f| f < MIN_FREE)
    }

    /// Writes the hook's plan.
    pub fn begin(&self, plan: &PlanRecord) -> Result<(), NetError> {
        let dir = self.plan_dir(&plan.id)?;
        self.ensure_root()?;
        let bytes = serde_json::to_vec_pretty(plan).map_err(|e| io::Error::other(e.to_string()))?;
        self.write(&dir.join("plan.json"), &bytes)?;
        Ok(())
    }

    /// The hook's plan, if it wrote one.
    #[must_use]
    pub fn plan(&self, plan: &str) -> Option<PlanRecord> {
        let dir = self.plan_dir(plan).ok()?;
        serde_json::from_slice(&std::fs::read(dir.join("plan.json")).ok()?).ok()
    }

    /// A new item folder of `plan`.
    fn new_item(&self, plan: &str) -> Result<(String, PathBuf), NetError> {
        let items = self.plan_dir(plan)?.join("items");
        self.ensure_root()?;
        std::fs::create_dir_all(&items)?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        for _ in 0..100 {
            let id = format!("{nanos:x}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
            let dir = items.join(&id);
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok((id, dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(NetError::Unsafe("no free item name".to_owned()))
    }

    /// The real place of `path`: its parent folder canonical (symlinks in it followed, as the system does), its own
    /// name kept (a symlink is the link itself). `None` for `.`, `..` or no name.
    fn real_place(path: &Path) -> Option<(PathBuf, PathBuf)> {
        let name = path.file_name()?;
        if name == "." || name == ".." {
            return None;
        }
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => std::env::current_dir().ok()?,
        };
        let parent = std::fs::canonicalize(parent).ok()?;
        Some((parent.join(name), parent))
    }

    /// Moves `path` into `plan`'s trash. On any error nothing was moved.
    pub fn trash(&self, plan: &str, path: &Path, worktree: Option<WorktreeInfo>) -> Result<Item, NetError> {
        let (real, parent) =
            Self::real_place(path).ok_or_else(|| NetError::Unsafe("not a path to delete".to_owned()))?;
        let meta = std::fs::symlink_metadata(&real).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                NetError::Missing
            } else {
                NetError::Io(e)
            }
        })?;
        let root = self.ensure_root()?;
        // A name that is not text could not be put back exactly.
        let real_s = real.to_str().ok_or_else(|| NetError::Unsafe("a name that is not text".to_owned()))?.to_owned();
        if !reins_policy::effects::nets::trash_safe(&real_s, &root.to_string_lossy()) {
            return Err(NetError::Unsafe("Reins keeps its trash there".to_owned()));
        }
        let dev = (self.device_of)(&parent);
        if dev.is_none() || dev != (self.device_of)(&root) {
            return Err(NetError::OtherDisk);
        }
        let bytes = size_of(&real);
        self.make_room(plan, bytes)?;
        let (id, dir) = self.new_item(plan)?;
        let item = Item {
            id,
            at: crate::now_unix(),
            kind: ItemKind::Trash,
            original: real_s,
            parent: parent.to_string_lossy().into_owned(),
            what: what_of(&meta).to_owned(),
            worktree,
            mode: mode_of(&meta),
            bytes,
        };
        let saved = serde_json::to_vec_pretty(&item)
            .map_err(|e| io::Error::other(e.to_string()))
            .and_then(|b| self.write(&dir.join("item.json"), &b))
            .and_then(|()| std::fs::rename(&real, dir.join("data")));
        if let Err(e) = saved {
            let _gone = std::fs::remove_dir_all(&dir);
            return Err(if e.raw_os_error() == Some(18) {
                // EXDEV: a mount point in between.
                NetError::OtherDisk
            } else if e.kind() == io::ErrorKind::NotFound && std::fs::symlink_metadata(&real).is_err() {
                // Removed by someone else in between: as rm finds it, missing.
                NetError::Missing
            } else {
                NetError::Io(e)
            });
        }
        self.mark_ran(plan);
        Ok(item)
    }

    /// Keeps a copy of the file at `path` (a symlink's target) in `plan`. `Ok(None)`: there is no file to keep (a new
    /// file). Larger than `max` bytes: [`NetError::TooLarge`].
    pub fn snapshot(&self, plan: &str, path: &Path, max: u64) -> Result<Option<Item>, NetError> {
        let real = match std::fs::canonicalize(path) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let meta = std::fs::metadata(&real)?;
        if !meta.is_file() {
            return Ok(None);
        }
        if meta.len() > max {
            return Err(NetError::TooLarge);
        }
        if self.fault != Some(Fault::DiskFull)
            && free_bytes(&self.root).is_some_and(|f| f < meta.len().saturating_add(MIN_FREE))
        {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "not enough space to keep a copy").into());
        }
        let root = self.ensure_root()?;
        let real_s = real.to_str().ok_or_else(|| NetError::Unsafe("a name that is not text".to_owned()))?.to_owned();
        if !reins_policy::effects::nets::trash_safe(&real_s, &root.to_string_lossy()) {
            return Err(NetError::Unsafe("Reins keeps its trash there".to_owned()));
        }
        let parent = real.parent().map(Path::to_path_buf).unwrap_or_default();
        self.make_room(plan, meta.len())?;
        let (id, dir) = self.new_item(plan)?;
        let item = Item {
            id,
            at: crate::now_unix(),
            kind: ItemKind::Snapshot,
            original: real_s,
            parent: parent.to_string_lossy().into_owned(),
            what: "file".to_owned(),
            worktree: None,
            mode: mode_of(&meta),
            bytes: meta.len(),
        };
        let saved = serde_json::to_vec_pretty(&item)
            .map_err(|e| io::Error::other(e.to_string()))
            .and_then(|b| self.write(&dir.join("item.json"), &b))
            .and_then(|()| {
                if self.fault == Some(Fault::DiskFull) {
                    return Err(io::Error::new(io::ErrorKind::StorageFull, "No space left on device"));
                }
                std::fs::copy(&real, dir.join("data")).map(|_| ())
            });
        if let Err(e) = saved {
            let _gone = std::fs::remove_dir_all(&dir);
            return Err(e.into());
        }
        self.mark_ran(plan);
        Ok(Some(item))
    }

    /// Records a git backup ref of `plan`.
    pub fn record_ref(&self, plan: &str, r: &RefBackup) -> Result<(), NetError> {
        let dir = self.plan_dir(plan)?;
        self.ensure_root()?;
        std::fs::create_dir_all(&dir)?;
        if self.fault == Some(Fault::DiskFull) {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "No space left on device").into());
        }
        let mut line = serde_json::to_vec(r).map_err(|e| io::Error::other(e.to_string()))?;
        line.push(b'\n');
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("refs.jsonl"))?;
        io::Write::write_all(&mut f, &line)?;
        drop(f);
        self.mark_ran(plan);
        Ok(())
    }

    /// The next backup ref name of `plan` (`refs/reins/backup/<plan>/<n>`), unique in this process and across runs.
    #[must_use]
    pub fn backup_ref(plan: &str) -> String {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        format!("refs/reins/backup/{plan}/{nanos:x}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
    }

    /// `plan`'s items, newest first.
    #[must_use]
    pub fn items(&self, plan: &str) -> Vec<Item> {
        let Ok(dir) = self.plan_dir(plan) else {
            return Vec::new();
        };
        let mut out: Vec<Item> = std::fs::read_dir(dir.join("items"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let item: Item = serde_json::from_slice(&std::fs::read(e.path().join("item.json")).ok()?).ok()?;
                // The folder's own name, whatever the record says.
                (item.id == e.file_name().to_string_lossy()).then_some(item)
            })
            .collect();
        out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
        out
    }

    /// `plan`'s backup refs, oldest first.
    #[must_use]
    pub fn refs(&self, plan: &str) -> Vec<RefBackup> {
        let Ok(dir) = self.plan_dir(plan) else {
            return Vec::new();
        };
        std::fs::read_to_string(dir.join("refs.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Whether `plan` was restored.
    #[must_use]
    pub fn restored(&self, plan: &str) -> bool {
        self.plan_dir(plan).is_ok_and(|d| d.join("restored").exists())
    }

    /// Every plan with something kept, newest first.
    #[must_use]
    pub fn list(&self) -> Vec<NetSummary> {
        let mut out: Vec<NetSummary> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let id = e.file_name().to_string_lossy().into_owned();
                self.summary(&id)
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.at));
        out
    }

    /// One plan's summary; `None` when it keeps nothing.
    #[must_use]
    pub fn summary(&self, id: &str) -> Option<NetSummary> {
        if !reins_policy::effects::nets::valid_plan_id(id) {
            return None;
        }
        let items = self.items(id);
        let refs = self.refs(id);
        if items.is_empty() && refs.is_empty() {
            return None;
        }
        let record = self.plan(id);
        let at = items.iter().map(|i| i.at).chain(refs.iter().map(|r| r.at)).min().unwrap_or_default();
        let until = items
            .iter()
            .map(|i| i.at + TRASH_DAYS * DAY)
            .chain(refs.iter().map(|r| r.at + REF_DAYS * DAY))
            .max()
            .unwrap_or_default();
        let mut lines = record.map(|r| r.lines).unwrap_or_default();
        if lines.is_empty() {
            let trashed = items.iter().filter(|i| i.kind == ItemKind::Trash).count();
            let copies = items.len() - trashed;
            if trashed > 0 {
                lines.push(format!("{trashed} in the Reins trash"));
            }
            if copies > 0 {
                lines.push(format!("{copies} file copies"));
            }
            if !refs.is_empty() {
                lines.push(format!("{} backup refs", refs.len()));
            }
        }
        Some(NetSummary {
            id: id.to_owned(),
            at,
            lines,
            items: items.len(),
            refs: refs.len(),
            until,
            restored: self.restored(id),
        })
    }

    /// Puts `item` back. A trashed path goes back where it was, only into the same real folder, and never over
    /// something that is there now. A snapshot is written over the file, after the file as it is now is kept as a
    /// new snapshot of the same plan.
    pub fn restore_item(&self, plan: &str, item: &Item) -> Result<String, String> {
        let dir = self.plan_dir(plan).map_err(|e| e.to_string())?.join("items").join(&item.id);
        let (original, parent) = self.place_of(item)?;
        let data = dir.join("data");
        match item.kind {
            ItemKind::Trash => {
                // Something is there again (a hard reset wrote the tracked file there): never over it, beside it.
                let (to, beside) = if std::fs::symlink_metadata(&original).is_ok() {
                    let name = original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (parent.join(format!("{name}.reins-{plan}")), true)
                } else {
                    (original, false)
                };
                if std::fs::symlink_metadata(&to).is_ok() {
                    return Err(format!("{} is there again; move it away first", item.original));
                }
                std::fs::rename(&data, &to).map_err(|e| format!("{}: {e}", item.original))?;
                let _done = std::fs::remove_dir_all(&dir);
                Ok(if beside {
                    format!("Put back beside {} as {}", item.original, to.display())
                } else {
                    format!("Put back {}", item.original)
                })
            }
            ItemKind::Snapshot => {
                if std::fs::symlink_metadata(&original).is_ok_and(|m| !m.is_file()) {
                    return Err(format!("{} is not a file now", item.original));
                }
                if original.exists() {
                    self.snapshot(plan, &original, u64::MAX).map_err(|e| format!("{}: {e}", item.original))?;
                }
                let mut tmp = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
                let mut src = std::fs::File::open(&data).map_err(|e| e.to_string())?;
                io::copy(&mut src, &mut tmp).map_err(|e| e.to_string())?;
                let file = tmp.persist(&original).map_err(|e| e.error.to_string())?;
                // The mode it had (a temporary file is the owner's only).
                #[cfg(unix)]
                if let Some(mode) = item.mode {
                    use std::os::unix::fs::PermissionsExt as _;
                    file.set_permissions(std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
                }
                drop(file);
                let _done = std::fs::remove_dir_all(&dir);
                Ok(format!("Put back the earlier {}", item.original))
            }
        }
    }

    /// Where `item` goes back: its original path and parent, checked. The path is absolute and plain, not in the trash,
    /// and the parent is still where it really was (made again when it is gone, then checked; never a symlink to
    /// somewhere else now).
    pub fn place_of(&self, item: &Item) -> Result<(PathBuf, PathBuf), String> {
        let original = PathBuf::from(&item.original);
        let parent = PathBuf::from(&item.parent);
        let root = self.real_root();
        if !plain_absolute(&original)
            || !plain_absolute(&parent)
            || original.parent() != Some(parent.as_path())
            || !reins_policy::effects::nets::trash_safe(&item.original, &root.to_string_lossy())
        {
            return Err(format!("{} is not a place to restore to", item.original));
        }
        if !parent.exists() {
            std::fs::create_dir_all(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let real_parent = std::fs::canonicalize(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        if real_parent != parent {
            return Err(format!("{} now leads somewhere else", parent.display()));
        }
        Ok((original, parent))
    }

    /// Marks `plan` as run: `reins net run` used it, so its net was applied (Activity says Netted only then).
    pub fn mark_ran(&self, plan: &str) {
        if let Ok(dir) = self.plan_dir(plan)
            && !dir.join("ran").exists()
        {
            let _kept = self.write(&dir.join("ran"), crate::now_unix().to_string().as_bytes());
        }
    }

    /// Whether `plan`'s net was applied: `reins net run` ran it, or (file tools) the hook took its copies.
    #[must_use]
    pub fn ran(&self, plan: &str) -> bool {
        self.plan_dir(plan).is_ok_and(|d| d.join("ran").exists())
    }

    /// Marks `plan` restored.
    pub fn mark_restored(&self, plan: &str) -> Result<(), String> {
        let dir = self.plan_dir(plan).map_err(|e| e.to_string())?;
        self.write(&dir.join("restored"), crate::now_unix().to_string().as_bytes()).map_err(|e| e.to_string())
    }

    /// Records that a net of the command with `key` failed: it is planned without the net for an hour.
    pub fn fail(&self, key: &str, why: &str) {
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return;
        }
        let _kept =
            self.write(&self.root.join(".failed").join(key), format!("{}\n{why}\n", crate::now_unix()).as_bytes());
    }

    /// [`Self::fail`] for the command of `plan`.
    pub fn fail_plan(&self, plan: &str, why: &str) {
        if let Some(p) = self.plan(plan) {
            self.fail(&p.key, why);
        }
    }

    /// The net of the command with `key` failed in the last hour.
    #[must_use]
    pub fn failed_recently(&self, key: &str, now: i64) -> bool {
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return false;
        }
        std::fs::read_to_string(self.root.join(".failed").join(key))
            .ok()
            .and_then(|t| t.lines().next().and_then(|l| l.trim().parse::<i64>().ok()))
            .is_some_and(|at| now - at < FAILED_SECS)
    }

    /// Drops what is older than its keeping time: items after [`TRASH_DAYS`], backup refs after [`REF_DAYS`] (deleted
    /// from their repository with `drop_ref`), failures after an hour, and plans with nothing left. How many items and
    /// refs went.
    pub fn prune(&self, now: i64, drop_ref: &dyn Fn(&RefBackup) -> bool) -> usize {
        let mut gone = 0;
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".failed" {
                for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
                    let key = f.file_name().to_string_lossy().into_owned();
                    if !self.failed_recently(&key, now) {
                        let _gone = std::fs::remove_file(f.path());
                    }
                }
                continue;
            }
            if !reins_policy::effects::nets::valid_plan_id(&name) {
                continue;
            }
            // Items an earlier prune claimed but couldn't remove (a read-only folder inside): again.
            for f in std::fs::read_dir(e.path().join("items")).into_iter().flatten().flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                if n.starts_with('.') && n.ends_with(".pruning") && remove_tree(&f.path()) {
                    gone += 1;
                }
            }
            for item in self.items(&name) {
                if now - item.at >= TRASH_DAYS * DAY && self.drop_item(&name, &item.id) {
                    gone += 1;
                }
            }
            let refs = self.refs(&name);
            let (old, young): (Vec<RefBackup>, Vec<RefBackup>) =
                refs.into_iter().partition(|r| now - r.at >= REF_DAYS * DAY);
            if !old.is_empty() {
                let mut keep = young;
                for r in old {
                    if drop_ref(&r) {
                        gone += 1;
                    } else {
                        keep.push(r);
                    }
                }
                let text: String =
                    keep.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
                let _kept = crate::config::write_private(&e.path().join("refs.jsonl"), text.as_bytes());
            }
            // Empty and a day old (a plan the hook just wrote waits for its run).
            let born = self.plan(&name).map(|p| p.at).or_else(|| {
                let m = e.metadata().ok()?.modified().ok()?;
                m.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok())
            });
            if self.items(&name).is_empty() && self.refs(&name).is_empty() && born.is_none_or(|b| now - b >= DAY) {
                remove_tree(&e.path());
            }
        }
        gone
    }
}

#[cfg(all(test, unix))]
mod tests;
