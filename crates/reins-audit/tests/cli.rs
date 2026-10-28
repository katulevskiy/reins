//! The `reins-audit` binary on fixture exports.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use reins_proto::canon::encode_sig;
use reins_proto::managed::{AuditEvent, Checkpoint, Envelope, EnvelopeKind, ExportLine, envelope_message};
use reins_proto::org::GENESIS_HASH;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::Value;
use tempfile::TempDir;

struct Signer {
    key: Ed25519KeyPair,
    id: String,
}

impl Signer {
    fn new(id: &str) -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Self {
            key: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
            id: id.to_owned(),
        }
    }

    /// `<id>=<base64url>`, as the console shows it.
    fn spec(&self) -> String {
        format!("{}={}", self.id, data_encoding::BASE64URL_NOPAD.encode(self.key.public_key().as_ref()))
    }

    fn envelope(&self, kind: EnvelopeKind, cp: &Checkpoint) -> Envelope {
        let body = serde_json::to_string(cp).unwrap();
        let sig = self.key.sign(&envelope_message(kind, &cp.org, &self.id, &body));
        Envelope {
            kind,
            org: cp.org.clone(),
            body,
            key_id: self.id.clone(),
            sig: encode_sig(sig.as_ref().try_into().unwrap()),
        }
    }
}

const KINDS: &[(&str, &str)] = &[
    ("member.provisioned", "{}"),
    ("member.provisioned", "{}"),
    ("device.enrolled", "{}"),
    ("change.applied", "{\"action\":{},\"sole_holder\":true}"),
    ("member.deprovisioned", "{}"),
];

/// Events at 2026-10-01 00:00 UTC plus `seq` days.
const OCT_1: i64 = 1_790_812_800;
const DAY: i64 = 86_400;

fn events() -> Vec<AuditEvent> {
    let mut out: Vec<AuditEvent> = Vec::new();
    let mut prev = GENESIS_HASH.to_owned();
    for (seq, (kind, detail)) in (1..).zip(KINDS) {
        let mut e = AuditEvent {
            org: "o1".into(),
            seq,
            at: OCT_1 + seq * DAY,
            kind: (*kind).into(),
            actor: Some("m1".into()),
            subject: None,
            detail: (*detail).into(),
            prev_hash: prev.clone(),
            hash: String::new(),
        };
        e.hash = e.expected_hash();
        prev.clone_from(&e.hash);
        out.push(e);
    }
    out
}

fn cp(e: &AuditEvent) -> Checkpoint {
    Checkpoint {
        org: e.org.clone(),
        seq: e.seq,
        hash: e.hash.clone(),
        at: e.at,
    }
}

/// The export of `events`, with a checkpoint after event 2 and a signed head.
fn export(events: &[AuditEvent], s: &Signer) -> String {
    let mut lines: Vec<String> = Vec::new();
    for e in events {
        lines.push(serde_json::to_string(&ExportLine::Event(e.clone())).unwrap());
        if e.seq == 2 {
            lines.push(
                serde_json::to_string(&ExportLine::Checkpoint(s.envelope(EnvelopeKind::Checkpoint, &cp(e)))).unwrap(),
            );
        }
    }
    let head = s.envelope(EnvelopeKind::Head, &cp(events.last().unwrap()));
    lines.push(serde_json::to_string(&ExportLine::Head(head)).unwrap());
    lines.join("\n") + "\n"
}

struct Fixture {
    dir: TempDir,
    signer: Signer,
    events: Vec<AuditEvent>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let f = Self {
            dir,
            signer: Signer::new("dk1"),
            events: events(),
        };
        f.write("export.jsonl", &export(&f.events, &f.signer));
        f
    }

    fn write(&self, name: &str, text: &str) -> String {
        let path: PathBuf = self.dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_str().unwrap().to_owned()
    }

    fn head_hash(&self) -> &str {
        &self.events.last().unwrap().hash
    }

    /// The export with event 2's detail edited (its stored hash kept).
    fn edited(&self) -> String {
        let mut events = self.events.clone();
        events[1].detail = "{\"edited\":true}".into();
        self.write("edited.jsonl", &export(&events, &self.signer))
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn reins_audit(args: &[&str], stdin: Option<&str>, cwd: &Path) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_reins-audit"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        pipe.write_all(text.as_bytes()).unwrap();
    }
    drop(pipe);
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

fn run(f: &Fixture, args: &[&str]) -> Run {
    reins_audit(args, None, f.dir.path())
}

#[test]
fn verify_ok_prints_the_range_and_exits_0() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--key", &key]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.starts_with("OK: 5 events (1..5) of o1, 1 checkpoints, head "), "{}", r.stdout);
    assert!(r.stdout.contains(f.head_hash()), "{}", r.stdout);
    assert!(r.stderr.is_empty(), "{}", r.stderr);
    // A repeated --key: any of them may sign.
    let other = Signer::new("dk0").spec();
    let r = run(&f, &["verify", "--key", &other, &f.path("export.jsonl"), "--key", &key]);
    assert_eq!(r.code, 0, "{}", r.stderr);
}

