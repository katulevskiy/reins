//! The blob store (files spec, S1): large files held for one operation, under the phone's control.
//!
//! Bytes live in files under `$DATA_FOLDER/reins-blobs/` (0700, random names, wiped when the store is first used
//! after a start); everything else lives in memory and dies with the process, which is fine: every blob is short-lived.
//! Capabilities (upload and download URLs) are random 32-byte secrets; only their SHA-256 is kept and looked up, so
//! neither a memory dump nor a timing difference gives them away.
//!
//! Quotas ([`BlobLimits`], the `REINS_BLOB_*` settings) keep a public server from becoming free file hosting: files
//! and bytes held per account and in total, a per-file size, downloads per link, and the bytes each account moves in a
//! rolling day (stored by uploads, fetches and outputs; read by downloads, sends and the phone).

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
};

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use reins_proto::{
    PROTOCOL_VERSION,
    blob::{
        BlobId, BlobInfo, BlobPreview, BlobPurpose, BlobSlotRequest, BlobState, DEFAULT_BLOB_TTL_SECS, MAX_BLOB_BYTES,
        MAX_BLOB_TTL_SECS,
    },
    ids::{ConnectionId, RequestId},
};

use super::{relay::ItemSignal, sniff::valid_file_name};
use crate::crypto::{encode_random_bytes, get_random_bytes};

/// Default for the most files one account holds at once.
pub const DEFAULT_ACCOUNT_FILES: usize = 20;
/// Default for the most bytes one account holds (or has reserved for uploads in progress).
pub const DEFAULT_ACCOUNT_BYTES: u64 = 2 << 30;
/// Default for the most bytes one account moves through the store in a rolling day.
pub const DEFAULT_ACCOUNT_DAILY_BYTES: u64 = 20 << 30;
/// Default for the most bytes the whole server holds.
pub const DEFAULT_TOTAL_BYTES: u64 = 8 << 30;
/// Default for downloads per link.
pub const DEFAULT_MAX_DOWNLOADS: u32 = 20;
/// Directory under `DATA_FOLDER`.
pub const DIR_NAME: &str = "reins-blobs";

const HOUR: i64 = 3_600;
const DAY: i64 = 24 * HOUR;

/// The quotas of the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobLimits {
    /// Largest single file; larger requests (up to the contract's [`MAX_BLOB_BYTES`]) are capped to it.
    pub max_blob_bytes: u64,
    /// Files one account holds at once (stored, or slots still waiting; denied uploads hold nothing).
    pub account_files: usize,
    /// Bytes one account holds or has reserved at once.
    pub account_bytes: u64,
    /// Bytes one account moves in a rolling 24 hours; `0`: unlimited.
    pub account_daily_bytes: u64,
    /// Bytes all accounts hold together.
    pub total_bytes: u64,
    /// Downloads per link.
    pub max_downloads: u32,
}

impl Default for BlobLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: MAX_BLOB_BYTES,
            account_files: DEFAULT_ACCOUNT_FILES,
            account_bytes: DEFAULT_ACCOUNT_BYTES,
            account_daily_bytes: DEFAULT_ACCOUNT_DAILY_BYTES,
            total_bytes: DEFAULT_TOTAL_BYTES,
            max_downloads: DEFAULT_MAX_DOWNLOADS,
        }
    }
}

impl BlobLimits {
    /// The `REINS_BLOB_*` settings.
    pub fn from_config() -> Self {
        let config = &crate::CONFIG;
        Self {
            max_blob_bytes: config.reins_blob_max_bytes().clamp(1, MAX_BLOB_BYTES),
            account_files: usize::try_from(config.reins_blob_account_files()).unwrap_or(usize::MAX).max(1),
            account_bytes: config.reins_blob_account_bytes(),
            account_daily_bytes: config.reins_blob_account_daily_bytes(),
            total_bytes: config.reins_blob_total_bytes(),
            max_downloads: config.reins_blob_max_downloads().max(1),
        }
    }
}

