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

/// How long a request waits for the phone before "Check your phone" shows, in milliseconds.
pub fn set_notify_delay(paths: &Paths, ms: u64) -> Result<Change, String> {
    edit(paths, |doc| {
        table(doc, "notify")?["delay_ms"] = value(i64::try_from(ms).map_err(|e| e.to_string())?);
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

/// This computer's Auto setup is done (`[auto]`): `setup_done` and the zones in `facts` (all of them, as they are to
/// be). The matrix and `matrix_decides` are left as they are.
pub fn set_auto_setup(paths: &Paths, facts: &reins_policy::context::Facts) -> Result<Change, String> {
    use std::collections::BTreeMap;

    use reins_policy::context::Zone;
    fn zones(map: &BTreeMap<String, Zone>) -> Result<Table, String> {
        let mut t = Table::new();
        for (k, z) in map {
            let word = toml::Value::try_from(z).map_err(|e| e.to_string())?;
            t.insert(k, value(word.as_str().ok_or("a zone is not a word")?));
        }
        Ok(t)
    }
    let mut t = Table::new();
    t.set_implicit(true);
    for (name, map) in [
        ("owners", &facts.owners),
        ("repos", &facts.repos),
        ("domains", &facts.domains),
        ("people", &facts.people),
        ("hosts", &facts.hosts),
        ("chats", &facts.chats),
    ] {
        if !map.is_empty() {
            t.insert(name, Item::Table(zones(map)?));
        }
    }
    if !facts.own_addresses.is_empty() {
        t.insert("own_addresses", value(strings(&facts.own_addresses)));
    }
    edit(paths, |doc| {
        let auto = table(doc, "auto")?;
        auto["setup_done"] = value(true);
        auto.remove("facts");
        if !t.is_empty() {
            auto.insert("facts", Item::Table(t));
        }
        Ok(())
    })
}

/// Which list of `[workspace]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceList {
    /// Folders whose subfolders are projects.
    Roots,
    /// Folders where deleting loses nothing.
    Scratch,
}

impl WorkspaceList {
    fn key(self) -> &'static str {
        match self {
            Self::Roots => "roots",
            Self::Scratch => "scratch",
        }
    }

    /// What the list is when the file doesn't have it.
    fn default(self) -> Vec<String> {
        let d = crate::config::WorkspaceConfig::default();
        match self {
            Self::Roots => d.roots,
            Self::Scratch => d.scratch,
        }
    }
}

/// `list` as the file has it: its own entries, or the defaults when the file doesn't name the list.
fn workspace_entries(doc: &DocumentMut, list: WorkspaceList) -> Result<Vec<String>, String> {
    let Some(item) = doc.get("workspace").and_then(|w| w.get(list.key())) else {
        return Ok(list.default());
    };
    let array = item.as_array().ok_or_else(|| format!("`workspace.{}` in config.toml is not a list", list.key()))?;
    array
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`workspace.{}` in config.toml holds something that is not text", list.key()))
        })
        .collect()
}

/// Removes the value at `i` from `array`. A comment written before it (in a list of several lines, that is the
/// comment at the end of the line above) stays: before the value that follows, or at the list's end.
fn remove_keeping_comments(array: &mut Array, i: usize) {
    let removed = array.remove(i);
    let before = removed.decor().prefix().and_then(|p| p.as_str()).unwrap_or_default();
    if !before.contains('#') {
        return;
    }
    // The comment lines, without the indent of the value that is gone. What follows a comment must start on a line
    // of its own, or the comment would swallow it.
    let comments = before.rfind('\n').map_or(before, |end| &before[..end]);
    if let Some(next) = array.get_mut(i) {
        let own = next.decor().prefix().and_then(|p| p.as_str()).unwrap_or_default();
        let prefix = if own.contains('\n') {
            format!("{comments}{own}")
        } else {
            before.to_owned()
        };
        next.decor_mut().set_prefix(prefix);
    } else {
        let trailing = array.trailing().as_str().unwrap_or_default();
        let trailing = if trailing.contains('\n') {
            format!("{comments}{trailing}")
        } else {
            format!("{comments}\n")
        };
        array.set_trailing(trailing);
    }
}

