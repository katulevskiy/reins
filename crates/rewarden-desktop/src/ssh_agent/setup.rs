//! `rewarden ssh setup|unsetup|status`: a marked `Host *` block with `IdentityAgent` at the end of `~/.ssh/config`,
//! so ssh asks this app's agent for keys (more specific settings earlier in the file still win). Setting up twice
//! changes nothing, a moved socket updates the block, and `unsetup` removes exactly the block and nothing else.

use std::path::{Path, PathBuf};

use crate::config::{Config, Paths};

pub const BEGIN: &str = "# >>> rewarden ssh agent: keys on your phone (`rewarden ssh unsetup` removes this block) >>>";
pub const END: &str = "# <<< rewarden ssh agent <<<";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Updated,
    Unchanged,
}

#[derive(Clone, Debug, clap::Subcommand)]
pub enum SshCmd {
    /// Make ssh use Rewarden's agent (an `IdentityAgent` block in ~/.ssh/config).
    Setup,
    /// Remove exactly the block `setup` added.
    Unsetup,
    /// Where the agent listens, whether it runs, and whether ssh uses it.
    Status,
}

/// `~/.ssh/config` under `home`.
#[must_use]
pub fn config_file(home: &Path) -> PathBuf {
    home.join(".ssh").join("config")
}

/// How `socket` is written in `~/.ssh/config`: as it is; on Windows with forward slashes (`//./pipe/…`), which Windows
/// opens like `\\.\pipe\…` and which reads the same in OpenSSH versions that take a backslash as an escape and in those
/// that do not.
#[must_use]
pub fn config_form(socket: &str) -> String {
    if cfg!(windows) {
        crate::win::forward_slashes(socket)
    } else {
        socket.to_owned()
    }
}

/// Whether ssh's `IdentityAgent` value `effective` is `socket` (on Windows ignoring case and the slashes' direction).
fn same_agent(effective: &str, socket: &Path) -> bool {
    let socket = socket.display().to_string();
    if cfg!(windows) {
        config_form(effective).eq_ignore_ascii_case(&config_form(&socket))
    } else {
        effective == socket
    }
}

/// The block for `socket` (quoted; `%` escaped, since ssh expands `%` tokens in `IdentityAgent`).
pub fn block(socket: &Path) -> Result<String, String> {
    let s = socket.to_str().ok_or("the socket path is not UTF-8")?;
    if s.contains('"') || s.chars().any(char::is_control) {
        return Err(format!("the socket path {s} cannot be written in ~/.ssh/config"));
    }
    Ok(format!("{BEGIN}\nHost *\n    IdentityAgent \"{}\"\n{END}\n", config_form(s).replace('%', "%%")))
}

/// Byte range of this app's block (with the blank line before it, when there is one).
fn find(text: &str) -> Option<std::ops::Range<usize>> {
    let mut offset = 0;
    let mut start = None;
    let mut blank_before = None;
    let mut previous_blank: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if start.is_none() && trimmed == BEGIN {
            start = Some(offset);
            blank_before = previous_blank;
        } else if let Some(s) = start
            && trimmed == END
        {
            return Some(blank_before.unwrap_or(s)..offset + line.len());
        }
        previous_blank = trimmed.trim().is_empty().then_some(offset);
        offset += line.len();
    }
    None
}

fn read(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Replaces `file` (through a symlink, keeping its permissions; a new file is 0600 in a 0700 `.ssh`).
fn write(file: &Path, text: &str) -> Result<(), String> {
    let target = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
    let dir = target.parent().ok_or("~/.ssh/config has no directory")?;
    if !dir.exists() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let permissions = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    let permissions =
        permissions.or_else(|| Some(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600)));
    if let Some(p) = permissions {
        tmp.as_file().set_permissions(p).map_err(|e| e.to_string())?;
    }
    std::io::Write::write_all(&mut tmp, text.as_bytes()).map_err(|e| e.to_string())?;
    tmp.persist(&target).map_err(|e| format!("{}: {}", target.display(), e.error))?;
    Ok(())
}

