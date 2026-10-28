//! The `reins-audit` command line: `verify` and `report`. [`run`] does all of it and returns what to print, so the
//! binary (`main.rs`) only prints and exits, and tests can run the exact same path.
//!
//! Exit codes: 0 verified (or reported), 1 verification failed, 2 usage (or an unreadable input file).

use std::io::{BufRead, BufReader, Read};

use reins_proto::managed::{Checkpoint, Envelope};
use serde::{Deserialize, Serialize};

use crate::evidence::{self, Framework, Period};
use crate::{Report, TrustedKey, verify_collect};

pub const USAGE: &str = "\
Verifies a Reins organization's audit export offline, and turns it into evidence for auditors.

usage:
  reins-audit verify <export.jsonl|-> (--key <id>=<base64> ... | --keys <keys.json>)
                     [--anchor <checkpoint.json>] [--json]
  reins-audit report --framework soc2|iso27001 <export.jsonl|-> (--key <id>=<base64> ... | --keys <keys.json>)
                     [--anchor <checkpoint.json>] [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--format md|json]
  reins-audit --version

  -                read the export from stdin
  --key            the decision service's public key, as the console shows it (may repeat)
  --keys           the key file downloaded from the console (only its decision key is trusted)
  --anchor         a checkpoint kept from earlier: the export must still contain it
  --from, --to     UTC dates; the report covers --from through the whole --to day

exit codes: 0 verified, 1 verification failed, 2 usage
";

/// What a run prints and how it exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    fn ok(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    fn failed(why: &str) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: format!("FAILED: {why}\n"),
        }
    }

    fn usage(why: &str) -> Self {
        Self {
            code: 2,
            stdout: String::new(),
            stderr: if why.is_empty() {
                USAGE.to_owned()
            } else {
                format!("{why}\n\n{USAGE}")
            },
        }
    }

    /// An input that could not be read (no usage text: the command was right).
    fn input(why: &str) -> Self {
        Self {
            code: 2,
            stdout: String::new(),
            stderr: format!("{why}\n"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Verify,
    Report,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Markdown,
    Json,
}

/// The key file the console offers for download. Its `issuer` key signs policies and rosters, never checkpoints or
/// heads, so it is not read.
#[derive(Deserialize)]
struct KeyFile {
    #[serde(default)]
    org: Option<String>,
    decision: String,
}

struct Options {
    command: Command,
    file: String,
    keys: Vec<TrustedKey>,
    /// The org the key file is for: the export must be of it.
    key_org: Option<String>,
    anchor: Option<Checkpoint>,
    json: bool,
    framework: Option<Framework>,
    from: Option<i64>,
    to: Option<i64>,
    format: Option<Format>,
}

/// Why the arguments can't run: a usage mistake, or a file named in them that can't be read.
enum ArgError {
    Usage(String),
    Input(String),
}

impl From<&str> for ArgError {
    fn from(s: &str) -> Self {
        Self::Usage(s.to_owned())
    }
}

/// An anchor file: a bare checkpoint, or a signed checkpoint envelope (whose body is read).
fn read_anchor(path: &str) -> Result<Checkpoint, ArgError> {
    let text = std::fs::read_to_string(path).map_err(|e| ArgError::Input(format!("{path}: {e}")))?;
    if let Ok(cp) = serde_json::from_str::<Checkpoint>(&text) {
        return Ok(cp);
    }
    let env: Envelope =
        serde_json::from_str(&text).map_err(|e| ArgError::Input(format!("{path}: not a checkpoint: {e}")))?;
    env.parse().map_err(|e| ArgError::Input(format!("{path}: not a checkpoint: {e}")))
}

fn read_keys(path: &str) -> Result<(TrustedKey, Option<String>), ArgError> {
    let text = std::fs::read_to_string(path).map_err(|e| ArgError::Input(format!("{path}: {e}")))?;
    let file: KeyFile =
        serde_json::from_str(&text).map_err(|e| ArgError::Input(format!("{path}: not a key file: {e}")))?;
    let key = TrustedKey::parse(&file.decision)
        .ok_or_else(|| ArgError::Input(format!("{path}: the decision key is not <id>=<base64>")))?;
    Ok((key, file.org.filter(|o| !o.is_empty())))
}

fn date(flag: &str, value: &str) -> Result<i64, ArgError> {
    evidence::day_start(value).ok_or_else(|| ArgError::Usage(format!("{flag}: not a date (YYYY-MM-DD): {value}")))
}

fn parse(args: &[String]) -> Result<Options, ArgError> {
    let mut it = args.iter();
    let command = match it.next().map(String::as_str) {
        Some("verify") => Command::Verify,
        Some("report") => Command::Report,
        Some(other) => return Err(ArgError::Usage(format!("unknown command: {other}"))),
        None => return Err("".into()),
    };
    let mut o = Options {
        command,
        file: String::new(),
        keys: Vec::new(),
        key_org: None,
        anchor: None,
        json: false,
        framework: None,
        from: None,
        to: None,
        format: None,
    };
    let mut file: Option<&str> = None;
    let report = command == Command::Report;
    while let Some(a) = it.next() {
        let mut value = || it.next().map(String::as_str).ok_or_else(|| ArgError::Usage(format!("{a} needs a value")));
        match a.as_str() {
            "--key" => {
                let spec = value()?;
                o.keys.push(TrustedKey::parse(spec).ok_or_else(|| ArgError::Usage(format!("not a key: {spec}")))?);
            }
            "--keys" => {
                let (key, org) = read_keys(value()?)?;
                o.keys.push(key);
                o.key_org = org;
            }
            "--anchor" => o.anchor = Some(read_anchor(value()?)?),
            "--json" if !report => o.json = true,
            "--framework" if report => {
                let v = value()?;
                o.framework =
                    Some(Framework::parse(v).ok_or_else(|| ArgError::Usage(format!("unknown framework: {v}")))?);
            }
            "--from" if report => o.from = Some(date(a, value()?)?),
            // The whole --to day: up to the start of the next.
            "--to" if report => o.to = Some(date(a, value()?)? + 86_400),
            "--format" if report => {
                o.format = Some(match value()? {
                    "md" => Format::Markdown,
                    "json" => Format::Json,
                    v => return Err(ArgError::Usage(format!("unknown format: {v}"))),
                });
            }
            f if file.is_none() && (f == "-" || !f.starts_with('-')) => file = Some(f),
            f if !f.starts_with('-') => return Err("name one export".into()),
            f => return Err(ArgError::Usage(format!("unknown option: {f}"))),
        }
    }
    file.ok_or("name the export file, or - for stdin")?.clone_into(&mut o.file);
    if o.keys.is_empty() {
        return Err("name the decision service key with --key or --keys".into());
    }
    if report && o.framework.is_none() {
        return Err("name the framework with --framework soc2 or iso27001".into());
    }
    if let (Some(from), Some(to)) = (o.from, o.to)
        && from >= to
    {
        return Err("--from must not be after --to".into());
    }
    Ok(o)
}

/// What `verify --json` prints.
#[derive(Serialize)]
struct VerifiedJson<'a> {
    ok: bool,
    org: &'a str,
    events: u64,
    first_seq: i64,
    last_seq: i64,
    checkpoints: u64,
    head_hash: &'a str,
}

#[derive(Serialize)]
struct FailedJson<'a> {
    ok: bool,
    error: &'a str,
}