/// Adds `entry` (`~/x` or an absolute path, as it is to be written) to `[workspace] roots` or `scratch`.
///
/// A list the file doesn't name is the defaults (the standard roots; no scratch folder): they are written out with
/// the new entry, so adding a folder never drops them. A list the file has keeps its entries, order and comments.
pub fn add_workspace_folder(paths: &Paths, list: WorkspaceList, entry: &str) -> Result<Change, String> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err("no folder given".to_owned());
    }
    edit(paths, |doc| {
        let mut entries = workspace_entries(doc, list)?;
        if entries.iter().any(|e| e == entry) {
            return Err(format!("{entry} is already in `workspace.{}`", list.key()));
        }
        let workspace = table(doc, "workspace")?;
        if let Some(array) = workspace.get_mut(list.key()).and_then(Item::as_array_mut) {
            array.push(entry);
        } else {
            entries.push(entry.to_owned());
            workspace[list.key()] = value(strings(&entries));
        }
        Ok(())
    })
}

/// Removes `entry` (exactly as the list has it) from `[workspace] roots` or `scratch`.
///
/// Removing a standard root from a file that doesn't name the roots writes the remaining ones out (an empty list
/// stays in the file: without it the standard roots would be back).
pub fn remove_workspace_folder(paths: &Paths, list: WorkspaceList, entry: &str) -> Result<Change, String> {
    edit(paths, |doc| {
        let mut entries = workspace_entries(doc, list)?;
        if !entries.iter().any(|e| e == entry) {
            return Err(format!("{entry} is not in `workspace.{}`", list.key()));
        }
        entries.retain(|e| e != entry);
        let workspace = table(doc, "workspace")?;
        if entries == list.default() {
            workspace.remove(list.key());
        } else if let Some(array) = workspace.get_mut(list.key()).and_then(Item::as_array_mut) {
            let at = |array: &Array| array.iter().position(|v| v.as_str() == Some(entry));
            while let Some(i) = at(array) {
                remove_keeping_comments(array, i);
            }
        } else {
            workspace[list.key()] = value(strings(&entries));
        }
        if workspace.is_empty() {
            doc.remove("workspace");
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_a_folder_keeps_the_standard_roots_and_the_file() {
        let (_dir, paths) = setup();
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        let original =
            "# my settings\nmode = \"auto\"   # the phone decides\n\n[guard]\ncommands = [\"make deploy\"]\n";
        std::fs::write(paths.config_file(), original).unwrap();
        let standard = crate::config::WorkspaceConfig::default().roots;
        // A scratch folder: the roots stay the standard ones, unnamed in the file.
        assert_eq!(add_workspace_folder(&paths, WorkspaceList::Scratch, "~/Documents/dev/tmp").unwrap(), Change::Live);
        let text = std::fs::read_to_string(paths.config_file()).unwrap();
        assert!(text.starts_with(original), "{text}");
        assert!(!text.contains("roots"), "{text}");
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.workspace.roots, standard);
        assert_eq!(c.workspace.scratch, ["~/Documents/dev/tmp"]);
        // A root: the standard ones are written out with it, in their order.
        add_workspace_folder(&paths, WorkspaceList::Roots, "/mnt/data/dev").unwrap();
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.workspace.roots[..standard.len()], standard[..]);
        assert_eq!(c.workspace.roots.last().map(String::as_str), Some("/mnt/data/dev"));
        assert_eq!(c.workspace.scratch, ["~/Documents/dev/tmp"]);
        // Once each.
        assert!(add_workspace_folder(&paths, WorkspaceList::Roots, "/mnt/data/dev").is_err());
        assert!(add_workspace_folder(&paths, WorkspaceList::Roots, "~/src").is_err(), "a standard root");
        assert!(add_workspace_folder(&paths, WorkspaceList::Scratch, "~/Documents/dev/tmp").is_err());
        assert!(add_workspace_folder(&paths, WorkspaceList::Scratch, "  ").is_err());
        assert_eq!(Config::load(&paths).unwrap(), c, "a refused change writes nothing");
    }

    #[test]
    fn a_workspace_list_in_the_file_keeps_its_comments() {
        let (_dir, paths) = setup();
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        let original = "[workspace]\n# where my projects are\nroots = [\n    \"~/work\", # the day job\n    \"~/play\",\n]\n\
                        scratch = [\"~/work/tmp\"] # throwaway\n";
        std::fs::write(paths.config_file(), original).unwrap();
        add_workspace_folder(&paths, WorkspaceList::Roots, "~/more").unwrap();
        add_workspace_folder(&paths, WorkspaceList::Scratch, "/data/scratch").unwrap();
        let text = std::fs::read_to_string(paths.config_file()).unwrap();
        for kept in ["# where my projects are", "\"~/work\", # the day job", "# throwaway"] {
            assert!(text.contains(kept), "{kept}: {text}");
        }
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.workspace.roots, ["~/work", "~/play", "~/more"], "the file's own roots, not the standard ones");
        assert_eq!(c.workspace.scratch, ["~/work/tmp", "/data/scratch"]);
        remove_workspace_folder(&paths, WorkspaceList::Roots, "~/play").unwrap();
        remove_workspace_folder(&paths, WorkspaceList::Scratch, "~/work/tmp").unwrap();
        let text = std::fs::read_to_string(paths.config_file()).unwrap();
        assert!(text.contains("# where my projects are") && text.contains("\"~/work\", # the day job"), "{text}");
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.workspace.roots, ["~/work", "~/more"]);
        assert_eq!(c.workspace.scratch, ["/data/scratch"]);
    }

    #[test]
    fn removing_a_folder() {
        let (_dir, paths) = setup();
        let standard = crate::config::WorkspaceConfig::default().roots;
        // A standard root, from a file that names no roots: the others are written out.
        remove_workspace_folder(&paths, WorkspaceList::Roots, "~/src").unwrap();
        let c = Config::load(&paths).unwrap();
        assert_eq!(c.workspace.roots.len(), standard.len() - 1);
        assert!(!c.workspace.roots.iter().any(|r| r == "~/src"));
        assert!(remove_workspace_folder(&paths, WorkspaceList::Roots, "~/src").is_err(), "gone already");
        assert!(remove_workspace_folder(&paths, WorkspaceList::Scratch, "~/Documents/dev").is_err(), "the other list");
        // Every root removed: an empty list stays, or the standard roots would be back.
        for root in &c.workspace.roots {
            remove_workspace_folder(&paths, WorkspaceList::Roots, root).unwrap();
        }
        assert!(Config::load(&paths).unwrap().workspace.roots.is_empty());
        assert!(std::fs::read_to_string(paths.config_file()).unwrap().contains("roots = []"));
        // The last scratch folder removed: the list leaves the file.
        add_workspace_folder(&paths, WorkspaceList::Scratch, "/data/scratch").unwrap();
        remove_workspace_folder(&paths, WorkspaceList::Scratch, "/data/scratch").unwrap();
        let text = std::fs::read_to_string(paths.config_file()).unwrap();
        assert!(!text.contains("scratch"), "{text}");
        // Back to the standard roots, in their order.
        for root in &standard {
            add_workspace_folder(&paths, WorkspaceList::Roots, root).unwrap();
        }
        assert_eq!(Config::load(&paths).unwrap().workspace, crate::config::WorkspaceConfig::default());
        // A list that is no list is never rewritten.
        std::fs::write(paths.config_file(), "[workspace]\nroots = \"~/dev\"\n").unwrap();
        assert!(add_workspace_folder(&paths, WorkspaceList::Roots, "~/more").is_err());
    }

    #[test]
    fn the_auto_setup_keeps_the_matrix() {
        use reins_policy::context::{FactKind, Zone};
        let (_dir, paths) = setup();
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(paths.config_file(), "[auto]\nmatrix_decides = true\n\n[auto.facts.owners]\nold = \"work\"\n")
            .unwrap();
        let mut facts = reins_policy::context::Facts::default();
        assert!(facts.set(FactKind::Owner, "dkat", Zone::Mine));
        assert!(facts.set(FactKind::Host, "build.example.com", Zone::Work));
        assert_eq!(set_auto_setup(&paths, &facts).unwrap(), Change::Live);
        let c = Config::load(&paths).unwrap();
        assert!(c.auto.setup_done && c.auto.matrix_decides);
        assert_eq!(c.auto.facts, facts, "the facts given replace the ones before");
    }

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
        assert_eq!(c.notify.delay_ms, crate::notify::DEFAULT_DELAY_MS);
        set_notify_delay(&paths, 5_000).unwrap();
        assert_eq!(Config::load(&paths).unwrap().notify.delay_ms, 5_000);
        assert!(set_notify_delay(&paths, 60_000).is_err(), "at most 15 s");
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
