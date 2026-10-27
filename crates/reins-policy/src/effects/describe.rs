//! Effects in plain words, one line each: what a person reads in Activity, never the raw command.

use super::model::{Area, Channel, CodeSource, Effect, FileKind, InfraChange, NetMethod, PathRef};

fn area_words(area: &Area) -> String {
    match area {
        Area::Generated => "build output".to_owned(),
        Area::Scratch => "scratch space".to_owned(),
        Area::Source => "your projects".to_owned(),
        Area::Home => "your home folder, outside your projects".to_owned(),
        Area::System => "system folders".to_owned(),
        Area::Remote(host) => host.clone(),
        Area::Unknown => "a place that couldn't be worked out".to_owned(),
    }
}

/// The last two parts of a path, for a line.
fn short(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => path.to_owned(),
        1 => parts[0].to_owned(),
        n => format!("{}/{}", parts[n - 2], parts[n - 1]),
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// Paths grouped by area, in order of first appearance.
fn by_area<'a>(paths: impl IntoIterator<Item = &'a PathRef>) -> Vec<(Area, Vec<&'a PathRef>)> {
    let mut groups: Vec<(Area, Vec<&PathRef>)> = Vec::new();
    for p in paths {
        match groups.iter_mut().find(|(a, _)| *a == p.area) {
            Some((_, v)) => {
                if !v.iter().any(|q| q.path == p.path) {
                    v.push(p);
                }
            }
            None => groups.push((p.area.clone(), vec![p])),
        }
    }
    groups
}

fn examples(paths: &[&PathRef]) -> String {
    let mut names: Vec<String> = paths.iter().take(2).map(|p| short(&p.path)).collect();
    if paths.len() > 2 {
        names.push("…".to_owned());
    }
    names.join(", ")
}

