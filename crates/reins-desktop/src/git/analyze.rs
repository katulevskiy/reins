//! What a push does, from the exact commands and pack git sent, with the server for what the pack builds on.

use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reins_proto::desktop::{
    CommitInfo, FileChange, FileStatus, MAX_COMMITS, MAX_FILES, MAX_NOTES, MAX_PUSH_OPTIONS, PushSummary, RefChange,
    RefUpdate,
};

use super::object::{Commit, EntryKind, ObjectKind, Oid, TreeEntry, hash_object, parse_commit, parse_tag, parse_tree};
use super::pack::{MAX_OBJECT, Pack, PackError};
use super::pktline::Command;
use super::remote::{CommitMeta, Remote, RemoteError};

/// Commits walked per ref before giving "at least N".
const MAX_WALK: usize = 10_000;
/// Files compared per ref.
const MAX_COUNTED: usize = 5_000;
/// Boundary commits whose ancestry is asked per ref.
const MAX_BOUNDARY_CHECKS: usize = 20;
/// Thin-pack bases fetched from the server per push.
const MAX_BASES: usize = 60;
/// Line counts: files up to this size on both sides, this much read in total per push.
const MAX_TEXT: u64 = 1 << 20;
const LINE_BUDGET: u64 = 32 << 20;
const DIFF_TIMEOUT: Duration = Duration::from_secs(1);
/// A NUL byte this early means binary, as git decides.
const BINARY_SNIFF: usize = 8 << 10;
const MAX_TREE: u64 = 16 << 20;
/// Objects fetched from the server and kept for the rest of the analysis.
const FETCHED_BYTES: u64 = 64 << 20;
const MAX_LINE: usize = 200;
const MAX_PATH: usize = 1_024;

/// Describes a push for the user. Never fails: what cannot be worked out is left out and said in `notes`, and an
/// update whose ancestry is unknown has `fast_forward: None` (asked every time, like a force push).
///
/// `pack`: the pack git sent, when it sent one (a file holding exactly the pack bytes).
///
/// Refs are described as git sent them, never dropped: a push the summary cannot describe (more refs than the phone
/// shows, SHA-256 ids) fails [`PushSummary::validate`] and is refused rather than shown in part.
pub async fn analyze_push(
    repo: &str,
    commands: &[Command],
    push_options: &[String],
    pack: Option<&Path>,
    remote: &dyn Remote,
) -> PushSummary {
    let mut notes = Notes::default();
    let pack_bytes = match pack {
        Some(p) => tokio::fs::metadata(p).await.map_or(0, |m| m.len()),
        None => 0,
    };
    let mut store = Store {
        pack: None,
        remote,
        fetched: HashMap::new(),
        fetched_bytes: 0,
        missing: HashSet::new(),
        commits: HashMap::new(),
        remote_dead: None,
    };
    if let Some(path) = pack {
        store.pack = open_pack(path, remote, &mut notes).await;
    }
    let mut updates = Vec::with_capacity(commands.len());
    let mut diffs = Vec::with_capacity(commands.len());
    for c in commands {
        let (update, diff) = describe(&mut store, c, &mut notes).await;
        updates.push(update);
        diffs.push(diff);
    }
    count_lines(&mut store, &mut updates, diffs, &mut notes).await;
    let mut options: Vec<String> =
        push_options.iter().take(MAX_PUSH_OPTIONS).map(|o| non_empty(clip(o, MAX_LINE), "(empty)")).collect();
    if push_options.len() > MAX_PUSH_OPTIONS {
        notes.add(format!("{} more push options are not shown.", push_options.len() - MAX_PUSH_OPTIONS));
        options.truncate(MAX_PUSH_OPTIONS);
    }
    if let Some(e) = &store.remote_dead {
        notes.add(format!("GitHub: {e}"));
    }
    log::debug!("analyzed a push to {repo}: {} refs, {pack_bytes} pack bytes", updates.len());
    PushSummary {
        updates,
        pack_bytes,
        push_options: options,
        notes: notes.finish(),
    }
}

