//! Repositories built with the real git CLI, packs made the way `git push` makes them, and a [`Remote`] backed by a
//! bare repository (what GitHub has before the push).

#![allow(dead_code, reason = "each test binary uses a part")]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use reins_desktop::git::Command;
use reins_desktop::git::object::{ObjectKind, Oid};
use reins_desktop::git::remote::{Remote, RemoteError};

pub const ZERO: &str = "0000000000000000000000000000000000000000";

/// Runs git in `dir` with a clean environment; panics with git's output on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(dir, args, None)).unwrap().trim_end().to_owned()
}

pub fn git_bytes(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Vec<u8> {
    let out = git_run(dir, args, stdin);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

pub fn git_run(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> std::process::Output {
    static CLOCK: AtomicU64 = AtomicU64::new(1_700_000_000);
    let date = format!("@{} +0000", CLOCK.fetch_add(60, Ordering::Relaxed));
    let mut child = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}

/// A working repository and the bare "server" it pushes to.
pub struct Fixture {
    _dir: tempfile::TempDir,
    pub work: PathBuf,
    pub server: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        let server = dir.path().join("server.git");
        git(dir.path(), &["init", "-q", "--bare", server.to_str().unwrap()]);
        git(dir.path(), &["init", "-q", work.to_str().unwrap()]);
        git(&work, &["remote", "add", "origin", server.to_str().unwrap()]);
        Self {
            _dir: dir,
            work,
            server,
        }
    }

    /// Writes (Some) or deletes (None) files, commits everything, returns the commit id.
    pub fn commit(&self, files: &[(&str, Option<&[u8]>)], message: &str) -> String {
        for (path, content) in files {
            let p = self.work.join(path);
            match content {
                Some(c) => {
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(&p, c).unwrap();
                }
                None => {
                    if p.is_dir() {
                        std::fs::remove_dir_all(&p).unwrap();
                    } else {
                        std::fs::remove_file(&p).unwrap();
                    }
                }
            }
        }
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "--allow-empty", "-m", message]);
        self.head()
    }

    pub fn head(&self) -> String {
        git(&self.work, &["rev-parse", "HEAD"])
    }

    pub fn rev(&self, rev: &str) -> String {
        git(&self.work, &["rev-parse", rev])
    }

    /// Updates the server for real (what it had before the push being described).
    pub fn publish(&self, refspec: &str) {
        git(&self.work, &["push", "-q", "--force", "origin", refspec]);
    }

    /// The pack `git push` sends: `new`, minus what the server has, thin, OFS deltas.
    pub fn push_pack(&self, new: &str, have: &[&str]) -> tempfile::NamedTempFile {
        self.pack(new, have, &["--thin", "--delta-base-offset"])
    }

    pub fn pack(&self, new: &str, have: &[&str], options: &[&str]) -> tempfile::NamedTempFile {
        let mut input = format!("{new}\n");
        for h in have {
            writeln!(input, "^{h}").unwrap();
        }
        let mut args = vec!["pack-objects", "-q", "--stdout", "--revs"];
        args.extend_from_slice(options);
        let bytes = git_bytes(&self.work, &args, Some(input.as_bytes()));
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&bytes).unwrap();
        f
    }

    pub fn remote(&self) -> GitRemote {
        GitRemote {
            dir: self.server.clone(),
        }
    }
}

pub fn cmd(old: &str, new: &str, name: &str) -> Command {
    Command {
        old: old.to_owned(),
        new: new.to_owned(),
        name: name.to_owned(),
    }
}

pub fn oid(hex: &str) -> Oid {
    Oid::from_hex(hex).unwrap()
}

/// What a full git server can tell: any object raw, and ancestry.
pub struct GitRemote {
    pub dir: PathBuf,
}

#[async_trait::async_trait]
impl Remote for GitRemote {
    async fn object(
        &self,
        oid: &Oid,
        kind: Option<ObjectKind>,
        max_len: u64,
    ) -> Result<Option<(ObjectKind, Vec<u8>)>, RemoteError> {
        let hex = oid.to_hex();
        let t = git_run(&self.dir, &["cat-file", "-t", &hex], None);
        if !t.status.success() {
            return Ok(None);
        }
        let found = ObjectKind::from_name(String::from_utf8_lossy(&t.stdout).trim()).unwrap();
        if kind.is_some_and(|k| k != found) {
            return Ok(None);
        }
        let data = git_bytes(&self.dir, &["cat-file", found.name(), &hex], None);
        if data.len() as u64 > max_len {
            return Err(RemoteError::TooLarge);
        }
        Ok(Some((found, data)))
    }

    async fn is_ancestor(&self, ancestor: &Oid, descendant: &Oid) -> Result<Option<bool>, RemoteError> {
        let out = git_run(&self.dir, &["merge-base", "--is-ancestor", &ancestor.to_hex(), &descendant.to_hex()], None);
        Ok(match out.status.code() {
            Some(0) => Some(true),
            Some(1) => Some(false),
            _ => None,
        })
    }
}

/// Lines of text: "line 0\n" … for big files edited slightly (so git makes deltas).
pub fn lines(n: usize, tag: &str) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..n {
        writeln!(s, "{tag} line {i} of a file large enough for git to store it as a delta").unwrap();
    }
    s.into_bytes()
}
