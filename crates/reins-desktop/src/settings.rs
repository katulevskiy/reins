//! Changing `config.toml` from the desktop app: one setting at a time, the rest of the file (comments, order, the
//! user's own entries) kept as it was. Every change is checked like a loaded config before it is written, so the app
//! cannot leave a file `reins` refuses.
//!
//! The hooks (`reins hook`) and the notifications read the file each time; the daemon reads it when it starts, so a
//! change to the git hosts, the SSH agent or the approval timeout takes effect after a restart of the background
//! service ([`needs_restart`]).

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::config::{Config, Paths};
use crate::guard::{GROUPS, OnNoAnswer};

/// Which of the guard's own lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardList {
    /// Commands asked about besides the built-in ones.
    Commands,
    /// Files asked about besides the built-in ones.
    Files,
    /// Commands never asked about.
    AllowCommands,
    /// Files never asked about.
    AllowFiles,
}

impl GuardList {
    fn key(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Files => "files",
            Self::AllowCommands => "allow_commands",
            Self::AllowFiles => "allow_files",
        }
    }
}

/// A change the daemon only sees after it restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Live,
    AfterRestart,
}

/// Whether going from `before` to `after` needs the background service restarted.
#[must_use]
pub fn needs_restart(before: &Config, after: &Config) -> bool {
    before.git != after.git
        || before.github != after.github
        || before.ssh != after.ssh
        || before.approval_timeout_secs != after.approval_timeout_secs
        || before.api != after.api
        || before.mode != after.mode
}

/// Reads `config.toml` (empty when there is none), applies `change`, checks the result and writes it. Says whether
/// the daemon must restart for it.
pub fn edit(paths: &Paths, change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<Change, String> {
    let file = paths.config_file();
    let before_text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let before: Config = toml::from_str(&before_text).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut doc: DocumentMut = before_text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    change(&mut doc)?;
    let text = doc.to_string();
    let after: Config = toml::from_str(&text).map_err(|e| format!("the new settings do not read back: {e}"))?;
    after.validate()?;
    std::fs::create_dir_all(&paths.config_dir).map_err(|e| format!("{}: {e}", paths.config_dir.display()))?;
    let tmp = file.with_extension("toml.reins-new");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(if needs_restart(&before, &after) {
        Change::AfterRestart
    } else {
        Change::Live
    })
}

/// The table `name` (made when missing).
fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> Result<&'a mut Table, String> {
    let item = doc.entry(name).or_insert_with(|| Item::Table(Table::new()));
    item.as_table_mut().ok_or_else(|| format!("`{name}` in config.toml is not a table"))
}

fn strings(items: impl IntoIterator<Item = impl AsRef<str>>) -> Array {
    items.into_iter().map(|s| s.as_ref().to_owned()).collect()
}

