//! `reins run` through the daemon (Unix): the daemon, not the caller, decides who runs the command and runs it.
//!
//! The caller connects to the daemon's socket (`run.sock` in the state folder, which only this user can open), hands
//! over its stdin, stdout and stderr (`SCM_RIGHTS`) and sends a [`RunRequest`]: the command's arguments, its folder, its
//! environment, which environments of which projects it needs and how the variables are named. The daemon then
//!
//! - checks the caller: the same user (the socket's peer credentials) and the control token;
//! - decides the runner from the caller's process and its ancestors ([`crate::runner`]); what the caller says can only
//!   make it an agent;
//! - resolves the program to its absolute path, gets each environment from a lease it holds or from the phone (bound to
//!   that runner, folder, command and variables), and starts the command itself, in the caller's folder, with the
//!   caller's stdin and terminal; its output goes through the redactor unless a person asked for it raw (an agent
//!   cannot);
//! - passes on the signals the caller forwards, and answers with the exit code.
//!
//! No value is ever sent back on the socket: the values reach only the process the daemon starts.

use std::collections::BTreeMap;
use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, Interest};
use tokio::net::UnixStream;
use zeroize::Zeroizing;

use crate::leases::SecretsService;
use crate::run::{Var, assemble};
use crate::secrets::{EnvRef, SecretRef, VarRef};
use crate::secrets_client::LeaseRequest;

/// Received descriptors are close-on-exec at once where the kernel can do it (Linux); elsewhere right after.
#[cfg(any(target_os = "linux", target_os = "android"))]
const RECV_FLAGS: RecvFlags = RecvFlags::CMSG_CLOEXEC;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const RECV_FLAGS: RecvFlags = RecvFlags::empty();

/// The largest request.
const MAX_REQUEST: u32 = 1024 * 1024;
/// Frames on the socket after the request.
const EXIT: u8 = b'X';
const ERROR: u8 = b'E';
const SIGNAL: u8 = b'S';

/// The socket's path.
#[must_use]
pub fn socket_path(paths: &crate::config::Paths) -> PathBuf {
    paths.state_dir.join("run.sock")
}

/// An environment the command needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvAsk {
    pub project: String,
    pub env: String,
    /// `None`: every variable, set under their own names.
    pub fields: Option<Vec<String>>,
    pub lease_secs: u32,
    pub confirm_always: bool,
}

/// Where one variable of the command comes from.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunSource {
    /// A literal of `.env.reins` (not a secret).
    Literal {
        value: String,
    },
    /// A variable of an environment asked for.
    Var {
        project: String,
        env: String,
        name: String,
    },
    /// A `vault:Item/field` value the caller got from the phone itself, and how to redact it.
    Given {
        value: String,
        label: String,
    },
}

impl std::fmt::Debug for RunSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal {
                ..
            } => f.write_str("Literal"),
            Self::Var {
                project,
                env,
                name,
            } => write!(f, "Var({project}/{env}/{name})"),
            Self::Given {
                label,
                ..
            } => write!(f, "Given({label})"),
        }
    }
}

impl Drop for RunSource {
    fn drop(&mut self) {
        if let Self::Given {
            value,
            ..
        } = self
        {
            zeroize::Zeroize::zeroize(value);
        }
    }
}

/// What the caller asks the daemon to run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRequest {
    /// The control token (`control.token`).
    pub token: String,
    /// The program as typed, then its arguments.
    pub argv: Vec<String>,
    /// Where it runs (absolute).
    pub cwd: String,
    /// The project's folder (canonical), which `cwd` is in.
    pub root: String,
    /// The caller's environment, passed on to the command (and read for markers that make it an agent).
    pub env: Vec<(String, String)>,
    pub asks: Vec<EnvAsk>,
    /// The variables named one by one (`.env.reins`, profiles, `--env`).
    pub vars: Vec<(String, RunSource)>,
    /// The whole environments (every variable under its own name), among `asks`.
    pub whole: Vec<(String, String)>,
    /// A person asked for the output raw.
    pub no_redact: bool,
    pub purpose: Option<String>,
}

// ---- the caller -------------------------------------------------------------------------------------------------

