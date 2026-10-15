//! The import receipt: what an import moved and every change it made, kept as `<state>/imports/<time>.json` (the
//! folder 0700, the file 0600, written through a temporary file), read by `--undo`, `--finish` and
//! `--shred`.
//!
//! A receipt is **authenticated**: an HMAC-SHA256 over its JSON with a key of this install
//! (`<state>/import-receipts.key`, 32 random bytes, 0600). One that does not verify (written or changed by anything
//! else) is refused. Even a valid one is not trusted with paths: undo acts only on files inside the home folder, through
//! no symbolic link, owned by the user; it never overwrites a file (a file back in an original's place is left alone,
//! and said so); and it brings files back only through the trash's own records ([`Machine::restore`] with the item
//! the trash gave at import), never by building a path from the receipt.
//!
//! A receipt holds no value: names, places, line numbers and references only. Undo restores whole files from the
//! trash; it never re-creates content from the receipt.

use std::path::{Component, Path, PathBuf};

use ring::hmac;
use serde::{Deserialize, Serialize};

use super::Machine;
use crate::secrets_scan::Source;

/// The receipt's format.
pub const RECEIPT_VERSION: u32 = 1;
const KEY_FILE: &str = "import-receipts.key";

/// One change to this computer, as the receipt keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// A new file (undo: it goes to the trash).
    Created {
        path: PathBuf,
    },
    /// The file went to the trash (the trash's item `trash_id`) and a new one is there now (undo: the new one to the
    /// trash, the old one back).
    Replaced {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trash_id: Option<String>,
    },
    /// The file went to the trash (undo: it comes back).
    Trashed {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trash_id: Option<String>,
    },
    /// `[run.profiles.<name>]` was added.
    Profile {
        name: String,
    },
    /// ssh was pointed at the Reins agent.
    SshSetup,
    /// git for the host goes through Reins.
    GitHost {
        host: String,
    },
    /// The GitHub CLI was signed out of the host.
    GhLogout {
        host: String,
    },
}

/// A secret that reached the vault, and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    pub item: String,
    pub finding: String,
    /// `vault:acme-web/development/OPENAI_API_KEY`, `Reins/ssh/id_ed25519 (Dana's MacBook)`, `git: github.com`.
    pub to: String,
    /// It is still on this computer too (local copies kept, a tool that reads only its own file, a key waiting for
    /// the agent): in the vault, but not moved off the disk.
    #[serde(default)]
    pub on_disk: bool,
    /// Its file left its place for the trash, where it is still plain text (so it counts as on disk) until it is
    /// shredded or the trash is emptied.
    #[serde(default)]
    pub in_trash: bool,
}

/// What the Done screen lists: one line per kind of thing that changed, and how to use it now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    pub source: Source,
    /// `2 .env files → .env.reins`.
    pub what: String,
    /// `reins run -- <your command>`.
    pub how: String,
    /// It went as planned (else `how` says what is left, or what stays on disk).
    pub ok: bool,
}

/// An SSH key still waiting for the agent's signature before it goes to the trash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingKey {
    pub path: PathBuf,
    /// The public key blob, base64.
    pub public_key: String,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub at: i64,
    pub computer: String,
    pub keep_local: bool,
    pub moved: Vec<Moved>,
    pub changes: Vec<Change>,
    pub done: Vec<Done>,
    /// What did not go, and why; what to do about it.
    pub notes: Vec<String>,
    #[serde(default)]
    pub pending_keys: Vec<PendingKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<i64>,
    /// When the originals in the trash were deleted for good (undo can no longer bring them back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged_at: Option<i64>,
}

impl Receipt {
    /// The originals this import sent to the trash (they still hold the secrets).
    #[must_use]
    pub fn trashed(&self) -> Vec<(&Path, Option<&str>)> {
        self.changes
            .iter()
            .filter_map(|c| match c {
                Change::Trashed {
                    path,
                    trash_id,
                }
                | Change::Replaced {
                    path,
                    trash_id,
                } => Some((path.as_path(), trash_id.as_deref())),
                _ => None,
            })
            .collect()
    }

