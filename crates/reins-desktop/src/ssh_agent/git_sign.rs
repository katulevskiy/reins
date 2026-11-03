//! `reins` as git's SSH signing program: `git config gpg.ssh.program <reins>` makes git run
//! `reins -Y sign -n git -f <key> [-U] <file>` for every signed commit and tag, as it would run `ssh-keygen`.
//!
//! When the key is one of the phone's (the daemon's SSH agent lists it), the whole object goes to the agent in a
//! [`GIT_SIGN_EXTENSION`](super::GIT_SIGN_EXTENSION) message, the phone shows the commit and signs it, and the
//! signature is written to `<file>.sig` in ssh-keygen's armor. Everything else (`-Y verify`, `find-principals`,
//! `check-novalidate`, a key that is not the phone's, the agent turned off) is handed to the real `ssh-keygen` with
//! the same arguments, so verifying and other keys work as before.
//!
//! `reins ssh git setup|unsetup|status` ([`setup`], [`unsetup`], [`status`]) sets git up for it in the global git
//! config, keeping what was there to put back.

use std::ffi::{OsStr, OsString};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use reins_proto::sshsig;
use serde::{Deserialize, Serialize};

use super::wire::{Reader, Writer};
use super::{GIT_SIGN_EXTENSION, keys};
use crate::config::{Config, Paths};

const EXTENSION: u8 = 27;
const SUCCESS: u8 = 6;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const MAX_REPLY: usize = 256 * 1024;
/// When nothing came back by then, the terminal is told the phone is asked.
const NOTICE_AFTER: Duration = Duration::from_millis(800);

/// Whether `reins` was started as a signing program (`reins -Y …`), before any subcommand parsing.
#[must_use]
pub fn is_signer(args: &[OsString]) -> bool {
    args.get(1).is_some_and(|a| a == "-Y")
}

/// `ssh-keygen -Y sign`'s arguments, as git passes them.
#[derive(Debug, Default, PartialEq, Eq)]
struct SignArgs {
    namespace: Option<String>,
    key_file: Option<PathBuf>,
    hash: Option<String>,
    files: Vec<PathBuf>,
}

/// `Some` when the arguments are a `-Y sign` this program does itself; `None`: hand them to ssh-keygen.
fn parse_sign(args: &[OsString]) -> Option<SignArgs> {
    let mut out = SignArgs::default();
    let mut op = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("-Y") => op = Some(it.next()?.to_str()?.to_owned()),
            Some("-n") => out.namespace = Some(it.next()?.to_str()?.to_owned()),
            Some("-f") => out.key_file = Some(PathBuf::from(it.next()?)),
            Some("-O") => {
                // Another option changes what ssh-keygen does: leave it to ssh-keygen.
                let h = it.next()?.to_str()?.strip_prefix("hashalg=")?;
                out.hash = Some(h.to_owned());
            }
            Some("-U" | "-q" | "-v") => {}
            Some(s) if s.starts_with('-') && s.len() > 1 => return None,
            _ => out.files.push(PathBuf::from(a)),
        }
    }
    (op.as_deref() == Some("sign")).then_some(out)
}

/// The real ssh-keygen: `REINS_SSH_KEYGEN`, else Windows' own, else the one on `PATH`.
fn ssh_keygen() -> PathBuf {
    if let Some(p) = std::env::var_os("REINS_SSH_KEYGEN").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        let own = crate::win::system32("OpenSSH\\ssh-keygen.exe");
        if own.exists() {
            return own;
        }
    }
    PathBuf::from("ssh-keygen")
}

/// Runs ssh-keygen with `args` and the same stdin, stdout and stderr; its exit code.
fn hand_over(args: &[OsString]) -> ExitCode {
    match std::process::Command::new(ssh_keygen()).args(args).status() {
        Ok(s) => s.code().and_then(|c| u8::try_from(c).ok()).map_or(ExitCode::FAILURE, ExitCode::from),
        Err(e) => {
            eprintln!("reins: cannot run ssh-keygen: {e}");
            ExitCode::from(255)
        }
    }
}

