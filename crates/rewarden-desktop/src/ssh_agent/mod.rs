//! An SSH agent whose keys stay on the phone. ssh (via `IdentityAgent` in `~/.ssh/config`, see [`setup`]) asks this
//! agent on a unix socket (`$XDG_RUNTIME_DIR/rewarden/ssh-agent.sock`, 0600, this user only) for identities, which come
//! from `vault_ssh_keys` (kept a few minutes), and for signatures, which the phone makes after the user approved them
//! (`vault_ssh_sign`, sealed to this app). ssh's `session-bind@openssh.com` tells the agent which server it is
//! talking to (its host key, checked against the key exchange signature), and `~/.ssh/known_hosts` its name when the
//! entry is not hashed; the phone shows both. While the phone decides, the ssh process is told so on its stderr.

pub mod keys;
pub mod setup;
pub mod wire;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use data_encoding::{BASE64, BASE64URL_NOPAD};
use rewarden_proto::desktop::SshSignature;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::keys::Verified;
use self::wire::{Reader, Writer};
use crate::auth::Refusal;
use crate::config::Paths;
use crate::phone::{PhoneLink, awaiting, teller, unavailable};

pub const KEYS_TOOL: &str = "vault_ssh_keys";
pub const SIGN_TOOL: &str = "vault_ssh_sign";
pub const SOCKET_NAME: &str = "ssh-agent.sock";
/// How long the phone's key list is used before it is asked again.
const KEYS_TTL: Duration = Duration::from_mins(5);
const MAX_MESSAGE: usize = 256 * 1024;
/// The phone takes at most 65,536 base64 characters of data to sign.
const MAX_SIGN_DATA: usize = 49_152;
const MAX_BINDS: usize = 16;

const FAILURE: u8 = 5;
const SUCCESS: u8 = 6;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;
const EXTENSION: u8 = 27;
const USERAUTH_REQUEST: u8 = 50;
const SESSION_BIND: &str = "session-bind@openssh.com";

/// `[ssh]` in `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshConfig {
    /// Whether the daemon runs the SSH agent.
    pub enabled: bool,
    /// The agent's socket; by default `$XDG_RUNTIME_DIR/rewarden/ssh-agent.sock`, else in the state directory.
    pub socket: Option<PathBuf>,
    /// Where server names are looked up; by default `~/.ssh/known_hosts` and `~/.ssh/known_hosts2`.
    pub known_hosts: Vec<PathBuf>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            socket: None,
            known_hosts: Vec::new(),
        }
    }
}

impl SshConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.socket.as_ref().is_some_and(|s| !s.is_absolute()) {
            return Err("`ssh.socket` must be an absolute path".to_owned());
        }
        if self.known_hosts.iter().any(|p| !p.is_absolute()) {
            return Err("`ssh.known_hosts` must be absolute paths".to_owned());
        }
        Ok(())
    }

    fn known_hosts_files(&self) -> Vec<PathBuf> {
        if !self.known_hosts.is_empty() {
            return self.known_hosts.clone();
        }
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| {
                let ssh = PathBuf::from(h).join(".ssh");
                vec![ssh.join("known_hosts"), ssh.join("known_hosts2")]
            })
            .unwrap_or_default()
    }
}

/// Where the agent listens: `ssh.socket`; else, for the app's usual state directory, `$XDG_RUNTIME_DIR/rewarden/`;
/// else (`REWARDEN_STATE_DIR` or another state directory, as in tests, or no runtime directory) the state directory,
/// so a second instance never takes over the usual socket.
#[must_use]
pub fn socket_path(paths: &Paths, config: &SshConfig) -> PathBuf {
    if let Some(s) = &config.socket {
        return s.clone();
    }
    let usual = std::env::var_os("REWARDEN_STATE_DIR").is_none_or(|d| d.is_empty())
        && Paths::from_env().is_ok_and(|p| p.state_dir == paths.state_dir);
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(run) if usual => PathBuf::from(run).join("rewarden").join(SOCKET_NAME),
        _ => paths.state_dir.join(SOCKET_NAME),
    }
}

