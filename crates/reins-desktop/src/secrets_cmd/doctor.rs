//! `reins secrets doctor`: checks a project. `reins.toml` and `.env.reins` read; every reference in `.env.reins` names
//! an environment and a variable that are in the vault (names from the phone, never a value); no plaintext `.env` file
//! left in the project; the guard keeps agents' tools off `.env` files and has hooks in an AI tool; the daemon runs.

use std::path::{Path, PathBuf};

use super::manage::{Backend, Here, find_env};
use crate::guard::GuardConfig;
use crate::project::{ENV_FILE, EnvFile, EnvValue, PROJECT_FILE};
use crate::secrets::Resolved;

/// What the checks need besides the project.
pub struct Facts {
    pub guard: GuardConfig,
    /// The AI tools with Reins' hooks in their settings.
    pub hooked: Vec<&'static str>,
    /// `Err`: why the daemon cannot be reached.
    pub daemon: Result<(), String>,
}

/// How deep the search for plaintext files goes, and how many entries it reads at most.
const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 20_000;
/// Folders not searched: dependencies and build output.
const SKIP: &[&str] =
    &[".git", "node_modules", "target", "vendor", "dist", "build", ".venv", "venv", "__pycache__", ".next"];

/// Whether a file name is a plaintext dotenv file (not `.env.reins`, nor a template).
#[must_use]
pub fn plaintext_name(name: &str) -> bool {
    (name == ".env" || name.starts_with(".env."))
        && !matches!(name, ".env.reins" | ".env.example" | ".env.sample" | ".env.template" | ".env.dist")
}

/// The plaintext dotenv files under `root`.
#[must_use]
pub fn plaintext(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen = 0;
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return found;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH && !SKIP.contains(&name.as_str()) {
                    stack.push((entry.path(), depth + 1));
                }
            } else if kind.is_file() && plaintext_name(&name) {
                found.push(entry.path());
            }
        }
    }
    found.sort();
    found
}