/// The public key blob in `file` (an OpenSSH public key line), else in `file.pub` (git may name the private key).
fn key_blob(file: &Path) -> Option<Vec<u8>> {
    let read = |p: &Path| {
        let text = std::fs::read_to_string(p).ok()?;
        let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
        keys::parse_line(line.strip_prefix("key::").unwrap_or(line)).ok().map(|(blob, _)| blob)
    };
    read(file).or_else(|| {
        let mut public = file.as_os_str().to_owned();
        public.push(".pub");
        read(Path::new(&public))
    })
}

// ---- the agent ------------------------------------------------------------------------------------------------------

trait Duplex: std::io::Read + std::io::Write {}
impl<T: std::io::Read + std::io::Write> Duplex for T {}

fn connect(socket: &Path) -> std::io::Result<Box<dyn Duplex>> {
    #[cfg(unix)]
    {
        Ok(Box::new(std::os::unix::net::UnixStream::connect(socket)?))
    }
    #[cfg(not(unix))]
    {
        Ok(Box::new(std::fs::OpenOptions::new().read(true).write(true).open(socket)?))
    }
}

fn request(stream: &mut dyn Duplex, msg: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut w = Writer::new();
    w.string(msg);
    stream.write_all(&w.buf)?;
    stream.flush()?;
    let mut len = [0; 4];
    stream.read_exact(&mut len)?;
    let len = usize::try_from(u32::from_be_bytes(len)).unwrap_or(usize::MAX);
    if len == 0 || len > MAX_REPLY {
        return Err(std::io::Error::other("the agent's answer is malformed"));
    }
    let mut reply = vec![0; len];
    stream.read_exact(&mut reply)?;
    Ok(reply)
}

/// The keys the agent lists (blob, name).
fn identities(stream: &mut dyn Duplex) -> std::io::Result<Vec<(Vec<u8>, String)>> {
    let reply = request(stream, &[REQUEST_IDENTITIES])?;
    let mut r = Reader::new(&reply);
    let malformed = || std::io::Error::other("the agent's key list is malformed");
    if r.byte() != Some(IDENTITIES_ANSWER) {
        return Err(malformed());
    }
    let n = r.u32().ok_or_else(malformed)?;
    (0..n)
        .map(|_| {
            let blob = r.string().ok_or_else(malformed)?.to_vec();
            let name = String::from_utf8_lossy(r.string().ok_or_else(malformed)?).into_owned();
            Ok((blob, name))
        })
        .collect()
}

/// Where the daemon's agent listens (unless it is turned off), and its control token, which a git signing request
/// carries.
fn agent_socket() -> Option<(PathBuf, zeroize::Zeroizing<String>)> {
    let paths = Paths::from_env().ok()?;
    let config = Config::load(&paths).ok()?;
    let token = std::fs::read_to_string(paths.control_token_file()).ok()?;
    config
        .ssh
        .enabled
        .then(|| (super::socket_path(&paths, &config.ssh), zeroize::Zeroizing::new(token.trim().to_owned())))
}

/// What the computer says about the repository the signing is for: its `owner/name` (from `origin`), else its
/// folder's name; and the branch checked out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub repo: String,
    pub branch: String,
}

fn git_out(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// How a remote URL's repository is shown: `owner/name` on GitHub, else `host/path`
/// (`https://github.com/o/n.git`, `git@github.com:o/n`, Reins's proxy `http://127.0.0.1:7457/github.com/o/n`,
/// `ssh://git@gitlab.example.com:2222/group/sub/app.git` → `gitlab.example.com/group/sub/app`).
#[must_use]
pub fn repo_of_url(url: &str) -> Option<String> {
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?,
        // scp-like: [user@]host:path
        None => url.split_once(':')?,
    };
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    // Reins's git proxy names the real host first.
    let (host, path) = match host.as_str() {
        "127.0.0.1" | "localhost" | "[::1]" => {
            let (h, p) = path.split_once('/')?;
            (h.to_ascii_lowercase(), p)
        }
        _ => (host, path),
    };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || host.is_empty() {
        return None;
    }
    let path = parts.join("/");
    Some(if host == "github.com" {
        path
    } else {
        format!("{host}/{path}")
    })
}