/// Notes for the user: one line each, deduplicated, at most [`MAX_NOTES`].
#[derive(Default)]
struct Notes(Vec<String>);

impl Notes {
    fn add(&mut self, note: impl AsRef<str>) {
        let note = clip(note.as_ref(), MAX_LINE);
        if !note.is_empty() && !self.0.contains(&note) {
            self.0.push(note);
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.0.len() > MAX_NOTES {
            let more = self.0.len() - (MAX_NOTES - 1);
            self.0.truncate(MAX_NOTES - 1);
            self.0.push(format!("{more} more problems are not listed."));
        }
        self.0
    }
}

/// One line without control characters, at most `max` characters (cut with "…").
fn clip(s: &str, max: usize) -> String {
    let line: String = s
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let line = line.trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut out: String = line.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn non_empty(s: String, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_owned()
    } else {
        s
    }
}

/// A path as shown: control characters replaced, and a path too long cut in the middle so both ends stay visible.
fn show_path(path: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(path)
        .chars()
        .map(|c| {
            if c.is_control() {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect();
    let count = text.chars().count();
    if count <= MAX_PATH {
        return text;
    }
    let head = (MAX_PATH - 1) / 2;
    let tail = MAX_PATH - 1 - head;
    let mut out: String = text.chars().take(head).collect();
    out.push('…');
    out.extend(text.chars().skip(count - tail));
    out
}

enum Got {
    Found(ObjectKind, Arc<[u8]>),
    Missing,
    TooLarge,
    Failed(String),
}

/// Objects from the pack, else the server (checked against their id, kept within a size bound).
struct Store<'a> {
    pack: Option<Pack>,
    remote: &'a dyn Remote,
    fetched: HashMap<Oid, (ObjectKind, Arc<[u8]>)>,
    fetched_bytes: u64,
    missing: HashSet<Oid>,
    commits: HashMap<Oid, Option<CommitMeta>>,
    /// Set once the server stops answering (budget used up): no more requests.
    remote_dead: Option<String>,
}

impl Store<'_> {
    fn pack_has(&self, oid: &Oid, kind: ObjectKind) -> bool {
        self.pack.as_ref().is_some_and(|p| p.has(oid) && p.header(oid).is_some_and(|(k, _)| k == kind))
    }

    fn size_hint(&self, oid: &Oid) -> Option<u64> {
        if let Some((_, d)) = self.fetched.get(oid) {
            return Some(d.len() as u64);
        }
        self.pack.as_ref().and_then(|p| p.header(oid)).map(|(_, len)| len)
    }

    fn remote_error(&mut self, e: RemoteError) -> Got {
        match e {
            RemoteError::TooLarge => Got::TooLarge,
            RemoteError::Budget => {
                self.remote_dead = Some(e.to_string());
                Got::Failed(e.to_string())
            }
            RemoteError::Failed(m) => Got::Failed(m),
        }
    }

    async fn object(&mut self, oid: &Oid, kind: Option<ObjectKind>, max: u64) -> Got {
        if let Some(pack) = &mut self.pack
            && let Some((_, len)) = pack.header(oid)
        {
            if len > max {
                return Got::TooLarge;
            }
            match pack.read(oid) {
                Ok(Some((k, d))) => return Got::Found(k, d),
                Ok(None) => {}
                Err(PackError::TooLarge) => return Got::TooLarge,
                Err(e) => return Got::Failed(e.to_string()),
            }
        }
        if let Some((k, d)) = self.fetched.get(oid) {
            return Got::Found(*k, Arc::clone(d));
        }
        if self.missing.contains(oid) {
            return Got::Missing;
        }
        if let Some(e) = &self.remote_dead {
            return Got::Failed(e.clone());
        }
        match self.remote.object(oid, kind, max).await {
            Ok(Some((k, data))) => {
                if hash_object(k, &data) != *oid {
                    return Got::Failed(format!("the server's object {oid} does not match its id"));
                }
                if data.len() as u64 > max {
                    return Got::TooLarge;
                }
                let data: Arc<[u8]> = data.into();
                if self.fetched_bytes + data.len() as u64 <= FETCHED_BYTES {
                    self.fetched_bytes += data.len() as u64;
                    self.fetched.insert(*oid, (k, Arc::clone(&data)));
                }
                Got::Found(k, data)
            }
            Ok(None) => {
                self.missing.insert(*oid);
                Got::Missing
            }
            Err(e) => self.remote_error(e),
        }
    }

    /// A commit the pack carries, parsed.
    fn pack_commit(&mut self, oid: &Oid) -> Option<Result<Commit, String>> {
        if !self.pack_has(oid, ObjectKind::Commit) {
            return None;
        }
        let pack = self.pack.as_mut()?;
        Some(match pack.read(oid) {
            Ok(Some((_, data))) => parse_commit(&data),
            Ok(None) => Err("not readable".to_owned()),
            Err(e) => Err(e.to_string()),
        })
    }

    async fn commit_meta(&mut self, oid: &Oid) -> Result<CommitMeta, String> {
        if let Some(c) = self.pack_commit(oid) {
            return c.map(|c| CommitMeta {
                tree: c.tree,
                parents: c.parents,
            });
        }
        if let Some(c) = self.commits.get(oid) {
            return c.clone().ok_or_else(|| format!("commit {} is not available", short(oid)));
        }
        if let Some(e) = &self.remote_dead {
            return Err(e.clone());
        }
        match self.remote.commit(oid).await {
            Ok(c) => {
                self.commits.insert(*oid, c.clone());
                c.ok_or_else(|| format!("commit {} is not available", short(oid)))
            }
            Err(e) => match self.remote_error(e) {
                Got::Failed(m) => Err(m),
                _ => Err("the commit is too large".to_owned()),
            },
        }
    }

    async fn tree(&mut self, oid: &Oid) -> Result<Vec<TreeEntry>, String> {
        match self.object(oid, Some(ObjectKind::Tree), MAX_TREE).await {
            Got::Found(ObjectKind::Tree, data) => parse_tree(&data),
            Got::Found(..) => Err(format!("{} is not a tree", short(oid))),
            Got::Missing => Err(format!("tree {} is not available", short(oid))),
            Got::TooLarge => Err(format!("tree {} is too large", short(oid))),
            Got::Failed(m) => Err(m),
        }
    }

    /// Follows annotated tags in the pack to what they point at (a tag the server has is left as is).
    fn peel(&mut self, oid: Oid) -> Oid {
        let mut cur = oid;
        for _ in 0..8 {
            if !self.pack_has(&cur, ObjectKind::Tag) {
                break;
            }
            let Some(pack) = self.pack.as_mut() else {
                break;
            };
            match pack.read(&cur) {
                Ok(Some((_, data))) => match parse_tag(&data) {
                    Ok(t) => cur = t.object,
                    Err(_) => break,
                },
                _ => break,
            }
        }
        cur
    }
}

fn short(oid: &Oid) -> String {
    oid.to_hex()[..7].to_owned()
}

/// Opens the pack and resolves what it builds on from the server. A pack that cannot be read is a note.
async fn open_pack(path: &Path, remote: &dyn Remote, notes: &mut Notes) -> Option<Pack> {
    let owned = path.to_owned();
    let mut pack = match tokio::task::spawn_blocking(move || Pack::open(&owned)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            notes.add(format!("The pack could not be read ({e}); commits and files are unknown."));
            return None;
        }
        Err(e) => {
            notes.add(format!("The pack could not be read ({e}); commits and files are unknown."));
            return None;
        }
    };
    let mut tried = HashSet::new();
    'fetch: loop {
        let wanted: Vec<Oid> = pack.missing_bases().into_iter().filter(|o| !tried.contains(o)).collect();
        if wanted.is_empty() {
            break;
        }
        for oid in wanted {
            if tried.len() >= MAX_BASES {
                notes.add("Too many objects of this push build on GitHub's; some were not read.");
                break 'fetch;
            }
            tried.insert(oid);
            match remote.object(&oid, None, MAX_OBJECT).await {
                Ok(Some((kind, data))) => {
                    let joined = tokio::task::spawn_blocking(move || {
                        let r = pack.add_base(oid, kind, data);
                        (pack, r)
                    })
                    .await;
                    match joined {
                        Ok((p, r)) => {
                            pack = p;
                            if let Err(e) = r {
                                notes.add(format!("A base object from GitHub was refused ({e})."));
                            }
                        }
                        Err(e) => {
                            notes.add(format!("The pack could not be read ({e}); commits and files are unknown."));
                            return None;
                        }
                    }
                }
                Ok(None) => {}
                Err(RemoteError::Budget) => {
                    notes.add(format!("GitHub: {}", RemoteError::Budget));
                    break 'fetch;
                }
                Err(e) => notes.add(format!("GitHub: {e}")),
            }
        }
    }
    if let Some(e) = pack.damage() {
        notes.add(format!("The pack is damaged ({e}); GitHub will likely refuse it."));
    }
    let unresolved = pack.unresolved();
    if unresolved > 0 {
        notes.add(format!(
            "{unresolved} of {} objects in the pack could not be read (their base is missing or too large).",
            pack.object_count()
        ));
    }
    Some(pack)
}