/// Lines for `effects`, at most `max` (the last says how many more).
#[must_use]
pub fn lines(effects: &[Effect], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |line: String| {
        if !out.contains(&line) {
            out.push(line);
        }
    };
    // Files: one line per (verb, area).
    let reads: Vec<&PathRef> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileRead {
                path,
                kind: FileKind::Other,
            } => Some(path),
            _ => None,
        })
        .collect();
    let writes: Vec<&PathRef> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileWrite {
                path,
            } => Some(path),
            _ => None,
        })
        .collect();
    let deletes: Vec<&PathRef> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::FileDelete {
                paths,
                ..
            } => Some(paths.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    for (area, paths) in by_area(deletes) {
        push(format!("Deletes {} in {} ({})", plural(paths.len(), "path"), area_words(&area), examples(&paths)));
    }
    for e in effects {
        if let Effect::FileRead {
            path,
            kind: kind @ (FileKind::Secret(_) | FileKind::Env),
        } = e
        {
            let what = if *kind == FileKind::Env {
                "a .env file"
            } else {
                "a secret"
            };
            push(format!("Reads {what}: {}", short(&path.path)));
        }
    }
    for (area, paths) in by_area(writes) {
        push(format!("Writes {} in {} ({})", plural(paths.len(), "file"), area_words(&area), examples(&paths)));
    }
    for e in effects {
        let line = match e {
            Effect::FileRead {
                ..
            }
            | Effect::FileWrite {
                ..
            }
            | Effect::FileDelete {
                ..
            } => continue,
            Effect::GitPush {
                repo,
                refname,
                default_branch,
                rewrites,
                commits,
            } => {
                let branch = refname.trim_start_matches("refs/heads/");
                let whose = match default_branch {
                    Some(true) => " (the default branch)",
                    None => " (maybe the default branch)",
                    Some(false) => "",
                };
                let how = match rewrites {
                    Some(true) => "Rewrites",
                    None => "Pushes (maybe rewriting)",
                    Some(false) => "Pushes to",
                };
                let commits = if *commits > 0 {
                    format!(", {}", plural(*commits as usize, "commit"))
                } else {
                    String::new()
                };
                let repo = if repo.is_empty() {
                    String::new()
                } else {
                    format!(" on {repo}")
                };
                format!("{how} {branch}{whose}{repo}{commits}")
            }
            Effect::GitRefDelete {
                repo,
                refname,
                default_branch,
            } => {
                let whose = if default_branch.unwrap_or(false) {
                    " (the default branch)"
                } else {
                    ""
                };
                let repo = if repo.is_empty() {
                    String::new()
                } else {
                    format!(" on {repo}")
                };
                format!("Deletes branch {}{whose}{repo}", refname.trim_start_matches("refs/heads/"))
            }
            Effect::Message {
                channel,
                to,
                attachments,
            } => {
                let how = match channel {
                    Channel::Email => "Emails".to_owned(),
                    Channel::Telegram => "Messages on Telegram".to_owned(),
                    Channel::Sms => "Texts".to_owned(),
                    Channel::Calendar => "Invites".to_owned(),
                    Channel::GitHub => "Posts on GitHub to the people of".to_owned(),
                    Channel::Other(s) => format!("Posts on {s} to"),
                };
                let who = match to.len() {
                    0 => "someone".to_owned(),
                    1 | 2 => to.join(", "),
                    n => format!("{} and {} more", to[..2].join(", "), n - 2),
                };
                let att = if *attachments > 0 {
                    format!(" with {}", plural(*attachments as usize, "attachment"))
                } else {
                    String::new()
                };
                format!("{how} {who}{att}")
            }
            Effect::Publish {
                target,
            } => format!("Publishes {target}"),
            Effect::Permission {
                resource,
                change,
            } => format!("Changes who can do what: {change} on {resource}"),
            Effect::Money {
                amount,
                merchant,
            } => format!(
                "Spends {}{}",
                amount.as_deref().unwrap_or("money"),
                merchant.as_deref().map(|m| format!(" at {m}")).unwrap_or_default()
            ),
            Effect::SecretOut {
                secret,
                destination,
            } => format!("Sends {secret} to {destination}"),
            Effect::SecretUse {
                secret,
                ..
            } => format!("Uses {secret} on this computer"),
            Effect::Network {
                host,
                method,
            } => match method {
                NetMethod::Read => format!("Fetches from {host}"),
                NetMethod::Write => format!("Sends data to {host}"),
                NetMethod::UploadExecutable => format!("Uploads a program to {host}"),
                NetMethod::Connect => format!("Connects to {host}"),
            },
            Effect::Infra {
                target,
                change,
                local,
            } => {
                let verb = match change {
                    InfraChange::Apply => "Changes",
                    InfraChange::Destroy => "Destroys",
                };
                let place = if *local {
                    " on this computer"
                } else {
                    ""
                };
                format!("{verb} {target}{place}")
            }
            Effect::Exec {
                program,
                source,
            } => match source {
                CodeSource::Downloaded => format!("Runs a program downloaded from {program}"),
                CodeSource::Generated => "Runs code made while it runs, unseen".to_owned(),
                CodeSource::Workspace => format!("Runs the project's {program}"),
                CodeSource::Toolchain => format!("Runs {program}"),
                CodeSource::Ci => format!("Starts {program}"),
                CodeSource::Unknown => format!("Runs {program}, which Reins has no model of"),
            },
            Effect::DataRead {
                service,
                resource,
            } => format!("Reads {resource} in {service}"),
            Effect::DataWrite {
                service,
                what,
                ..
            } => format!("{what} in {service}"),
            Effect::Unknown {
                reason,
            } => format!("Couldn't tell: {reason}"),
        };
        push(line);
    }
    if !reads.is_empty() {
        let groups = by_area(reads);
        let n: usize = groups.iter().map(|(_, v)| v.len()).sum();
        let outside: Vec<&(Area, Vec<&PathRef>)> = groups.iter().filter(|(a, _)| !a.in_workspace()).collect();
        if outside.is_empty() {
            push(format!("Reads {} in your projects", plural(n, "file")));
        } else {
            for (area, paths) in outside {
                push(format!("Reads {} in {} ({})", plural(paths.len(), "file"), area_words(area), examples(paths)));
            }
        }
    }
    if out.len() > max {
        let more = out.len() - (max - 1);
        out.truncate(max - 1);
        out.push(format!("and {more} more"));
    }
    out
}
