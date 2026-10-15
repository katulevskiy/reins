//! This computer's name as its owner knows it, which is how the phone names the connection: the Computer Name on a
//! Mac ("M2A", not the hostname "Mac.localdomain"), the computer name on Windows, the pretty hostname on Linux (else
//! the hostname); never with a domain (`reins_proto::computer::clean_name`).
//!
//! Not the name of this computer's machine project (`secrets_scan::computer_name`): that one names stored secrets
//! and stays as it was.

use reins_proto::computer::{self, Os};

/// This computer's name, "Computer" when it has none.
#[must_use]
pub fn name() -> String {
    let name = own_name()
        .map(|n| computer::clean_name(&n))
        .filter(|n| !n.is_empty())
        .or_else(|| hostname::get().ok().map(|h| computer::clean_name(&h.to_string_lossy())).filter(|n| !n.is_empty()));
    name.unwrap_or_else(|| "Computer".to_owned())
}

/// The name this app registers with the server, which the phone shows: "M2A (macOS)" (the phone shows "M2A" and the
/// Apple logo).
#[must_use]
pub fn client_name() -> String {
    computer::client_name(&name(), Os::current())
}

/// The Computer Name (System Settings > General > Sharing), else the local hostname, through SystemConfiguration's
/// `scutil`.
#[cfg(target_os = "macos")]
fn own_name() -> Option<String> {
    ["ComputerName", "LocalHostName"].into_iter().find_map(|what| {
        let out = std::process::Command::new("/usr/sbin/scutil").args(["--get", what]).output().ok()?;
        let name = String::from_utf8(out.stdout).ok()?;
        (out.status.success() && !name.trim().is_empty()).then(|| name.trim().to_owned())
    })
}

/// The computer name (`GetComputerNameExW`, which `hostname` reads), else `%COMPUTERNAME%`.
#[cfg(windows)]
fn own_name() -> Option<String> {
    hostname::get()
        .ok()
        .map(|h| h.to_string_lossy().into_owned())
        .filter(|h| !h.trim().is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

/// The pretty hostname (`PRETTY_HOSTNAME` in /etc/machine-info, what `hostnamectl --pretty` prints); none when it
/// is not set, and the hostname is used.
#[cfg(not(any(target_os = "macos", windows)))]
fn own_name() -> Option<String> {
    match std::fs::read_to_string("/etc/machine-info") {
        Ok(text) => pretty_hostname(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            let out = std::process::Command::new("hostnamectl").arg("--pretty").output().ok()?;
            let name = String::from_utf8(out.stdout).ok()?;
            (out.status.success() && !name.trim().is_empty()).then(|| name.trim().to_owned())
        }
    }
}

/// `PRETTY_HOSTNAME` of a machine-info file (shell-style `KEY=value` lines, the value maybe quoted).
#[cfg(any(test, not(any(target_os = "macos", windows))))]
fn pretty_hostname(machine_info: &str) -> Option<String> {
    let value = machine_info.lines().find_map(|l| l.trim().strip_prefix("PRETTY_HOSTNAME="))?.trim();
    let unquoted = match value.as_bytes() {
        [q @ (b'"' | b'\''), .., last] if last == q => &value[1..value.len() - 1],
        _ => value,
    };
    let name = unquoted.replace("\\\"", "\"").replace("\\\\", "\\");
    (!name.trim().is_empty()).then(|| name.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pretty_hostname_is_read_from_machine_info() {
        assert_eq!(
            pretty_hostname("PRETTY_HOSTNAME=\"Dana's Laptop\"\nCHASSIS=laptop\n").as_deref(),
            Some("Dana's Laptop")
        );
        assert_eq!(pretty_hostname("CHASSIS=desktop\nPRETTY_HOSTNAME=honeypot\n").as_deref(), Some("honeypot"));
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME='Work box'").as_deref(), Some("Work box"));
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME=\"say \\\"hi\\\"\"").as_deref(), Some("say \"hi\""));
        assert_eq!(pretty_hostname("PRETTY_HOSTNAME=\"\"\n"), None);
        assert_eq!(pretty_hostname("CHASSIS=vm\n"), None);
    }

    #[test]
    fn the_app_names_itself_after_this_computer_and_its_platform() {
        let name = name();
        assert!(!name.is_empty());
        let registered = client_name();
        assert!(registered.starts_with(&name), "{registered}");
        assert_eq!(computer::parse_client_name(&registered), (name, Os::current()));
        assert!(!registered.starts_with(computer::LEGACY_PREFIX));
    }
}