/// A file that differs, with both sides for the line count.
struct Diffed {
    path: Vec<u8>,
    status: FileStatus,
    old: Option<(Oid, EntryKind)>,
    new: Option<(Oid, EntryKind)>,
}

/// The files of one ref; `complete` when nothing was left out.
#[derive(Default)]
struct RefDiff {
    files: Vec<Diffed>,
    complete: bool,
}

fn change_of(old: &str, new: &str) -> RefChange {
    let zero = |s: &str| s.bytes().all(|b| b == b'0');
    match (zero(old), zero(new)) {
        (true, _) => RefChange::Create,
        (_, true) => RefChange::Delete,
        _ => RefChange::Update,
    }
}

async fn describe(store: &mut Store<'_>, c: &Command, notes: &mut Notes) -> (RefUpdate, RefDiff) {
    let mut u = RefUpdate {
        name: c.name.clone(),
        change: change_of(&c.old, &c.new),
        old: c.old.clone(),
        new: c.new.clone(),
        fast_forward: None,
        commit_count: 0,
        commits: Vec::new(),
        files_changed: 0,
        files: Vec::new(),
        additions: None,
        deletions: None,
    };
    let none = RefDiff::default();
    let (Some(old), Some(new)) = (Oid::from_hex(&c.old), Oid::from_hex(&c.new)) else {
        notes.add("SHA-256 repositories are not supported.");
        return (u, none);
    };
    let name = clip(&c.name, 80);
    if u.change == RefChange::Delete {
        return (
            u,
            RefDiff {
                files: Vec::new(),
                complete: true,
            },
        );
    }
    let old = (u.change == RefChange::Update).then_some(old);
    let target = store.peel(new);
    let walk = walk(store, target, old, notes);
    u.commit_count = u32::try_from(walk.count).unwrap_or(u32::MAX);
    u.commits = walk.listed.clone();
    if walk.capped {
        notes.add(format!("{name}: at least {} commits (not all were read).", walk.count));
    }
    if let Some(old) = old {
        u.fast_forward = fast_forward(store, &name, old, target, &walk, notes).await;
    }
    let base = match (old, &walk.first_parent) {
        (Some(old), _) => Some(old),
        (None, FirstParent::NoNewCommits) => {
            return (
                u,
                RefDiff {
                    files: Vec::new(),
                    complete: true,
                },
            );
        }
        (None, FirstParent::Base(b)) => Some(*b),
        (None, FirstParent::Root) => {
            notes.add(format!("{name}: new history with no parent on GitHub; files are not compared."));
            return (u, none);
        }
        (None, FirstParent::Unknown) => {
            notes.add(format!("{name}: where this history starts is unknown; files are not compared."));
            return (u, none);
        }
    };
    let diff = match diff_commits(store, base, target, notes, &name).await {
        Ok(d) => d,
        Err(e) => {
            notes.add(format!("{name}: files could not be compared ({e})."));
            none
        }
    };
    u.files_changed = u32::try_from(diff.files.len()).unwrap_or(u32::MAX);
    (u, diff)
}