fn place(tag: bool) -> Place {
    let repo = git_out(&["config", "--get", "remote.origin.url"])
        .and_then(|u| repo_of_url(&u))
        .or_else(|| {
            let top =
                git_out(&["rev-parse", "--show-toplevel"]).or_else(|| git_out(&["rev-parse", "--absolute-git-dir"]))?;
            Path::new(&top).file_name().map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let branch = if tag {
        String::new()
    } else {
        git_out(&["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default()
    };
    Place {
        repo,
        branch,
    }
}

/// Writes `line` to the terminal (git keeps the signing program's stderr until it fails). Unix only; nothing when
/// there is no terminal.
fn tell_terminal(line: &str) {
    #[cfg(unix)]
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        writeln!(tty, "{line}").ok();
    }
    #[cfg(not(unix))]
    let _ = line;
}

/// The phone's signature blob for `object`, or why not.
fn sign_object(
    stream: &mut dyn Duplex,
    token: &str,
    key: &[u8],
    namespace: &str,
    hash: sshsig::Hash,
    object: &[u8],
    place: &Place,
) -> Result<Vec<u8>, String> {
    let mut w = Writer::new();
    w.byte(EXTENSION)
        .string(GIT_SIGN_EXTENSION.as_bytes())
        .string(token.as_bytes())
        .string(key)
        .string(namespace.as_bytes())
        .string(hash.name().as_bytes())
        .string(object)
        .string(place.repo.as_bytes())
        .string(place.branch.as_bytes());
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let told = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let notice = {
        let (done, told) = (std::sync::Arc::clone(&done), std::sync::Arc::clone(&told));
        std::thread::spawn(move || {
            std::thread::sleep(NOTICE_AFTER);
            if !done.load(std::sync::atomic::Ordering::SeqCst) {
                told.store(true, std::sync::atomic::Ordering::SeqCst);
                tell_terminal("reins: approve signing on your phone…");
            }
        })
    };
    let reply = request(stream, &w.buf);
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(notice);
    let reply = reply.map_err(|e| format!("the Reins agent did not answer: {e}"))?;
    let mut r = Reader::new(&reply);
    let ok = r.byte() == Some(SUCCESS);
    let body = r.string().unwrap_or_default();
    if ok {
        if told.load(std::sync::atomic::Ordering::SeqCst) {
            tell_terminal("reins: signed on your phone.");
        }
        Ok(body.to_vec())
    } else {
        let why = String::from_utf8_lossy(body);
        Err(if why.is_empty() {
            "the Reins agent refused to sign".to_owned()
        } else {
            reins_proto::desktop::printable(&why)
        })
    }
}

/// `reins -Y …`: signs a git object on the phone, or hands the call to ssh-keygen.
#[must_use]
pub fn main(args: &[OsString]) -> ExitCode {
    let rest = args.get(1..).unwrap_or_default();
    let Some(sign) = parse_sign(rest) else {
        return hand_over(rest);
    };
    let (Some(namespace), Some(key_file)) = (sign.namespace.as_deref(), sign.key_file.as_deref()) else {
        return hand_over(rest);
    };
    let Some(blob) = key_blob(key_file) else {
        return hand_over(rest);
    };
    let Some((socket, token)) = agent_socket() else {
        return hand_over(rest);
    };
    let Ok(mut stream) = connect(&socket) else {
        eprintln!(
            "reins: the Reins SSH agent is not running at {} (`reins resume`); trying ssh-keygen",
            socket.display()
        );
        return hand_over(rest);
    };
    match identities(stream.as_mut()) {
        Ok(listed) if listed.iter().any(|(b, _)| *b == blob) => {}
        // Not one of the phone's keys: ssh-keygen signs with it as before.
        _ => return hand_over(rest),
    }
    if namespace != sshsig::GIT_NAMESPACE {
        eprintln!("reins: your phone signs git commits and tags only (namespace `git`), not `{namespace}`.");
        return ExitCode::from(255);
    }
    let hash = match sign.hash.as_deref().map(sshsig::Hash::parse) {
        None => sshsig::Hash::Sha512,
        Some(Some(h)) => h,
        Some(None) => {
            eprintln!("reins: the hash is sha512 or sha256.");
            return ExitCode::from(255);
        }
    };
    let one = |stream: &mut dyn Duplex, object: &[u8]| -> Result<String, String> {
        let parsed = sshsig::parse_object(object)
            .ok_or("your phone signs git commits and annotated tags only, and this is neither")?;
        let place = place(parsed.kind == sshsig::ObjectKind::Tag);
        let signature = sign_object(stream, &token, &blob, namespace, hash, object, &place)?;
        Ok(sshsig::armor(&blob, namespace, hash, &signature))
    };
    let stdin = sign.files.iter().any(|f| f.as_os_str() == OsStr::new("-"));
    if stdin && sign.files.len() > 1 {
        eprintln!("reins: sign either standard input (`-`) or files, not both.");
        return ExitCode::from(255);
    }
    if sign.files.is_empty() || stdin {
        let mut object = Vec::new();
        if let Err(e) = std::io::stdin().read_to_end(&mut object) {
            eprintln!("reins: cannot read what to sign: {e}");
            return ExitCode::from(255);
        }
        return match one(stream.as_mut(), &object) {
            Ok(armored) => {
                print!("{armored}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("reins: {e}");
                ExitCode::from(255)
            }
        };
    }
    for file in &sign.files {
        let result = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display())).and_then(|object| {
            let armored = one(stream.as_mut(), &object)?;
            let mut out = file.as_os_str().to_owned();
            out.push(".sig");
            std::fs::write(PathBuf::from(out), armored).map_err(|e| format!("cannot write the signature: {e}"))
        });
        if let Err(e) = result {
            eprintln!("reins: {e}");
            return ExitCode::from(255);
        }
    }
    ExitCode::SUCCESS
}

// ---- setting git up -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, clap::Subcommand)]
pub enum GitCmd {
    /// Make git sign commits and tags with a key on your phone (global git config; what was there is kept).
    Setup {
        /// The phone's key, by name or SHA256 fingerprint; needed when it has several.
        #[arg(long)]
        key: Option<String>,
        /// Only set the signing program and key; don't sign every commit and tag by default.
        #[arg(long)]
        not_default: bool,
    },
    /// Put back the git config `setup` changed.
    Unsetup,
    /// Whether git signs with Reins, and with which key.
    Status,
}

/// Which git config file: the global one, or one named (tests).
#[derive(Clone, Debug)]
pub struct GitConfig {
    pub file: Option<PathBuf>,
}

impl GitConfig {
    #[must_use]
    pub fn global() -> Self {
        Self {
            file: None,
        }
    }

    fn command(&self) -> std::process::Command {
        let mut c = std::process::Command::new("git");
        c.arg("config");
        match &self.file {
            Some(f) => c.arg("--file").arg(f),
            None => c.arg("--global"),
        };
        c.stdin(std::process::Stdio::null());
        c
    }

    /// The value of `key`, `None` when unset.
    pub fn get(&self, key: &str) -> Result<Option<String>, String> {
        let out = self.command().args(["--get", key]).output().map_err(|e| format!("cannot run git: {e}"))?;
        match out.status.code() {
            Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_owned())),
            // 1: not set.
            Some(1) => Ok(None),
            _ => Err(format!("git config --get {key}: {}", String::from_utf8_lossy(&out.stderr).trim())),
        }
    }

    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        let mut c = self.command();
        match value {
            Some(v) => c.args([key, v]),
            None => c.args(["--unset-all", key]),
        };
        let out = c.output().map_err(|e| format!("cannot run git: {e}"))?;
        // `--unset-all` of something not set exits 5.
        if out.status.success() || (value.is_none() && out.status.code() == Some(5)) {
            Ok(())
        } else {
            Err(format!("git config {key}: {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
}

/// What `setup` changed: each setting's value before (`None`: unset) and the value it set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    settings: Vec<(String, Option<String>, String)>,
}

fn saved_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("git-signing.json")
}

fn allowed_signers_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("allowed_signers")
}