    /// How many secrets are off the disk now (in the vault only).
    #[must_use]
    pub fn off_disk(&self) -> usize {
        self.moved.iter().filter(|m| !m.on_disk).count()
    }
}

/// A receipt as kept: the receipt and its HMAC (hex).
#[derive(Serialize, Deserialize)]
struct Signed {
    receipt: Receipt,
    mac: String,
}

/// Where the receipts are.
#[must_use]
pub fn receipts_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("imports")
}

/// This install's receipt key, made the first time.
fn key(state_dir: &Path, create: bool) -> Result<hmac::Key, String> {
    let file = state_dir.join(KEY_FILE);
    let bytes = match std::fs::read(&file) {
        Ok(b) if b.len() == 32 => b,
        Ok(_) => return Err(format!("{}: not a receipt key", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            let mut b = vec![0u8; 32];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b)
                .map_err(|_| "no randomness for the receipt key".to_owned())?;
            std::fs::create_dir_all(state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
            crate::config::write_private(&file, &b).map_err(|e| format!("{}: {e}", file.display()))?;
            b
        }
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    Ok(hmac::Key::new(hmac::HMAC_SHA256, &bytes))
}

fn mac(key: &hmac::Key, receipt: &Receipt) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(receipt).map_err(|e| e.to_string())?;
    Ok(hmac::sign(key, &bytes).as_ref().to_vec())
}

/// Writes `receipt` to `path` (0600, through a temporary file in the same folder), with its HMAC.
fn write(state_dir: &Path, path: &Path, receipt: &Receipt) -> Result<(), String> {
    let key = key(state_dir, true)?;
    let signed = Signed {
        receipt: receipt.clone(),
        mac: data_encoding::HEXLOWER.encode(&mac(&key, receipt)?),
    };
    let bytes = serde_json::to_vec_pretty(&signed).map_err(|e| e.to_string())?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    crate::config::write_private(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Saves `receipt` as a new file in the receipts folder; its path.
pub fn save(state_dir: &Path, receipt: &Receipt) -> Result<PathBuf, String> {
    let dir = receipts_dir(state_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut path = dir.join(format!("{}.json", receipt.at));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{}-{n}.json", receipt.at));
        n += 1;
    }
    write(state_dir, &path, receipt)?;
    Ok(path)
}

/// Writes a receipt read with [`load`] again (after undo, finish or purge).
pub fn update(state_dir: &Path, path: &Path, receipt: &Receipt) -> Result<(), String> {
    write(state_dir, path, receipt)
}

/// A receipt from the receipts folder: `last` (the newest), or one of its files; refused unless its HMAC verifies.
pub fn load(state_dir: &Path, which: &str) -> Result<(PathBuf, Receipt), String> {
    let dir = receipts_dir(state_dir);
    let path = if which == "last" {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|_| "no import to undo".to_owned())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        files.pop().ok_or("no import to undo")?
    } else {
        PathBuf::from(which)
    };
    let refused = || format!("{}: not a receipt of an import on this computer; refused", path.display());
    // Only the receipts folder's own files, not through a link.
    let in_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file())
        && match (path.parent().map(std::fs::canonicalize), std::fs::canonicalize(&dir)) {
            (Some(Ok(parent)), Ok(dir)) => parent == dir,
            _ => false,
        };
    if !in_dir {
        return Err(refused());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let signed: Signed = serde_json::from_str(&text).map_err(|_| refused())?;
    let key = key(state_dir, false).map_err(|_| refused())?;
    let given = data_encoding::HEXLOWER.decode(signed.mac.as_bytes()).map_err(|_| refused())?;
    let bytes = serde_json::to_vec(&signed.receipt).map_err(|_| refused())?;
    hmac::verify(&key, &bytes, &given).map_err(|_| refused())?;
    Ok((path, signed.receipt))
}

/// Whether undo may touch `path`: absolute, no `..`, inside the home folder, its folder reached through no symbolic
/// link and owned by the user.
pub fn check_place(home: &Path, path: &Path) -> Result<(), String> {
    let refused = |why: &str| Err(format!("{}: {why}; left alone", path.display()));
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return refused("not a plain absolute path");
    }
    let (Some(parent), Some(_)) = (path.parent(), path.file_name()) else {
        return refused("not a file");
    };
    let Ok(home_real) = std::fs::canonicalize(home) else {
        return refused("the home folder cannot be read");
    };
    let rel = match path.strip_prefix(home).or_else(|_| path.strip_prefix(&home_real)) {
        Ok(r) if !r.as_os_str().is_empty() => r,
        _ => return refused("outside the home folder"),
    };
    let Ok(parent_real) = std::fs::canonicalize(parent) else {
        return refused("its folder is not there");
    };
    if Some(parent_real.as_path()) != home_real.join(rel).parent() {
        return refused("its folder is reached through a symbolic link");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let mine = std::fs::metadata(&parent_real).is_ok_and(|m| m.uid() == rustix::process::geteuid().as_raw());
        if !mine {
            return refused("its folder is another user's");
        }
    }
    Ok(())
}

/// Whether the file at `path` is one undo may send to the trash: a plain file of the user's (not a link).
fn plain_file(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{}: not a plain file; left alone", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.uid() != rustix::process::geteuid().as_raw() {
            return Err(format!("{}: another user's file; left alone", path.display()));
        }
    }
    Ok(())
}

