//! The workspace: where an agent's work is expected (01 §2.5). The desktop daemon builds it from its configuration and
//! what it finds on disk (08 R1); this module only classifies paths against it, lexically.

use super::model::Area;
use crate::shell::glob;

/// Folders that hold what a toolchain generates: deleting or writing them loses nothing a build can't make again.
const GENERATED: &[&str] = &[
    "target",
    "build",
    "node_modules",
    "DerivedData",
    ".gradle",
    "dist",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    "venv",
    ".build",
    ".next",
    ".turbo",
    ".nuxt",
    ".parcel-cache",
    ".svelte-kit",
    "coverage",
    ".cxx",
    ".externalNativeBuild",
    "Pods",
    ".dart_tool",
    "zig-cache",
    "zig-out",
    "_build",
];

/// A generated folder's name (`target`, `target-198`, `build-ios`).
fn generated(name: &str) -> bool {
    GENERATED.contains(&name)
        || ["target-", "target_", "build-", "build_"].iter().any(|p| name.starts_with(p) && name.len() > p.len())
}

/// Where work is expected on this computer. Entries may start with `~` and may hold `*` in a part
/// (`~/Documents/dev/reins/.claude/worktrees/*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The home folder (`/home/me`); empty when not known (then `~` stays as written).
    pub home: String,
    /// Configured roots (`~/Documents/dev`): their folders are projects.
    pub roots: Vec<String>,
    /// Scratch roots besides the OS temp folders and `~/.cache`.
    pub scratch: Vec<String>,
    /// Known git worktrees: their tracked files are recoverable from git.
    pub worktrees: Vec<String>,
    /// Paths the daemon found tracked by git (08 R2).
    pub tracked: Vec<String>,
}

impl Workspace {
    /// The defaults for `home`: projects in `~/Documents/dev`, `~/src`, `~/code` and `~/projects`.
    #[must_use]
    pub fn standard(home: &str) -> Self {
        Self {
            home: home.trim_end_matches('/').to_owned(),
            roots: ["~/Documents/dev", "~/src", "~/code", "~/projects"].map(str::to_owned).to_vec(),
            ..Self::default()
        }
    }

    fn expand(&self, entry: &str) -> String {
        normalize(entry, "/", &self.home)
    }

    /// The OS temp folders, `~/.cache` and the configured scratch roots.
    fn scratch_roots(&self) -> Vec<String> {
        let mut all = vec!["/tmp".to_owned(), "/var/tmp".to_owned(), "/dev/shm".to_owned()];
        if !self.home.is_empty() {
            all.push(format!("{}/.cache", self.home));
        }
        all.extend(self.scratch.iter().map(|s| self.expand(s)));
        all
    }

    /// The area of a normalized path (see [`normalize`]).
    #[must_use]
    pub fn area(&self, path: &str) -> Area {
        if !path.starts_with('/') {
            return Area::Unknown;
        }
        let path = literal_prefix(path);
        if path.split('/').any(|p| p == "..") {
            return Area::Unknown;
        }
        if self.scratch_roots().iter().any(|r| within(&path, r)) {
            return Area::Scratch;
        }
        // The folder of the project the path is in: a root's child, or a worktree itself (the deepest wins).
        let roots = self.roots.iter().filter_map(|r| depth_within(&path, &self.expand(r)).map(|d| d + 1));
        let trees = self.worktrees.iter().filter_map(|w| depth_within(&path, &self.expand(w)));
        if let Some(project) = roots.chain(trees).max() {
            // A generated folder inside the project (the project's own name never counts).
            let below: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).skip(project).collect();
            return if below.iter().any(|p| generated(p)) {
                Area::Generated
            } else {
                Area::Source
            };
        }
        if !self.home.is_empty() && within(&path, &self.home) {
            Area::Home
        } else {
            Area::System
        }
    }

    /// `path` is a worktree's root or a folder holding one: deleting it takes the history with it.
    #[must_use]
    pub fn holds_worktree(&self, path: &str) -> bool {
        path.starts_with('/') && self.worktrees.iter().any(|w| holds(path, &self.expand(w)))
    }

    /// A delete of `path` can be brought back from git: it is inside (or is) a known worktree, or the daemon reported
    /// it tracked, and it is not the repository's own `.git`.
    #[must_use]
    pub fn recoverable(&self, path: &str) -> bool {
        // The repository's own `.git`, or a pattern that names it (`.*`).
        if !path.starts_with('/')
            || path
                .split('/')
                .any(|p| p == ".git" || (p.starts_with('.') && p.contains(['*', '?', '[']) && glob(p, ".git")))
        {
            return false;
        }
        let path = literal_prefix(path);
        self.worktrees.iter().chain(&self.tracked).any(|w| within(&path, &self.expand(w)))
    }
}