/// The checks, a line each: `ok` or `!` first.
pub async fn run(here: &Here, backend: Option<&dyn Backend>, facts: &Facts) -> Vec<String> {
    let mut lines = Vec::new();
    let mut ok = |good: bool, text: String| {
        lines.push(format!(
            "{}  {text}",
            if good {
                "ok"
            } else {
                "! "
            }
        ));
    };
    ok(true, format!("{PROJECT_FILE}: {} (default {})", here.project, here.default_env));
    let root = here.root.clone().unwrap_or_default();
    let env_file = root.join(ENV_FILE);
    let file = if env_file.is_file() {
        match EnvFile::load(&env_file) {
            Ok(f) => Some(f),
            Err(e) => {
                ok(false, e);
                None
            }
        }
    } else {
        ok(false, format!("no {ENV_FILE}: reins secrets pull writes one"));
        None
    };
    let names = match backend {
        Some(b) => Some(b.names(Some(&here.project)).await),
        None => None,
    };
    match &names {
        None => ok(false, "names not checked (--offline)".to_owned()),
        Some(Err(e)) => ok(false, format!("names not checked: {e}")),
        Some(Ok(listing)) => {
            if find_env(listing, &here.project, &here.default_env).is_none() {
                ok(
                    false,
                    format!("{}/{} is not in the vault: reins secrets push or set", here.project, here.default_env),
                );
            }
            let mut checked = 0;
            for v in file.iter().flat_map(EnvFile::vars) {
                let EnvValue::Reference(r) = &v.value else {
                    continue;
                };
                match r.resolve(Some(&here.project)) {
                    Ok(Resolved::Var(var)) => {
                        checked += 1;
                        match find_env(listing, &var.env.project, &var.env.env) {
                            Some(e) if e.variables.contains(&var.name) => {}
                            Some(_) => ok(
                                false,
                                format!("{ENV_FILE}:{} {}: {} has no {}", v.line, v.name, var.env.item(), var.name),
                            ),
                            None if var.env.project != here.project => {
                                ok(
                                    false,
                                    format!(
                                        "{ENV_FILE}:{} {}: {} not checked (another project)",
                                        v.line,
                                        v.name,
                                        var.env.item()
                                    ),
                                );
                            }
                            None => ok(
                                false,
                                format!("{ENV_FILE}:{} {}: no {} in the vault", v.line, v.name, var.env.item()),
                            ),
                        }
                    }
                    Ok(_) => {}
                    Err(e) => ok(false, format!("{ENV_FILE}:{} {}: {e}", v.line, v.name)),
                }
            }
            if file.is_some() {
                ok(true, format!("{ENV_FILE}: {checked} references checked"));
            }
        }
    }
    let left = if root.as_os_str().is_empty() {
        Vec::new()
    } else {
        plaintext(&root)
    };
    if left.is_empty() {
        ok(true, "no plaintext .env files".to_owned());
    }
    for p in &left {
        let shown = p.strip_prefix(&root).unwrap_or(p).display().to_string();
        ok(false, format!("{shown}: plaintext values (reins secrets push {shown}, then delete it)"));
    }
    let guarded = facts.guard.check_file(".env").is_some() && facts.guard.check_file(".env.local").is_some();
    ok(
        guarded,
        if guarded {
            "guard: .env files ask your phone".to_owned()
        } else {
            "guard: .env files are not protected ([guard] in config.toml)".to_owned()
        },
    );
    if facts.guard.check_file(ENV_FILE).is_some() {
        ok(false, format!("guard: {ENV_FILE} asks too (it holds no values; allow it)"));
    }
    if facts.hooked.is_empty() {
        ok(false, "no AI tool has Reins' hooks: reins setup".to_owned());
    } else {
        ok(true, format!("hooks: {}", facts.hooked.join(", ")));
    }
    match &facts.daemon {
        Ok(()) => ok(true, "daemon running".to_owned()),
        Err(e) => ok(false, e.clone()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets_cmd::manage::tests::fake;

    #[test]
    fn plaintext_files_are_found_but_not_templates_or_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for f in [".env", ".env.local", "api/.env.production", ".env.reins", ".env.example", "node_modules/x/.env"] {
            let p = d.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "A=1").unwrap();
        }
        let found: Vec<String> =
            plaintext(d).iter().map(|p| p.strip_prefix(d).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(found, [".env", ".env.local", "api/.env.production"]);
    }

    #[tokio::test]
    async fn doctor_checks_references_files_guard_and_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join(ENV_FILE),
            "API_KEY=env:development/API_KEY\nGONE=env:development/GONE\nX=env:staging/X\nLOG=debug\n",
        )
        .unwrap();
        std::fs::write(d.join(".env"), "API_KEY=sk-1").unwrap();
        let here = Here {
            project: "acme".into(),
            default_env: "development".into(),
            root: Some(d.to_path_buf()),
        };
        let facts = Facts {
            guard: GuardConfig::default(),
            hooked: vec!["Claude Code"],
            daemon: Err("the Reins daemon is not running".into()),
        };
        let f = fake();
        let lines = run(&here, Some(&f), &facts).await;
        let text = lines.join("\n");
        assert!(text.contains("!   .env.reins:2 GONE: acme/development has no GONE"), "{text}");
        assert!(text.contains("!   .env.reins:3 X: no acme/staging in the vault"), "{text}");
        assert!(text.contains("ok  .env.reins: 3 references checked"), "{text}");
        assert!(text.contains("!   .env: plaintext values"), "{text}");
        assert!(text.contains("ok  guard: .env files ask your phone"), "{text}");
        assert!(text.contains("ok  hooks: Claude Code"), "{text}");
        assert!(text.contains("!   the Reins daemon is not running"), "{text}");
        assert!(!text.contains("sk-1"));
        let offline = run(&here, None, &facts).await.join("\n");
        assert!(offline.contains("names not checked (--offline)"), "{offline}");
    }
}