/// Adds (or updates) the block for `socket` at the end of `file`.
pub fn setup(file: &Path, socket: &Path) -> Result<Change, String> {
    let wanted = block(socket)?;
    let text = read(file)?.unwrap_or_default();
    if let Some(range) = find(&text) {
        if text[range.clone()].trim_start_matches(['\n', '\r']) == wanted {
            return Ok(Change::Unchanged);
        }
        let mut updated = text.clone();
        let separator = if range.start > 0 && text[range.clone()].starts_with(['\n', '\r']) {
            "\n"
        } else {
            ""
        };
        updated.replace_range(range, &format!("{separator}{wanted}"));
        write(file, &updated)?;
        return Ok(Change::Updated);
    }
    let mut updated = text;
    if !updated.is_empty() {
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
    }
    updated.push_str(&wanted);
    write(file, &updated)?;
    Ok(Change::Added)
}

/// Removes the block; `false` when there was none.
pub fn unsetup(file: &Path) -> Result<bool, String> {
    let Some(text) = read(file)? else {
        return Ok(false);
    };
    let Some(range) = find(&text) else {
        return Ok(false);
    };
    let mut updated = text;
    updated.replace_range(range, "");
    write(file, &updated)?;
    Ok(true)
}

/// The socket the block points at, if the block is there.
pub fn current(file: &Path) -> Result<Option<String>, String> {
    let Some(text) = read(file)? else {
        return Ok(None);
    };
    Ok(find(&text).and_then(|r| {
        text[r].lines().find_map(|l| {
            let v = l.trim().strip_prefix("IdentityAgent")?.trim();
            Some(v.trim_matches('"').replace("%%", "%"))
        })
    }))
}

/// What ssh itself would use as `IdentityAgent` with `file` (`ssh -G`), when ssh is installed.
#[must_use]
pub fn effective_agent(file: &Path) -> Option<String> {
    let out = std::process::Command::new("ssh")
        .arg("-G")
        .arg("-F")
        .arg(file)
        .arg("rewarden-check.invalid")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("identityagent ").map(|v| v.trim().trim_matches('"').to_owned()))
}

#[cfg(unix)]
fn running(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// A named pipe answers an open; all instances busy (`ERROR_PIPE_BUSY`) means it runs too.
#[cfg(windows)]
fn running(socket: &Path) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    match std::fs::OpenOptions::new().read(true).write(true).open(socket) {
        Ok(_) => true,
        Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
    }
}

#[cfg(not(any(unix, windows)))]
fn running(_socket: &Path) -> bool {
    false
}

/// Runs `rewarden ssh …`; the lines to print.
pub fn run(cmd: &SshCmd, paths: &Paths, config: &Config, home: &Path) -> Result<Vec<String>, String> {
    let socket = super::socket_path(paths, &config.ssh);
    let file = config_file(home);
    let shown = file.display();
    let mut out = Vec::new();
    match cmd {
        SshCmd::Setup => {
            let line = match setup(&file, &socket)? {
                Change::Added => format!("{shown}: ssh now asks Rewarden's agent for keys ({}).", socket.display()),
                Change::Updated => format!("{shown}: updated to Rewarden's agent at {}.", socket.display()),
                Change::Unchanged => format!("{shown} already points ssh at Rewarden's agent."),
            };
            out.push(line);
            out.extend(warnings(&file, &socket, config));
            if cfg!(windows) {
                out.push(format!(
                    "Windows' OpenSSH (ssh.exe in C:\\Windows\\System32\\OpenSSH) reads this. Git Bash's own ssh cannot use the \
                     agent: for git over SSH run `git config --global core.sshCommand C:/Windows/System32/OpenSSH/ssh.exe`. \
                     Programs that read SSH_AUTH_SOCK instead: set it to {}.",
                    socket.display()
                ));
            }
            out.push("Your phone lists the keys and signs each login. `rewarden ssh unsetup` undoes this.".to_owned());
        }
        SshCmd::Unsetup => {
            out.push(if unsetup(&file)? {
                format!("{shown}: removed Rewarden's agent; ssh uses its usual keys again.")
            } else {
                format!("{shown} has no Rewarden block.")
            });
        }
        SshCmd::Status => {
            let state = if !config.ssh.enabled {
                "disabled (`[ssh] enabled = false`)"
            } else if running(&socket) {
                "running"
            } else {
                "not running (start the daemon: `rewarden resume`)"
            };
            out.push(format!("Agent:      {} ({state})", socket.display()));
            out.push(match current(&file)? {
                Some(s) => format!("ssh config: {shown} uses {s}"),
                None => "ssh config: not set up (`rewarden ssh setup`)".to_owned(),
            });
            out.extend(warnings(&file, &socket, config));
        }
    }
    Ok(out)
}