enum FirstParent {
    /// The ref points at a commit the server has.
    NoNewCommits,
    /// The first-parent chain ends in the pack (a new root commit).
    Root,
    /// The first parent outside the pack: where a new branch starts.
    Base(Oid),
    Unknown,
}

struct Walk {
    count: usize,
    listed: Vec<CommitInfo>,
    boundaries: Vec<Oid>,
    reached_old: bool,
    /// Stopped at [`MAX_WALK`].
    capped: bool,
    /// A commit of the pack could not be read: its parents are unknown.
    unreadable: bool,
    first_parent: FirstParent,
}

fn commit_info(oid: &Oid, c: &Commit) -> CommitInfo {
    CommitInfo {
        sha: oid.to_hex(),
        subject: clip(&c.subject, MAX_LINE),
        author: non_empty(clip(&c.author, MAX_LINE), "unknown"),
        date: c.author_time,
    }
}

/// Walks the commits the pack brings, newest first, from `start` down to commits the server has (or `old`).
fn walk(store: &mut Store<'_>, start: Oid, old: Option<Oid>, notes: &mut Notes) -> Walk {
    let mut w = Walk {
        count: 0,
        listed: Vec::new(),
        boundaries: Vec::new(),
        reached_old: Some(start) == old,
        capped: false,
        unreadable: false,
        first_parent: FirstParent::NoNewCommits,
    };
    if w.reached_old || !store.pack_has(&start, ObjectKind::Commit) {
        return w;
    }
    let mut queue = Queue::default();
    let mut parents: HashMap<Oid, Vec<Oid>> = HashMap::new();
    let mut seen = HashSet::from([start]);
    let mut boundary_seen = HashSet::new();
    queue.discover(store, start, &mut w, notes);
    while let Some((oid, c)) = queue.pop() {
        if w.count >= MAX_WALK {
            w.capped = true;
            break;
        }
        w.count += 1;
        if w.listed.len() < MAX_COMMITS {
            w.listed.push(commit_info(&oid, &c));
        }
        for p in &c.parents {
            if Some(*p) == old {
                w.reached_old = true;
            } else if store.pack_has(p, ObjectKind::Commit) {
                if seen.insert(*p) {
                    queue.discover(store, *p, &mut w, notes);
                }
            } else if boundary_seen.insert(*p) {
                w.boundaries.push(*p);
            }
        }
        parents.insert(oid, c.parents);
    }
    w.first_parent = first_parent(&parents, start, old, store);
    w
}