/// A vault SSH key as the phone listed it.
#[derive(Clone, Debug)]
struct VaultKey {
    blob: Vec<u8>,
    name: String,
    fingerprint: String,
}

/// The phone's key list: `items` (or `keys`) with `public_key` (an OpenSSH line), a name (`title`/`name`, else the
/// line's comment) and optionally `fingerprint`, which must match. Entries that do not parse are skipped.
fn parse_keys(data: &Value) -> Vec<VaultKey> {
    let list = data.get("items").or_else(|| data.get("keys")).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|item| {
            let line = ["public_key", "key", "public"].iter().find_map(|k| item.get(*k).and_then(Value::as_str))?;
            let Ok((blob, comment)) = keys::parse_line(line) else {
                log::warn!("the phone listed an SSH key that does not parse; skipped");
                return None;
            };
            let fingerprint = keys::fingerprint(&blob);
            if item.get("fingerprint").and_then(Value::as_str).is_some_and(|f| f != fingerprint) {
                log::warn!("the phone listed an SSH key whose fingerprint does not match; skipped");
                return None;
            }
            let name = ["title", "name"]
                .iter()
                .find_map(|k| item.get(*k).and_then(Value::as_str))
                .map_or(comment, str::to_owned);
            Some(VaultKey {
                blob,
                name,
                fingerprint,
            })
        })
        .collect()
}

/// What one sign request says about itself when it is an SSH user authentication.
#[derive(Debug, PartialEq, Eq)]
struct UserAuth<'a> {
    session_id: &'a [u8],
    user: &'a str,
    /// `publickey-hostbound-v00@openssh.com` carries the server's host key.
    host_key: Option<&'a [u8]>,
}

fn parse_userauth(data: &[u8]) -> Option<UserAuth<'_>> {
    let mut r = Reader::new(data);
    let session_id = r.string()?;
    if r.byte()? != USERAUTH_REQUEST {
        return None;
    }
    let user = r.str()?;
    let _service = r.string()?;
    let method = r.str()?;
    if !r.bool()? {
        return None;
    }
    let (_alg, _key) = (r.string()?, r.string()?);
    let host_key = match method {
        "publickey" => None,
        "publickey-hostbound-v00@openssh.com" => Some(r.string()?),
        _ => return None,
    };
    r.is_empty().then_some(UserAuth {
        session_id,
        user,
        host_key,
    })
}

/// A server this connection's ssh said it talks to (`session-bind@openssh.com`), checked.
struct Bind {
    host_key: Vec<u8>,
    session_id: Vec<u8>,
}

/// One client connection's state.
#[derive(Default)]
pub struct Session {
    binds: Vec<Bind>,
    pid: Option<i32>,
}

fn failure() -> Vec<u8> {
    vec![FAILURE]
}

type Tell = Option<crate::phone::Tell>;

pub struct Agent {
    phone: Arc<PhoneLink>,
    known_hosts: Vec<PathBuf>,
    keys: tokio::sync::Mutex<Option<(Instant, Vec<VaultKey>)>>,
}

impl Agent {
    #[must_use]
    pub fn new(phone: Arc<PhoneLink>, config: &SshConfig) -> Self {
        Self {
            phone,
            known_hosts: config.known_hosts_files(),
            keys: tokio::sync::Mutex::new(None),
        }
    }

    fn tell(session: &Session) -> Tell {
        session.pid.map(|pid| teller(move |line: &str| tell_pid(pid, line)))
    }