fn json_line<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default() + "\n"
}

/// Runs `reins-audit` with `args` (without the program name), reading `-` from `stdin`.
pub fn run(args: &[String], stdin: &mut dyn Read) -> Outcome {
    match args.first().map(String::as_str) {
        Some("--version" | "-V") if args.len() == 1 => {
            return Outcome::ok(format!("reins-audit {}\n", env!("CARGO_PKG_VERSION")));
        }
        Some("--help" | "-h" | "help") => return Outcome::ok(USAGE.to_owned()),
        _ => {}
    }
    let o = match parse(args) {
        Ok(o) => o,
        Err(ArgError::Usage(why)) => return Outcome::usage(&why),
        Err(ArgError::Input(why)) => return Outcome::input(&why),
    };
    let input: Box<dyn BufRead + '_> = if o.file == "-" {
        Box::new(BufReader::new(stdin))
    } else {
        match std::fs::File::open(&o.file) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => return Outcome::input(&format!("{}: {e}", o.file)),
        }
    };
    let verified =
        verify_collect(input, &o.keys, o.anchor.as_ref()).map_err(|e| e.to_string()).and_then(|v| match &o.key_org {
            Some(org) if *org != v.0.org => {
                Err(format!("the export is of organization {}, the key file is for {org}", v.0.org))
            }
            _ => Ok(v),
        });
    let (report, events) = match verified {
        Ok(v) => v,
        Err(why) if o.json => {
            return Outcome {
                code: 1,
                stdout: json_line(&FailedJson {
                    ok: false,
                    error: &why,
                }),
                stderr: String::new(),
            };
        }
        Err(why) => return Outcome::failed(&why),
    };
    match o.command {
        Command::Verify => Outcome::ok(verified_text(&report, o.json)),
        Command::Report => {
            let framework = o.framework.unwrap_or(Framework::Soc2);
            let period = Period {
                from: o.from,
                to: o.to,
            };
            let e = evidence::build(framework, &report, &events, period);
            Outcome::ok(match o.format.unwrap_or(Format::Markdown) {
                Format::Markdown => evidence::render_markdown(&e),
                Format::Json => serde_json::to_string_pretty(&e).unwrap_or_default() + "\n",
            })
        }
    }
}

fn verified_text(r: &Report, json: bool) -> String {
    if json {
        return json_line(&VerifiedJson {
            ok: true,
            org: &r.org,
            events: r.events,
            first_seq: r.first_seq,
            last_seq: r.last_seq,
            checkpoints: r.checkpoints,
            head_hash: &r.head_hash,
        });
    }
    format!(
        "OK: {} events ({}..{}) of {}, {} checkpoints, head {}\n",
        r.events, r.first_seq, r.last_seq, r.org, r.checkpoints, r.head_hash
    )
}