/// `n` bytes for people: whole GiB, MiB or KiB when exact, else bytes.
pub fn human_bytes(n: u64) -> String {
    for (size, unit) in [(1u64 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")] {
        if n >= size && n.is_multiple_of(size) {
            return format!("{} {unit}", n / size);
        }
    }
    format!("{n} bytes")
}

/// A capability secret's lookup key.
type Key = [u8; 32];

fn key_of(secret: &str) -> Key {
    let digest = ring::digest::digest(&ring::digest::SHA256, secret.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(digest.as_ref());
    key
}

fn new_secret() -> String {
    encode_random_bytes::<32>(&BASE64URL_NOPAD)
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlobError {
    /// Unknown, expired, another user's, or not usable that way.
    NotFound,
    /// Refused input (the text says why).
    Invalid(String),
    /// This account holds or moved too much already (the text says what, and when to retry if waiting helps).
    UserLimit(String),
    /// The server holds too much already.
    ServerFull,
    /// The blob is in the wrong state for this (still waiting, being uploaded).
    Conflict(String),
    /// The user already decided on this upload.
    AlreadyDecided,
    /// The blob directory could not be prepared.
    Storage(String),
}

struct Entry {
    user: String,
    info: BlobInfo,
    max_bytes: u64,
    /// Bytes counted against the limits: `max_bytes` while waiting or receiving, the size once stored.
    reserved: u64,
    upload_key: Option<Key>,
    download_key: Option<Key>,
    /// Bytes are being written right now.
    receiving: bool,
    /// The daily transfer charged for the write in progress (`(hour, bytes)`), settled when it ends.
    charged: Option<(i64, u64)>,
    downloads_left: u32,
    /// Handed to the phone in `Pending.blobs` (uploads awaiting a decision only).
    delivered: bool,
    file: PathBuf,
}

impl Entry {
    fn expired(&self, now: i64) -> bool {
        now >= self.info.expires_at
    }

    /// The bytes are complete and may be read.
    fn stored(&self) -> bool {
        !self.receiving && matches!(self.info.state, BlobState::Uploaded | BlobState::Approved)
    }
}

/// Bytes moved per account, in hourly buckets over a rolling day.
#[derive(Default)]
struct Ledger {
    users: HashMap<String, VecDeque<(i64, u64)>>,
}

impl Ledger {
    fn counts(start: i64, now: i64) -> bool {
        start + DAY > now
    }

    fn prune(&mut self, now: i64) {
        self.users.retain(|_, buckets| {
            while buckets.front().is_some_and(|(start, _)| !Self::counts(*start, now)) {
                buckets.pop_front();
            }
            !buckets.is_empty()
        });
    }

    fn used(&self, user: &str, now: i64) -> u64 {
        self.users.get(user).map_or(0, |buckets| {
            buckets.iter().filter(|(start, _)| Self::counts(*start, now)).fold(0u64, |s, (_, b)| s.saturating_add(*b))
        })
    }

    /// Adds `bytes` to this hour's bucket; returns the bucket's start.
    fn charge(&mut self, user: &str, bytes: u64, now: i64) -> i64 {
        let start = now - now.rem_euclid(HOUR);
        let buckets = self.users.entry(user.to_owned()).or_default();
        match buckets.back_mut() {
            Some((last, total)) if *last == start => *total = total.saturating_add(bytes),
            _ => buckets.push_back((start, bytes)),
        }
        start
    }

    /// Takes back `bytes` charged to the bucket starting at `start` (if it still counts).
    fn refund(&mut self, user: &str, start: i64, bytes: u64) {
        if let Some((_, total)) = self.users.get_mut(user).and_then(|b| b.iter_mut().find(|(s, _)| *s == start)) {
            *total = total.saturating_sub(bytes);
        }
    }

    /// Seconds until the oldest bytes still counted stop counting.
    fn retry_after(&self, user: &str, now: i64) -> i64 {
        self.users
            .get(user)
            .and_then(|b| b.iter().find(|(start, total)| Self::counts(*start, now) && *total > 0))
            .map_or(1, |(start, _)| (start + DAY - now).max(1))
    }
}

#[derive(Default)]
struct State {
    blobs: HashMap<String, Entry>,
    uploads: HashMap<Key, String>,
    downloads: HashMap<Key, String>,
    ledger: Ledger,
}

impl State {
    fn remove(&mut self, id: &str) -> Option<Entry> {
        let entry = self.blobs.remove(id)?;
        if let Some(k) = entry.upload_key {
            self.uploads.remove(&k);
        }
        if let Some(k) = entry.download_key {
            self.downloads.remove(&k);
        }
        Some(entry)
    }

    /// Drops expired blobs; returns their files for deletion outside the lock.
    fn purge(&mut self, now: i64) -> Vec<PathBuf> {
        let mut files = Vec::new();
        self.blobs.retain(|_, e| {
            let keep = !e.expired(now);
            if !keep {
                files.push(e.file.clone());
            }
            keep
        });
        let blobs = &self.blobs;
        self.uploads.retain(|_, id| blobs.contains_key(id));
        self.downloads.retain(|_, id| blobs.contains_key(id));
        self.ledger.prune(now);
        files
    }

    /// Files `owner` holds (and bytes they hold or reserved).
    fn held_by(&self, owner: &str) -> (usize, u64) {
        self.blobs
            .values()
            .filter(|e| e.user == owner && e.info.state != BlobState::Denied)
            .fold((0, 0), |(c, b), e| (c + 1, b.saturating_add(e.reserved)))
    }

    fn held_total(&self) -> u64 {
        self.blobs.values().fold(0u64, |b, e| b.saturating_add(e.reserved))
    }

    fn check_files(limits: &BlobLimits, count: usize) -> Result<(), BlobError> {
        if count >= limits.account_files {
            return Err(BlobError::UserLimit(format!(
                "At most {} files are held at once; delete one first.",
                limits.account_files
            )));
        }
        Ok(())
    }

    /// Room for one more file reserving `bytes`.
    fn check_room(&self, limits: &BlobLimits, owner: &str, bytes: u64) -> Result<(), BlobError> {
        let (count, held) = self.held_by(owner);
        Self::check_files(limits, count)?;
        if held.saturating_add(bytes) > limits.account_bytes {
            return Err(BlobError::UserLimit(format!(
                "The files held for this account would exceed {}.",
                human_bytes(limits.account_bytes)
            )));
        }
        if self.held_total().saturating_add(bytes) > limits.total_bytes {
            return Err(BlobError::ServerFull);
        }
        Ok(())
    }

    /// Bytes `owner` may still move today (`u64::MAX` without a daily limit); an error when nothing is left.
    fn daily_room(&self, limits: &BlobLimits, owner: &str, now: i64) -> Result<u64, BlobError> {
        if limits.account_daily_bytes == 0 {
            return Ok(u64::MAX);
        }
        match limits.account_daily_bytes.saturating_sub(self.ledger.used(owner, now)) {
            0 => Err(self.daily_limit_error(limits, owner, now)),
            room => Ok(room),
        }
    }

    fn daily_limit_error(&self, limits: &BlobLimits, owner: &str, now: i64) -> BlobError {
        BlobError::UserLimit(format!(
            "This account moved its daily limit of {} through Reins files in the last 24 hours; try again in {} s.",
            human_bytes(limits.account_daily_bytes),
            self.ledger.retry_after(owner, now)
        ))
    }

    /// Charges `bytes` moved by `owner` now, if the daily limit leaves room for all of them.
    fn charge_exact(&mut self, limits: &BlobLimits, owner: &str, bytes: u64, now: i64) -> Result<(), BlobError> {
        if limits.account_daily_bytes == 0 {
            return Ok(());
        }
        if bytes > self.daily_room(limits, owner, now)? {
            return Err(self.daily_limit_error(limits, owner, now));
        }
        self.ledger.charge(owner, bytes, now);
        Ok(())
    }

    /// Charges a write of up to `bytes` in advance (settled when it ends).
    fn charge_write(&mut self, limits: &BlobLimits, owner: &str, bytes: u64, now: i64) -> Option<(i64, u64)> {
        (limits.account_daily_bytes != 0).then(|| (self.ledger.charge(owner, bytes, now), bytes))
    }

    /// Bytes one more file of `owner` may hold now: the per-file size, capped by what the limits leave.
    fn room_for(&self, limits: &BlobLimits, owner: &str, now: i64) -> Result<u64, BlobError> {
        let (count, held) = self.held_by(owner);
        Self::check_files(limits, count)?;
        let mine = limits.account_bytes.saturating_sub(held);
        if mine == 0 {
            return Err(BlobError::UserLimit("No room is left for files on this account.".to_owned()));
        }
        let server = limits.total_bytes.saturating_sub(self.held_total());
        if server == 0 {
            return Err(BlobError::ServerFull);
        }
        let daily = self.daily_room(limits, owner, now)?;
        Ok(limits.max_blob_bytes.min(mine).min(server).min(daily))
    }
}

/// Who a new blob belongs to.
#[derive(Clone, Debug)]
pub struct Owner {
    pub user: String,
    pub connection_id: ConnectionId,
    pub connection_label: String,
    pub request_id: Option<RequestId>,
}

/// A slot just opened; the secrets are shown once and never kept.
#[derive(Debug)]
pub struct NewSlot {
    pub id: BlobId,
    pub upload_secret: String,
    pub download_secret: Option<String>,
    pub expires_at: i64,
}

/// Permission to write one blob's bytes now.
#[derive(Debug)]
pub struct WriteTicket {
    pub id: BlobId,
    pub file: PathBuf,
    /// Bytes allowed.
    pub limit: u64,
    /// For an output blob: its download secret.
    pub download_secret: Option<String>,
}

/// What was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub size: u64,
    pub sha256: String,
    pub content_type: String,
    pub preview: BlobPreview,
}

/// A blob's bytes, ready to read.
#[derive(Clone, Debug)]
pub struct Readable {
    pub file: PathBuf,
    pub size: u64,
    pub name: String,
    pub content_type: String,
}

pub struct BlobHub {
    signal: Arc<ItemSignal>,
    limits: BlobLimits,
    /// `None`: `$DATA_FOLDER/reins-blobs`, resolved on first use.
    base: Option<PathBuf>,
    dir: OnceLock<Result<PathBuf, String>>,
    state: Mutex<State>,
}

fn delete_files(files: Vec<PathBuf>) {
    for file in files {
        if let Err(e) = std::fs::remove_file(&file)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            warn!("Could not delete a Reins blob file: {e}");
        }
    }
}

/// Creates (or empties) the blob directory, readable by this user only.
fn prepare_dir(dir: &Path) -> Result<PathBuf, String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot empty {}: {e}", dir.display())),
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot restrict {}: {e}", dir.display()))?;
    }
    Ok(dir.to_path_buf())
}

