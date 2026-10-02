//! Telling the person running git what is going on while an approval is awaited. git shows nothing while the daemon
//! waits for the phone, so the daemon finds the git process on the other end of the connection and writes a line to
//! its stderr: the terminal, or whatever captures git's output (an AI agent's tool). Linux only (it reads `/proc`); a
//! no-op elsewhere, and silent when anything does not line up.

use std::net::SocketAddr;

/// Both ends of a connection to the daemon, kept in each request's extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub client: SocketAddr,
    pub server: SocketAddr,
}

/// Writes `line` (plus a newline) to the stderr of the local process that owns the client end of `peer`, if it runs as
/// this user. Blocking: call it from `spawn_blocking`.
pub fn tell(peer: Peer, line: &str) {
    #[cfg(target_os = "linux")]
    if let Err(e) = linux::tell(peer, line) {
        log::debug!("could not tell the git process: {e}");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (peer, line);
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Write as _;
    use std::os::unix::fs::MetadataExt as _;

    use super::Peer;

    /// The socket inode of the connection from `client_port` to `server_port`, from `/proc/net/tcp{,6}`, if it belongs
    /// to `uid`.
    pub(super) fn socket_inode(table: &str, client_port: u16, server_port: u16, uid: u32) -> Option<u64> {
        table.lines().skip(1).find_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let port = |addr: &str| addr.rsplit_once(':').and_then(|(_, p)| u16::from_str_radix(p, 16).ok());
            let (local, remote, owner, inode) = (f.get(1)?, f.get(2)?, f.get(7)?, f.get(9)?);
            (port(local)? == client_port && port(remote)? == server_port && owner.parse::<u32>().ok()? == uid)
                .then(|| inode.parse().ok())
                .flatten()
        })
    }

    pub(super) fn tell(peer: Peer, line: &str) -> Result<(), String> {
        let uid = rustix::process::getuid().as_raw();
        let wanted = |table: &str| {
            std::fs::read_to_string(table)
                .ok()
                .and_then(|t| socket_inode(&t, peer.client.port(), peer.server.port(), uid))
        };
        let inode = wanted("/proc/net/tcp").or_else(|| wanted("/proc/net/tcp6")).ok_or("connection not found")?;
        let target = format!("socket:[{inode}]");
        for entry in std::fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if !entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())
                || std::fs::metadata(&path).map_or(true, |m| m.uid() != uid)
            {
                continue;
            }
            let Ok(fds) = std::fs::read_dir(path.join("fd")) else {
                continue;
            };
            let owns =
                fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|l| l.as_os_str() == target.as_str()));
            if owns {
                let mut stderr = std::fs::OpenOptions::new()
                    .append(true)
                    .open(path.join("fd/2"))
                    .map_err(|e| format!("stderr: {e}"))?;
                return stderr.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string());
            }
        }
        Err("no process owns the connection".to_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::socket_inode;

        const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1D21 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0000000000000000 100 0 0 10 0
   1: 0100007F:B1A2 0100007F:1D21 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000000000000000 20 4 30 10 -1
   2: 0100007F:1D21 0100007F:B1A2 01 00000000:00000000 00:00000000 00000000  1000        0 33333 1 0000000000000000 20 4 30 10 -1
";

        #[test]
        fn the_client_end_of_the_connection_is_found_and_only_for_this_user() {
            assert_eq!(socket_inode(TABLE, 0xB1A2, 0x1D21, 1000), Some(22222));
            assert_eq!(socket_inode(TABLE, 0xB1A2, 0x1D21, 1001), None, "another user's socket");
            assert_eq!(socket_inode(TABLE, 0xB1A3, 0x1D21, 1000), None);
        }
    }
}
