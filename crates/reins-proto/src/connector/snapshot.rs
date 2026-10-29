//! The tool list as it stood before connectors became plugins, pinned: every tool in its order, with what the server
//! and the phone read from it. A change to a connector's tools shows up here as a diff to review.
//!
//! `REINS_UPDATE_SNAPSHOTS=1 cargo test -p reins-proto snapshot` rewrites the file after an intended change.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::*;

const SNAPSHOT: &str = include_str!("testdata/tool_specs.txt");

/// Services whose display name and classes are pinned alongside the tools.
const SERVICES: &[&str] = &[
    GMAIL,
    TELEGRAM,
    GCALENDAR,
    GCONTACTS,
    DEVICE_CALENDAR,
    DEVICE_CONTACTS,
    SMS,
    GITHUB,
    VAULT,
    DESKTOP,
    TASKS,
    GITLAB,
    CODEBERG,
    BITBUCKET,
    PAYMENTS,
    X,
];

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
    for service in SERVICES {
        let classes: Vec<String> = classes(service).iter().map(|c| format!("{}={}", c.id, c.label)).collect();
        let _ = writeln!(out, "service {service} | {} | {}", service_name(service), classes.join("; "));
    }
    for s in specs() {
        let _ = writeln!(
            out,
            "tool {} | {} | {} | {:?} | class={} | once={} | desktop={} | check={} | resource={} | {} | {}",
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