/// Brings back the original at `path` (the trash's item `trash_id`), never over a file that is there.
fn bring_back(home: &Path, path: &Path, trash_id: Option<&str>, machine: &dyn Machine) -> Result<(), String> {
    check_place(home, path)?;
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(format!("{}: a file is there again; it was not overwritten", path.display()));
    }
    machine.restore(path, trash_id)
}

/// Undoes what the receipt recorded, last change first: files import made go to the trash, the originals come back
/// from it, profiles go. Each step that cannot be done safely is skipped and said. The secrets stay in the vault.
pub fn undo(
    home: &Path,
    state_dir: &Path,
    path: &Path,
    receipt: &mut Receipt,
    machine: &dyn Machine,
) -> Result<Vec<String>, String> {
    if receipt.undone_at.is_some() {
        return Err("this import was undone already".to_owned());
    }
    let purged = receipt.purged_at.is_some();
    let mut lines = Vec::new();
    let mut failed = false;
    for change in receipt.changes.iter().rev() {
        let r = match change {
            Change::Created {
                path,
            } => check_place(home, path)
                .and_then(|()| plain_file(path))
                .and_then(|()| machine.trash(path))
                .map(|_| format!("{} is in the trash", path.display())),
            Change::Replaced {
                ..
            }
            | Change::Trashed {
                ..
            } if purged => Ok("the originals were deleted from the trash: nothing to bring back".to_owned()),
            Change::Replaced {
                path,
                trash_id,
            } => put_back(home, path, trash_id.as_deref(), machine)
                .map(|()| format!("{} is back as it was", path.display())),
            Change::Trashed {
                path,
                trash_id,
            } => bring_back(home, path, trash_id.as_deref(), machine).map(|()| format!("{} is back", path.display())),
            Change::Profile {
                name,
            } => machine.remove_profile(name).map(|()| format!("run profile {name} removed")),
            Change::SshSetup => machine.ssh_unsetup().map(|()| "ssh uses its own keys again".to_owned()),
            Change::GitHost {
                host,
            } => Ok(format!("git for {host} still goes through Reins (`reins pause` sends it directly)")),
            Change::GhLogout {
                host,
            } => Ok(format!("the GitHub CLI is still signed out of {host}: gh auth login -h {host}")),
        };
        match r {
            Ok(line) => lines.push(line),
            Err(e) => {
                failed = true;
                lines.push(format!("not undone: {e}"));
            }
        }
    }
    if !failed {
        receipt.undone_at = Some(crate::now_unix());
        update(state_dir, path, receipt)?;
    }
    lines.push("The secrets stay in your vault; delete them on your phone if you do not want them there.".to_owned());
    Ok(lines)
}