    async fn keys(&self, tell: Tell) -> Result<Vec<VaultKey>, Refusal> {
        let mut cache = self.keys.lock().await;
        if let Some((at, keys)) = cache.as_ref()
            && at.elapsed() < KEYS_TTL
        {
            return Ok(keys.clone());
        }
        let phone = self.phone.phone()?;
        let answer = awaiting(
            tell,
            "the list of SSH keys",
            phone.ask(KEYS_TOOL, Some(KEYS_TOOL), |_| json!({}), "No answer from your phone in time."),
        )
        .await?;
        let keys = parse_keys(&answer.data);
        log::info!("ssh agent: the phone listed {} key(s)", keys.len());
        *cache = Some((Instant::now(), keys.clone()));
        Ok(keys)
    }

    /// Answers one agent message.
    pub async fn handle(&self, msg: &[u8], session: &mut Session) -> Vec<u8> {
        let Some((&kind, body)) = msg.split_first() else {
            return failure();
        };
        match kind {
            REQUEST_IDENTITIES => self.identities(session).await,
            SIGN_REQUEST => match self.sign(body, session).await {
                Ok(reply) => reply,
                Err(r) => {
                    log::info!("ssh agent: signature refused: {}", r.message());
                    if let Some(tell) = Self::tell(session) {
                        let line = format!("rewarden: {}", r.message());
                        tokio::task::spawn_blocking(move || tell(&line)).await.ok();
                    }
                    failure()
                }
            },
            EXTENSION => Self::extension(body, session),
            _ => failure(),
        }
    }

    async fn identities(&self, session: &Session) -> Vec<u8> {
        let keys = match self.keys(Self::tell(session)).await {
            Ok(k) => k,
            Err(r) => {
                log::info!("ssh agent: no keys from the phone: {}", r.message());
                if !matches!(r, Refusal::Unavailable(_))
                    && let Some(tell) = Self::tell(session)
                {
                    let line = format!("rewarden: no SSH keys from your phone: {}", r.message());
                    tokio::task::spawn_blocking(move || tell(&line)).await.ok();
                }
                Vec::new()
            }
        };
        let mut w = Writer::new();
        w.byte(IDENTITIES_ANSWER).u32(u32::try_from(keys.len()).unwrap_or(0));
        for k in &keys {
            w.string(&k.blob).string(k.name.as_bytes());
        }
        w.buf
    }

    fn host_name(&self, host_key: &[u8]) -> Option<String> {
        self.known_hosts.iter().find_map(|f| keys::host_name(&std::fs::read_to_string(f).ok()?, host_key))
    }