fn warnings(file: &Path, socket: &Path, config: &Config) -> Vec<String> {
    let mut out = Vec::new();
    if !config.ssh.enabled {
        out.push("Note: the agent is disabled in config.toml (`[ssh] enabled = false`).".to_owned());
    }
    if current(file).ok().flatten().is_some()
        && let Some(effective) = effective_agent(file)
        && !same_agent(&effective, socket)
    {
        out.push(format!(
            "Note: an earlier `IdentityAgent` in {} wins ({effective}); remove it for ssh to use Rewarden's agent.",
            file.display()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_appends_a_block_and_unsetup_restores_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".ssh/config");
        let socket = Path::new("/run/user/1000/rewarden/ssh-agent.sock");
        assert_eq!(setup(&file, socket).unwrap(), Change::Added);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), block(socket).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert!(unsetup(&file).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");

        let user = "Host work\n    User me\n\nHost *\n    ServerAliveInterval 30\n";
        std::fs::write(&file, user).unwrap();
        assert_eq!(setup(&file, socket).unwrap(), Change::Added);
        assert_eq!(setup(&file, socket).unwrap(), Change::Unchanged);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with(user) && text.ends_with(&block(socket).unwrap()), "{text}");
        assert_eq!(current(&file).unwrap().as_deref(), Some("/run/user/1000/rewarden/ssh-agent.sock"));

        let moved = Path::new("/tmp/100%/agent.sock");
        assert_eq!(setup(&file, moved).unwrap(), Change::Updated);
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches(BEGIN).count(), 1);
        assert!(text.contains("IdentityAgent \"/tmp/100%%/agent.sock\""), "{text}");
        assert_eq!(current(&file).unwrap().as_deref(), Some("/tmp/100%/agent.sock"));
        assert!(unsetup(&file).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), user);
        assert!(!unsetup(&file).unwrap());
    }

    #[test]
    fn a_block_in_the_middle_is_removed_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        let socket = Path::new("/s.sock");
        let before = "Host a\n    User x\n";
        std::fs::write(&file, before).unwrap();
        setup(&file, socket).unwrap();
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("\nHost b\n    User y\n");
        std::fs::write(&file, &text).unwrap();
        assert!(unsetup(&file).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Host a\n    User x\n\nHost b\n    User y\n");
    }

    #[test]
    #[cfg(unix)]
    fn a_symlinked_config_is_edited_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-ssh-config");
        std::fs::write(&real, "Host a\n").unwrap();
        std::fs::create_dir(dir.path().join(".ssh")).unwrap();
        let link = dir.path().join(".ssh/config");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        setup(&link, Path::new("/s.sock")).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&real).unwrap().contains("IdentityAgent"));
        unsetup(&link).unwrap();
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "Host a\n");
    }

    #[test]
    fn unwritable_socket_paths_are_refused() {
        assert!(block(Path::new("/a\"b")).is_err());
        assert!(block(Path::new("/a\nb")).is_err());
    }
}