/// Hands this process's stdin, stdout and stderr to the daemon with `req`, passes on SIGINT, SIGTERM and SIGHUP, and
/// returns the command's exit code (`Err`: what went wrong before it ran).
pub async fn run(socket: &Path, req: &RunRequest) -> Result<u8, String> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("the Reins daemon is not running ({e}); `reins resume` starts it"))?;
    {
        let (stdin, stdout, stderr) = (std::io::stdin(), std::io::stdout(), std::io::stderr());
        let fds = [stdin.as_fd(), stdout.as_fd(), stderr.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        control.push(SendAncillaryMessage::ScmRights(&fds));
        stream.writable().await.map_err(|e| e.to_string())?;
        stream
            .try_io(Interest::WRITABLE, || {
                rustix::net::sendmsg(&stream, &[IoSlice::new(b"R")], &mut control, SendFlags::empty())
                    .map_err(std::io::Error::from)
            })
            .map_err(|e| format!("handing the terminal to the daemon: {e}"))?;
    }
    let body = Zeroizing::new(serde_json::to_vec(req).map_err(|e| e.to_string())?);
    let len = u32::try_from(body.len()).map_err(|_| "the request is too large".to_owned())?;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(&body).await.map_err(|e| e.to_string())?;
    let (mut reader, mut writer) = stream.into_split();
    let forward = async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let (Ok(mut int), Ok(mut term), Ok(mut hup)) =
                (signal(SignalKind::interrupt()), signal(SignalKind::terminate()), signal(SignalKind::hangup()))
            else {
                return std::future::pending::<()>().await;
            };
            loop {
                let sig: u8 = tokio::select! {
                    _ = int.recv() => 2,
                    _ = term.recv() => 15,
                    _ = hup.recv() => 1,
                };
                if writer.write_all(&[SIGNAL, sig]).await.is_err() {
                    return std::future::pending::<()>().await;
                }
            }
        }
    };
    let answer = async {
        let mut kind = [0u8; 1];
        reader.read_exact(&mut kind).await.map_err(|_| "the daemon ended the run".to_owned())?;
        match kind[0] {
            EXIT => {
                let mut code = [0u8; 1];
                reader.read_exact(&mut code).await.map_err(|e| e.to_string())?;
                Ok(code[0])
            }
            ERROR => {
                let mut len = [0u8; 4];
                reader.read_exact(&mut len).await.map_err(|e| e.to_string())?;
                let mut text = vec![0u8; usize::try_from(u32::from_be_bytes(len).min(4096)).unwrap_or(0)];
                reader.read_exact(&mut text).await.map_err(|e| e.to_string())?;
                Err(String::from_utf8_lossy(&text).into_owned())
            }
            _ => Err("unexpected answer from the daemon".to_owned()),
        }
    };
    tokio::select! {
        r = answer => r,
        () = forward => Err("signal forwarding stopped".to_owned()),
    }
}

// ---- the daemon -------------------------------------------------------------------------------------------------

/// Serves the run socket until the daemon stops.
pub async fn serve(
    listener: tokio::net::UnixListener,
    service: Arc<SecretsService>,
    token: Zeroizing<String>,
    info: crate::runner::ProcessReader,
) {
    let token = Arc::new(token);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            continue;
        };
        let (service, token) = (Arc::clone(&service), Arc::clone(&token));
        tokio::spawn(async move {
            if let Err(e) = handle(stream, &service, &token, info).await {
                log::debug!("run socket: {e}");
            }
        });
    }
}

/// Binds the socket (only this user can reach it: the state folder is 0700, and peers are checked).
pub fn bind(paths: &crate::config::Paths) -> Result<tokio::net::UnixListener, String> {
    let path = socket_path(paths);
    std::fs::remove_file(&path).ok();
    let listener = tokio::net::UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
    }
    Ok(listener)
}

/// The caller's stdin, stdout and stderr.
struct Stdio {
    stdin: OwnedFd,
    stdout: OwnedFd,
    stderr: OwnedFd,
}

async fn receive_fds(stream: &UnixStream) -> Result<Stdio, String> {
    let mut byte = [0u8; 1];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    stream.readable().await.map_err(|e| e.to_string())?;
    let got = stream
        .try_io(Interest::READABLE, || {
            rustix::net::recvmsg(stream, &mut [IoSliceMut::new(&mut byte)], &mut control, RECV_FLAGS)
                .map_err(std::io::Error::from)
        })
        .map_err(|e| e.to_string())?;
    if got.bytes != 1 || byte[0] != b'R' {
        return Err("no terminal handed over".to_owned());
    }
    let mut fds: Vec<OwnedFd> = Vec::new();
    for msg in control.drain() {
        if let RecvAncillaryMessage::ScmRights(received) = msg {
            fds.extend(received);
        }
    }
    // Where the kernel cannot mark them on receipt, they are marked now: none leaks into what the daemon starts.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    for fd in &fds {
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::CLOEXEC).map_err(|e| e.to_string())?;
    }
    let mut fds = fds.into_iter();
    match (fds.next(), fds.next(), fds.next(), fds.next()) {
        (Some(stdin), Some(stdout), Some(stderr), None) => Ok(Stdio {
            stdin,
            stdout,
            stderr,
        }),
        _ => Err("expected three file descriptors".to_owned()),
    }
}