/// Brings back the file import replaced at `path`: the new one steps aside, the original comes back from the trash,
/// then the new one goes to the trash.
fn put_back(home: &Path, path: &Path, trash_id: Option<&str>, machine: &dyn Machine) -> Result<(), String> {
    check_place(home, path)?;
    plain_file(path)?;
    let mut aside = path.as_os_str().to_owned();
    aside.push(".reins-undo");
    let aside = PathBuf::from(aside);
    if std::fs::symlink_metadata(&aside).is_ok() {
        return Err(format!("{}: in the way; left alone", aside.display()));
    }
    std::fs::rename(path, &aside).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Err(e) = machine.restore(path, trash_id) {
        std::fs::rename(&aside, path).ok();
        return Err(e);
    }
    machine.trash(&aside).map(|_| ())
}

/// Moves to the trash the SSH keys the import kept until the Reins agent signs with them; the receipt is updated.
pub async fn finish(
    home: &Path,
    state_dir: &Path,
    path: &Path,
    receipt: &mut Receipt,
    machine: &dyn Machine,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut left = Vec::new();
    for key in std::mem::take(&mut receipt.pending_keys) {
        let blob = data_encoding::BASE64.decode(key.public_key.as_bytes()).map_err(|e| e.to_string())?;
        let ready = check_place(home, &key.path).and_then(|()| plain_file(&key.path));
        let signed = match ready {
            Ok(()) => machine.agent_signs_with(&blob).await,
            Err(e) => Err(e),
        };
        match signed.and_then(|()| machine.trash(&key.path)) {
            Ok(trash_id) => {
                lines.push(format!("{}: the Reins agent signs with it; the file is in the trash.", key.path.display()));
                receipt.changes.push(Change::Trashed {
                    path: key.path.clone(),
                    trash_id,
                });
                let finding = crate::secrets_scan::finding_id(Source::SshKey, &key.path);
                for m in receipt.moved.iter_mut().filter(|m| m.finding == finding) {
                    // Still plain text in the trash until shredded.
                    m.in_trash = true;
                }
            }
            Err(why) => {
                lines.push(format!("{}: still kept ({why})", key.path.display()));
                left.push(key);
            }
        }
    }
    receipt.pending_keys = left;
    update(state_dir, path, receipt)?;
    if lines.is_empty() {
        lines.push("No SSH key is waiting.".to_owned());
    }
    Ok(lines)
}

/// Shreds the originals the import sent to the trash (they hold the secrets in plain text): exactly those items,
/// nothing else, each overwritten and then deleted ([`Machine::purge`]). Only secrets the vault confirmed storing
/// were ever trashed. Once all are shredded, the secrets that were in them count as off the disk; undo can no longer
/// bring them back.
pub fn purge(
    state_dir: &Path,
    path: &Path,
    receipt: &mut Receipt,
    machine: &dyn Machine,
) -> Result<Vec<String>, String> {
    if receipt.purged_at.is_some() {
        return Err("the originals of this import were deleted already".to_owned());
    }
    if receipt.undone_at.is_some() {
        return Err("this import was undone: its originals are back in their places".to_owned());
    }
    let mut lines = Vec::new();
    let mut failed = false;
    for (file, trash_id) in receipt.trashed() {
        match machine.purge(file, trash_id) {
            Ok(()) => lines.push(format!("{}: shredded", file.display())),
            Err(e) => {
                failed = true;
                lines.push(format!("{}: {e}", file.display()));
            }
        }
    }
    if !failed {
        receipt.purged_at = Some(crate::now_unix());
        for m in receipt.moved.iter_mut().filter(|m| m.in_trash) {
            m.in_trash = false;
            m.on_disk = false;
        }
        update(state_dir, path, receipt)?;
    }
    if lines.is_empty() {
        lines.push("This import sent nothing to the trash.".to_owned());
    }
    Ok(lines)
}