/// The phone's keys as the running agent lists them.
pub fn phone_keys(socket: &Path) -> Result<Vec<(Vec<u8>, String)>, String> {
    let mut stream = connect(socket)
        .map_err(|e| format!("the Reins SSH agent is not running at {} (`reins resume`): {e}", socket.display()))?;
    identities(stream.as_mut()).map_err(|e| e.to_string())
}

fn key_line(blob: &[u8]) -> String {
    format!("{} {}", keys::key_type(blob).unwrap_or("ssh-ed25519"), data_encoding::BASE64.encode(blob))
}

/// Picks the key `wanted` names (name or fingerprint, any case), or the only one.
fn pick<'a>(listed: &'a [(Vec<u8>, String)], wanted: Option<&str>) -> Result<&'a (Vec<u8>, String), String> {
    let shown =
        || listed.iter().map(|(b, n)| format!("  {n} ({})", keys::fingerprint(b))).collect::<Vec<_>>().join("\n");
    match wanted {
        Some(w) => {
            let w = w.trim().trim_end_matches('=');
            listed
                .iter()
                .find(|(b, n)| keys::fingerprint(b) == w || n.eq_ignore_ascii_case(w))
                .ok_or_else(|| format!("no key on your phone is named {w}; it has:\n{}", shown()))
        }
        None => match listed {
            [] => Err("your phone has no SSH keys in its vault (add one in the Reins app)".to_owned()),
            [only] => Ok(only),
            _ => Err(format!("your phone has several SSH keys; pick one with --key:\n{}", shown())),
        },
    }
}