async fn send_error(stream: &mut UnixStream, message: &str) {
    let text = message.as_bytes();
    let len = u32::try_from(text.len().min(4096)).unwrap_or(0);
    let mut frame = vec![ERROR];
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&text[..len as usize]);
    stream.write_all(&frame).await.ok();
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Checks a request; the folder it runs in.
fn checked(req: &RunRequest) -> Result<PathBuf, String> {
    if req.argv.is_empty() || req.argv[0].is_empty() {
        return Err("no command".to_owned());
    }
    if req.argv.iter().any(|a| a.contains('\0'))
        || req.env.iter().any(|(k, v)| k.contains(['=', '\0']) || v.contains('\0'))
    {
        return Err("a NUL character in the command or its environment".to_owned());
    }
    let cwd = PathBuf::from(&req.cwd);
    let root = PathBuf::from(&req.root);
    if !cwd.is_absolute() || !root.is_absolute() || !cwd.is_dir() {
        return Err("the folder must be an existing absolute path".to_owned());
    }
    let canonical = std::fs::canonicalize(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if canonical != root {
        return Err("the project's folder must be given canonical".to_owned());
    }
    let cwd = std::fs::canonicalize(&cwd).map_err(|e| format!("{}: {e}", cwd.display()))?;
    if !cwd.starts_with(&root) {
        return Err("the command runs inside the project's folder".to_owned());
    }
    Ok(cwd)
}

/// The runner of the peer `pid`: its ancestry, and what it and the request say (only towards an agent).
fn runner_for(pid: Option<i32>, req: &RunRequest, info: crate::runner::ProcessReader) -> String {
    let mut env: BTreeMap<String, String> = req.env.iter().cloned().collect();
    let Some(pid) = pid.and_then(|p| u32::try_from(p).ok()) else {
        // A caller whose process cannot be named is judged by what it says only, and never as a person unless it
        // says nothing at all.
        return crate::runner::claimed(&env).unwrap_or_else(|| reins_proto::desktop::PERSON.to_owned());
    };
    for (k, v) in crate::runner::process_env(pid) {
        env.entry(k).or_insert(v);
    }
    crate::runner::runner_of(pid, &env, &info)
}

async fn handle(
    mut stream: UnixStream,
    service: &SecretsService,
    token: &str,
    info: crate::runner::ProcessReader,
) -> Result<(), String> {
    let cred = stream.peer_cred().map_err(|e| e.to_string())?;
    if cred.uid() != rustix::process::getuid().as_raw() {
        return Err("another user".to_owned());
    }
    let stdio = receive_fds(&stream).await?;
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len);
    if len > MAX_REQUEST {
        send_error(&mut stream, "the request is too large").await;
        return Ok(());
    }
    let mut body = Zeroizing::new(vec![0u8; len as usize]);
    stream.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    let Ok(req) = serde_json::from_slice::<RunRequest>(&body) else {
        send_error(&mut stream, "the request is not a RunRequest").await;
        return Ok(());
    };
    drop(body);
    if !same(req.token.as_bytes(), token.as_bytes()) {
        send_error(&mut stream, "the control token does not match").await;
        return Ok(());
    }
    let runner = runner_for(cred.pid(), &req, info);
    match prepare(&req, &runner, service, &stdio).await {
        Ok(prepared) => run_child(stream, stdio, prepared).await,
        Err(e) => {
            send_error(&mut stream, &e).await;
            Ok(())
        }
    }
}

/// What to start.
struct Prepared {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    vars: Vec<Var>,
    redactor: Option<Arc<crate::redact::Redactor>>,
}

/// Where a waiting line goes: the caller's stderr.
fn teller(stderr: &OwnedFd) -> Option<crate::phone::Tell> {
    let file = std::fs::File::from(stderr.try_clone().ok()?);
    let file = Mutex::new(file);
    Some(crate::phone::teller(move |line: &str| {
        use std::io::Write as _;
        let mut f = file.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        writeln!(f, "{line}").ok();
    }))
}

