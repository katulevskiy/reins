//! The real [`VaultWriter`] and [`Machine`]: the phone through Reins Secrets' store (`vault_secrets_store`: every
//! environment, SSH key and git token of an import in one sealed box, one approval, one set of check digits), the OS
//! trash, the Reins SSH agent's socket, git's config, `gh` and `config.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use toml_edit::{InlineTable, Item, Table, value};
use zeroize::Zeroizing;

use super::{Batch, Machine, Stored, VaultWriter};
use crate::config::{Config, Paths};
use crate::secrets_client::PhoneVaultWriter;

/// Stores through the paired phone.
pub struct PhoneWriter {
    pub paths: Paths,
    pub config: Config,
    /// The twelve digits of the phone's key, typed by the person where no terminal can ask (the first time only).
    pub phone_key_digits: Option<String>,
}

impl PhoneWriter {
    fn phone(&self) -> PhoneVaultWriter {
        PhoneVaultWriter::new(&self.paths, &self.config, self.phone_key_digits.clone())
    }

    /// The phone's key has never been checked here: a window asks for its digits first.
    #[must_use]
    pub fn phone_key_unchecked(&self) -> bool {
        self.phone().phone_key_unchecked()
    }

    /// Checks the phone's key against the digits given (one approval on the phone) and keeps it.
    ///
    /// # Errors
    /// Why not, as the person can read it.
    pub async fn check_phone_key(&self) -> Result<(), String> {
        self.phone().check_phone_key().await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl VaultWriter for PhoneWriter {
    async fn store(&self, batch: &Batch) -> Stored {
        self.phone().store_import(batch).await
    }

    async fn projects(&self) -> Result<Vec<super::VaultProject>, String> {
        self.phone().import_projects().await
    }
}

/// This computer.
pub struct System {
    pub paths: Paths,
    pub config: Config,
    pub home: PathBuf,
}

impl System {
    fn socket(&self) -> PathBuf {
        crate::ssh_agent::socket_path(&self.paths, &self.config.ssh)
    }
}

/// Where the trash keeps records (freedesktop on Linux, the Recycle Bin on Windows): items are found and put back
/// through them, never by a path made up from a receipt.
#[cfg(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android"))))]
mod bin {
    use std::ffi::OsString;
    use std::path::Path;

    fn id_text(id: &OsString) -> String {
        id.to_string_lossy().into_owned()
    }

    /// The item the trash made for `path` just now (the latest from there).
    pub fn trashed_id(path: &Path) -> Option<String> {
        let items = trash::os_limited::list().ok()?;
        items.into_iter().filter(|i| i.original_path() == path).max_by_key(|i| i.time_deleted).map(|i| id_text(&i.id))
    }

    /// The trash's item `id`, from `path`.
    fn item(path: &Path, id: Option<&str>) -> Result<trash::TrashItem, String> {
        let id = id.ok_or_else(|| format!("{}: the trash did not say which item it is", path.display()))?;
        let items = trash::os_limited::list().map_err(|e| format!("cannot read the trash: {e}"))?;
        items
            .into_iter()
            .find(|i| id_text(&i.id) == id && i.original_path() == path)
            .ok_or_else(|| format!("{} is not in the trash any more", path.display()))
    }

    pub fn restore(path: &Path, id: Option<&str>) -> Result<(), String> {
        trash::os_limited::restore_all([item(path, id)?]).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Where the trash keeps the item's bytes: freedesktop names it after the item's info file
    /// (`<trash>/info/<name>.trashinfo` → `<trash>/files/<name>`); on Windows the id is that path.
    fn bytes_of(item: &trash::TrashItem) -> Option<std::path::PathBuf> {
        let id = Path::new(&item.id);
        if cfg!(windows) {
            return Some(id.to_path_buf());
        }
        let trash = id.parent()?.parent()?;
        Some(trash.join("files").join(id.file_stem()?))
    }

    /// Shreds the item: its bytes overwritten with zeros, then it is deleted from the trash.
    pub fn purge(path: &Path, id: Option<&str>) -> Result<(), String> {
        let item = item(path, id)?;
        if let Some(bytes) = bytes_of(&item) {
            super::overwrite(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        trash::os_limited::purge_all([item]).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Writes zeros over a plain file's bytes and flushes them to the disk (never through a link: anything else is left
/// to the delete as it is). On SSDs and copy-on-write file systems old blocks may survive: this is best effort.
#[cfg_attr(
    not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))),
    allow(dead_code)
)]
fn overwrite(file: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    let meta = std::fs::symlink_metadata(file)?;
    if !meta.is_file() {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new().write(true).open(file)?;
    #[cfg(unix)]
    {
        // The file opened is the one looked at (not a link put in its place since).
        use std::os::unix::fs::MetadataExt as _;
        let opened = f.metadata()?;
        if opened.dev() != meta.dev() || opened.ino() != meta.ino() {
            return Err(std::io::Error::other("the file changed while it was being shredded"));
        }
    }
    let zeros = [0u8; 8192];
    let mut left = meta.len();
    while left > 0 {
        let n = usize::try_from(left).map_or(zeros.len(), |l| l.min(zeros.len()));
        f.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    f.sync_all()
}

/// macOS's Trash keeps no record a program can read back: Finder's Put Back does it, and emptying the Trash.
#[cfg(not(any(windows, all(unix, not(target_os = "macos"), not(target_os = "ios"), not(target_os = "android")))))]
mod bin {
    use std::path::Path;

    pub fn trashed_id(_path: &Path) -> Option<String> {
        None
    }

    pub fn restore(path: &Path, _id: Option<&str>) -> Result<(), String> {
        Err(format!("put {} back from the Trash (Finder: Put Back)", path.display()))
    }

    pub fn purge(path: &Path, _id: Option<&str>) -> Result<(), String> {
        Err(format!("{} is in the Trash: empty the Trash in Finder", path.display()))
    }
}

#[async_trait::async_trait]
impl Machine for System {
    fn trash(&self, path: &Path) -> Result<Option<String>, String> {
        trash::delete(path).map_err(|e| format!("{} could not go to the trash: {e}", path.display()))?;
        Ok(bin::trashed_id(path))
    }

    fn restore(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(format!("{}: a file is there again; it was not overwritten", path.display()));
        }
        bin::restore(path, trash_id)
    }

    fn purge(&self, path: &Path, trash_id: Option<&str>) -> Result<(), String> {
        bin::purge(path, trash_id)
    }

    fn gh_token(&self, host: &str) -> Option<Zeroizing<String>> {
        let mut cmd = std::process::Command::new("gh");
        cmd.args(["auth", "token", "--hostname", host]);
        crate::secrets_scan::run_for(cmd, Duration::from_secs(10)).map(|t| Zeroizing::new(t.trim().to_owned()))
    }

    async fn agent_signs_with(&self, public_key: &[u8]) -> Result<(), String> {
        let timeout = Duration::from_secs(self.config.approval_timeout_secs + 15);
        tokio::time::timeout(timeout, agent_check(&self.socket(), public_key))
            .await
            .map_err(|_| "no signature in time".to_owned())?
    }

    fn ssh_setup(&self) -> Result<bool, String> {
        use crate::ssh_agent::setup::{Change, config_file, setup};
        Ok(setup(&config_file(&self.home), &self.socket())? != Change::Unchanged)
    }

    fn ssh_unsetup(&self) -> Result<(), String> {
        crate::ssh_agent::setup::unsetup(&crate::ssh_agent::setup::config_file(&self.home)).map(|_| ())
    }

    fn git_through_reins(&self, host: &str) -> Result<(), String> {
        crate::settings::set_host_enabled(&self.paths, host, true)?;
        let config = Config::load(&self.paths).map_err(|e| e.to_string())?;
        crate::setup::Git::default().setup_hosts(&crate::setup::Scope::Global, &config).map(|_| ())
    }

    fn gh_logout(&self, host: &str) -> Result<(), String> {
        let mut cmd = std::process::Command::new("gh");
        cmd.args(["auth", "logout", "--hostname", host]);
        crate::secrets_scan::run_for(cmd, Duration::from_secs(20))
            .map(|_| ())
            .ok_or_else(|| format!("gh auth logout --hostname {host} did not work"))
    }

    fn add_profile(&self, name: &str, env: &BTreeMap<String, String>, purpose: &str) -> Result<(), String> {
        crate::settings::edit(&self.paths, |doc| {
            let run = doc.entry("run").or_insert_with(|| Item::Table(Table::new()));
            let run = run.as_table_mut().ok_or("`run` in config.toml is not a table")?;
            let profiles = run.entry("profiles").or_insert_with(|| Item::Table(Table::new()));
            let profiles = profiles.as_table_mut().ok_or("`run.profiles` in config.toml is not a table")?;
            let mut vars = InlineTable::new();
            for (k, v) in env {
                vars.insert(k, v.as_str().into());
            }
            let mut t = Table::new();
            t["env"] = value(vars);
            t["purpose"] = value(purpose);
            profiles.insert(name, Item::Table(t));
            Ok(())
        })
        .map(|_| ())
    }

    fn remove_profile(&self, name: &str) -> Result<(), String> {
        crate::settings::edit(&self.paths, |doc| {
            if let Some(profiles) = doc
                .get_mut("run")
                .and_then(Item::as_table_mut)
                .and_then(|r| r.get_mut("profiles"))
                .and_then(Item::as_table_mut)
            {
                profiles.remove(name);
            }
            Ok(())
        })
        .map(|_| ())
    }
}

/// Asks the Reins SSH agent for its keys and a signature with `public_key` over random data, and checks it.
#[cfg(unix)]
async fn agent_check(socket: &Path, public_key: &[u8]) -> Result<(), String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use crate::ssh_agent::keys::{self, Verified};
    use crate::ssh_agent::wire::{Reader, Writer};

    async fn exchange(stream: &mut tokio::net::UnixStream, msg: &[u8]) -> Result<Vec<u8>, String> {
        let mut framed = Writer::new();
        framed.string(msg);
        stream.write_all(&framed.buf).await.map_err(|e| e.to_string())?;
        let len = stream.read_u32().await.map_err(|e| e.to_string())?;
        if len > 256 * 1024 {
            return Err("the agent's answer is too large".to_owned());
        }
        let mut buf = vec![0; len as usize];
        stream.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
        Ok(buf)
    }

    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| format!("the Reins SSH agent does not run ({e}): start the service (reins resume)"))?;
    let answer = exchange(&mut stream, &[11]).await?;
    let mut r = Reader::new(&answer);
    if r.byte() != Some(12) {
        return Err("the agent did not list its keys".to_owned());
    }
    let n = r.u32().unwrap_or(0);
    let mut listed = false;
    for _ in 0..n {
        let (Some(blob), Some(_)) = (r.string(), r.string()) else {
            break;
        };
        listed |= blob == public_key;
    }
    if !listed {
        return Err("your phone does not list the key yet (its list is read every few minutes)".to_owned());
    }
    let data = crate::server::random_token(32);
    let kind = keys::key_type(public_key).unwrap_or_default();
    // RSA: SHA-256 (flag 2); the others have one algorithm.
    let flags = if kind == "ssh-rsa" {
        2
    } else {
        0
    };
    let mut req = Writer::new();
    req.byte(13).string(public_key).string(data.as_bytes()).u32(flags);
    let answer = exchange(&mut stream, &req.buf).await?;
    let mut r = Reader::new(&answer);
    if r.byte() != Some(14) {
        return Err("the phone did not sign".to_owned());
    }
    let signature = r.string().ok_or("the agent's answer is malformed")?;
    match keys::verify(public_key, data.as_bytes(), signature) {
        Verified::Good => Ok(()),
        _ => Err("the signature does not verify".to_owned()),
    }
}

#[cfg(not(unix))]
#[allow(clippy::unused_async, reason = "the same call as on unix, where it waits for the agent")]
async fn agent_check(_socket: &Path, _public_key: &[u8]) -> Result<(), String> {
    Err("checking the agent's signature works on macOS and Linux for now".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shredding_writes_zeros_over_the_whole_file_and_never_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("item");
        let text = "API_SECRET=".to_owned() + &"x".repeat(20_000);
        std::fs::write(&file, &text).unwrap();
        overwrite(&file).unwrap();
        let after = std::fs::read(&file).unwrap();
        assert_eq!(after.len(), text.len());
        assert!(after.iter().all(|b| *b == 0));
        #[cfg(unix)]
        {
            let target = dir.path().join("elsewhere");
            std::fs::write(&target, "keep").unwrap();
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            overwrite(&link).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        }
    }
}
