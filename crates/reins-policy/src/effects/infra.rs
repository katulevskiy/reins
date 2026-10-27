//! Databases, clouds, clusters, containers and services: `Infra` effects, local or not.

use super::level::local_host;
use super::model::{Certainty, CodeSource, Effect, InfraChange, NetMethod};
use super::shell::{Exp, Input, Walk};

fn value_of(args: &[Exp], names: &[&str]) -> Option<String> {
    for (k, a) in args.iter().enumerate() {
        for n in names {
            if a.text == *n {
                return args.get(k + 1).map(|v| v.text.clone());
            }
            if let Some(v) = a.text.strip_prefix(&format!("{n}=")) {
                return Some(v.to_owned());
            }
            // `-hdb.example`, `-c'…'`: attached short values.
            if n.len() == 2
                && n.starts_with('-')
                && !n.starts_with("--")
                && a.text.len() > 2
                && a.text.starts_with(n)
                && !a.text.starts_with("--")
            {
                return Some(a.text[2..].to_owned());
            }
        }
    }
    None
}

/// What SQL (or a store's commands) does: `Some(Destroy)` for drops, truncates and deletes, `Some(Apply)` for other
/// changes, `None` for reads.
#[must_use]
pub fn sql_change(sql: &str) -> Option<InfraChange> {
    // Strings and comments are text, not statements.
    let mut plain = String::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                }
                plain.push_str(" '' ");
            }
            '-' if chars.peek() == Some(&'-') => {
                for q in chars.by_ref() {
                    if q == '\n' {
                        break;
                    }
                }
                plain.push('\n');
            }
            c => plain.push(c),
        }
    }
    let upper = plain.to_ascii_uppercase();
    let words: Vec<&str> =
        upper.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    let pair = |a: &str, b: &[&str]| words.windows(2).any(|w| w[0] == a && b.contains(&w[1]));
    if pair(
        "DROP",
        &["TABLE", "DATABASE", "SCHEMA", "INDEX", "VIEW", "USER", "ROLE", "COLUMN", "OWNED", "COLLECTION", "KEYSPACE"],
    ) || words.contains(&"TRUNCATE")
        || pair("DELETE", &["FROM"])
        || words.iter().any(|w| matches!(*w, "FLUSHALL" | "FLUSHDB" | "DROPDATABASE"))
        || upper.contains(".DROP(")
        || upper.contains("DELETEMANY(")
    {
        return Some(InfraChange::Destroy);
    }
    let changes = [
        "INSERT",
        "UPDATE",
        "CREATE",
        "ALTER",
        "GRANT",
        "REVOKE",
        "REPLACE",
        "MERGE",
        "UPSERT",
        "VACUUM",
        "REINDEX",
        "COPY",
        "SET",
        "DEL",
        "HSET",
        "LPUSH",
        "EXPIRE",
        "INSERTONE",
        "UPDATEONE",
        "ATTACH",
        "DETACH",
        "PRAGMA",
    ];
    if words.iter().any(|w| changes.contains(w)) || upper.contains("INSERTONE(") || upper.contains("UPDATEMANY(") {
        return Some(InfraChange::Apply);
    }
    None
}

