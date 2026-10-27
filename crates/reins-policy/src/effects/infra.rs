//! Databases: the statements a client runs, against which database (`Infra` effects, local or not).

use super::level::local_host;
use super::model::{Certainty, Effect, InfraChange, NetMethod};
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
            // `/* … */` is a space; MySQL runs what is in `/*! … */`.
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let runs = chars.peek() == Some(&'!');
                let mut prev = ' ';
                let mut inner = String::new();
                for q in chars.by_ref() {
                    if prev == '*' && q == '/' {
                        break;
                    }
                    inner.push(q);
                    prev = q;
                }
                plain.push(' ');
                if runs {
                    plain.push_str(inner.trim_start_matches('!').trim_end_matches('*'));
                    plain.push(' ');
                }
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
    // Any DROP drops something (a function, a trigger, a policy, a publication…).
    if words.contains(&"DROP")
        || words.contains(&"TRUNCATE")
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
pub(super) fn database(w: &mut Walk<'_>, program: &str, args: &[Exp], input: &Input) -> Input {
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
    // A conninfo string (`'host=db dbname=x'`).
    let conninfo = |key: &str| {
        positional.iter().find_map(|a| {
            a.text
                .split_whitespace()
                .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')).map(str::to_owned))
        })
    };
    let env_host = match program {
        "psql" | "pg_dump" | "pg_restore" | "dropdb" | "createdb" | "pgcli" => w.var_state("PGHOST"),
        "mysql" | "mysqldump" | "mysqladmin" | "mariadb" | "mycli" => w.var_state("MYSQL_HOST"),
        _ => None,
    };
    let mut host = value_of(args, &["-h", "--host"])
        .filter(|_| !sqlite)
        .or(url_host)
        .or_else(|| conninfo("host"))
        .or_else(|| env_host.clone().flatten());
    // A host or a service named where this can't see: any database.
    let service = conninfo("service").is_some()
        || (program.starts_with("pg") || program == "psql") && w.var_state("PGSERVICE").is_some();
    if service || matches!(env_host, Some(None)) || host.as_deref().is_some_and(|h| h.contains(super::shell::UNKNOWN)) {
        w.lower(Certainty::Partial, format!("which database {program} connects to"));
        host = Some("a database not known".to_owned());
    }
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
        if let Some(text) = w.files.get(&path.path).cloned().flatten() {
            sql.push(text);
        } else {
            w.lower(Certainty::None, format!("the statements in {f}"));
            sql.push("UPDATE".to_owned());
        }
    }
    match input {
        Input::Text(t) => sql.push(t.clone()),
        Input::None => {}
        _ => {
            w.lower(Certainty::None, format!("the statements {program} reads from its input"));
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
                    host: host.unwrap_or_else(|| "localhost".to_owned()),
                    method: NetMethod::Connect,
                });
                w.lower(Certainty::None, format!("what an interactive {program} session does"));
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