/// Commits of the walk, parsed once when found, taken newest committer time first.
#[derive(Default)]
struct Queue {
    heap: BinaryHeap<(i64, Oid)>,
    found: HashMap<Oid, Commit>,
}

impl Queue {
    fn discover(&mut self, store: &mut Store<'_>, oid: Oid, w: &mut Walk, notes: &mut Notes) {
        match store.pack_commit(&oid) {
            Some(Ok(c)) => {
                self.heap.push((c.commit_time, oid));
                self.found.insert(oid, c);
            }
            Some(Err(e)) => {
                notes.add(format!("Commit {} could not be read ({e}).", short(&oid)));
                w.unreadable = true;
            }
            None => w.unreadable = true,
        }
    }

    fn pop(&mut self) -> Option<(Oid, Commit)> {
        let (_, oid) = self.heap.pop()?;
        self.found.remove(&oid).map(|c| (oid, c))
    }
}

fn first_parent(parents: &HashMap<Oid, Vec<Oid>>, start: Oid, old: Option<Oid>, store: &Store<'_>) -> FirstParent {
    let mut cur = start;
    for _ in 0..=MAX_WALK {
        let Some(ps) = parents.get(&cur) else {
            return FirstParent::Unknown;
        };
        let Some(&first) = ps.first() else {
            return FirstParent::Root;
        };
        if Some(first) == old || !store.pack_has(&first, ObjectKind::Commit) {
            return FirstParent::Base(first);
        }
        cur = first;
    }
    FirstParent::Unknown
}

