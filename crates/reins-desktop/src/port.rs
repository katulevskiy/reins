//! Who holds the daemon's port, for the message when the daemon cannot listen: "port 7457 is used by
//! `rewarden daemon` (pid 1437)" says what to quit where "Address already in use" does not.

use std::net::SocketAddr;

/// The program listening on `port` on this computer: its command line and pid when this user may see it, "another
/// program" when only the socket is visible, `None` when nothing listens or this system cannot tell.
#[must_use]
pub fn holder(port: u16) -> Option<String> {
    imp::holder(port)
}

/// Why the daemon did not come up on `addr`: the program in its way when there is one, else where to look.
#[must_use]
pub fn not_started(addr: SocketAddr) -> String {
    match holder(addr.port()) {
        Some(who) => in_use(addr, &who),
        None => "the background service did not start (run `reins daemon` in a terminal to see why)".to_owned(),
    }
}

/// `addr` is taken by `who`: what to do about it.
#[must_use]
pub fn in_use(addr: SocketAddr, who: &str) -> String {
    format!(
        "port {} is already used by {who}: quit it and try again, or give Reins another port with `listen = \"{}:<port>\"` in its config.toml",
        addr.port(),
        addr.ip()
    )
}

/// One line of a command line, short enough for a message.
fn shorten(cmd: &str) -> String {
    let cmd = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    if cmd.chars().count() > 80 {
        format!("{}…", cmd.chars().take(79).collect::<String>())
    } else {
        cmd
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashSet;

    /// Listening sockets come from `/proc/net/tcp{,6}`, their owner from the `socket:[inode]` links in
    /// `/proc/<pid>/fd`.
    pub fn holder(port: u16) -> Option<String> {
        let mut inodes = HashSet::new();
        for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(text) = std::fs::read_to_string(table) {
                inodes.extend(listening(&text, port));
            }
        }
        if inodes.is_empty() {
            return None;
        }
        Some(owner(&inodes).unwrap_or_else(|| "another program".to_owned()))
    }

    /// The inodes of the sockets in a `/proc/net/tcp` table listening on `port`.
    pub fn listening(table: &str, port: u16) -> Vec<String> {
        const LISTEN: &str = "0A";
        table
            .lines()
            .skip(1)
            .filter_map(|line| {
                let f: Vec<&str> = line.split_whitespace().collect();
                let local_port = u16::from_str_radix(f.get(1)?.rsplit(':').next()?, 16).ok()?;
                let inode = (*f.get(9)?).to_owned();
                (local_port == port && *f.get(3)? == LISTEN).then_some(inode)
            })
            .collect()
    }

    /// The process holding one of `inodes`. The daemon makes itself non-dumpable, which hides its `fd` links even
    /// from its own user, so a Reins (or Rewarden) daemon other than this process stands in when no owner is seen.
    fn owner(inodes: &HashSet<String>) -> Option<String> {
        let me = std::process::id();
        let mut daemon = None;
        for proc in std::fs::read_dir("/proc").ok()?.flatten() {
            let Some(pid) = proc.file_name().to_str().and_then(|p| p.parse::<u32>().ok()) else {
                continue;
            };
            let cmd = std::fs::read(proc.path().join("cmdline")).unwrap_or_default();
            let args: Vec<String> = cmd
                .split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            let shown = format!("`{}` (pid {pid})", super::shorten(&args.join(" ")));
            let held = std::fs::read_dir(proc.path().join("fd")).is_ok_and(|fds| {
                fds.flatten().any(|fd| {
                    std::fs::read_link(fd.path()).ok().and_then(|l| {
                        l.to_str()?.strip_prefix("socket:[")?.strip_suffix(']').map(|i| inodes.contains(i))
                    }) == Some(true)
                })
            });
            if held {
                return Some(shown);
            }
            if daemon.is_none() && pid != me && is_daemon(&args) {
                daemon = Some(shown);
            }
        }
        daemon
    }

    /// `reins daemon` or `rewarden daemon`, whatever the directory.
    pub fn is_daemon(args: &[String]) -> bool {
        let program = args.first().map(|a| a.rsplit('/').next().unwrap_or(a));
        matches!(program, Some("reins" | "rewarden")) && args.get(1).is_some_and(|a| a == "daemon")
    }
}

#[cfg(target_os = "macos")]
mod imp {
    /// `lsof -F` prints `p<pid>` then `c<command>` for each listener.
    pub fn holder(port: u16) -> Option<String> {
        let out = std::process::Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fpc"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let pid = text.lines().find_map(|l| l.strip_prefix('p'))?;
        let cmd = text.lines().find_map(|l| l.strip_prefix('c')).unwrap_or("another program");
        Some(format!("`{}` (pid {pid})", super::shorten(cmd)))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod imp {
    pub fn holder(_port: u16) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_the_holder_and_the_way_out() {
        let text = in_use("127.0.0.1:7457".parse().unwrap(), "`rewarden daemon` (pid 1437)");
        assert!(text.contains("port 7457 is already used by `rewarden daemon` (pid 1437)"), "{text}");
        assert!(text.contains("listen = \"127.0.0.1:<port>\""), "{text}");
    }

    #[test]
    fn long_command_lines_are_shortened() {
        assert_eq!(shorten("a  b\tc "), "a b c");
        assert_eq!(shorten(&"x".repeat(200)).chars().count(), 80);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_listeners_on_the_port_count() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1D21 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1
   1: 0100007F:1D21 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1000        0 4343 1
   2: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4444 1
";
        assert_eq!(imp::listening(table, 7457), ["4242"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reins_and_rewarden_daemons_stand_in_for_a_hidden_owner() {
        let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(imp::is_daemon(&args(&["/home/me/.local/bin/rewarden", "daemon"])));
        assert!(imp::is_daemon(&args(&["reins", "daemon", "--log-file", "x"])));
        assert!(!imp::is_daemon(&args(&["/home/me/.local/bin/reins", "mcp"])));
        assert!(!imp::is_daemon(&args(&["/usr/bin/python3", "daemon"])));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_listener_of_this_process_is_found() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let who = holder(port).unwrap();
        assert!(who.contains(&format!("(pid {})", std::process::id())), "{who}");
        drop(listener);
        assert_eq!(holder(port), None);
    }
}