/// The part of a path before any glob (`/a/b/*/c` → `/a/b`), unless the globs are only in parts below a literal
/// generated name, which decide nothing about the area: then the whole path, globs kept.
fn literal_prefix(path: &str) -> String {
    if !path.contains(['*', '?', '[']) {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let first_glob = parts.iter().position(|p| p.contains(['*', '?', '['])).unwrap_or(parts.len());
    // Keep the whole pattern: a part is matched as a glob against the roots, and generated names after it still
    // count. `area` only reads it through `within`/`depth_within`, which match parts as globs one by one.
    if first_glob >= 2 {
        path.to_owned()
    } else {
        "/".to_owned()
    }
}

/// How many parts of `path` the root `root` covers, when `path` is inside it or is it. Parts of `root` match as globs,
/// and a glob in `path` only matches a root part it is equal to (a pattern is inside a root only if all it can name
/// is).
fn depth_within(path: &str, root: &str) -> Option<usize> {
    let p: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let r: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    if r.len() > p.len() {
        return None;
    }
    let ok = r.iter().zip(&p).all(|(rp, pp)| {
        if pp.contains(['*', '?', '[']) {
            rp == pp
        } else {
            glob(rp, pp)
        }
    });
    ok.then_some(r.len())
}

fn within(path: &str, root: &str) -> bool {
    depth_within(path, root).is_some()
}

/// Whether `path` is `root` or a folder above it (`root`'s parts match as globs).
fn holds(path: &str, root: &str) -> bool {
    let p: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let r: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    p.len() <= r.len() && p.iter().zip(&r).all(|(a, b)| glob(b, a))
}

/// A path made absolute and lexical: `~` is `home`, a relative path is joined to `cwd`, `.` and empty parts dropped,
/// `..` resolved (never above `/`). A relative path with no `cwd` stays relative (its area is then unknown).
#[must_use]
pub fn normalize(path: &str, cwd: &str, home: &str) -> String {
    let path = path.trim();
    let joined = if path == "~" || path.starts_with("~/") {
        if home.is_empty() {
            return path.to_owned();
        }
        format!("{home}{}", &path[1..])
    } else if path.starts_with('/') {
        path.to_owned()
    } else if cwd.starts_with('/') {
        format!("{cwd}/{path}")
    } else {
        return path.to_owned();
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> Workspace {
        Workspace {
            scratch: vec!["~/Documents/dev/tmp".to_owned()],
            worktrees: vec!["~/Documents/dev/reins".to_owned(), "~/Documents/dev/reins/.claude/worktrees/*".to_owned()],
            ..Workspace::standard("/home/me")
        }
    }

    #[test]
    fn generated_scratch_source_home_system() {
        let w = ws();
        let area = |p: &str| w.area(&normalize(p, "/home/me/Documents/dev/reins", "/home/me"));
        assert_eq!(area("target/debug"), Area::Generated);
        assert_eq!(area("android/app/build/tmp"), Area::Generated);
        assert_eq!(area("target-198"), Area::Generated);
        assert_eq!(area("/tmp/x"), Area::Scratch);
        assert_eq!(area("~/.cache/yay"), Area::Scratch);
        assert_eq!(area("~/Documents/dev/tmp/run-1"), Area::Scratch);
        assert_eq!(area("crates/x.rs"), Area::Source);
        assert_eq!(area("~/Documents/dev/other-project"), Area::Source);
        assert_eq!(area("~/Documents/dev/build"), Area::Source, "a project named build is source");
        assert_eq!(area("~/laya-runs/smoke"), Area::Home);
        assert_eq!(area("/etc/sudoers.d/x"), Area::System);
        assert_eq!(area("~/Documents/dev/reins/.claude/worktrees/*/target"), Area::Generated);
        assert_eq!(w.area("relative/path"), Area::Unknown);
    }

    #[test]
    fn prefix_lookalikes_are_outside() {
        let w = ws();
        let area = |p: &str| w.area(&normalize(p, "/home/me", "/home/me"));
        assert_eq!(area("~/Documents/dev-evil/x"), Area::Home);
        assert_eq!(area("/tmpx/y"), Area::System);
        assert_eq!(area("/home/me/Documents/dev/../.ssh"), Area::Home);
        assert_eq!(area("~/Documents/dev/*/../../.ssh"), Area::Home);
        assert_eq!(area("~/*"), Area::Home, "a glob over home is home");
        assert_eq!(area("/*"), Area::System);
        assert_eq!(area("/home/me/.cache/../.ssh/id_rsa"), Area::Home);
    }

    #[test]
    fn worktree_paths_are_recoverable() {
        let w = ws();
        assert!(w.recoverable("/home/me/Documents/dev/reins/crates"));
        assert!(w.recoverable("/home/me/Documents/dev/reins/.claude/worktrees/agent-1/src"));
        assert!(!w.recoverable("/home/me/Documents/dev/reins/.git"));
        assert!(!w.recoverable("/home/me/Documents/dev/notes"));
        assert!(!w.recoverable("/home/me/Documents/dev"), "the parent of a worktree");
    }
}