/// Turns the built-in hook rules of group `id` ([`GROUPS`]) on or off.
pub fn set_guard_group(paths: &Paths, id: &str, on: bool) -> Result<Change, String> {
    if !GROUPS.iter().any(|g| g.id == id) {
        return Err(format!("`{id}` is not a group of built-in rules"));
    }
    edit(paths, |doc| {
        let guard = table(doc, "guard")?;
        let mut off: Vec<String> = guard
            .get("disabled_groups")
            .and_then(Item::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        off.retain(|g| g != id);
        if !on {
            off.push(id.to_owned());
        }
        if off.is_empty() {
            guard.remove("disabled_groups");
        } else {
            guard["disabled_groups"] = value(strings(&off));
        }
        Ok(())
    })
}

/// Replaces one of the guard's own lists (empty lines dropped).
pub fn set_guard_list(paths: &Paths, list: GuardList, items: &[String]) -> Result<Change, String> {
    let items: Vec<&str> = items.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    edit(paths, |doc| {
        let guard = table(doc, "guard")?;
        if items.is_empty() {
            guard.remove(list.key());
        } else {
            guard[list.key()] = value(strings(&items));
        }
        Ok(())
    })
}

/// What a hook does when nobody answers.
pub fn set_guard_on_no_answer(paths: &Paths, on_no_answer: OnNoAnswer) -> Result<Change, String> {
    edit(paths, |doc| {
        table(doc, "guard")?["on_no_answer"] = value(match on_no_answer {
            OnNoAnswer::Deny => "deny",
            OnNoAnswer::Ask => "ask",
        });
        Ok(())
    })
}

/// How long a hook waits for the phone.
pub fn set_guard_timeout(paths: &Paths, secs: u64) -> Result<Change, String> {
    edit(paths, |doc| {
        table(doc, "guard")?["timeout_secs"] = value(i64::try_from(secs).map_err(|e| e.to_string())?);
        Ok(())
    })
}

/// How long git, SSH, the API proxy and `reins run` wait for the phone.
pub fn set_approval_timeout(paths: &Paths, secs: u64) -> Result<Change, String> {
    edit(paths, |doc| {
        doc["approval_timeout_secs"] = value(i64::try_from(secs).map_err(|e| e.to_string())?);
        Ok(())
    })
}

/// "Check your phone" on this computer, and its chime.
pub fn set_notify(paths: &Paths, phone: bool, sound: bool) -> Result<Change, String> {
    edit(paths, |doc| {
        let notify = table(doc, "notify")?;
        notify["phone"] = value(phone);
        notify["sound"] = value(sound);
        Ok(())
    })
}

/// Whether the daemon runs the SSH agent.
pub fn set_ssh_enabled(paths: &Paths, on: bool) -> Result<Change, String> {
    edit(paths, |doc| {
        table(doc, "ssh")?["enabled"] = value(on);
        Ok(())
    })
}

/// Whether git for `host` goes through Reins (its `[[git.hosts]]` entry, added when missing).
pub fn set_host_enabled(paths: &Paths, host: &str, on: bool) -> Result<Change, String> {
    let host = host.trim().to_ascii_lowercase();
    edit(paths, |doc| {
        let git = table(doc, "git")?;
        let hosts = git.entry("hosts").or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
        let hosts = hosts.as_array_of_tables_mut().ok_or("`git.hosts` in config.toml is not a list of tables")?;
        let existing = hosts
            .iter_mut()
            .find(|t| t.get("host").and_then(Item::as_str).is_some_and(|h| h.eq_ignore_ascii_case(&host)));
        if let Some(t) = existing {
            t["enabled"] = value(on);
        } else {
            let mut t = Table::new();
            t["host"] = value(host.as_str());
            t["enabled"] = value(on);
            hosts.push(t);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        (dir, paths)
    }

    #[test]
    fn changes_keep_the_rest_of_the_file() {
        let (_dir, paths) = setup();
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        let original =
            "# my settings\nmode = \"auto\"   # the phone decides\n\n[guard]\ncommands = [\"make deploy\"]\n";
        std::fs::write(paths.config_file(), original).unwrap();
        assert_eq!(set_guard_group(&paths, "publishing", false).unwrap(), Change::Live);
        set_notify(&paths, true, false).unwrap();
        let text = std::fs::read_to_string(paths.config_file()).unwrap();
        assert!(text.starts_with("# my settings\nmode = \"auto\"   # the phone decides\n"), "{text}");
        assert!(text.contains("commands = [\"make deploy\"]"));
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.guard.disabled_groups, vec!["publishing"]);
        assert!(c.notify.phone && !c.notify.sound);
        set_guard_group(&paths, "publishing", true).unwrap();
        assert!(Config::load(&paths).unwrap().guard.disabled_groups.is_empty());
        assert!(set_guard_group(&paths, "nothing", false).is_err());
    }

    #[test]
    fn hosts_are_switched_by_their_entry_and_need_a_restart() {
        let (_dir, paths) = setup();
        assert_eq!(set_host_enabled(&paths, "gitlab.com", true).unwrap(), Change::AfterRestart);
        assert_eq!(set_host_enabled(&paths, "GitHub.com", false).unwrap(), Change::AfterRestart);
        set_host_enabled(&paths, "gitlab.com", false).unwrap();
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.git.hosts.len(), 2);
        assert!(c.enabled_hosts().unwrap().is_empty());
        assert_eq!(set_ssh_enabled(&paths, false).unwrap(), Change::AfterRestart);
        assert!(!Config::load(&paths).unwrap().ssh.enabled);
    }

    #[test]
    fn lists_and_numbers_are_checked_before_they_are_written() {
        let (_dir, paths) = setup();
        set_guard_list(&paths, GuardList::AllowCommands, &["git push --force* origin feature/*".into(), " ".into()])
            .unwrap();
        assert_eq!(Config::load(&paths).unwrap().guard.allow_commands, vec!["git push --force* origin feature/*"]);
        set_guard_list(&paths, GuardList::AllowCommands, &[]).unwrap();
        assert!(Config::load(&paths).unwrap().guard.allow_commands.is_empty());
        assert!(set_guard_timeout(&paths, 2).is_err(), "out of range");
        set_guard_timeout(&paths, 300).unwrap();
        set_guard_on_no_answer(&paths, OnNoAnswer::Ask).unwrap();
        set_approval_timeout(&paths, 600).unwrap();
        let c = Config::load(&paths).unwrap();
        assert_eq!((c.guard.timeout_secs, c.guard.on_no_answer, c.approval_timeout_secs), (300, OnNoAnswer::Ask, 600));
    }
}