    async fn sign(&self, body: &[u8], session: &Session) -> Result<Vec<u8>, Refusal> {
        let mut r = Reader::new(body);
        let (Some(blob), Some(data), Some(flags)) = (r.string(), r.string(), r.u32()) else {
            return Err(unavailable("Malformed sign request."));
        };
        if data.len() > MAX_SIGN_DATA {
            return Err(unavailable("The data to sign is too large for the phone."));
        }
        let tell = Self::tell(session);
        let key = self
            .keys(tell.clone())
            .await?
            .into_iter()
            .find(|k| k.blob == blob)
            .ok_or_else(|| unavailable("That key is not one of your phone's SSH keys."))?;
        let auth = parse_userauth(data);
        if let (Some(a), Some(last)) = (&auth, session.binds.last())
            && a.session_id != last.session_id.as_slice()
        {
            return Err(unavailable("The sign request is for another SSH session than the one ssh announced."));
        }
        let host_key = auth.as_ref().and_then(|a| {
            a.host_key
                .map(<[u8]>::to_vec)
                .or_else(|| session.binds.iter().find(|b| b.session_id == a.session_id).map(|b| b.host_key.clone()))
        });
        let host = host_key.as_deref().and_then(|k| self.host_name(k));
        let host_fp = host_key.as_deref().map(keys::fingerprint);
        let what = match (&auth, &host, &host_fp) {
            (Some(a), Some(h), _) => format!("sign in to {h} as {} with {}", a.user, key.name),
            (Some(a), None, Some(fp)) => format!("sign in to the server {fp} as {} with {}", a.user, key.name),
            (Some(a), None, None) => format!("sign in as {} with {}", a.user, key.name),
            (None, ..) => format!("sign with {}", key.name),
        };
        let phone = self.phone.phone()?;
        let data_b64 = BASE64.encode(data);
        let answer = awaiting(
            tell,
            &what,
            phone.ask(
                SIGN_TOOL,
                None,
                |_| {
                    let mut args = json!({"key": key.fingerprint, "data_base64": data_b64, "flags": flags});
                    if let Some(h) = &host {
                        args["host"] = json!(h);
                    }
                    if let Some(fp) = &host_fp {
                        args["host_key"] = json!(fp);
                    }
                    args
                },
                "No answer from your phone in time. Approve it, then connect again.",
            ),
        )
        .await?;
        let sig: SshSignature = phone.open(&answer.data)?;
        if sig.nonce != answer.nonce {
            return Err(unavailable("The phone's answer is for another request (the nonce differs); refused."));
        }
        let raw = BASE64
            .decode(sig.signature_base64.trim().as_bytes())
            .or_else(|_| BASE64URL_NOPAD.decode(sig.signature_base64.trim().as_bytes()))
            .map_err(|_| unavailable("The phone's signature is malformed; refused."))?;
        let kind = keys::key_type(&key.blob).unwrap_or_default();
        if keys::signature_type(&raw) != Some(keys::signature_algorithm(kind, flags)) {
            return Err(unavailable("The phone signed with another algorithm than ssh asked for; refused."));
        }
        if keys::verify(&key.blob, data, &raw) == Verified::Bad {
            return Err(unavailable("The phone's signature does not verify; refused."));
        }
        log::info!("ssh agent: signed with {} ({})", key.fingerprint, host.as_deref().unwrap_or("no host name"));
        let mut w = Writer::new();
        w.byte(SIGN_RESPONSE).string(&raw);
        Ok(w.buf)
    }

    fn extension(body: &[u8], session: &mut Session) -> Vec<u8> {
        let mut r = Reader::new(body);
        if r.str() != Some(SESSION_BIND) {
            return failure();
        }
        let (Some(host_key), Some(session_id), Some(signature), Some(_forwarding)) =
            (r.string(), r.string(), r.string(), r.bool())
        else {
            return failure();
        };
        if keys::verify(host_key, session_id, signature) != Verified::Good || session.binds.len() >= MAX_BINDS {
            log::info!("ssh agent: session-bind not accepted");
            return failure();
        }
        log::info!("ssh agent: session bound to the server key {}", keys::fingerprint(host_key));
        session.binds.push(Bind {
            host_key: host_key.to_vec(),
            session_id: session_id.to_vec(),
        });
        vec![SUCCESS]
    }
}

/// Writes `line` to the stderr of process `pid` when it runs as this user (Linux; elsewhere nothing). Blocking.
fn tell_pid(pid: i32, line: &str) {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as _;
        use std::os::unix::fs::MetadataExt as _;
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let uid = rustix::process::getuid().as_raw();
        if std::fs::metadata(&proc).map_or(true, |m| m.uid() != uid) {
            return;
        }
        if let Ok(mut stderr) = std::fs::OpenOptions::new().append(true).open(proc.join("fd/2")) {
            stderr.write_all(format!("{line}\n").as_bytes()).ok();
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (pid, line);
}

/// The socket file this agent made; removed when the agent stops, unless something else replaced it meanwhile.
struct SocketFile {
    path: PathBuf,
    id: Option<(u64, u64)>,
}

#[cfg(unix)]
fn file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::symlink_metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn file_id(_path: &Path) -> Option<(u64, u64)> {
    None
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        if self.id.is_some() && file_id(&self.path) == self.id {
            std::fs::remove_file(&self.path).ok();
        }
    }
}