/// `psql`, `mysql`, `sqlite3`, `redis-cli`, …: the statements they run, against which database.
pub(super) fn database(w: &mut Walk<'_>, program: &str, args: &[Exp], input: Input) -> Input {
    let sqlite = matches!(program, "sqlite3" | "sqlite" | "duckdb");
    let positional: Vec<&Exp> = {
        let values = [
            "-h",
            "--host",
            "-p",
            "--port",
            "-U",
            "--username",
            "-u",
            "--user",
            "-d",
            "--dbname",
            "-c",
            "--command",
            "-e",
            "--execute",
            "-f",
            "--file",
            "-a",
            "-n",
            "--eval",
            "-cmd",
            "-separator",
            "-P",
            "-D",
            "--database",
        ];
        let mut out = Vec::new();
        let mut k = 0;
        while k < args.len() {
            let t = args[k].text.as_str();
            if values.contains(&t) {
                k += 2;
                continue;
            }
            if !t.starts_with('-') {
                out.push(&args[k]);
            }
            k += 1;
        }
        out
    };
    let url_host = args.iter().find_map(|a| super::programs::url_host(&a.text));
    let host = value_of(args, &["-h", "--host"]).filter(|_| !sqlite).or(url_host);
    let (target, local) = if sqlite {
        match positional.first() {
            Some(db) => {
                let p = w.path(db);
                (format!("the database {}", p.path), p.area.in_workspace())
            }
            None => ("an in-memory database".to_owned(), true),
        }
    } else {
        match &host {
            Some(h) => (format!("the database on {h}"), local_host(h) || h.starts_with('/')),
            None => ("the local database".to_owned(), true),
        }
    };
    let mut sql: Vec<String> = Vec::new();
    for names in [&["-c", "--command"][..], &["-e", "--execute"], &["--eval"], &["-cmd"]] {
        if let Some(v) = value_of(args, names) {
            sql.push(v);
        }
    }
    if sqlite && positional.len() >= 2 {
        sql.extend(positional[1..].iter().map(|e| e.text.clone()));
    }
    if program == "redis-cli" {
        sql.push(positional.iter().map(|e| e.text.clone()).collect::<Vec<_>>().join(" "));
    }
    if let Some(f) = value_of(args, &["-f", "--file"]) {
        let path = w.path(&Exp::lit(&f));
        match w.files.get(&path.path).cloned().flatten() {
            Some(text) => sql.push(text),
            None => {
                w.lower(Certainty::Partial, format!("the statements in {f}"));
                sql.push("UPDATE".to_owned());
            }
        }
    }
    match &input {
        Input::Text(t) => sql.push(t.clone()),
        Input::None => {}
        _ => {
            w.lower(Certainty::Partial, format!("the statements {program} reads from its input"));
            sql.push("UPDATE".to_owned());
        }
    }
    let change = match program {
        "dropdb" => Some(InfraChange::Destroy),
        "createdb" | "mysqladmin" => Some(InfraChange::Apply),
        "pg_restore" => Some(if args.iter().any(|a| a.text == "--clean" || a.text == "-c") {
            InfraChange::Destroy
        } else {
            InfraChange::Apply
        }),
        "pg_dump" | "mysqldump" => {
            if let Some(o) = value_of(args, &["-f", "--file", "-r", "--result-file"]) {
                let p = w.path(&Exp::lit(&o));
                w.write(p);
            }
            None
        }
        _ => {
            if sql.is_empty() {
                w.effect(Effect::Network {
                    host: host.clone().unwrap_or_else(|| "localhost".to_owned()),
                    method: NetMethod::Connect,
                });
                w.lower(Certainty::Partial, format!("what an interactive {program} session does"));
                None
            } else {
                sql.iter().filter_map(|s| sql_change(s)).max_by_key(|c| matches!(c, InfraChange::Destroy))
            }
        }
    };
    if let Some(change) = change {
        w.effect(Effect::Infra {
            target,
            change,
            local,
        });
    }
    Input::Opaque(program.to_owned())
}

const DESTROY: &[&str] = &[
    "delete",
    "destroy",
    "remove",
    "rm",
    "rb",
    "terminate",
    "purge",
    "drop",
    "deregister",
    "uninstall",
    "prune",
    "wipe",
    "reset",
    "nuke",
];
const APPLY: &[&str] = &[
    "create",
    "put",
    "update",
    "set",
    "deploy",
    "apply",
    "run",
    "start",
    "stop",
    "attach",
    "detach",
    "add",
    "publish",
    "push",
    "cp",
    "mv",
    "sync",
    "invoke",
    "scale",
    "restart",
    "enable",
    "disable",
    "import",
    "patch",
    "replace",
    "edit",
    "rollout",
    "label",
    "annotate",
    "cordon",
    "uncordon",
    "taint",
    "untaint",
    "expose",
    "autoscale",
    "exec",
    "install",
    "upgrade",
    "rollback",
    "up",
    "refresh",
    "modify",
    "associate",
    "tag",
    "upload",
    "promote",
    "release",
    "secrets",
    "ssh",
    "drain",
];
const PERMISSION: &[&str] = &[
    "iam",
    "policy",
    "policies",
    "grant",
    "role",
    "roles",
    "add-iam-policy-binding",
    "set-iam-policy",
    "access",
    "acl",
    "permissions",
    "service-account",
    "keys",
    "rbac",
];
const PUBLISHERS: &[&str] = &[
    "vercel",
    "netlify",
    "firebase",
    "wrangler",
    "fly",
    "flyctl",
    "heroku",
    "railway",
    "render",
    "serverless",
    "sls",
    "supabase",
];