async fn prepare(req: &RunRequest, runner: &str, service: &SecretsService, stdio: &Stdio) -> Result<Prepared, String> {
    let cwd = checked(req)?;
    let agent = runner != reins_proto::desktop::PERSON;
    if agent && req.no_redact {
        return Err(
            "--no-redact is for people at a terminal; for an agent the secrets stay out of the output".to_owned()
        );
    }
    let path_var = req.env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone());
    // What the phone is shown and what is started come from this one argument list (`req.argv`), never re-joined.
    let program_names = crate::secrets_client::program_names(
        &req.argv[0],
        &cwd,
        path_var.as_deref(),
        std::env::var("PATH").ok().as_deref(),
    );
    if !req.asks.is_empty() && !program_names.found() {
        return Err(format!("cannot find {}", reins_proto::desktop::shell_word(&req.argv[0])));
    }
    let program = program_names.program.clone();
    let args: Vec<String> = req.argv[1..].to_vec();
    let exec_env = crate::secrets_client::exec_env_digest(&req.env);
    let mut released: Vec<(VarRef, Zeroizing<String>)> = Vec::new();
    let mut under_pattern = false;
    for ask in &req.asks {
        let lease = LeaseRequest {
            project: ask.project.clone(),
            env: ask.env.clone(),
            fields: ask.fields.clone(),
            program: program.clone(),
            args: args.clone(),
            root: req.root.clone(),
            runner: runner.to_owned(),
            lease_secs: ask.lease_secs,
            purpose: req.purpose.clone(),
            confirm_always: ask.confirm_always,
            print: false,
            typed: Some(program_names.typed.clone()),
            aliases: program_names.aliases.clone(),
            path_check: program_names.path_check.clone(),
            exec_env: exec_env.clone(),
        };
        let answer = service.lease(&lease, teller(&stdio.stderr)).await.map_err(|r| r.message().to_owned())?;
        under_pattern |= answer.prefix;
        let env = EnvRef::new(&ask.project, &ask.env)?;
        for (name, value) in &answer.values {
            released.push((env.var(name), Zeroizing::new(value.clone())));
        }
    }
    let whole: Vec<EnvRef> = req.whole.iter().map(|(p, e)| EnvRef::new(p, e)).collect::<Result<_, _>>()?;
    let mut named: Vec<(String, crate::run::Source)> = Vec::new();
    let mut given: Vec<(SecretRef, Zeroizing<String>)> = Vec::new();
    for (name, source) in &req.vars {
        crate::secrets::check_var(name)?;
        named.push((
            name.clone(),
            match source {
                RunSource::Literal {
                    value,
                } => crate::run::Source::Literal(value.clone()),
                RunSource::Var {
                    project,
                    env,
                    name: var,
                } => crate::run::Source::Var(EnvRef::new(project, env)?.var(var)),
                RunSource::Given {
                    value,
                    label,
                } => {
                    let r = SecretRef::parse(label)?;
                    given.push((r.clone(), Zeroizing::new(value.clone())));
                    crate::run::Source::Field(r)
                }
            },
        ));
    }
    let vars = assemble(&whole, &released, &named, &given).map_err(|r| r.message().to_owned())?;
    let mut env: Vec<(String, String)> = req.env.iter().filter(|(k, _)| k != "REINS_RUNNER").cloned().collect();
    if under_pattern {
        // Released under a remembered pattern, not for exactly this command: what would change the program it runs is
        // not passed on (the lease is bound to `PATH` as it was).
        let gone = crate::secrets_client::strip_exec_env(&mut env);
        if !gone.is_empty()
            && let Some(tell) = teller(&stdio.stderr)
        {
            tell(&format!("reins: not passed on under a remembered pattern: {}", gone.join(", ")));
        }
    }
    if agent {
        // What runs for an agent stays an agent's, also in a `reins run` inside it.
        env.push(("REINS_RUNNER".to_owned(), runner.to_owned()));
    }
    // The filter is built before anything starts: a run that asked for it never runs without it.
    let redactor = if agent || !req.no_redact {
        let secrets: Vec<(String, &str)> =
            vars.iter().filter_map(|v| v.label.as_ref().map(|l| (l.clone(), v.value.as_str()))).collect();
        Some(Arc::new(crate::redact::Redactor::new(&secrets).map_err(|e| e.to_string())?)).filter(|r| !r.is_empty())
    } else {
        None
    };
    Ok(Prepared {
        program,
        args,
        cwd,
        env,
        vars,
        redactor,
    })
}

