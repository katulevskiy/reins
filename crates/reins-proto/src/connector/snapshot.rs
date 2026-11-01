//! The tool list, pinned: every connector's name and classes, and every tool in its order with what the server and
//! the phone read from it. It was recorded before connectors moved into manifests, so that move changed nothing; a
//! change to a connector or its tools shows up here as a diff to review.
//!
//! `REINS_UPDATE_SNAPSHOTS=1 cargo test -p reins-proto snapshot` rewrites the file after an intended change.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::*;

const SNAPSHOT: &str = include_str!("testdata/tool_specs.txt");

fn digest(s: &ToolSpec) -> String {
    let mut h = Sha256::new();
    h.update(s.title.as_bytes());
    h.update([0]);
    h.update(s.description.as_bytes());
    h.update([0]);
    h.update(s.input_schema().to_string().as_bytes());
    let out = h.finalize();
    out.iter().take(8).fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

fn render() -> String {
    let mut out = String::new();
    // Scopes only where there are some, so that what names none reads as it did before scopes were pinned. A wider
    // sign-in shows up here.
    let scopes = |scopes: &[&str]| {
        if scopes.is_empty() {
            String::new()
        } else {
            format!(" | scopes={}", scopes.join(","))
        }
    };
    for m in manifest::manifests() {
        let service = m.service;
        let classes: Vec<String> = classes(service).iter().map(|c| format!("{}={}", c.id, c.label)).collect();
        let _ =
            writeln!(out, "service {service} | {} | {}{}", service_name(service), classes.join("; "), scopes(m.scopes));
    }
    for s in specs() {
        let scopes = scopes(s.scopes);
        let _ = writeln!(
            out,
            "tool {} | {} | {} | {:?} | class={} | once={} | desktop={} | check={} | resource={} | {} | {}{scopes}",
            s.tool,
            s.service,
            s.op,
            s.effect,
            s.class,
            s.once_only,
            s.desktop_only,
            s.check.is_some(),
            s.resource_param.unwrap_or("-"),
            s.annotations(),
            digest(s),
        );
    }
    out
}

#[test]
fn the_tool_list_matches_the_snapshot() {
    let now = render();
    if std::env::var_os("REINS_UPDATE_SNAPSHOTS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/connector/testdata/tool_specs.txt");
        std::fs::write(path, &now).unwrap();
        return;
    }
    if now != SNAPSHOT {
        let was: Vec<&str> = SNAPSHOT.lines().collect();
        let is: Vec<&str> = now.lines().collect();
        let gone: Vec<&&str> = was.iter().filter(|l| !is.contains(l)).collect();
        let new: Vec<&&str> = is.iter().filter(|l| !was.contains(l)).collect();
        panic!("the tool list changed (order or content)\n- {gone:#?}\n+ {new:#?}");
    }
}