#[test]
fn verify_json_is_machine_readable() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--key", &key, "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["org"], "o1");
    assert_eq!(v["events"], 5);
    assert_eq!((v["first_seq"].as_i64(), v["last_seq"].as_i64()), (Some(1), Some(5)));
    assert_eq!(v["checkpoints"], 1);
    assert_eq!(v["head_hash"], f.head_hash());
    // A failure is machine-readable too.
    let r = run(&f, &["verify", &f.edited(), "--key", &key, "--json"]);
    assert_eq!(r.code, 1);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("event 2 was changed"), "{v}");
}

#[test]
fn an_edited_event_exits_1_and_names_its_seq() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let r = run(&f, &["verify", &f.edited(), "--key", &key]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.is_empty(), "{}", r.stdout);
    assert!(r.stderr.starts_with("FAILED: "), "{}", r.stderr);
    assert!(r.stderr.contains("event 2 was changed"), "{}", r.stderr);
    // Signed by a key nobody trusts.
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--key", &Signer::new("dk1").spec()]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("not signed by a trusted key"), "{}", r.stderr);
}

#[test]
fn an_anchor_from_an_earlier_export_is_checked() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let anchor = cp(&f.events[2]);
    let bare = f.write("anchor.json", &serde_json::to_string(&anchor).unwrap());
    let signed = f.write(
        "anchor-signed.json",
        &serde_json::to_string(&f.signer.envelope(EnvelopeKind::Checkpoint, &anchor)).unwrap(),
    );
    for a in [&bare, &signed] {
        let r = run(&f, &["verify", &f.path("export.jsonl"), "--key", &key, "--anchor", a]);
        assert_eq!(r.code, 0, "{}", r.stderr);
    }
    let mut wrong = anchor;
    wrong.hash = f.events[0].hash.clone();
    let wrong = f.write("wrong.json", &serde_json::to_string(&wrong).unwrap());
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--key", &key, "--anchor", &wrong]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("does not extend the checkpoint at event 3"), "{}", r.stderr);
}

#[test]
fn keys_file_from_the_console_works() {
    let f = Fixture::new();
    let issuer = Signer::new("ik1");
    let keys = f.write(
        "keys.json",
        &serde_json::json!({"org": "o1", "decision": f.signer.spec(), "issuer": issuer.spec()}).to_string(),
    );
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--keys", &keys]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.starts_with("OK: 5 events"), "{}", r.stdout);
    // Only the decision key is trusted: an export signed by the issuer key fails.
    let by_issuer = f.write("by-issuer.jsonl", &export(&f.events, &issuer));
    let r = run(&f, &["verify", &by_issuer, "--keys", &keys]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("not signed by a trusted key"), "{}", r.stderr);
    // The keys of another org don't vouch for this export.
    let other = f.write(
        "other.json",
        &serde_json::json!({"org": "o2", "decision": f.signer.spec(), "issuer": issuer.spec()}).to_string(),
    );
    let r = run(&f, &["verify", &f.path("export.jsonl"), "--keys", &other]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.is_empty(), "{}", r.stdout);
    assert!(r.stderr.starts_with("FAILED: "), "{}", r.stderr);
    // Not a keys file.
    let junk = f.write("junk.json", "{\"decision\": 1}");
    assert_eq!(run(&f, &["verify", &f.path("export.jsonl"), "--keys", &junk]).code, 2);
}

#[test]
fn stdin_dash_reads_the_export() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let text = std::fs::read_to_string(f.path("export.jsonl")).unwrap();
    let r = reins_audit(&["verify", "-", "--key", &key], Some(&text), f.dir.path());
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.starts_with("OK: 5 events (1..5) of o1"), "{}", r.stdout);
    let r = reins_audit(&["report", "--framework", "soc2", "-", "--key", &key], Some(&text), f.dir.path());
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("## CC6.1 "), "{}", r.stdout);
}

#[test]
fn report_refuses_an_unverified_export() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let edited = f.edited();
    for format in ["md", "json"] {
        let r = run(&f, &["report", "--framework", "soc2", &edited, "--key", &key, "--format", format]);
        assert_eq!(r.code, 1);
        assert!(r.stdout.is_empty(), "{}", r.stdout);
        assert!(r.stderr.starts_with("FAILED: "), "{}", r.stderr);
        assert!(r.stderr.contains("event 2 was changed"), "{}", r.stderr);
    }
    // A cut export (no signed head) is refused too.
    let text = std::fs::read_to_string(f.path("export.jsonl")).unwrap();
    let headless: Vec<&str> = text.lines().collect();
    let cut = f.write("cut.jsonl", &headless[..headless.len() - 1].join("\n"));
    let r = run(&f, &["report", "--framework", "iso27001", &cut, "--key", &key]);
    assert_eq!((r.code, r.stdout.as_str()), (1, ""));
    assert!(r.stderr.contains("no signed head"), "{}", r.stderr);
}