fn ttl_of(ttl_secs: u32) -> Result<i64, BlobError> {
    match ttl_secs {
        0 => Ok(i64::from(DEFAULT_BLOB_TTL_SECS)),
        t if t <= MAX_BLOB_TTL_SECS => Ok(i64::from(t)),
        _ => Err(BlobError::Invalid(format!("`ttl_secs` must be at most {MAX_BLOB_TTL_SECS}"))),
    }
}

fn one_line(text: &str, max: usize, what: &str) -> Result<String, BlobError> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(BlobError::Invalid(format!("`{what}` must be one line of 1..={max} characters")));
    }
    Ok(text.to_owned())
}

/// The contract's bound on a requested size; within it, the server caps sizes to its own per-file limit.
fn requested_size(max_bytes: u64) -> Result<u64, BlobError> {
    if (1..=MAX_BLOB_BYTES).contains(&max_bytes) {
        Ok(max_bytes)
    } else {
        Err(BlobError::Invalid(format!("`max_bytes` must be 1..={MAX_BLOB_BYTES}")))
    }
}

/// Checks the purpose of a slot the phone opens; outputs are made by the server, never uploaded into.
fn slot_purpose(purpose: &BlobPurpose) -> Result<BlobPurpose, BlobError> {
    match purpose {
        BlobPurpose::ToolInput {
            tool,
        } => {
            let ok = (1..=64).contains(&tool.len())
                && tool.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
            if !ok {
                return Err(BlobError::Invalid("`purpose.tool` must be a tool name".to_owned()));
            }
            Ok(purpose.clone())
        }
        BlobPurpose::Upload {
            reason,
        } => Ok(BlobPurpose::Upload {
            reason: one_line(reason, 300, "purpose.reason")?,
        }),
        BlobPurpose::Output => Err(BlobError::Invalid("an output is made by the server, not uploaded".to_owned())),
    }
}

impl BlobHub {
    pub fn with_limits(signal: Arc<ItemSignal>, limits: BlobLimits) -> Self {
        Self {
            signal,
            limits,
            base: None,
            dir: OnceLock::new(),
            state: Mutex::default(),
        }
    }