async fn run_child(mut stream: UnixStream, stdio: Stdio, p: Prepared) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&p.program);
    cmd.args(&p.args).current_dir(&p.cwd).env_clear().envs(p.env.iter().map(|(k, v)| (k, v))).process_group(0);
    for v in &p.vars {
        cmd.env(&v.name, v.value.as_str());
    }
    cmd.stdin(std::process::Stdio::from(stdio.stdin));
    let redactor = p.redactor.clone();
    let (out_fd, err_fd) = (stdio.stdout, stdio.stderr);
    let sinks = if redactor.is_some() {
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        Some((out_fd, err_fd))
    } else {
        cmd.stdout(std::process::Stdio::from(out_fd)).stderr(std::process::Stdio::from(err_fd));
        None
    };
    let spawned = cmd.spawn();
    drop(cmd);
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                crate::run::EXIT_NOT_FOUND
            } else {
                crate::run::EXIT_CANNOT_RUN
            };
            if let Some((_, err)) = sinks {
                let mut f = tokio::fs::File::from_std(std::fs::File::from(err));
                f.write_all(format!("reins: cannot run {}: {e}\n", p.program).as_bytes()).await.ok();
            }
            stream.write_all(&[EXIT, code]).await.ok();
            return Ok(());
        }
    };
    drop(p.vars);
    let pumps = match (redactor, sinks, child.stdout.take(), child.stderr.take()) {
        (Some(r), Some((out, err)), Some(o), Some(e)) => vec![
            tokio::spawn(crate::run::pump(o, tokio::fs::File::from_std(std::fs::File::from(out)), Arc::clone(&r))),
            tokio::spawn(crate::run::pump(e, tokio::fs::File::from_std(std::fs::File::from(err)), r)),
        ],
        _ => Vec::new(),
    };
    let pid = child.id().and_then(|p| rustix::process::Pid::from_raw(i32::try_from(p).ok()?));
    let (mut reader, mut writer) = stream.into_split();
    let status = loop {
        let mut frame = [0u8; 2];
        tokio::select! {
            s = child.wait() => break s,
            r = reader.read_exact(&mut frame) => {
                let sig = match r {
                    // The caller went away: what it started goes too.
                    Err(_) => Some(rustix::process::Signal::HUP),
                    Ok(_) if frame[0] == SIGNAL => match frame[1] {
                        2 => Some(rustix::process::Signal::INT),
                        15 => Some(rustix::process::Signal::TERM),
                        1 => Some(rustix::process::Signal::HUP),
                        _ => None,
                    },
                    Ok(_) => None,
                };
                if let (Some(sig), Some(pid)) = (sig, pid) {
                    rustix::process::kill_process_group(pid, sig).ok();
                }
                if r.is_err() {
                    break child.wait().await;
                }
            }
        }
    };
    for task in pumps {
        task.await.ok();
    }
    let code = match status {
        Ok(s) => crate::run::exit_code(s),
        Err(_) => crate::run::EXIT_FAILED,
    };
    writer.write_all(&[EXIT, code]).await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(dir: &Path) -> RunRequest {
        let root = std::fs::canonicalize(dir).unwrap();
        RunRequest {
            token: "t".into(),
            argv: vec!["true".into()],
            cwd: root.to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
            env: vec![],
            asks: vec![],
            vars: vec![],
            whole: vec![],
            no_redact: false,
            purpose: None,
        }
    }

    #[test]
    fn requests_run_inside_their_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let ok = req(dir.path());
        assert!(checked(&ok).is_ok());
        let mut outside = req(dir.path());
        outside.cwd = "/".into();
        assert!(checked(&outside).is_err());
        let mut relative = req(dir.path());
        relative.root = "rel".into();
        assert!(checked(&relative).is_err());
        let mut nul = req(dir.path());
        nul.argv.push("a\0b".into());
        assert!(checked(&nul).is_err());
        let mut empty = req(dir.path());
        empty.argv.clear();
        assert!(checked(&empty).is_err());
    }

    #[test]
    fn what_the_request_says_can_only_make_the_caller_an_agent() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = req(dir.path());
        let alone = |pid: u32| {
            Some(crate::runner::ProcessInfo {
                parent: None,
                program: format!("/bin/p{pid}"),
                args: vec![],
            })
        };
        let pid = Some(4242);
        r.env = vec![("REINS_RUNNER".into(), "person".into())];
        assert_eq!(runner_for(pid, &r, alone), "person");
        r.env = vec![("REINS_RUNNER".into(), "agent:codex".into())];
        assert_eq!(runner_for(pid, &r, alone), "agent:codex");
        assert_eq!(runner_for(None, &r, alone), "agent:codex");
        let harness = |_: u32| {
            Some(crate::runner::ProcessInfo {
                parent: None,
                program: "/usr/bin/claude".into(),
                args: vec![],
            })
        };
        r.env = vec![("REINS_RUNNER".into(), "person".into())];
        assert_eq!(runner_for(pid, &r, harness), "agent:claude-code", "the process decides, not what it says");
        assert!(
            !format!(
                "{:?}",
                RunSource::Given {
                    value: "s3cret".into(),
                    label: "vault:a/b".into()
                }
            )
            .contains("s3cret")
        );
    }
}