/// Sets git up to sign with `key` through `program`. Running it again changes the key or the program, and keeps the
/// values from before the first run.
pub fn setup(
    git: &GitConfig,
    paths: &Paths,
    program: &Path,
    key: &(Vec<u8>, String),
    sign_all: bool,
) -> Result<Vec<String>, String> {
    let line = key_line(&key.0);
    let mut wanted: Vec<(&str, String)> = vec![
        ("gpg.format", "ssh".to_owned()),
        ("gpg.ssh.program", program.to_str().ok_or("the path of reins is not UTF-8")?.to_owned()),
        ("user.signingkey", format!("key::{line}")),
    ];
    if sign_all {
        wanted.push(("commit.gpgsign", "true".to_owned()));
        wanted.push(("tag.gpgsign", "true".to_owned()));
    }
    // So that `git log --show-signature` checks these signatures: the user's email with this key.
    if git.get("gpg.ssh.allowedsignersfile")?.is_none() || saved_value(paths, "gpg.ssh.allowedsignersfile").is_some() {
        // An email that would break the line (or add one) is not written: anyone then.
        let email = git
            .get("user.email")?
            .filter(|e| {
                !e.is_empty() && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | ','))
            })
            .unwrap_or_else(|| "*".to_owned());
        let file = allowed_signers_file(paths);
        paths.ensure().map_err(|e| e.to_string())?;
        std::fs::write(&file, format!("{email} namespaces=\"git\" {line}\n"))
            .map_err(|e| format!("{}: {e}", file.display()))?;
        wanted.push(("gpg.ssh.allowedsignersfile", file.to_str().ok_or("the state folder is not UTF-8")?.to_owned()));
    }
    let mut saved = read_saved(paths)?;
    let mut out = Vec::new();
    for (k, v) in &wanted {
        let before = git.get(k)?;
        if !saved.settings.iter().any(|(s, ..)| s == k) {
            saved.settings.push(((*k).to_owned(), before.clone(), v.clone()));
        } else if let Some(entry) = saved.settings.iter_mut().find(|(s, ..)| s == k) {
            entry.2.clone_from(v);
        }
        if before.as_deref() != Some(v) {
            git.set(k, Some(v))?;
            out.push(format!("  {k} = {v}"));
        }
    }
    write_saved(paths, &saved)?;
    Ok(out)
}