    /// A store in `dir` (tests).
    #[cfg(test)]
    pub fn with_dir(signal: Arc<ItemSignal>, dir: PathBuf, limits: BlobLimits) -> Self {
        Self {
            base: Some(dir),
            ..Self::with_limits(signal, limits)
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn dir(&self) -> Result<&Path, BlobError> {
        let dir = self.dir.get_or_init(|| {
            let base = self.base.clone().unwrap_or_else(|| Path::new(&crate::CONFIG.data_folder()).join(DIR_NAME));
            prepare_dir(&base)
        });
        match dir {
            Ok(dir) => Ok(dir),
            Err(e) => Err(BlobError::Storage(e.clone())),
        }
    }

    /// Empties the blob directory now (at startup) instead of on first use, so files of a previous run never linger.
    pub fn prepare(&self) -> Result<(), String> {
        self.dir().map(|_| ()).map_err(|e| match e {
            BlobError::Storage(e) => e,
            other => format!("{other:?}"),
        })
    }

    fn new_file(&self) -> Result<PathBuf, BlobError> {
        Ok(self.dir()?.join(HEXLOWER.encode(&get_random_bytes::<16>())))
    }

    fn new_entry(
        &self,
        owner: &Owner,
        name: String,
        purpose: BlobPurpose,
        max_bytes: u64,
        ttl: i64,
        now: i64,
    ) -> Result<Entry, BlobError> {
        Ok(Entry {
            user: owner.user.clone(),
            info: BlobInfo {
                v: PROTOCOL_VERSION,
                id: BlobId(encode_random_bytes::<16>(&BASE64URL_NOPAD)),
                connection_id: owner.connection_id.clone(),
                connection_label: owner.connection_label.clone(),
                request_id: owner.request_id.clone(),
                name,
                purpose,
                state: BlobState::Waiting,
                size: 0,
                sha256: String::new(),
                content_type: String::new(),
                preview: BlobPreview::None,
                created_at: now,
                expires_at: now.saturating_add(ttl),
            },
            max_bytes,
            reserved: max_bytes,
            upload_key: None,
            download_key: None,
            receiving: false,
            charged: None,
            downloads_left: self.limits.max_downloads,
            delivered: false,
            file: self.new_file()?,
        })
    }

    /// `POST /blobs`: a slot the AI uploads into. A size above the server's per-file limit is capped to it.
    pub fn open_slot(&self, owner: &Owner, request: &BlobSlotRequest, now: i64) -> Result<NewSlot, BlobError> {
        let name = valid_file_name(&request.name).map_err(BlobError::Invalid)?;
        let purpose = slot_purpose(&request.purpose)?;
        let max_bytes = requested_size(request.max_bytes)?.min(self.limits.max_blob_bytes);
        let ttl = ttl_of(request.ttl_secs)?;
        let mut entry = self.new_entry(owner, name, purpose, max_bytes, ttl, now)?;
        let upload_secret = new_secret();
        entry.upload_key = Some(key_of(&upload_secret));
        let download_secret = matches!(entry.info.purpose, BlobPurpose::Upload { .. }).then(new_secret);
        entry.download_key = download_secret.as_deref().map(key_of);
        let slot = NewSlot {
            id: entry.info.id.clone(),
            upload_secret,
            download_secret,
            expires_at: entry.info.expires_at,
        };
        let limits = self.limits;
        self.insert(entry, now, |state, entry| {
            state.daily_room(&limits, &entry.user, now)?;
            state.check_room(&limits, &entry.user, entry.reserved)
        })?;
        Ok(slot)
    }

    /// Purges, runs `admit` on the new entry (it may change it), and stores it if admitted.
    fn insert(
        &self,
        mut entry: Entry,
        now: i64,
        admit: impl FnOnce(&mut State, &mut Entry) -> Result<(), BlobError>,
    ) -> Result<(), BlobError> {
        let (result, expired) = {
            let mut guard = self.lock();
            let expired = guard.purge(now);
            let result = admit(&mut guard, &mut entry).map(|()| {
                let id = entry.info.id.0.clone();
                if let Some(k) = entry.upload_key {
                    guard.uploads.insert(k, id.clone());
                }
                if let Some(k) = entry.download_key {
                    guard.downloads.insert(k, id.clone());
                }
                guard.blobs.insert(id, entry);
            });
            (result, expired)
        };
        delete_files(expired);
        result
    }

    /// An output blob the server is about to write (`PUT /blobs/output`, a fetch, a proxied result): at most
    /// `max_bytes` (`None`: the per-file size), capped by what the limits leave. The ticket's `limit` says how much.
    pub fn open_output(
        &self,
        owner: &Owner,
        name: &str,
        max_bytes: Option<u64>,
        ttl_secs: u32,
        now: i64,
    ) -> Result<WriteTicket, BlobError> {
        let name = valid_file_name(name).map_err(BlobError::Invalid)?;
        let ttl = ttl_of(ttl_secs)?;
        let wanted = max_bytes.map(requested_size).transpose()?;
        let mut entry = self.new_entry(owner, name, BlobPurpose::Output, 0, ttl, now)?;
        entry.receiving = true;
        let download_secret = new_secret();
        entry.download_key = Some(key_of(&download_secret));
        let (id, file) = (entry.info.id.clone(), entry.file.clone());
        let limits = self.limits;
        let mut limit = 0;
        self.insert(entry, now, |state, entry| {
            let room = state.room_for(&limits, &entry.user, now)?;
            limit = wanted.map_or(room, |w| w.min(room));
            entry.max_bytes = limit;
            entry.reserved = limit;
            entry.charged = state.charge_write(&limits, &entry.user, limit, now);
            Ok(())
        })?;
        Ok(WriteTicket {
            id,
            file,
            limit,
            download_secret: Some(download_secret),
        })
    }

    /// The public upload: the slot behind `secret`, if it still waits. Marks it as receiving so a second upload at
    /// the same time is refused. The bytes allowed are capped by the owner's daily transfer left.
    pub fn begin_upload(&self, secret: &str, now: i64) -> Result<WriteTicket, BlobError> {
        let mut state = self.lock();
        let id = state.uploads.get(&key_of(secret)).cloned().ok_or(BlobError::NotFound)?;
        let entry = state.blobs.get(&id).filter(|e| !e.expired(now)).ok_or(BlobError::NotFound)?;
        if entry.receiving {
            return Err(BlobError::Conflict("An upload to this link is already in progress.".to_owned()));
        }
        if entry.info.state != BlobState::Waiting {
            return Err(BlobError::NotFound);
        }
        let (user, max_bytes) = (entry.user.clone(), entry.max_bytes);
        let limit = max_bytes.min(state.daily_room(&self.limits, &user, now)?);
        let charged = state.charge_write(&self.limits, &user, limit, now);
        let entry = state.blobs.get_mut(&id).ok_or(BlobError::NotFound)?;
        entry.receiving = true;
        entry.charged = charged;
        Ok(WriteTicket {
            id: entry.info.id.clone(),
            file: entry.file.clone(),
            limit,
            download_secret: None,
        })
    }

    /// A write that failed: the slot waits again (uploads) or is dropped (outputs). Nothing was moved for good, so
    /// the daily transfer charged for it is given back.
    pub fn abort_write(&self, id: &BlobId) {
        let file = {
            let mut state = self.lock();
            let Some(entry) = state.blobs.get_mut(&id.0) else {
                return;
            };
            let (charged, user) = (entry.charged.take(), entry.user.clone());
            let output = matches!(entry.info.purpose, BlobPurpose::Output);
            if !output {
                entry.receiving = false;
            }
            let partial = entry.file.clone();
            if let Some((start, bytes)) = charged {
                state.ledger.refund(&user, start, bytes);
            }
            if output {
                state.remove(&id.0).map(|e| e.file)
            } else {
                Some(partial)
            }
        };
        delete_files(file.into_iter().collect());
    }

    /// The bytes are on disk: records what they are. An upload's link is used up; an `Upload` blob now waits for the
    /// user (it goes to `Pending.blobs` and the phone is woken). The daily transfer is settled to the real size.
    pub fn finish_write(&self, id: &BlobId, received: Received, now: i64) -> Result<(String, BlobInfo), BlobError> {
        let result = {
            let mut state = self.lock();
            let State {
                blobs,
                uploads,
                ledger,
                ..
            } = &mut *state;
            match blobs.get_mut(&id.0).filter(|e| e.receiving && !e.expired(now)) {
                Some(entry) => {
                    if let Some((start, bytes)) = entry.charged.take() {
                        ledger.refund(&entry.user, start, bytes.saturating_sub(received.size));
                    }
                    entry.receiving = false;
                    entry.reserved = received.size;
                    entry.info.size = received.size;
                    entry.info.sha256 = received.sha256;
                    entry.info.content_type = received.content_type;
                    entry.info.preview = received.preview;
                    entry.info.state = if matches!(entry.info.purpose, BlobPurpose::Output) {
                        BlobState::Approved
                    } else {
                        BlobState::Uploaded
                    };
                    if let Some(k) = entry.upload_key.take() {
                        uploads.remove(&k);
                    }
                    Ok((entry.user.clone(), entry.info.clone()))
                }
                None => Err(BlobError::NotFound),
            }
        };
        match &result {
            Ok((_, info)) if matches!(info.purpose, BlobPurpose::Upload { .. }) => self.signal.notify(),
            Ok(_) => {}
            // Deleted or expired while the bytes came in.
            Err(_) => self.abort_write(id),
        }
        result
    }

    /// `GET /blobs/<id>`.
    pub fn info(&self, user: &str, id: &str, now: i64) -> Option<BlobInfo> {
        self.lock().blobs.get(id).filter(|e| e.user == user && !e.expired(now)).map(|e| e.info.clone())
    }

    /// `POST /blobs/<id>/decision`: only for an uploaded `Upload` blob. A denied file is deleted at once.
    pub fn decide(&self, user: &str, id: &str, approved: bool, now: i64) -> Result<BlobInfo, BlobError> {
        let (info, file) = {
            let mut state = self.lock();
            let entry =
                state.blobs.get_mut(id).filter(|e| e.user == user && !e.expired(now)).ok_or(BlobError::NotFound)?;
            if !matches!(entry.info.purpose, BlobPurpose::Upload { .. }) {
                return Err(BlobError::Invalid(
                    "Only uploads made with reins_upload are decided on their own.".to_owned(),
                ));
            }
            match entry.info.state {
                BlobState::Uploaded if !entry.receiving => {}
                BlobState::Waiting => return Err(BlobError::Conflict("The file has not arrived yet.".to_owned())),
                BlobState::Uploaded => return Err(BlobError::Conflict("The file is still arriving.".to_owned())),
                BlobState::Approved | BlobState::Denied => return Err(BlobError::AlreadyDecided),
            }
            entry.delivered = true;
            if approved {
                entry.info.state = BlobState::Approved;
                (entry.info.clone(), None)
            } else {
                entry.info.state = BlobState::Denied;
                entry.reserved = 0;
                (entry.info.clone(), Some(entry.file.clone()))
            }
        };
        delete_files(file.into_iter().collect());
        Ok(info)
    }

    /// The phone reads or sends a blob: its bytes must be complete (and not denied).
    pub fn readable(&self, user: &str, id: &str, now: i64) -> Result<Readable, BlobError> {
        let state = self.lock();
        let entry = state.blobs.get(id).filter(|e| e.user == user && !e.expired(now)).ok_or(BlobError::NotFound)?;
        if !entry.stored() {
            return Err(BlobError::Conflict("The file has not arrived yet.".to_owned()));
        }
        Ok(Readable {
            file: entry.file.clone(),
            size: entry.info.size,
            name: entry.info.name.clone(),
            content_type: entry.info.content_type.clone(),
        })
    }

    /// Counts `bytes` read out of the store for `user` (the phone reading content, the server sending it) against
    /// their daily transfer; refused when the limit leaves less.
    pub fn charge_transfer(&self, user: &str, bytes: u64, now: i64) -> Result<(), BlobError> {
        self.lock().charge_exact(&self.limits, user, bytes, now)
    }

    /// The public download: outputs, and uploads the user approved, until they expire, at most
    /// [`BlobLimits::max_downloads`] times. Counts this download, and its bytes against the owner's daily transfer.
    pub fn begin_download(&self, secret: &str, now: i64) -> Result<Readable, BlobError> {
        let mut state = self.lock();
        let id = state.downloads.get(&key_of(secret)).cloned().ok_or(BlobError::NotFound)?;
        let entry = state.blobs.get(&id).filter(|e| !e.expired(now)).ok_or(BlobError::NotFound)?;
        let allowed = match entry.info.purpose {
            BlobPurpose::Output => entry.stored(),
            BlobPurpose::Upload {
                ..
            } => entry.info.state == BlobState::Approved && !entry.receiving,
            BlobPurpose::ToolInput {
                ..
            } => false,
        };
        if !allowed || entry.downloads_left == 0 {
            return Err(BlobError::NotFound);
        }
        let (user, size) = (entry.user.clone(), entry.info.size);
        state.charge_exact(&self.limits, &user, size, now)?;
        let entry = state.blobs.get_mut(&id).ok_or(BlobError::NotFound)?;
        entry.downloads_left -= 1;
        Ok(Readable {
            file: entry.file.clone(),
            size: entry.info.size,
            name: entry.info.name.clone(),
            content_type: entry.info.content_type.clone(),
        })
    }

    /// `DELETE /blobs/<id>`.
    pub fn remove(&self, user: &str, id: &str) -> Result<(), BlobError> {
        let file = {
            let mut state = self.lock();
            if state.blobs.get(id).is_none_or(|e| e.user != user) {
                return Err(BlobError::NotFound);
            }
            state.remove(id).map(|e| e.file)
        };
        delete_files(file.into_iter().collect());
        Ok(())
    }

    /// Uploads that arrived and wait for the user, each handed out once (`Pending.blobs`).
    pub fn take_undelivered(&self, user: &str, now: i64) -> Vec<BlobInfo> {
        let mut state = self.lock();
        let mut out: Vec<BlobInfo> = state
            .blobs
            .values_mut()
            .filter(|e| {
                e.user == user
                    && !e.delivered
                    && !e.expired(now)
                    && e.info.state == BlobState::Uploaded
                    && matches!(e.info.purpose, BlobPurpose::Upload { .. })
            })
            .map(|e| {
                e.delivered = true;
                e.info.clone()
            })
            .collect();
        out.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        out
    }

    /// Deletes expired blobs and their files (the blob purge job, every minute by default; also done whenever a blob
    /// is added and whenever the phone polls).
    pub fn purge(&self, now: i64) {
        let stale = self.lock().purge(now);
        delete_files(stale);
    }

    /// Files on disk right now (tests).
    #[cfg(test)]
    fn file_count(&self) -> usize {
        self.dir().map_or(0, |dir| std::fs::read_dir(dir).map_or(0, Iterator::count))
    }
}

#[cfg(test)]
mod tests {
    use reins_proto::blob::BlobSlotRequest;

    use super::*;

    const NOW: i64 = 1_000;

    fn hub() -> (BlobHub, PathBuf) {
        hub_with(BlobLimits::default())
    }

    fn hub_with(limits: BlobLimits) -> (BlobHub, PathBuf) {
        let dir = std::env::temp_dir().join(format!("reins-blob-unit-{}", crate::util::get_uuid()));
        (BlobHub::with_dir(Arc::new(ItemSignal::new()), dir.clone(), limits), dir)
    }

    fn owner(user: &str) -> Owner {
        Owner {
            user: user.to_owned(),
            connection_id: "c1".into(),
            connection_label: "Claude".to_owned(),
            request_id: None,
        }
    }

    fn slot(purpose: BlobPurpose, max_bytes: u64) -> BlobSlotRequest {
        BlobSlotRequest {
            v: 1,
            connection_id: "c1".into(),
            request_id: None,
            name: "report.pdf".to_owned(),
            content_type: None,
            max_bytes,
            purpose,
            ttl_secs: 600,
        }
    }

    fn upload() -> BlobPurpose {
        BlobPurpose::Upload {
            reason: "share the report".to_owned(),
        }
    }

    fn received(size: u64) -> Received {
        Received {
            size,
            sha256: "ab".repeat(32),
            content_type: "application/pdf".to_owned(),
            preview: BlobPreview::Binary {
                description: "PDF document".to_owned(),
            },
        }
    }

    /// Opens an upload slot, "uploads" `size` bytes and returns the slot.
    fn uploaded(h: &BlobHub, user: &str, size: u64) -> NewSlot {
        let s = h.open_slot(&owner(user), &slot(upload(), 1000), NOW).unwrap();
        let t = h.begin_upload(&s.upload_secret, NOW).unwrap();
        std::fs::write(&t.file, vec![1u8; usize::try_from(size).unwrap()]).unwrap();
        h.finish_write(&t.id, received(size), NOW).unwrap();
        s
    }

    #[test]
    fn an_upload_is_single_use_waits_for_the_user_and_then_downloads() {
        let (h, dir) = hub();
        let s = h.open_slot(&owner("u1"), &slot(upload(), 1000), NOW).unwrap();
        assert!(s.download_secret.is_some());
        assert!(h.begin_download(s.download_secret.as_ref().unwrap(), NOW).is_err(), "nothing to download yet");
        let t = h.begin_upload(&s.upload_secret, NOW).unwrap();
        assert_eq!(t.limit, 1000);
        assert!(matches!(h.begin_upload(&s.upload_secret, NOW), Err(BlobError::Conflict(_))), "one upload at a time");
        std::fs::write(&t.file, b"x").unwrap();
        let (user, info) = h.finish_write(&t.id, received(1), NOW).unwrap();
        assert_eq!((user.as_str(), info.state, info.size), ("u1", BlobState::Uploaded, 1));
        assert_eq!(h.begin_upload(&s.upload_secret, NOW).unwrap_err(), BlobError::NotFound, "the link is used up");
        assert!(h.begin_download(s.download_secret.as_ref().unwrap(), NOW).is_err(), "not approved yet");
        assert_eq!(h.take_undelivered("u1", NOW).len(), 1);
        assert!(h.take_undelivered("u1", NOW).is_empty(), "delivered once");
        assert!(h.take_undelivered("u2", NOW).is_empty());
        assert_eq!(h.decide("u2", &s.id.0, true, NOW).unwrap_err(), BlobError::NotFound, "owner checked");
        assert_eq!(h.decide("u1", &s.id.0, true, NOW).unwrap().state, BlobState::Approved);
        assert_eq!(h.decide("u1", &s.id.0, false, NOW).unwrap_err(), BlobError::AlreadyDecided);
        let secret = s.download_secret.unwrap();
        for _ in 0..DEFAULT_MAX_DOWNLOADS {
            assert!(h.begin_download(&secret, NOW).is_ok());
        }
        assert!(h.begin_download(&secret, NOW).is_err(), "at most {DEFAULT_MAX_DOWNLOADS} downloads");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_denied_upload_is_deleted_and_never_downloadable() {
        let (h, dir) = hub();
        let s = uploaded(&h, "u1", 5);
        let file = h.readable("u1", &s.id.0, NOW).unwrap().file;
        assert!(file.exists());
        assert_eq!(h.decide("u1", &s.id.0, false, NOW).unwrap().state, BlobState::Denied);
        assert!(!file.exists());
        assert!(h.begin_download(s.download_secret.as_ref().unwrap(), NOW).is_err());
        assert!(h.readable("u1", &s.id.0, NOW).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tool_inputs_are_never_downloadable_and_not_decided_alone() {
        let (h, dir) = hub();
        let purpose = BlobPurpose::ToolInput {
            tool: "github_release_asset_upload".to_owned(),
        };
        let s = h.open_slot(&owner("u1"), &slot(purpose, 10), NOW).unwrap();
        assert!(s.download_secret.is_none());
        assert!(matches!(h.readable("u1", &s.id.0, NOW), Err(BlobError::Conflict(_))), "not uploaded yet");
        let t = h.begin_upload(&s.upload_secret, NOW).unwrap();
        std::fs::write(&t.file, b"abc").unwrap();
        h.finish_write(&t.id, received(3), NOW).unwrap();
        assert!(matches!(h.decide("u1", &s.id.0, true, NOW), Err(BlobError::Invalid(_))));
        assert!(h.take_undelivered("u1", NOW).is_empty(), "tool inputs are approved with their call");
        assert_eq!(h.readable("u1", &s.id.0, NOW).unwrap().size, 3);
        assert!(h.readable("u2", &s.id.0, NOW).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn slots_are_validated() {
        let (h, dir) = hub();
        let o = owner("u1");
        let mut bad_name = slot(upload(), 10);
        bad_name.name = "../x".to_owned();
        assert!(matches!(h.open_slot(&o, &bad_name, NOW), Err(BlobError::Invalid(_))));
        assert!(matches!(h.open_slot(&o, &slot(upload(), 0), NOW), Err(BlobError::Invalid(_))));
        assert!(matches!(h.open_slot(&o, &slot(upload(), MAX_BLOB_BYTES + 1), NOW), Err(BlobError::Invalid(_))));
        assert!(matches!(h.open_slot(&o, &slot(BlobPurpose::Output, 10), NOW), Err(BlobError::Invalid(_))));
        let mut long_ttl = slot(upload(), 10);
        long_ttl.ttl_secs = MAX_BLOB_TTL_SECS + 1;
        assert!(matches!(h.open_slot(&o, &long_ttl, NOW), Err(BlobError::Invalid(_))));
        let mut default_ttl = slot(upload(), 10);
        default_ttl.ttl_secs = 0;
        assert_eq!(h.open_slot(&o, &default_ttl, NOW).unwrap().expires_at, NOW + i64::from(DEFAULT_BLOB_TTL_SECS));
        let bad_tool = BlobPurpose::ToolInput {
            tool: "Bad Tool!".to_owned(),
        };
        assert!(matches!(h.open_slot(&o, &slot(bad_tool, 10), NOW), Err(BlobError::Invalid(_))));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn limits_count_blobs_and_reserved_bytes() {
        let (h, dir) = hub();
        for _ in 0..DEFAULT_ACCOUNT_FILES {
            h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).unwrap();
        }
        assert!(matches!(h.open_slot(&owner("u1"), &slot(upload(), 10), NOW), Err(BlobError::UserLimit(_))));
        h.open_slot(&owner("u2"), &slot(upload(), MAX_BLOB_BYTES), NOW).unwrap();
        h.open_slot(&owner("u2"), &slot(upload(), MAX_BLOB_BYTES), NOW).unwrap();
        assert!(
            matches!(h.open_slot(&owner("u2"), &slot(upload(), 1), NOW), Err(BlobError::UserLimit(_))),
            "2 GiB per user"
        );
        for user in ["u3", "u4"] {
            for _ in 0..2 {
                h.open_slot(&owner(user), &slot(upload(), MAX_BLOB_BYTES), NOW).unwrap();
            }
        }
        // 6 GiB and 200 bytes are reserved; fill the server to exactly 8 GiB.
        h.open_slot(&owner("u5"), &slot(upload(), MAX_BLOB_BYTES), NOW).unwrap();
        h.open_slot(&owner("u5"), &slot(upload(), MAX_BLOB_BYTES - 200), NOW).unwrap();
        assert_eq!(h.open_slot(&owner("u6"), &slot(upload(), 1), NOW).unwrap_err(), BlobError::ServerFull);
        assert!(matches!(h.open_output(&owner("u2"), "x.bin", None, 0, NOW), Err(BlobError::UserLimit(_))));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn outputs_download_until_they_expire_and_purge_deletes_their_files() {
        let (h, dir) = hub();
        let t = h.open_output(&owner("u1"), "result.bin", None, 60, NOW).unwrap();
        assert_eq!(t.limit, MAX_BLOB_BYTES);
        let secret = t.download_secret.clone().unwrap();
        assert!(h.begin_download(&secret, NOW).is_err(), "still being written");
        std::fs::write(&t.file, b"data").unwrap();
        let (_, info) = h.finish_write(&t.id, received(4), NOW).unwrap();
        assert_eq!(info.state, BlobState::Approved);
        assert!(h.take_undelivered("u1", NOW).is_empty());
        assert_eq!(h.begin_download(&secret, NOW + 59).unwrap().size, 4);
        assert!(h.begin_download(&secret, NOW + 60).is_err(), "expired");
        assert!(h.info("u1", &t.id.0, NOW + 60).is_none());
        assert!(t.file.exists());
        h.purge(NOW + 60);
        assert!(!t.file.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_upload_can_be_retried_and_a_failed_output_is_dropped() {
        let (h, dir) = hub();
        let s = h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).unwrap();
        let t = h.begin_upload(&s.upload_secret, NOW).unwrap();
        h.abort_write(&t.id);
        assert!(h.begin_upload(&s.upload_secret, NOW).is_ok());
        let o = h.open_output(&owner("u1"), "o.bin", Some(5), 60, NOW).unwrap();
        h.abort_write(&o.id);
        assert!(h.info("u1", &o.id.0, NOW).is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deleting_frees_the_file_and_checks_the_owner() {
        let (h, dir) = hub();
        let s = uploaded(&h, "u1", 3);
        let file = h.readable("u1", &s.id.0, NOW).unwrap().file;
        assert_eq!(h.remove("u2", &s.id.0), Err(BlobError::NotFound));
        h.remove("u1", &s.id.0).unwrap();
        assert!(!file.exists());
        assert_eq!(h.remove("u1", &s.id.0), Err(BlobError::NotFound));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_directory_is_wiped_on_first_use() {
        let dir = std::env::temp_dir().join(format!("reins-blob-wipe-{}", crate::util::get_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("left-over"), b"old").unwrap();
        let h = BlobHub::with_dir(Arc::new(ItemSignal::new()), dir.clone(), BlobLimits::default());
        h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).unwrap();
        assert!(!dir.join("left-over").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    fn small_limits() -> BlobLimits {
        BlobLimits {
            max_blob_bytes: 50,
            account_files: 2,
            account_bytes: 80,
            account_daily_bytes: 100,
            total_bytes: 1000,
            max_downloads: 3,
        }
    }

    /// Writes `size` bytes through `ticket` and finishes it.
    fn write(h: &BlobHub, ticket: &WriteTicket, size: u64) -> BlobInfo {
        std::fs::write(&ticket.file, vec![7u8; usize::try_from(size).unwrap()]).unwrap();
        h.finish_write(&ticket.id, received(size), NOW).unwrap().1
    }

    #[test]
    fn configured_limits_cap_sizes_and_count_files_and_bytes() {
        let (h, dir) = hub_with(small_limits());
        let me = owner("u1");
        let first = h.open_slot(&me, &slot(upload(), 1000), NOW).unwrap();
        let ticket = h.begin_upload(&first.upload_secret, NOW).unwrap();
        assert_eq!(ticket.limit, 50, "capped to the per-file size, not refused");
        assert!(
            matches!(h.open_slot(&me, &slot(upload(), MAX_BLOB_BYTES + 1), NOW), Err(BlobError::Invalid(_))),
            "the contract's bound still holds"
        );
        let err = h.open_slot(&me, &slot(upload(), 40), NOW).unwrap_err();
        assert!(matches!(&err, BlobError::UserLimit(m) if m.contains("80 bytes")), "50 + 40 > 80: {err:?}");
        h.open_slot(&me, &slot(upload(), 30), NOW).unwrap();
        let err = h.open_slot(&me, &slot(upload(), 1), NOW).unwrap_err();
        assert!(matches!(&err, BlobError::UserLimit(m) if m.contains("At most 2 files")), "{err:?}");
        let out = h.open_output(&owner("u2"), "o.bin", None, 60, NOW).unwrap();
        assert_eq!(out.limit, 50, "an output of unknown size gets the per-file size");
        let out = h.open_output(&owner("u3"), "o.bin", Some(10), 60, NOW).unwrap();
        assert_eq!(out.limit, 10);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn denied_uploads_hold_no_place() {
        let (h, dir) = hub_with(small_limits());
        let a = h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).unwrap();
        let t = h.begin_upload(&a.upload_secret, NOW).unwrap();
        write(&h, &t, 5);
        h.decide("u1", &a.id.0, false, NOW).unwrap();
        h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).unwrap();
        h.open_slot(&owner("u1"), &slot(upload(), 10), NOW).expect("the denied upload does not count");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_daily_transfer_counts_uploads_downloads_and_sends_and_frees_up_after_a_day() {
        let (h, dir) = hub_with(BlobLimits {
            account_files: 10,
            account_bytes: 1000,
            ..small_limits()
        });
        // An upload is charged its link's size while it runs, and settled to the real size.
        let s = h.open_slot(&owner("u1"), &slot(upload(), 50), NOW).unwrap();
        let t = h.begin_upload(&s.upload_secret, NOW).unwrap();
        assert_eq!(h.lock().ledger.used("u1", NOW), 50);
        write(&h, &t, 40);
        assert_eq!(h.lock().ledger.used("u1", NOW), 40);
        h.decide("u1", &s.id.0, true, NOW).unwrap();
        // Downloads count whole: 40 + 40 = 80, and a third would need 120.
        let secret = s.download_secret.unwrap();
        h.begin_download(&secret, NOW).unwrap();
        let err = h.begin_download(&secret, NOW).unwrap_err();
        assert!(
            matches!(&err, BlobError::UserLimit(m) if m.contains("daily limit") && m.contains("try again in")),
            "{err:?}"
        );
        // What is left (20) caps the next upload; a failed write gives its charge back.
        let s2 = h.open_slot(&owner("u1"), &slot(upload(), 50), NOW).unwrap();
        let t2 = h.begin_upload(&s2.upload_secret, NOW).unwrap();
        assert_eq!(t2.limit, 20);
        h.abort_write(&t2.id);
        assert_eq!(h.lock().ledger.used("u1", NOW), 80);
        assert!(h.charge_transfer("u1", 21, NOW).is_err(), "a send needs room for all its bytes");
        h.charge_transfer("u1", 20, NOW).unwrap();
        assert!(matches!(h.begin_upload(&s2.upload_secret, NOW), Err(BlobError::UserLimit(_))), "nothing left");
        assert!(matches!(h.open_slot(&owner("u1"), &slot(upload(), 5), NOW), Err(BlobError::UserLimit(_))));
        assert!(matches!(h.open_output(&owner("u1"), "o", None, 0, NOW), Err(BlobError::UserLimit(_))));
        h.charge_transfer("u2", 100, NOW).expect("other accounts have their own budget");
        // A rolling day later the budget is back.
        let later = NOW + DAY;
        h.purge(later);
        assert_eq!(h.lock().ledger.used("u1", later), 0);
        assert!(h.lock().ledger.users.is_empty(), "old buckets are dropped");
        h.charge_transfer("u1", 100, later).unwrap();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn retry_hints_name_when_the_oldest_bytes_stop_counting() {
        let mut ledger = Ledger::default();
        let start = ledger.charge("u", 10, 7_250);
        assert_eq!(start, 7_200);
        ledger.charge("u", 5, 7_200 + HOUR);
        assert_eq!(ledger.used("u", 7_300), 15);
        assert_eq!(ledger.retry_after("u", 7_300), 7_200 + DAY - 7_300);
        assert_eq!(ledger.used("u", 7_200 + DAY), 5, "the first hour's bytes stopped counting");
        ledger.refund("u", start, 100);
        assert_eq!(ledger.used("u", 7_300), 5, "refunds never go below zero");
        assert_eq!(ledger.retry_after("nobody", 0), 1);
    }

    #[test]
    fn a_zero_daily_limit_turns_the_ledger_off() {
        let (h, dir) = hub_with(BlobLimits {
            account_daily_bytes: 0,
            ..small_limits()
        });
        for _ in 0..10 {
            h.charge_transfer("u1", u64::MAX / 2, NOW).unwrap();
        }
        let s = h.open_slot(&owner("u1"), &slot(upload(), 50), NOW).unwrap();
        assert_eq!(h.begin_upload(&s.upload_secret, NOW).unwrap().limit, 50);
        assert!(h.lock().ledger.users.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn downloads_per_link_follow_the_setting() {
        let (h, dir) = hub_with(small_limits());
        let t = h.open_output(&owner("u1"), "r.bin", Some(1), 60, NOW).unwrap();
        write(&h, &t, 1);
        let secret = t.download_secret.unwrap();
        for _ in 0..3 {
            h.begin_download(&secret, NOW).unwrap();
        }
        assert_eq!(h.begin_download(&secret, NOW).unwrap_err(), BlobError::NotFound);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn expired_files_leave_the_disk_at_the_next_purge() {
        let (h, dir) = hub();
        let t = h.open_output(&owner("u1"), "a.bin", Some(3), 60, NOW).unwrap();
        write(&h, &t, 3);
        let s = h.open_slot(&owner("u1"), &slot(upload(), 3), NOW).unwrap();
        write(&h, &h.begin_upload(&s.upload_secret, NOW).unwrap(), 3);
        assert_eq!(h.file_count(), 2);
        h.purge(NOW + 599);
        assert_eq!(h.file_count(), 1, "the 60 s output is gone, the 600 s upload stays");
        h.purge(NOW + 600);
        assert_eq!(h.file_count(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sizes_read_like_people_write_them() {
        assert_eq!(human_bytes(2 << 30), "2 GiB");
        assert_eq!(human_bytes(100 << 20), "100 MiB");
        assert_eq!(human_bytes(1536), "1536 bytes");
        assert_eq!(human_bytes(2048), "2 KiB");
        assert_eq!(human_bytes(80), "80 bytes");
        assert_eq!(human_bytes(0), "0 bytes");
    }
}