/// The walk reaching `old` is a fast-forward; else `old` must be an ancestor of a commit the walk stopped at (asked
/// of the server). With no new commits, of `new` itself.
async fn fast_forward(
    store: &mut Store<'_>,
    name: &str,
    old: Oid,
    new: Oid,
    walk: &Walk,
    notes: &mut Notes,
) -> Option<bool> {
    if walk.reached_old {
        return Some(true);
    }
    let candidates = if walk.count == 0 {
        vec![new]
    } else {
        walk.boundaries.clone()
    };
    let mut all_false = !walk.capped && !walk.unreadable && candidates.len() <= MAX_BOUNDARY_CHECKS;
    for b in candidates.iter().take(MAX_BOUNDARY_CHECKS) {
        if let Some(e) = &store.remote_dead {
            notes.add(format!("{name}: history could not be checked ({e})."));
            return None;
        }
        match store.remote.is_ancestor(&old, b).await {
            Ok(Some(true)) => return Some(true),
            Ok(Some(false)) => {}
            Ok(None) => all_false = false,
            Err(e) => {
                if e == RemoteError::Budget {
                    store.remote_dead = Some(e.to_string());
                }
                notes.add(format!("{name}: history could not be checked ({e})."));
                all_false = false;
            }
        }
    }
    if all_false {
        Some(false)
    } else {
        notes.add(format!("{name}: whether this push rewrites history is unknown."));
        None
    }
}

async fn diff_commits(
    store: &mut Store<'_>,
    base: Option<Oid>,
    new: Oid,
    notes: &mut Notes,
    name: &str,
) -> Result<RefDiff, String> {
    let new_tree = store.commit_meta(&new).await?.tree;
    let old_tree = match base {
        Some(b) => Some(store.commit_meta(&b).await?.tree),
        None => None,
    };
    let mut diff = RefDiff {
        files: Vec::new(),
        complete: true,
    };
    let mut stack: Vec<(Vec<u8>, Option<Oid>, Option<Oid>)> = vec![(Vec::new(), old_tree, Some(new_tree))];
    'trees: while let Some((prefix, a, b)) = stack.pop() {
        if a == b {
            continue;
        }
        let mut sides: [Vec<TreeEntry>; 2] = [Vec::new(), Vec::new()];
        for (side, oid) in sides.iter_mut().zip([a, b]) {
            if let Some(oid) = oid {
                match store.tree(&oid).await {
                    Ok(t) => *side = t,
                    Err(e) => {
                        let at = if prefix.is_empty() {
                            "the top".to_owned()
                        } else {
                            show_path(&prefix)
                        };
                        notes.add(format!("{name}: files under {at} could not be read ({e})."));
                        diff.complete = false;
                        continue 'trees;
                    }
                }
            }
        }
        let [old_entries, new_entries] = sides;
        let mut merged: BTreeMap<Vec<u8>, (Option<TreeEntry>, Option<TreeEntry>)> = BTreeMap::new();
        for e in old_entries {
            let key = e.name.clone();
            merged.entry(key).or_default().0 = Some(e);
        }
        for e in new_entries {
            let key = e.name.clone();
            merged.entry(key).or_default().1 = Some(e);
        }
        for (entry_name, (o, n)) in merged {
            if diff.files.len() >= MAX_COUNTED {
                notes.add(format!("{name}: more than {MAX_COUNTED} files changed; only that many are counted."));
                diff.complete = false;
                break 'trees;
            }
            let path = if prefix.is_empty() {
                entry_name
            } else {
                [prefix.as_slice(), b"/", &entry_name].concat()
            };
            let side = |e: &TreeEntry| (e.oid, e.kind());
            let file = |status, old: Option<&TreeEntry>, new: Option<&TreeEntry>| Diffed {
                path: path.clone(),
                status,
                old: old.map(side),
                new: new.map(side),
            };
            match (o, n) {
                (Some(o), Some(n)) if o.oid == n.oid && o.mode == n.mode => {}
                (Some(o), Some(n)) => match (o.kind() == EntryKind::Tree, n.kind() == EntryKind::Tree) {
                    (true, true) => stack.push((path, Some(o.oid), Some(n.oid))),
                    (true, false) => {
                        diff.files.push(file(FileStatus::Added, None, Some(&n)));
                        stack.push((path, Some(o.oid), None));
                    }
                    (false, true) => {
                        diff.files.push(file(FileStatus::Deleted, Some(&o), None));
                        stack.push((path, None, Some(n.oid)));
                    }
                    (false, false) => {
                        let status = if o.kind() == n.kind() {
                            FileStatus::Modified
                        } else {
                            FileStatus::TypeChanged
                        };
                        diff.files.push(file(status, Some(&o), Some(&n)));
                    }
                },
                (Some(o), None) if o.kind() == EntryKind::Tree => stack.push((path, Some(o.oid), None)),
                (Some(o), None) => diff.files.push(file(FileStatus::Deleted, Some(&o), None)),
                (None, Some(n)) if n.kind() == EntryKind::Tree => stack.push((path, None, Some(n.oid))),
                (None, Some(n)) => diff.files.push(file(FileStatus::Added, None, Some(&n))),
                (None, None) => {}
            }
        }
    }
    diff.files.sort_by(|x, y| x.path.cmp(&y.path));
    Ok(diff)
}