/// Clusters, clouds and infrastructure as code.
pub(super) fn cloud(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Input {
    let words: Vec<String> =
        args.iter().filter(|a| !a.text.starts_with('-')).map(|a| a.text.to_ascii_lowercase()).collect();
    let first = words.first().map(String::as_str).unwrap_or_default();
    let context = value_of(args, &["--context", "--kube-context", "--profile", "--project", "--subscription"]);
    let local_context = |c: &str| {
        ["kind-", "minikube", "docker-desktop", "k3d-", "rancher-desktop", "orbstack", "colima", "localhost", "local"]
            .iter()
            .any(|p| c.starts_with(p))
    };
    let local = matches!(program, "kind" | "minikube") || context.as_deref().is_some_and(local_context);
    let target = context.map_or_else(|| program.to_owned(), |c| format!("{program} ({c})"));
    let change = match program {
        "kubectl" | "oc" => match first {
            "delete" | "drain" => Some(InfraChange::Destroy),
            "get" | "describe" | "logs" | "top" | "explain" | "api-resources" | "api-versions" | "cluster-info"
            | "version" | "config" | "auth" | "diff" | "events" | "wait" | "completion" | "plugin" | "kustomize"
            | "" => None,
            "port-forward" | "proxy" => {
                w.effect(Effect::Network {
                    host: "localhost".to_owned(),
                    method: NetMethod::Connect,
                });
                None
            }
            _ => Some(InfraChange::Apply),
        },
        "helm" => match first {
            "uninstall" | "delete" => Some(InfraChange::Destroy),
            "install" | "upgrade" | "rollback" => Some(InfraChange::Apply),
            "push" => {
                w.effect(Effect::Publish {
                    target: "a Helm chart".to_owned(),
                });
                None
            }
            _ => None,
        },
        "terraform" | "tofu" | "terragrunt" | "pulumi" | "cdk" => match first {
            "destroy" => Some(InfraChange::Destroy),
            "apply" | "import" | "taint" | "untaint" | "up" | "deploy" | "refresh" | "state" | "force-unlock"
            | "cancel" => {
                if first == "state" && words.get(1).is_some_and(|s| matches!(s.as_str(), "list" | "show" | "pull")) {
                    None
                } else if first == "state" && words.get(1).is_some_and(|s| s == "rm") {
                    Some(InfraChange::Destroy)
                } else {
                    Some(InfraChange::Apply)
                }
            }
            "init" | "get" => {
                w.effect(Effect::Network {
                    host: "the provider registry".to_owned(),
                    method: NetMethod::Read,
                });
                None
            }
            _ => None,
        },
        "kind" | "minikube" => match first {
            "delete" => Some(InfraChange::Destroy),
            "create" | "start" | "load" | "addons" => Some(InfraChange::Apply),
            _ => None,
        },
        "ansible" | "ansible-playbook" => {
            if program == "ansible" && args.iter().any(|a| a.text == "ping") {
                None
            } else {
                Some(InfraChange::Apply)
            }
        }
        _ => {
            let destroy = words.iter().any(|x| {
                DESTROY.iter().any(|d| x == d || x.starts_with(&format!("{d}-")) || x.ends_with(&format!("-{d}")))
            });
            let apply = words.iter().any(|x| {
                APPLY.iter().any(|d| x == d || x.starts_with(&format!("{d}-")) || x.ends_with(&format!("-{d}")))
            });
            let deploys = PUBLISHERS.contains(&program)
                && (words.is_empty()
                    || words.iter().any(|x| matches!(x.as_str(), "deploy" | "publish" | "up" | "release" | "push"))
                    || args.iter().any(|a| a.text == "--prod"));
            if deploys && !destroy {
                w.effect(Effect::Publish {
                    target: format!("a deployment with {program}"),
                });
                None
            } else if (destroy || apply) && words.iter().any(|x| PERMISSION.contains(&x.as_str())) {
                w.effect(Effect::Permission {
                    resource: target.clone(),
                    change: words.join(" "),
                });
                None
            } else if destroy {
                Some(InfraChange::Destroy)
            } else if apply || (program == "gsutil" && words.iter().any(|x| x == "cp" || x == "rsync")) {
                Some(InfraChange::Apply)
            } else {
                None
            }
        }
    };
    if let Some(change) = change {
        w.effect(Effect::Infra {
            target,
            change,
            local,
        });
    }
    Input::Opaque(program.to_owned())
}

/// Containers: local images and containers are recoverable; pushing an image publishes it.
pub(super) fn containers(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Input {
    let words: Vec<&str> = args.iter().filter(|a| !a.text.starts_with('-')).map(|a| a.text.as_str()).collect();
    match words.as_slice() {
        ["push", ..] | ["image", "push", ..] | ["buildx", "build", ..]
            if words.first() == Some(&"push")
                || words.get(1) == Some(&"push")
                || args.iter().any(|a| a.text == "--push") =>
        {
            w.effect(Effect::Publish {
                target: format!("a container image ({})", words.last().copied().unwrap_or("image")),
            });
        }
        ["login", ..] => w.effect(Effect::SecretUse {
            secret: "a registry password".to_owned(),
            local_only: false,
        }),
        ["rm" | "rmi" | "prune", ..] | [_, "rm" | "prune" | "down", ..] => w.effect(Effect::DataWrite {
            service: program.to_owned(),
            what: format!("Removes {program} containers, images or volumes"),
            lost: true,
        }),
        [
            "ps" | "images" | "inspect" | "logs" | "info" | "version" | "stats" | "top" | "port" | "history" | "search"
            | "events" | "diff",
            ..,
        ]
        | [] => {}
        _ => {
            let privileged = args.iter().any(|a| a.text == "--privileged" || a.text == "--pid=host");
            let host_mount = args.windows(2).any(|p| {
                (p[0].text == "-v" || p[0].text == "--volume")
                    && (p[1].text.starts_with("/:")
                        || p[1].text.starts_with("/etc")
                        || p[1].text.starts_with("/var/run/docker.sock"))
            });
            if privileged || host_mount {
                w.effect(Effect::Exec {
                    program: format!("a {program} container with access to this computer"),
                    source: CodeSource::Unknown,
                });
                w.lower(Certainty::Partial, "what a container with access to this computer does");
            } else {
                w.effect(Effect::Exec {
                    program: format!("{program} {}", words.first().copied().unwrap_or_default()),
                    source: CodeSource::Toolchain,
                });
            }
        }
    }
    Input::Opaque(program.to_owned())
}

/// Services and scheduled jobs.
pub(super) fn services(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Input {
    let words: Vec<&str> = args.iter().filter(|a| !a.text.starts_with('-')).map(|a| a.text.as_str()).collect();
    let user = args.iter().any(|a| a.text == "--user");
    let read = match program {
        "systemctl" => words.first().is_none_or(|v| {
            matches!(
                *v,
                "status"
                    | "is-active"
                    | "is-enabled"
                    | "is-failed"
                    | "list-units"
                    | "list-unit-files"
                    | "show"
                    | "cat"
                    | "list-timers"
                    | "list-sockets"
                    | "help"
                    | "list-dependencies"
                    | "get-default"
            )
        }),
        "launchctl" => words
            .first()
            .is_none_or(|v| matches!(*v, "list" | "print" | "print-disabled" | "blame" | "version" | "help")),
        "crontab" => args.iter().any(|a| a.text == "-l"),
        _ => words.len() <= 1 || words.get(1).is_some_and(|v| *v == "status"),
    };
    if read {
        return Input::None;
    }
    let destroy = program == "crontab" && args.iter().any(|a| a.text == "-r");
    let local =
        user || matches!(program, "launchctl" | "crontab" | "at") && !args.iter().any(|a| a.text.contains("system"));
    let target = match program {
        "crontab" | "at" => "scheduled jobs".to_owned(),
        _ => format!("the service {}", words.get(1).or(words.first()).copied().unwrap_or("?")),
    };
    w.effect(Effect::Infra {
        target,
        change: if destroy {
            InfraChange::Destroy
        } else {
            InfraChange::Apply
        },
        local,
    });
    Input::None
}