/// The agent bound to its socket, not yet serving.
pub struct SshAgent {
    #[cfg(unix)]
    listener: tokio::net::UnixListener,
    file: SocketFile,
    agent: Arc<Agent>,
}

impl SshAgent {
    /// Binds `path` (its directory made 0700 when missing, the socket 0600). A stale socket is replaced; a live one
    /// (another agent) is an error.
    #[cfg(unix)]
    pub fn bind(path: &Path, agent: Agent) -> Result<Self, String> {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        let shown = path.display();
        if let Some(dir) = path.parent()
            && !dir.exists()
        {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        if std::fs::symlink_metadata(path).is_ok() {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(format!("another SSH agent is already serving {shown}"));
            }
            std::fs::remove_file(path).map_err(|e| format!("{shown}: {e}"))?;
        }
        let listener = tokio::net::UnixListener::bind(path).map_err(|e| format!("cannot listen on {shown}: {e}"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{shown}: {e}"))?;
        Ok(Self {
            listener,
            file: SocketFile {
                path: path.to_owned(),
                id: file_id(path),
            },
            agent: Arc::new(agent),
        })
    }

    #[cfg(not(unix))]
    pub fn bind(_path: &Path, _agent: Agent) -> Result<Self, String> {
        Err("the SSH agent needs unix sockets".to_owned())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file.path
    }

    /// Serves until dropped; the socket file goes with it.
    pub async fn serve(self) {
        #[cfg(unix)]
        loop {
            let stream = match self.listener.accept().await {
                Ok((s, _)) => s,
                Err(e) => {
                    log::warn!("ssh agent: accept: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            tokio::spawn(connection(stream, Arc::clone(&self.agent)));
        }
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    }
}

/// The daemon's agent: `None` when `[ssh] enabled = false` or the socket cannot be bound (logged; the rest of the
/// daemon runs on).
#[must_use]
pub fn start(paths: &Paths, config: &SshConfig, phone: Arc<PhoneLink>) -> Option<SshAgent> {
    if !config.enabled {
        return None;
    }
    let path = socket_path(paths, config);
    match SshAgent::bind(&path, Agent::new(phone, config)) {
        Ok(a) => {
            log::info!("ssh agent on {}", path.display());
            Some(a)
        }
        Err(e) => {
            log::warn!("ssh agent not started: {e}");
            None
        }
    }
}

/// Serves `agent` when there is one, else never finishes.
pub async fn serve(agent: Option<SshAgent>) {
    match agent {
        Some(a) => a.serve().await,
        None => std::future::pending().await,
    }
}

#[cfg(unix)]
async fn connection(mut stream: tokio::net::UnixStream, agent: Arc<Agent>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    // SO_PEERCRED: only this user's processes, and the pid to tell while the phone decides.
    let Ok(cred) = stream.peer_cred() else {
        return;
    };
    if cred.uid() != rustix::process::getuid().as_raw() {
        log::warn!("ssh agent: refused a connection from another user");
        return;
    }
    let mut session = Session {
        pid: cred.pid(),
        ..Session::default()
    };
    loop {
        let Ok(len) = stream.read_u32().await else {
            return;
        };
        let Ok(len) = usize::try_from(len) else {
            return;
        };
        if len == 0 || len > MAX_MESSAGE {
            return;
        }
        let mut msg = vec![0; len];
        if stream.read_exact(&mut msg).await.is_err() {
            return;
        }
        let reply = agent.handle(&msg, &mut session).await;
        let mut out = Writer::new();
        out.string(&reply);
        if stream.write_all(&out.buf).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn userauth(method: &str, host_key: Option<&[u8]>) -> Vec<u8> {
        let mut w = Writer::new();
        w.string(b"sid").byte(USERAUTH_REQUEST).string(b"git").string(b"ssh-connection").string(method.as_bytes());
        w.byte(1).string(b"ssh-ed25519").string(b"key");
        if let Some(k) = host_key {
            w.string(k);
        }
        w.buf
    }

    #[test]
    fn user_authentications_are_recognised() {
        let a = userauth("publickey", None);
        assert_eq!(
            parse_userauth(&a),
            Some(UserAuth {
                session_id: b"sid",
                user: "git",
                host_key: None
            })
        );
        let b = userauth("publickey-hostbound-v00@openssh.com", Some(b"hk"));
        assert_eq!(parse_userauth(&b).unwrap().host_key, Some(&b"hk"[..]));
        assert_eq!(parse_userauth(b"SSHSIG\0\0\0\x01"), None);
        let mut trailing = a;
        trailing.push(0);
        assert_eq!(parse_userauth(&trailing), None);
        assert_eq!(parse_userauth(&userauth("password", None)), None);
    }

    fn session_bind(host: &ring::signature::Ed25519KeyPair, session_id: &[u8], signed: &[u8]) -> Vec<u8> {
        use ring::signature::KeyPair as _;
        let mut key = Writer::new();
        key.string(b"ssh-ed25519").string(host.public_key().as_ref());
        let mut sig = Writer::new();
        sig.string(b"ssh-ed25519").string(host.sign(signed).as_ref());
        let mut w = Writer::new();
        w.string(SESSION_BIND.as_bytes()).string(&key.buf).string(session_id).string(&sig.buf).byte(0);
        w.buf
    }

    #[test]
    fn session_binds_are_recorded_only_when_the_server_signed_them() {
        let host = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
        let mut session = Session::default();
        assert_eq!(Agent::extension(&session_bind(&host, b"sid-1", b"sid-1"), &mut session), [SUCCESS]);
        assert_eq!(session.binds.len(), 1);
        assert_eq!(session.binds[0].session_id, b"sid-1");
        assert_eq!(keys::key_type(&session.binds[0].host_key), Some("ssh-ed25519"));
        assert_eq!(Agent::extension(&session_bind(&host, b"sid-2", b"forged"), &mut session), [FAILURE]);
        assert_eq!(session.binds.len(), 1);
        let mut other = Writer::new();
        other.string(b"query");
        assert_eq!(Agent::extension(&other.buf, &mut session), [FAILURE]);
        assert_eq!(Agent::extension(b"", &mut session), [FAILURE]);
    }

    #[test]
    fn the_phones_key_list_is_read_leniently_but_checked() {
        let blob = {
            let mut w = Writer::new();
            w.string(b"ssh-ed25519").string(&[9; 32]);
            w.buf
        };
        let line = format!("ssh-ed25519 {} laptop", BASE64.encode(&blob));
        let fp = keys::fingerprint(&blob);
        let keys = parse_keys(&json!({"items": [
            {"id": "1", "title": "Deploy", "public_key": line, "fingerprint": fp},
            {"id": "2", "public_key": line},
            {"id": "3", "public_key": line, "fingerprint": "SHA256:other"},
            {"id": "4", "public_key": "garbage"},
        ]}));
        let names: Vec<&str> = keys.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["Deploy", "laptop"]);
        assert_eq!(keys[0].fingerprint, fp);
        assert!(parse_keys(&json!({"nothing": []})).is_empty());
    }

    #[test]
    fn the_socket_lives_in_the_state_directory_unless_it_is_the_usual_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert_eq!(socket_path(&paths, &SshConfig::default()), paths.state_dir.join(SOCKET_NAME));
        let set = SshConfig {
            socket: Some(PathBuf::from("/tmp/x.sock")),
            ..SshConfig::default()
        };
        assert_eq!(socket_path(&paths, &set), PathBuf::from("/tmp/x.sock"));
        assert!(
            SshConfig {
                socket: Some(PathBuf::from("rel.sock")),
                ..SshConfig::default()
            }
            .validate()
            .is_err()
        );
    }
}
