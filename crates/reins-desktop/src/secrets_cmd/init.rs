//! `reins secrets init [NAME]`: writes `reins.toml` in this folder.

use std::path::Path;

use crate::project::{PROJECT_FILE, Project, ProjectFile};

/// A project name from a folder name: lower case, anything else than a letter or a digit becomes `-`
/// (`My App` → `my-app`).
pub(crate) fn name_from_folder(folder: &str) -> Option<String> {
    let mapped: String = folder
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let name = mapped.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let name: String = name.chars().take(64).collect::<String>().trim_end_matches('-').to_owned();
    crate::secrets::check_name("project", &name).ok().map(|()| name)
}

pub(super) fn run(dir: &Path, name: Option<&str>, default_env: &str) -> Result<Vec<String>, String> {
    let path = dir.join(PROJECT_FILE);
    if path.exists() {
        return Err(format!("{} exists already", path.display()));
    }
    let name = match name {
        Some(n) => n.to_owned(),
        None => dir
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(name_from_folder)
            .ok_or("give the project a name: reins secrets init NAME")?,
    };
    let file = ProjectFile::new(&name, default_env)?;
    let mut lines = Vec::new();
    if let Some(outer) = dir.parent().map(Project::find).transpose()?.flatten() {
        lines.push(format!(
            "Note: this folder is inside the project {} ({}); it becomes a project of its own.",
            outer.file.name,
            outer.root.display()
        ));
    }
    file.write(&path)?;
    lines.push(format!("Wrote {} (project {name}, default environment {default_env}).", path.display()));
    lines.push(format!("Next: reins secrets set {default_env}/NAME, or reins secrets push .env"));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_writes_a_project_once() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("My App");
        std::fs::create_dir(&repo).unwrap();
        let lines = run(&repo, None, "development").unwrap();
        assert!(lines[0].contains("project my-app"), "{lines:?}");
        let p = Project::find(&repo).unwrap().unwrap();
        assert_eq!((p.file.name.as_str(), p.file.default_env.as_str()), ("my-app", "development"));
        assert!(run(&repo, None, "development").unwrap_err().contains("exists"));
        let inner = repo.join("svc");
        std::fs::create_dir(&inner).unwrap();
        let lines = run(&inner, Some("svc"), "dev").unwrap();
        assert!(lines[0].starts_with("Note"), "{lines:?}");
        assert!(run(&dir.path().join("x"), Some("a/b"), "dev").is_err());
    }

    #[test]
    fn folder_names_become_project_names() {
        assert_eq!(name_from_folder("acme-web").as_deref(), Some("acme-web"));
        assert_eq!(name_from_folder(".dotfiles").as_deref(), Some("dotfiles"));
        assert_eq!(name_from_folder("My App").as_deref(), Some("my-app"));
        assert_eq!(name_from_folder("web_2.0").as_deref(), Some("web-2-0"));
        assert_eq!(name_from_folder("..."), None);
    }
}