fn saved_value(paths: &Paths, key: &str) -> Option<String> {
    read_saved(paths).ok()?.settings.into_iter().find(|(k, ..)| k == key).map(|(.., set)| set)
}

fn read_saved(paths: &Paths) -> Result<Saved, String> {
    match std::fs::read(saved_file(paths)) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", saved_file(paths).display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
        Err(e) => Err(format!("{}: {e}", saved_file(paths).display())),
    }
}

fn write_saved(paths: &Paths, saved: &Saved) -> Result<(), String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(saved).map_err(|e| e.to_string())?;
    crate::config::write_private(&saved_file(paths), &json).map_err(|e| e.to_string())
}

/// Puts back what `setup` changed; a setting changed since by someone else is left as it is. The lines to print.
pub fn unsetup(git: &GitConfig, paths: &Paths) -> Result<Vec<String>, String> {
    let saved = read_saved(paths)?;
    if saved.settings.is_empty() {
        return Ok(vec!["git is not set up to sign with Reins (nothing to put back).".to_owned()]);
    }
    let mut out = Vec::new();
    for (key, before, set) in &saved.settings {
        let now = git.get(key)?;
        if now.as_deref() != Some(set.as_str()) {
            out.push(format!("  {key}: changed since, left as it is"));
            continue;
        }
        git.set(key, before.as_deref())?;
        out.push(match before {
            Some(v) => format!("  {key} = {v} (as before)"),
            None => format!("  {key}: unset (as before)"),
        });
    }
    std::fs::remove_file(saved_file(paths)).ok();
    if git.get("gpg.ssh.allowedsignersfile")?.as_deref() != allowed_signers_file(paths).to_str() {
        std::fs::remove_file(allowed_signers_file(paths)).ok();
    }
    Ok(out)
}

/// Whether git signs through `program`, and with which key.
pub fn status(git: &GitConfig, program: &Path, listed: Option<&[(Vec<u8>, String)]>) -> Result<Vec<String>, String> {
    let ours = git.get("gpg.format")?.as_deref() == Some("ssh")
        && git.get("gpg.ssh.program")?.is_some_and(|p| Path::new(&p) == program);
    let mut out = Vec::new();
    if !ours {
        out.push("git signing: not through Reins (`reins ssh git setup`)".to_owned());
        return Ok(out);
    }
    let key = git.get("user.signingkey")?.unwrap_or_default();
    let blob = keys::parse_line(key.strip_prefix("key::").unwrap_or(&key)).ok().map(|(b, _)| b);
    let name = match (&blob, listed) {
        (Some(b), Some(l)) => l.iter().find(|(lb, _)| lb == b).map_or_else(
            || format!("{} (not one of your phone's keys)", keys::fingerprint(b)),
            |(_, n)| format!("{n} ({})", keys::fingerprint(b)),
        ),
        (Some(b), None) => keys::fingerprint(b),
        (None, _) => "a key file (not one of your phone's keys)".to_owned(),
    };
    out.push(format!("git signing: through Reins with {name}"));
    let all = git.get("commit.gpgsign")?.as_deref() == Some("true");
    out.push(format!(
        "Every commit signed: {}",
        if all {
            "yes"
        } else {
            "no (`git commit -S`)"
        }
    ));
    Ok(out)
}