/// Content of one side for a line count.
enum Side {
    /// No file there, or a submodule.
    Absent,
    Text(Arc<[u8]>),
    Binary,
    /// Too large, over budget, or not available.
    Unknown,
}

fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_SNIFF)].contains(&0)
}

#[allow(clippy::naive_bytecount, reason = "a dependency is not worth it for files of at most 1 MiB")]
fn line_count(data: &[u8]) -> usize {
    let n = data.iter().filter(|&&b| b == b'\n').count();
    if data.last().is_some_and(|&b| b != b'\n') {
        n + 1
    } else {
        n
    }
}

fn lines_of(data: &[u8]) -> Vec<&[u8]> {
    data.split_inclusive(|&b| b == b'\n').collect()
}

/// Lines added and removed between two texts (Myers, approximated past a deadline).
fn diff_lines(old: &[u8], new: &[u8]) -> (usize, usize) {
    let (a, b) = (lines_of(old), lines_of(new));
    let ops =
        similar::capture_diff_slices_deadline(similar::Algorithm::Myers, &a, &b, Some(Instant::now() + DIFF_TIMEOUT));
    ops.iter().fold((0, 0), |(add, del), op| {
        let (tag, o, n) = op.as_tag_tuple();
        match tag {
            similar::DiffTag::Equal => (add, del),
            similar::DiffTag::Insert => (add + n.len(), del),
            similar::DiffTag::Delete => (add, del + o.len()),
            similar::DiffTag::Replace => (add + n.len(), del + o.len()),
        }
    })
}

struct LineBudget {
    left: u64,
}

async fn load_side(store: &mut Store<'_>, side: Option<(Oid, EntryKind)>, budget: &mut LineBudget) -> Side {
    let Some((oid, kind)) = side else {
        return Side::Absent;
    };
    if !matches!(kind, EntryKind::File | EntryKind::Link) {
        return Side::Absent;
    }
    if budget.left == 0 || store.size_hint(&oid).is_some_and(|n| n > MAX_TEXT || n > budget.left) {
        return Side::Unknown;
    }
    match store.object(&oid, Some(ObjectKind::Blob), MAX_TEXT.min(budget.left)).await {
        Got::Found(ObjectKind::Blob, data) => {
            budget.left -= data.len() as u64;
            if is_binary(&data) {
                Side::Binary
            } else {
                Side::Text(data)
            }
        }
        _ => Side::Unknown,
    }
}