#[test]
fn report_soc2_markdown_and_iso_json() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let export = f.path("export.jsonl");
    let r = run(&f, &["report", "--framework", "soc2", &export, "--key", &key]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let md = &r.stdout;
    assert!(md.starts_with("# SOC 2 evidence: o1\n"), "{md}");
    assert!(md.contains(f.head_hash()), "{md}");
    for id in ["CC6.1", "CC6.2", "CC6.3", "CC6.6", "CC6.7", "CC7.2", "CC7.3", "CC8.1", "A1.2", "C1.2"] {
        assert!(md.contains(&format!("## {id} ")), "{id}: {md}");
    }
    assert!(md.contains("applied without a second person"), "{md}");
    // The same export gives the same bytes.
    assert_eq!(run(&f, &["report", "--framework", "soc2", &export, "--key", &key]).stdout, *md);

    // Events 1..5 are on Oct 2..6; Oct 3 to Oct 4 (inclusive) keeps events 2 and 3.
    let r = run(
        &f,
        &[
            "report",
            "--framework",
            "iso27001",
            &export,
            "--key",
            &key,
            "--format",
            "json",
            "--from",
            "2026-10-03",
            "--to",
            "2026-10-04",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert_eq!(v["framework"], "iso27001");
    assert_eq!(v["org"], "o1");
    assert_eq!(v["period"]["from"], OCT_1 + 2 * DAY);
    assert_eq!(v["period"]["to"], OCT_1 + 4 * DAY);
    assert_eq!(v["summary"]["events"], 2);
    assert_eq!(v["summary"]["members_provisioned"], 1);
    assert_eq!(v["summary"]["devices_enrolled"], 1);
    assert_eq!(v["export"]["head_hash"], f.head_hash());
    assert_eq!((v["export"]["first_seq"].as_i64(), v["export"]["last_seq"].as_i64()), (Some(1), Some(5)));
    let controls = v["controls"].as_array().unwrap();
    assert!(controls.iter().all(|c| c["id"].as_str().unwrap().starts_with("A.")));
    let a516 = controls.iter().find(|c| c["id"] == "A.5.16").unwrap();
    assert_eq!(a516["total"], 1);
    assert_eq!(a516["samples"], serde_json::json!([2]));
    assert_eq!(v["exceptions"], serde_json::json!([]));
}

#[test]
fn usage_exits_2() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let export = f.path("export.jsonl");
    let (missing, missing_anchor) = (f.path("missing.jsonl"), f.path("missing.json"));
    let cases: Vec<Vec<&str>> = vec![
        vec![],
        vec!["frobnicate"],
        vec!["verify"],
        vec!["verify", &export],
        vec!["verify", &export, "--key"],
        vec!["verify", &export, "--key", "not-a-key"],
        vec!["verify", &export, "--key", &key, "--bogus"],
        vec!["verify", &export, &export, "--key", &key],
        vec!["verify", &export, "--key", &key, "--from", "2026-10-01"],
        vec!["verify", &missing, "--key", &key],
        vec!["verify", &export, "--key", &key, "--anchor", &missing_anchor],
        vec!["report", &export, "--key", &key],
        vec!["report", "--framework", "pci", &export, "--key", &key],
        vec!["report", "--framework", "soc2", &export, "--key", &key, "--from", "2026-02-30"],
        vec!["report", "--framework", "soc2", &export, "--key", &key, "--from", "2026-10-05", "--to", "2026-10-01"],
        vec!["report", "--framework", "soc2", &export, "--key", &key, "--format", "pdf"],
        vec!["report", "--framework", "soc2", &export, "--key", &key, "--json"],
    ];
    for args in cases {
        let r = run(&f, &args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{args:?}: {}", r.stdout);
        assert!(!r.stderr.is_empty(), "{args:?}");
    }
    let r = run(&f, &[]);
    assert!(r.stderr.contains("reins-audit verify") && r.stderr.contains("reins-audit report"), "{}", r.stderr);
    let r = run(&f, &["--help"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("reins-audit report"), "{}", r.stdout);
}

#[test]
fn version_prints_the_package_version() {
    let f = Fixture::new();
    let r = run(&f, &["--version"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stdout, format!("reins-audit {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_library_runs_the_same_path() {
    let f = Fixture::new();
    let key = f.signer.spec();
    let text = std::fs::read_to_string(f.path("export.jsonl")).unwrap();
    let args: Vec<String> = ["verify", "-", "--key", &key].iter().map(|s| (*s).to_owned()).collect();
    let o = reins_audit::cli::run(&args, &mut text.as_bytes());
    let bin = reins_audit(&["verify", "-", "--key", &key], Some(&text), f.dir.path());
    assert_eq!((i32::from(o.code), o.stdout, o.stderr), (bin.code, bin.stdout, bin.stderr));
}