/// The program git runs: this binary.
pub fn program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the reins program: {e}"))?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// Runs `reins ssh git …`; the lines to print.
pub fn run(cmd: &GitCmd, paths: &Paths, config: &Config) -> Result<Vec<String>, String> {
    let git = GitConfig::global();
    let socket = super::socket_path(paths, &config.ssh);
    match cmd {
        GitCmd::Setup {
            key,
            not_default,
        } => {
            if !config.ssh.enabled {
                return Err("the SSH agent is disabled in config.toml (`[ssh] enabled = false`)".to_owned());
            }
            let listed = phone_keys(&socket)?;
            let chosen = pick(&listed, key.as_deref())?;
            let mut out = vec![format!("git now signs with {} on your phone:", chosen.1)];
            let changes = setup(&git, paths, &program()?, chosen, !not_default)?;
            if changes.is_empty() {
                out = vec![format!("git already signs with {} on your phone.", chosen.1)];
            }
            out.extend(changes);
            out.push(format!(
                "Add the key to GitHub as a signing key for Verified commits: {}. `reins ssh git unsetup` puts the old \
                 settings back.",
                key_line(&chosen.0)
            ));
            Ok(out)
        }
        GitCmd::Unsetup => unsetup(&git, paths),
        GitCmd::Status => status(&git, &program()?, phone_keys(&socket).ok().as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn gits_sign_call_is_recognised_and_everything_else_goes_to_ssh_keygen() {
        assert!(is_signer(&os(&["reins", "-Y", "sign"])));
        assert!(!is_signer(&os(&["reins", "ssh", "status"])));
        let s = parse_sign(&os(&["-Y", "sign", "-n", "git", "-f", "/k.pub", "-U", "/tmp/buf"])).unwrap();
        assert_eq!(
            s,
            SignArgs {
                namespace: Some("git".to_owned()),
                key_file: Some(PathBuf::from("/k.pub")),
                hash: None,
                files: vec![PathBuf::from("/tmp/buf")],
            }
        );
        assert_eq!(
            parse_sign(&os(&["-Y", "sign", "-n", "git", "-f", "k", "-O", "hashalg=sha256"])).unwrap().hash.as_deref(),
            Some("sha256")
        );
        for other in [
            &["-Y", "verify", "-n", "git", "-f", "a", "-I", "me", "-s", "x"][..],
            &["-Y", "find-principals", "-f", "a", "-s", "x"],
            &["-Y", "check-novalidate", "-n", "git", "-s", "x"],
            &["-Y", "sign", "-n", "git", "-f", "k", "-O", "verify-time=1"],
            &["-Y", "sign", "-n", "git", "-f", "k", "-Z", "x"],
            &["-Y"],
        ] {
            assert!(parse_sign(&os(other)).is_none(), "{other:?}");
        }
    }

    #[test]
    fn repositories_are_named_from_their_remote() {
        for (url, repo) in [
            ("https://github.com/katulevskiy/reins.git", "katulevskiy/reins"),
            ("git@github.com:katulevskiy/reins.git", "katulevskiy/reins"),
            ("http://127.0.0.1:7457/github.com/katulevskiy/reins", "katulevskiy/reins"),
            ("ssh://git@gitlab.example.com:2222/group/sub/app.git/", "gitlab.example.com/group/sub/app"),
            ("git@evil.example:katulevskiy/reins.git", "evil.example/katulevskiy/reins"),
        ] {
            assert_eq!(repo_of_url(url).as_deref(), Some(repo), "{url}");
        }
        assert_eq!(repo_of_url("/srv/git/app.git"), None);
        assert_eq!(repo_of_url("https://example.com/"), None);
    }

    #[test]
    fn a_key_is_read_from_the_file_git_names_or_its_public_half() {
        let dir = tempfile::tempdir().unwrap();
        let mut blob = Writer::new();
        blob.string(b"ssh-ed25519").string(&[7; 32]);
        let line = key_line(&blob.buf);
        let public = dir.path().join("id.pub");
        std::fs::write(&public, format!("{line} me@pc\n")).unwrap();
        assert_eq!(key_blob(&public), Some(blob.buf.clone()));
        let private = dir.path().join("id");
        std::fs::write(&private, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        assert_eq!(key_blob(&private), Some(blob.buf.clone()));
        assert_eq!(key_blob(&dir.path().join("missing")), None);
    }

    fn git_available() -> bool {
        std::process::Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn setup_sets_git_up_and_unsetup_puts_everything_back() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let file = dir.path().join("gitconfig");
        std::fs::write(
            &file,
            "[user]\n\temail = me@example.com\n\tsigningkey = ~/.ssh/old.pub\n[gpg]\n\tformat = ssh\n[gpg \"ssh\"]\n\tprogram = /opt/1Password/op-ssh-sign\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&file).unwrap();
        let git = GitConfig {
            file: Some(file.clone()),
        };
        let mut blob = Writer::new();
        blob.string(b"ssh-ed25519").string(&[7; 32]);
        let key = (blob.buf.clone(), "Laptop".to_owned());
        let program = Path::new("/usr/bin/reins");
        let changed = setup(&git, &paths, program, &key, true).unwrap();
        assert_eq!(changed.len(), 5, "{changed:?}");
        assert_eq!(git.get("gpg.ssh.program").unwrap().as_deref(), Some("/usr/bin/reins"));
        assert_eq!(git.get("user.signingkey").unwrap(), Some(format!("key::{}", key_line(&key.0))));
        assert_eq!(git.get("commit.gpgsign").unwrap().as_deref(), Some("true"));
        let signers = std::fs::read_to_string(allowed_signers_file(&paths)).unwrap();
        assert_eq!(signers, format!("me@example.com namespaces=\"git\" {}\n", key_line(&key.0)));
        assert!(setup(&git, &paths, program, &key, true).unwrap().is_empty(), "twice changes nothing");
        let st = status(&git, program, Some(std::slice::from_ref(&key))).unwrap();
        assert!(st[0].contains("through Reins with Laptop"), "{st:?}");
        assert!(status(&git, Path::new("/other/reins"), None).unwrap()[0].contains("not through Reins"));

        // Someone changes one setting afterwards: that one stays.
        git.set("tag.gpgsign", Some("false")).unwrap();
        let lines = unsetup(&git, &paths).unwrap();
        assert!(lines.iter().any(|l| l.contains("tag.gpgsign: changed since")), "{lines:?}");
        assert_eq!(git.get("gpg.ssh.program").unwrap().as_deref(), Some("/opt/1Password/op-ssh-sign"));
        assert_eq!(git.get("user.signingkey").unwrap().as_deref(), Some("~/.ssh/old.pub"));
        assert_eq!(git.get("commit.gpgsign").unwrap(), None);
        assert_eq!(git.get("gpg.ssh.allowedsignersfile").unwrap(), None);
        assert!(!allowed_signers_file(&paths).exists());
        git.set("tag.gpgsign", None).unwrap();
        let after = std::fs::read_to_string(&file).unwrap();
        for line in before.lines().filter(|l| l.contains('=')) {
            assert!(after.contains(line), "{line} kept: {after}");
        }
        assert!(unsetup(&git, &paths).unwrap()[0].contains("nothing to put back"));
    }

    #[test]
    fn a_key_is_picked_by_name_or_fingerprint_or_alone() {
        let mut a = Writer::new();
        a.string(b"ssh-ed25519").string(&[1; 32]);
        let mut b = Writer::new();
        b.string(b"ssh-ed25519").string(&[2; 32]);
        let two = vec![(a.buf.clone(), "Laptop".to_owned()), (b.buf.clone(), "Deploy".to_owned())];
        assert!(pick(&two, None).unwrap_err().contains("several"));
        assert_eq!(pick(&two, Some("deploy")).unwrap().1, "Deploy");
        assert_eq!(pick(&two, Some(&keys::fingerprint(&a.buf))).unwrap().1, "Laptop");
        assert!(pick(&two, Some("nope")).is_err());
        assert_eq!(pick(&two[..1], None).unwrap().1, "Laptop");
        assert!(pick(&[], None).is_err());
    }
}