/// Fills in line counts, listed files first, within [`LINE_BUDGET`]; lists at most [`MAX_FILES`] files per ref.
async fn count_lines(store: &mut Store<'_>, updates: &mut [RefUpdate], diffs: Vec<RefDiff>, notes: &mut Notes) {
    let mut budget = LineBudget {
        left: LINE_BUDGET,
    };
    let mut uncounted = 0usize;
    for (u, diff) in updates.iter_mut().zip(diffs) {
        if u.change == RefChange::Delete {
            continue;
        }
        let mut totals = diff.complete.then_some((0u64, 0u64));
        for (i, f) in diff.files.iter().enumerate() {
            let (counted, binary) = if f.old.map(|o| o.0) == f.new.map(|n| n.0) {
                (Some((0, 0)), false)
            } else {
                let old = load_side(store, f.old, &mut budget).await;
                let new = load_side(store, f.new, &mut budget).await;
                match (old, new) {
                    (Side::Binary, _) | (_, Side::Binary) => (None, true),
                    (Side::Unknown, _) | (_, Side::Unknown) => {
                        uncounted += 1;
                        totals = None;
                        (None, false)
                    }
                    (Side::Text(a), Side::Text(b)) => (Some(diff_lines(&a, &b)), false),
                    (Side::Text(t), _) => (Some((0, line_count(&t))), false),
                    (_, Side::Text(t)) => (Some((line_count(&t), 0)), false),
                    // Submodules have no lines.
                    _ => (None, false),
                }
            };
            // Binary files and submodules have no lines: they leave the totals as they are.
            if let (Some((add, del)), Some((ta, td))) = (counted, &mut totals) {
                *ta += add as u64;
                *td += del as u64;
            }
            if i < MAX_FILES {
                u.files.push(FileChange {
                    path: non_empty(show_path(&f.path), "?"),
                    status: f.status,
                    additions: counted.map(|c| u32::try_from(c.0).unwrap_or(u32::MAX)),
                    deletions: counted.map(|c| u32::try_from(c.1).unwrap_or(u32::MAX)),
                    binary,
                });
            }
        }
        (u.additions, u.deletions) = totals.unzip();
    }
    if uncounted > 0 {
        notes.add(format!("Lines were not counted in {uncounted} files (too large, or not available)."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_single_bounded_lines() {
        let mut n = Notes::default();
        n.add("a\nb");
        n.add("a\nb");
        n.add("x".repeat(500));
        for i in 0..20 {
            n.add(format!("note {i}"));
        }
        let out = n.finish();
        assert_eq!(out.len(), MAX_NOTES);
        assert_eq!(out[0], "a b");
        assert_eq!(out[1].chars().count(), MAX_LINE);
        assert!(out[1].ends_with('…'));
        assert_eq!(out[9], "13 more problems are not listed.");
    }

    #[test]
    fn long_paths_keep_both_ends() {
        let long = format!("{}/.github/workflows/x.yml", "a".repeat(2000));
        let shown = show_path(long.as_bytes());
        assert_eq!(shown.chars().count(), MAX_PATH);
        assert!(shown.starts_with("aaa"));
        assert!(shown.ends_with("/.github/workflows/x.yml"));
        assert_eq!(show_path(b"a\nb\xff"), "a\u{fffd}b\u{fffd}");
    }

    #[test]
    fn lines_are_counted_like_git() {
        assert_eq!(line_count(b""), 0);
        assert_eq!(line_count(b"a"), 1);
        assert_eq!(line_count(b"a\nb\n"), 2);
        assert_eq!(diff_lines(b"a\nb\nc\n", b"a\nB\nc\nd\n"), (2, 1));
        assert_eq!(diff_lines(b"a\nb", b"a\nb\n"), (1, 1));
        assert_eq!(diff_lines(b"", b""), (0, 0));
        assert!(is_binary(b"abc\0def"));
        assert!(!is_binary(&[b'a'; 10_000]));
        let mut late = vec![b'a'; BINARY_SNIFF];
        late.push(0);
        assert!(!is_binary(&late));
    }

    #[test]
    fn changes_follow_zero_ids() {
        let z = "0".repeat(40);
        let a = "1".repeat(40);
        assert_eq!(change_of(&z, &a), RefChange::Create);
        assert_eq!(change_of(&a, &z), RefChange::Delete);
        assert_eq!(change_of(&a, &a), RefChange::Update);
    }
}
