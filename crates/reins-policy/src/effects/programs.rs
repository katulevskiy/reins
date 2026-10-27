//! Program models: what one simple command does, from its program and its expanded words (01 §2.2).
//!
//! Each model reads the options the program really has and names effects on the paths, refs, hosts and people its
//! words name. A program with no model is `Exec{Unknown}` with partial certainty: never silently harmless.

use super::inline::{self, Lang};
use super::level::local_host;
use super::model::{Area, Certainty, CodeSource, Effect, FileKind, InfraChange, NetMethod, SecretKind};
use super::shell::{Exp, Input, UNKNOWN, Walk, file_kind};
use crate::shell::program_name;

/// A Hyprland dispatch in its Lua form that only moves, sizes or focuses a window: exactly one call
/// `hl.dsp.window.<action>({ name = value, … })` of the actions below, whose values are numbers, `true`/`false`, or
/// strings with no quote, backslash, `;` or `,` in them. Hyprland runs the text as Lua: nothing else in it (another
/// function, a concatenation, a long string, a second statement) is taken.
fn lua_window_action(text: &str) -> bool {
    const ACTIONS: &[&str] = &["float", "resize", "center", "move", "focus"];
    let ident = |s: &str| {
        let mut c = s.chars();
        c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
            && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
    };
    let value = |v: &str| {
        let number = v.strip_prefix('-').unwrap_or(v);
        (!number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || c == '.') && !number.starts_with('.'))
            || matches!(v, "true" | "false")
            || v.len() >= 2
                && v.starts_with('"')
                && v.ends_with('"')
                && !v[1..v.len() - 1].contains(['"', '\\', ';', ',', '\n', '['])
    };
    let Some(rest) = text.trim().strip_prefix("hl.dsp.window.") else {
        return false;
    };
    let Some((action, call)) = rest.split_once('(') else {
        return false;
    };
    let Some(table) = call.trim_end().strip_suffix(')').map(str::trim) else {
        return false;
    };
    let Some(fields) = table.strip_prefix('{').and_then(|t| t.strip_suffix('}')) else {
        return false;
    };
    ACTIONS.contains(&action)
        && fields
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .all(|f| f.split_once('=').is_some_and(|(k, v)| ident(k.trim()) && value(v.trim())))
}

/// Programs that only read their file arguments and print (writes with `-o`/`--output` are read too, see
/// [`reader`]). Each was checked: none runs code, writes elsewhere or reaches the network, whatever its arguments.
const READERS: &[&str] = &[
    "cat",
    "bat",
    "less",
    "more",
    "head",
    "tail",
    "wc",
    "sort",
    "uniq",
    "cut",
    "paste",
    "nl",
    "tac",
    "rev",
    "fold",
    "fmt",
    "column",
    "comm",
    "join",
    "diff",
    "cmp",
    "file",
    "stat",
    "du",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "b2sum",
    "cksum",
    "xxd",
    "hexdump",
    "od",
    "strings",
    "tree",
    "realpath",
    "readlink",
    "basename",
    "dirname",
    "iconv",
    "base32",
    "zcat",
    "zless",
    "bzcat",
    "xzcat",
    "zstdcat",
    "lsattr",
    "getfacl",
    "identify",
    "pdfinfo",
    "mediainfo",
    "ffprobe",
    "otool",
    "nm",
    "objdump",
    "readelf",
    "lipo",
    "zipinfo",
    "expand",
    "unexpand",
    "look",
    "tsort",
    "llvm-readelf",
    "llvm-objdump",
    "llvm-nm",
];

/// Readers that read a file's name, size, times or link, never its contents: their operands are no secret read.
const METADATA_READERS: &[&str] = &["ls", "exa", "eza", "du", "stat", "realpath", "readlink", "basename", "dirname"];

/// A reader's options that take a value as the next word.
fn reader_values(p: &str) -> &'static [&'static str] {
    match p {
        "head" | "tail" => {
            &["-n", "-c", "--lines", "--bytes", "-s", "--sleep-interval", "--pid", "--max-unchanged-stats"]
        }
        "sort" => &[
            "-k",
            "-t",
            "-o",
            "-S",
            "-T",
            "--key",
            "--field-separator",
            "--output",
            "--buffer-size",
            "--temporary-directory",
            "--parallel",
            "--batch-size",
            "--compress-program",
            "--random-source",
        ],
        "uniq" => &["-f", "-s", "-w", "--skip-fields", "--skip-chars", "--check-chars"],
        "cut" => &["-b", "-c", "-d", "-f", "--bytes", "--characters", "--delimiter", "--fields", "--output-delimiter"],
        "paste" => &["-d", "--delimiters"],
        "nl" => &["-b", "-d", "-f", "-h", "-i", "-l", "-n", "-s", "-v", "-w"],
        "fold" | "fmt" => &["-w", "--width", "-p", "--prefix", "-g", "--goal"],
        "column" => &["-s", "-c", "-o", "-N", "-R", "-W", "-H", "-E", "-d", "-l", "-r"],
        "join" => &["-t", "-1", "-2", "-j", "-o", "-a", "-v", "-e"],
        "diff" => &["-I", "-F", "-L", "-U", "-C", "-D", "-x", "-X", "-S", "--label", "--exclude", "--exclude-from"],
        "cmp" => &["-i", "-n", "--bytes", "--ignore-initial"],
        "file" => &["-m", "-f", "-F", "-e", "-P", "--magic-file", "--files-from", "--separator", "--exclude"],
        "stat" => &["-c", "--format", "--printf"],
        "du" => &["-d", "--max-depth", "-B", "--block-size", "-t", "--threshold", "--exclude", "-X", "--time-style"],
        "tree" => &["-L", "-P", "-I", "-o", "--filelimit", "--charset", "-H", "-T"],
        "bat" => &[
            "-l",
            "--language",
            "-H",
            "--highlight-line",
            "-r",
            "--line-range",
            "--style",
            "--theme",
            "--tabs",
            "-m",
            "--map-syntax",
            "--wrap",
        ],
        "less" => &["-b", "-h", "-j", "-k", "-o", "-O", "-p", "-P", "-t", "-T", "-x", "-y", "-z"],
        "more" => &["-n"],
        "xxd" => &["-c", "-g", "-l", "-o", "-s", "-C"],
        "hexdump" | "od" => &["-n", "-s", "-e", "-f", "-j", "-N", "-A", "-t", "-w"],
        "strings" => &["-n", "-t", "-e", "--bytes", "--radix", "--encoding"],
        "iconv" => &["-f", "-t", "-o", "--from-code", "--to-code", "--output"],
        "expand" | "unexpand" => &["-t", "--tabs"],
        "look" => &["-t"],
        _ => &[],
    }
}

/// Programs whose first argument is a pattern or a program text, the rest files they read.
const PATTERN_READERS: &[&str] =
    &["grep", "egrep", "fgrep", "rg", "ag", "ack", "zgrep", "jq", "yq", "gojq", "awk", "gawk", "mawk"];

/// Shell built-ins and programs with no effect on anything a request cares about, whatever their arguments: they
/// can't run code, write a file or reach the network. Anything that can, with some argument, has its own arm.
const HARMLESS: &[&str] = &[
    "echo",
    "printf",
    "tr",
    "true",
    "false",
    ":",
    "test",
    "[",
    "sleep",
    "wait",
    "set",
    "shopt",
    "exit",
    "return",
    "break",
    "continue",
    "unalias",
    "hash",
    "type",
    "which",
    "whereis",
    "printenv",
    "id",
    "whoami",
    "groups",
    "cal",
    "uname",
    "uptime",
    "df",
    "free",
    "pgrep",
    "top",
    "htop",
    "lsof",
    "ss",
    "netstat",
    "lsblk",
    "lscpu",
    "lsusb",
    "lspci",
    "nproc",
    "arch",
    "locale",
    "getconf",
    "tput",
    "clear",
    "reset",
    "stty",
    "tty",
    "seq",
    "expr",
    "yes",
    "jobs",
    "disown",
    "umask",
    "ulimit",
    "times",
    "history",
    "ls",
    "exa",
    "eza",
    "locate",
    "notify-send",
    "sw_vers",
    "system_profiler",
    "wl-copy",
    "wl-paste",
    "xclip",
    "xsel",
    "pbcopy",
    "pbpaste",
    "logger",
    "systemd-analyze",
    "journalctl",
    "dmesg",
    "fc-list",
    "fc-match",
    "sensors",
    "vmstat",
    "iostat",
    "mpstat",
    "pidof",
    "pstree",
    "getent",
    "dirs",
    "let",
    "caller",
    "complete",
    "compgen",
    "shellcheck",
    "tokei",
    "cloc",
    "scc",
];

/// Developer tools doing their usual job: builds, tests, formatters, linters, compilers, media tools. Build
/// systems run the project's own code (L0 in the workspace, like `cargo test`); none of these is a general code
/// runner, an editor that installs extensions, a server or a client of someone else's API. Publishing subcommands
/// are caught in [`toolchain`].
const TOOLCHAIN: &[&str] = &[
    "make",
    "gmake",
    "just",
    "ninja",
    "cmake",
    "gradle",
    "gradlew",
    "mvn",
    "xcodebuild",
    "xcrun",
    "swift",
    "swiftc",
    "swiftlint",
    "swiftformat",
    "rustc",
    "rustup",
    "rustfmt",
    "cargo-nextest",
    "uniffi-bindgen",
    "javac",
    "kotlinc",
    "javap",
    "tsc",
    "eslint",
    "prettier",
    "ruff",
    "black",
    "mypy",
    "pytest",
    "protoc",
    "typos",
    "shfmt",
    "clang-format",
    "actionlint",
    "zizmor",
    "hadolint",
    "xcbeautify",
    "xcpretty",
    "ktlint",
    "apksigner",
    "zipalign",
    "aapt2",
    "emulator",
    "ffmpeg",
    "magick",
    "convert",
    "montage",
    "grim",
    "slurp",
    "pdftotext",
];

/// System package managers: installing changes the whole computer.
const SYSTEM_PACKAGES: &[&str] = &[
    "apt", "apt-get", "dpkg", "pacman", "yay", "paru", "dnf", "yum", "zypper", "apk", "snap", "flatpak", "rpm", "port",
];

fn opt_value<'e>(args: &'e [Exp], names: &[&str]) -> Option<&'e Exp> {
    for (k, a) in args.iter().enumerate() {
        for n in names {
            if a.text == *n {
                return args.get(k + 1);
            }
        }
    }
    None
}

fn has(args: &[Exp], flag: &str) -> bool {
    args.iter().any(|a| a.text == flag)
}

/// A short flag present, alone or in a group (`-rf` has `r`).
fn short(args: &[Exp], c: char) -> bool {
    args.iter()
        .take_while(|a| a.text != "--")
        .any(|a| a.text.len() > 1 && a.text.starts_with('-') && !a.text.starts_with("--") && a.text[1..].contains(c))
}

/// The words that are not options, skipping the values of `value_opts` (exact option words that take a value).
fn positional<'e>(args: &'e [Exp], value_opts: &[&str]) -> Vec<&'e Exp> {
    let mut out = Vec::new();
    let mut k = 0;
    let mut options = true;
    while k < args.len() {
        let a = &args[k];
        if options && a.text == "--" {
            options = false;
        } else if options && a.text.starts_with('-') && a.text.len() > 1 {
            if value_opts.contains(&a.text.as_str()) {
                k += 1;
            }
        } else {
            out.push(a);
        }
        k += 1;
    }
    out
}

/// Where a wrapper's command starts: after its options (`values`: short options taking a value; `long_values`
/// likewise) and `operands` words.
fn after_options(args: &[Exp], values: &str, long_values: &[&str], operands: usize) -> usize {
    let mut k = 0;
    while let Some(a) = args.get(k) {
        let t = a.text.as_str();
        if t == "--" {
            k += 1;
            break;
        }
        if let Some(long) = t.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            k += 1 + usize::from(long_values.contains(&name) && value.is_none());
            continue;
        }
        if t.len() > 1 && t.starts_with('-') {
            let body = &t[1..];
            let mut skip = 1;
            for (i, c) in body.char_indices() {
                if values.contains(c) {
                    skip += usize::from(i + c.len_utf8() == body.len());
                    break;
                }
            }
            k += skip;
            continue;
        }
        // `env NAME=value cmd`
        if t.contains('=')
            && t.split('=')
                .next()
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            k += 1;
            continue;
        }
        break;
    }
    (k + operands).min(args.len())
}

/// The text a command writes to its standard output, when it is known (for a file the line writes and then runs).
pub(super) fn writes_text(_w: &Walk<'_>, words: &[Exp], input: &Input) -> Option<String> {
    let program = program_name(&words[0].text);
    match program.as_str() {
        "cat" | "tee" if words.len() == 1 || words[1..].iter().all(|a| a.text.starts_with('-') || program == "tee") => {
            match input {
                Input::Text(t) => Some(t.clone()),
                _ => None,
            }
        }
        "echo" => {
            let args: Vec<&str> =
                words[1..].iter().map(|a| a.text.as_str()).filter(|t| !matches!(*t, "-n" | "-e" | "-E")).collect();
            Some(format!("{}\n", args.join(" ")))
        }
        "printf" => words.get(1).map(|f| f.text.replace("\\n", "\n").replace("\\t", "\t")),
        _ => None,
    }
}

/// What `program`'s output looks like to the next command in a pipe.
fn output(program: &str, args: &[Exp]) -> Input {
    match program {
        "curl" | "wget" | "http" | "https" | "xh" | "fetch" | "iwr" | "invoke-webrequest" | "irm"
        | "invoke-restmethod" => {
            let host = args.iter().find_map(|a| url_host(&a.text)).unwrap_or_else(|| "the network".to_owned());
            Input::Downloaded(host)
        }
        "base64" | "base32" | "xxd" | "openssl" | "gpg" | "age" | "gunzip" | "zcat" | "unxz" | "xz" | "bzip2"
        | "gzip" | "rot13" | "uudecode" => {
            if program == "base64" || program == "base32" {
                if short(args, 'd') || has(args, "--decode") {
                    return Input::Decoded;
                }
                return Input::Opaque(program.to_owned());
            }
            Input::Decoded
        }
        "echo" => Input::Text(format!(
            "{}\n",
            args.iter()
                .map(|a| a.text.as_str())
                .filter(|t| !matches!(*t, "-n" | "-e" | "-E"))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        _ => Input::Opaque(format!("the output of {program}")),
    }
}

/// The host of a URL (`https://x.example/y` → `x.example`), or of `user@host:path`.
#[must_use]
pub fn url_host(text: &str) -> Option<String> {
    let rest = text.split_once("://").map(|(_, r)| r)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') {
        host.split(']').next().map(|h| format!("{h}]"))?
    } else {
        host.split(':').next()?.to_owned()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Walks one program with its words (the program first, wrappers included).
#[allow(clippy::too_many_lines, reason = "one arm per program family")]
#[allow(clippy::similar_names, reason = "the program's `args` and inline code's `argv`")]
#[allow(clippy::many_single_char_names, reason = "w(alk), p(rogram), k, f, t as in every arm")]
pub(super) fn run(w: &mut Walk<'_>, words: &[Exp], input: Input) -> Input {
    let program = program_name(&words[0].text);
    let lower = program.to_ascii_lowercase();
    let args = &words[1..];
    let runs_from_path = words[0].text.contains('/');
    if let Some(out) = special(w, &lower, args) {
        return out;
    }
    // Programs whose options are closed: any option this model doesn't know makes the request unreadable. Wrappers'
    // options are only theirs (up to the program they run).
    if let Some((_, flags, values, long)) = OPTIONS.iter().find(|(p, ..)| *p == lower) {
        if WRAPPERS.contains(&lower.as_str()) {
            // Up to the program it runs (a value-taking option's value is not the program).
            let own = after_options(args, values, &[], 0).min(args.len());
            known_options(w, &lower, &args[..own], flags, values, long);
        } else {
            known_options(w, &lower, args, flags, values, long);
        }
    }
    match lower.as_str() {
        // ---- wrappers: what they run is read
        "sudo" | "doas" | "run0" | "pkexec" => {
            let at =
                after_options(args, "ugChpUrtDR", &["user", "group", "host", "prompt", "chdir", "role", "type"], 0);
            if at < args.len() {
                return run(w, &args[at..], input);
            }
            Input::None
        }
        "env" => {
            if let Some(k) = args.iter().position(|a| a.text == "-S" || a.text.starts_with("--split-string")) {
                let line: Vec<String> = args[k + 1..].iter().map(|a| a.text.clone()).collect();
                w.run_script(&line.join(" "), Vec::new());
                return Input::Opaque("env -S".to_owned());
            }
            let at = after_options(args, "uCP", &["unset", "chdir"], 0);
            for a in &args[..at] {
                if let Some((n, v)) = a.text.split_once('=')
                    && !n.starts_with('-')
                {
                    w.set_var(n, a.known().then(|| v.to_owned()));
                }
            }
            if at < args.len() {
                return run(w, &args[at..], input);
            }
            // Printed: the environment holds tokens.
            secret_to_agent(w, "the environment");
            Input::Opaque("the environment".to_owned())
        }
        "printenv" => {
            let names: Vec<&Exp> = args.iter().filter(|a| !a.text.starts_with('-')).collect();
            if names.is_empty() || names.iter().any(|n| !n.known() || secret_name(&n.text)) {
                secret_to_agent(w, "the environment");
            }
            Input::Opaque("the environment".to_owned())
        }
        "set" | "export" | "declare" | "typeset"
            if args.is_empty()
                || (lower != "set" && args.iter().all(|a| matches!(a.text.as_str(), "-p" | "-x" | "-px"))) =>
        {
            secret_to_agent(w, "the shell's variables");
            Input::Opaque("the environment".to_owned())
        }
        "nice" | "nohup" | "time" | "command" | "builtin" | "exec" | "setsid" | "stdbuf" | "ionice" | "chrt"
        | "caffeinate" | "unbuffer" | "catchsegv" | "torsocks" | "proxychains" | "proxychains4" | "systemd-run"
        | "strace" | "ltrace" | "valgrind" | "rr" | "firejail" | "bwrap" => {
            if lower == "command" && (has(args, "-v") || has(args, "-V")) {
                return Input::Opaque("a lookup".to_owned());
            }
            if matches!(lower.as_str(), "strace" | "ltrace")
                && args.iter().any(|a| a.text == "-p" || a.text.starts_with("-p"))
            {
                secret_to_agent(w, "another process's system calls and data");
            }
            let at = after_options(
                args,
                "nfoaioecpkSsEuUgGtT",
                &["adjustment", "format", "output", "unit", "uid", "gid", "property"],
                0,
            );
            if at < args.len() {
                return run(w, &args[at..], input);
            }
            Input::None
        }
        "timeout" | "taskset" | "chroot" | "direnv" => {
            let ops = if lower == "direnv" {
                2
            } else {
                1
            };
            let at = after_options(args, "skc", &["signal", "kill-after"], ops);
            if at < args.len() {
                return run(w, &args[at..], input);
            }
            Input::None
        }
        "watch" => {
            let at = after_options(args, "nd", &["interval"], 0);
            let line: Vec<String> = args[at..].iter().map(|a| a.text.clone()).collect();
            w.run_script(&line.join(" "), Vec::new());
            Input::Opaque("watch".to_owned())
        }
        "flock" => {
            if let Some(c) = opt_value(args, &["-c", "--command"]) {
                let c = c.text.clone();
                w.run_script(&c, Vec::new());
                return Input::None;
            }
            let at = after_options(args, "wE", &["wait", "timeout", "conflict-exit-code"], 1);
            if at < args.len() {
                return run(w, &args[at..], input);
            }
            Input::None
        }
        "xargs" | "parallel" => {
            let at = after_options(
                args,
                "IinPLdEase",
                &["replace", "max-args", "max-procs", "max-lines", "delimiter", "eof", "arg-file", "jobs"],
                0,
            );
            let mut cmd: Vec<Exp> = if at < args.len() {
                args[at..].to_vec()
            } else {
                vec![Exp::lit("echo")]
            };
            match &input {
                Input::Text(t) => {
                    let words: Vec<&str> = t.split_whitespace().collect();
                    if words.len() > 64 {
                        w.lower(Certainty::None, "more words on xargs's input than Reins reads");
                    }
                    cmd.extend(words.into_iter().take(64).map(Exp::lit));
                }
                _ => cmd.push(super::shell::unknown("the words xargs reads from its input".to_owned())),
            }
            run(w, &cmd, Input::None)
        }
        "infisical" | "op" | "doppler" | "dotenv" | "dotenvx" | "chamber" | "aws-vault" | "teller" | "sops"
        | "berglas" => {
            let is_run = args.first().is_some_and(|a| matches!(a.text.as_str(), "run" | "exec"));
            if lower == "op"
                && args.first().is_some_and(|a| matches!(a.text.as_str(), "read" | "item" | "inject" | "document"))
            {
                secret_to_agent(w, "a 1Password secret");
                return Input::Opaque("a secret".to_owned());
            }
            if is_run || lower == "dotenv" || lower == "dotenvx" {
                w.effect(Effect::SecretUse {
                    secret: format!("secrets from {lower}"),
                    local_only: true,
                });
                if let Some(k) = args.iter().position(|a| a.text == "--") {
                    return run(w, &args[k + 1..], input);
                }
                return Input::None;
            }
            if args.iter().any(|a| matches!(a.text.as_str(), "get" | "export" | "secrets" | "read" | "decrypt" | "-d"))
            {
                secret_to_agent(w, &format!("secrets from {lower}"));
                return Input::Opaque("a secret".to_owned());
            }
            unknown_program(w, &lower);
            Input::None
        }
        "uv" | "poetry" | "pipenv" | "bundle" | "pnpm" | "npx" | "bunx" | "uvx" | "pipx" if is_runner(&lower, args) => {
            // The runner's own effect, then the program it runs on top of it. `npx`, `bunx`, `uvx`, `pipx run` and
            // `pnpm dlx` fetch the package they run; the others run the project's own environment.
            let fetches = matches!(lower.as_str(), "npx" | "bunx" | "uvx" | "pipx")
                || args.first().is_some_and(|a| a.text == "dlx");
            let start = runner_start(&lower, args);
            w.effect(Effect::Exec {
                program: args.get(start).map_or_else(|| lower.clone(), |a| format!("{lower} {}", a.text)),
                source: if fetches {
                    CodeSource::Downloaded
                } else {
                    CodeSource::Workspace
                },
            });
            if start < args.len() {
                return run(w, &args[start..], input);
            }
            w.lower(Certainty::None, format!("what {lower} runs"));
            Input::None
        }

        // ---- shells and code
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "ash" | "mksh" | "busybox" => {
            if let Some(k) = args.iter().position(|a| {
                a.text == "-c"
                    || (a.text.starts_with('-')
                        && !a.text.starts_with("--")
                        && a.text.ends_with('c')
                        && a.text.len() <= 4)
            }) {
                match args.get(k + 1) {
                    Some(script) if script.known() => {
                        let rest = args[k + 2..].to_vec();
                        let text = script.text.clone();
                        w.run_script(&text, rest.into_iter().skip(1).collect());
                    }
                    Some(script) if script.text.trim_matches(UNKNOWN).trim().is_empty() => {
                        w.note(script);
                        w.lower(Certainty::None, "a shell runs a script that is not in the command");
                    }
                    Some(script) => {
                        // A script with values from elsewhere in it: read with those values unknown.
                        w.note(script);
                        let text = script.text.replace(UNKNOWN, "\"$__reins_unknown__\"");
                        let rest = args[k + 2..].to_vec();
                        w.run_script(&text, rest.into_iter().skip(1).collect());
                        w.lower(Certainty::None, "a shell runs a script with values that are not in the command");
                    }
                    None => w.lower(Certainty::None, "a shell's -c without a script"),
                }
                return Input::Opaque("a shell".to_owned());
            }
            // `sh -s -- args`: the script comes from the input, the words are its arguments.
            let from_input = args.iter().take_while(|a| a.text != "--").any(|a| {
                a.text == "-s"
                    || (a.text.starts_with('-')
                        && !a.text.starts_with("--")
                        && a.text.contains('s')
                        && a.text.len() <= 4)
            });
            let file = positional(args, &["-o", "+o", "-O", "+O"])
                .into_iter()
                .next()
                .filter(|a| !from_input && a.text != "-s" && a.text != "-");
            match file {
                Some(f) => run_file(w, f, &args[args.iter().position(|a| a == f).unwrap_or(0) + 1..], Lang::Shell),
                None => run_input(w, input, Lang::Shell, &lower, &[]),
            }
            Input::Opaque("a shell".to_owned())
        }
        p if inline::interpreter(p).is_some() => {
            let lang = inline::interpreter(p).unwrap_or(Lang::Python);
            let code_flag = match lang {
                Lang::Python => ["-c"].as_slice(),
                Lang::Node => &["-e", "--eval", "-p", "--print"],
                Lang::Ruby | Lang::Perl => &["-e", "-E"],
                Lang::Shell => &["-c"],
            };
            if let Some(code) = opt_value(args, code_flag) {
                let argv: Vec<Exp> = args.iter().skip_while(|a| a.text != code.text).skip(1).cloned().collect();
                if code.known() {
                    let code = code.text.clone();
                    inline::analyze_code(w, lang, &code, &argv);
                } else if code.text.trim_matches(UNKNOWN).trim().is_empty() {
                    // The whole program is a value from elsewhere.
                    w.note(code);
                    w.lower(Certainty::None, format!("{p} runs code that is not in the command"));
                } else {
                    // Code with values from elsewhere in it: what can be read is shown, but code that isn't all in
                    // the command is never read with certainty (the floor stays `None`).
                    w.note(code);
                    let text = code.text.replace(UNKNOWN, "__reins_unknown__");
                    inline::analyze_code(w, lang, &text, &argv);
                    w.lower(Certainty::None, format!("{p} runs code with values that are not in the command"));
                }
                return Input::Opaque(p.to_owned());
            }
            if lang == Lang::Perl && args.iter().any(|a| a.text.starts_with("-i") || a.text.starts_with("-pi")) {
                for f in positional(args, &["-e", "-E", "-I", "-M"]).into_iter().skip(1) {
                    let path = w.path(f);
                    w.write(path);
                }
                return Input::None;
            }
            if let Some(module) = opt_value(args, &["-m"]) {
                let m = module.text.clone();
                match m.as_str() {
                    "pip" | "venv" | "pytest" | "unittest" | "build" | "twine" | "mypy" | "black" | "ruff"
                    | "json.tool" | "compileall" | "py_compile" | "http.server" | "pydoc" | "timeit" | "cProfile"
                    | "zipfile" | "tarfile" | "ensurepip" | "pipx" | "uv" | "coverage" | "doctest" | "tabnanny" => {
                        if m == "twine" && has(args, "upload") {
                            w.effect(Effect::Publish {
                                target: "a Python package".to_owned(),
                            });
                        } else {
                            w.effect(Effect::Exec {
                                program: format!("{p} -m {m}"),
                                source: CodeSource::Toolchain,
                            });
                        }
                    }
                    _ => unknown_program(w, &format!("{p} -m {m}")),
                }
                return Input::None;
            }
            let value_opts: &[&str] = match lang {
                Lang::Python => &["-W", "-X", "--check-hash-based-pycs"],
                Lang::Node => {
                    &["-r", "--require", "--import", "--loader", "--conditions", "-C", "--env-file", "--inspect-port"]
                }
                Lang::Ruby => &["-I", "-r", "-C", "-E", "-F", "-K", "-T", "-x"],
                Lang::Perl => &["-I", "-M", "-m", "-x"],
                Lang::Shell => &[],
            };
            let script = positional(args, value_opts).into_iter().next();
            let rest_at = script.and_then(|s| args.iter().position(|a| a == s)).map_or(args.len(), |k| k + 1);
            match script {
                Some(s) if s.text != "-" && s.text != "/dev/stdin" => run_file(w, s, &args[rest_at..], lang),
                _ => run_input(w, input, lang, p, &args[rest_at..]),
            }
            Input::Opaque(p.to_owned())
        }
        "eval" => {
            let text: Vec<&Exp> = args.iter().collect();
            if text.iter().all(|a| a.known()) {
                let line = text.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(" ");
                w.run_script(&line, Vec::new());
            } else {
                w.lower(Certainty::None, "eval runs text with values not known");
            }
            Input::None
        }
        "source" | "." => {
            if let Some(f) = args.first() {
                run_file(w, f, &args[1..], Lang::Shell);
            }
            Input::None
        }
        "trap" => {
            if let Some(action) = args.first().filter(|a| !a.text.starts_with('-'))
                && action.known()
            {
                let text = action.text.clone();
                w.run_script(&text, Vec::new());
            }
            Input::None
        }
        "cd" | "pushd" => {
            let before = w.cwd.clone();
            let target = args.iter().find(|a| !a.text.starts_with('-') || a.text == "-");
            match target {
                // `cd -`: back to the folder before; `pushd` with no folder swaps the top two.
                Some(t) if t.text == "-" => w.cwd = std::mem::take(&mut w.oldpwd),
                Some(t) if t.text.starts_with('+') && lower == "pushd" => w.cwd.clear(),
                Some(t) if t.known() => {
                    w.cwd = super::workspace::normalize(&t.text, &before, &w.ctx.workspace.home);
                }
                Some(t) => {
                    w.note(t);
                    w.cwd.clear();
                }
                None if lower == "pushd" => match w.dirs.pop() {
                    Some(top) => {
                        w.dirs.push(before.clone());
                        w.cwd = top;
                    }
                    None => w.cwd.clear(),
                },
                None => w.cwd.clone_from(&w.ctx.workspace.home),
            }
            if lower == "pushd" && target.is_some() {
                w.dirs.push(before.clone());
            }
            w.oldpwd = before;
            Input::None
        }
        "popd" => {
            let before = w.cwd.clone();
            if args.iter().any(|a| a.text != "-n") {
                // `popd +N`, `popd -N`: a folder further down the stack.
                w.cwd.clear();
                w.dirs.clear();
            } else if args.iter().any(|a| a.text == "-n") {
                w.dirs.pop();
            } else {
                // An empty stack: popd fails and the folder stays; a stack not known: neither is the folder.
                w.cwd = w.dirs.pop().unwrap_or_else(|| {
                    if w.dirs_known {
                        before.clone()
                    } else {
                        String::new()
                    }
                });
            }
            w.oldpwd = before;
            Input::None
        }
        "unset" => {
            let functions = args.iter().any(|a| a.text == "-f");
            for a in args.iter().filter(|a| !a.text.starts_with('-')) {
                if !a.known() {
                    w.lower(Certainty::Partial, "a variable unset by a name not known");
                } else if !functions {
                    w.unset(&a.text);
                }
            }
            Input::None
        }
        "export" | "declare" | "typeset" | "local" | "readonly" => {
            for a in args.iter().filter(|a| !a.text.starts_with('-')) {
                if let Some((n, v)) = a.text.split_once('=') {
                    w.set_var(n, a.known().then(|| v.to_owned()));
                }
            }
            Input::None
        }
        "shift" => {
            let n = args.first().and_then(|a| a.text.parse::<usize>().ok()).unwrap_or(1);
            w.shift(n);
            Input::None
        }
        "read" | "mapfile" | "readarray" | "getopts" => {
            for a in positional(args, &["-p", "-d", "-n", "-N", "-t", "-u", "-a"]) {
                w.forget(&a.text);
            }
            Input::None
        }
        "mktemp" => {
            let texts: Vec<String> = args.iter().map(|a| a.text.clone()).collect();
            let _path: String = w.mktemp(&texts);
            Input::Opaque("a new path".to_owned())
        }

        // ---- files
        "rm" | "unlink" | "rmdir" | "shred" | "srm" | "trash" | "trash-put" | "gio" => {
            if lower == "gio" && args.first().is_none_or(|a| a.text != "trash" && a.text != "remove") {
                return Input::None;
            }
            let recursive = lower == "rmdir" || short(args, 'r') || short(args, 'R') || has(args, "--recursive");
            let targets: Vec<&Exp> = positional(args, &[])
                .into_iter()
                .filter(|a| !(lower == "gio" && matches!(a.text.as_str(), "trash" | "remove")))
                .collect();
            let paths: Vec<_> = targets
                .iter()
                .map(|t| {
                    let mut p = w.path(t);
                    // A worktree's root (`rm -rf .` there): its history goes with it.
                    if recursive && w.ctx.workspace.holds_worktree(&p.path) {
                        p.recoverable = false;
                    }
                    p
                })
                .collect();
            if !paths.is_empty() {
                w.effect(Effect::FileDelete {
                    paths,
                    recursive,
                });
            }
            Input::None
        }
        "mv" | "cp" | "ln" | "install" | "rsync" | "scp" | "ditto" => copy_like(w, &lower, args),
        "mkdir" | "touch" => {
            for t in positional(args, &["-m", "--mode", "-t", "-d", "-r", "--date", "--reference"]) {
                let p = w.path(t);
                if !matches!(p.area, Area::Generated | Area::Scratch | Area::Source | Area::Home | Area::Unknown) {
                    w.write(p);
                }
            }
            Input::None
        }
        "chmod" | "chown" | "chgrp" | "chattr" | "setfacl" | "truncate" => {
            let value_opts: &[&str] = if lower == "truncate" {
                &["-s", "--size", "-r", "--reference"]
            } else {
                &["--reference"]
            };
            let mut targets = positional(args, value_opts);
            // `chmod -x f`: the mode is the dash word, and every positional word is a file.
            let dash_mode = lower == "chmod"
                && args.iter().any(|a| {
                    a.text.len() > 1 && a.text.starts_with('-') && a.text[1..].chars().all(|c| "rwxXst".contains(c))
                });
            if lower != "truncate" && !dash_mode && !targets.is_empty() {
                targets.remove(0);
            }
            for t in targets {
                let p = w.path(t);
                if lower == "truncate" {
                    w.write(p);
                } else {
                    // A mode or owner change leaves what the file says as it was.
                    w.write_keeping_content(p);
                }
            }
            Input::None
        }
        "tee" => {
            for t in positional(args, &[]) {
                let p = w.path(t);
                if let (true, Input::Text(text)) = (t.known(), &input) {
                    w.files.insert(p.path.clone(), Some(text.clone()));
                }
                w.write(p);
            }
            input
        }
        "dd" => {
            // Operands are `key=value`; any other is not read.
            for a in args {
                let key = a.text.split_once('=').map(|(k, _)| k);
                if !key.is_some_and(|k| {
                    matches!(
                        k,
                        "if" | "of"
                            | "bs"
                            | "ibs"
                            | "obs"
                            | "cbs"
                            | "count"
                            | "skip"
                            | "seek"
                            | "iseek"
                            | "oseek"
                            | "conv"
                            | "status"
                            | "iflag"
                            | "oflag"
                    )
                }) {
                    w.lower(Certainty::None, format!("the dd operand {}", a.text));
                }
            }
            for a in args {
                if let Some(of) = a.text.strip_prefix("of=") {
                    let e = Exp {
                        text: of.to_owned(),
                        ..a.clone()
                    };
                    if of.starts_with("/dev/") && !matches!(of, "/dev/null" | "/dev/stdout" | "/dev/stderr") {
                        w.effect(Effect::Infra {
                            target: format!("the disk {of}"),
                            change: InfraChange::Destroy,
                            local: false,
                        });
                    } else {
                        let p = w.path(&e);
                        w.write(p);
                    }
                }
                if let Some(input_file) = a.text.strip_prefix("if=") {
                    let e = Exp {
                        text: input_file.to_owned(),
                        ..a.clone()
                    };
                    w.read_file(&e);
                }
            }
            Input::Opaque("dd".to_owned())
        }
        p if p.starts_with("mkfs")
            || matches!(
                p,
                "wipefs"
                    | "fdisk"
                    | "sfdisk"
                    | "parted"
                    | "gdisk"
                    | "sgdisk"
                    | "mkswap"
                    | "cryptsetup"
                    | "blkdiscard"
                    | "format"
                    | "diskutil"
            ) =>
        {
            let target =
                positional(args, &["-t", "-L", "-n"]).last().map_or_else(|| "a disk".to_owned(), |a| a.text.clone());
            if matches!(p, "diskutil") && args.first().is_some_and(|a| matches!(a.text.as_str(), "list" | "info")) {
                return Input::None;
            }
            w.effect(Effect::Infra {
                target: format!("the disk {target}"),
                change: InfraChange::Destroy,
                local: false,
            });
            Input::None
        }
        "sed" | "gsed" => {
            // Scripts from `-e x`, `-ex`, `--expression=x`, `--expression x`; script files from `-f x`, `-fx`,
            // `--file=x`; the first word when none is given.
            let mut scripts: Vec<Exp> = Vec::new();
            let mut script_files: Vec<Exp> = Vec::new();
            let mut files: Vec<Exp> = Vec::new();
            let mut in_place = false;
            let mut options = true;
            let mut k = 0;
            while k < args.len() {
                let a = &args[k];
                let t = a.text.as_str();
                k += 1;
                if !options || !t.starts_with('-') || t == "-" {
                    files.push(a.clone());
                    continue;
                }
                if t == "--" {
                    options = false;
                    continue;
                }
                if let Some(long) = t.strip_prefix("--") {
                    let (name, value) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
                    let take = |k: &mut usize| {
                        value
                            .map(|v| Exp {
                                text: v.to_owned(),
                                ..a.clone()
                            })
                            .or_else(|| {
                                *k += 1;
                                args.get(*k - 1).cloned()
                            })
                    };
                    match name {
                        "expression" => scripts.extend(take(&mut k)),
                        "file" => script_files.extend(take(&mut k)),
                        "in-place" => in_place = true,
                        "line-length" => {
                            if value.is_none() {
                                k += 1;
                            }
                        }
                        "quiet" | "silent" | "regexp-extended" | "null-data" | "zero-terminated" | "separate"
                        | "unbuffered" | "posix" | "debug" | "sandbox" | "follow-symlinks" | "help" | "version" => {}
                        other => w.lower(Certainty::None, format!("the option --{other} of sed")),
                    }
                    continue;
                }
                let short = &t[1..];
                for (i, ch) in short.char_indices() {
                    let rest = &short[i + ch.len_utf8()..];
                    let value = || {
                        if rest.is_empty() {
                            args.get(k).cloned()
                        } else {
                            Some(Exp {
                                text: rest.to_owned(),
                                ..a.clone()
                            })
                        }
                    };
                    match ch {
                        'n' | 'r' | 'E' | 's' | 'u' | 'z' => {}
                        // `-i` takes an attached suffix only.
                        'i' => {
                            in_place = true;
                            break;
                        }
                        'e' | 'f' | 'l' => {
                            let v = value();
                            if rest.is_empty() {
                                k += 1;
                            }
                            match ch {
                                'e' => scripts.extend(v),
                                'f' => script_files.extend(v),
                                _ => {}
                            }
                            break;
                        }
                        other => {
                            w.lower(Certainty::None, format!("the option -{other} of sed"));
                            break;
                        }
                    }
                }
            }
            if scripts.is_empty() && script_files.is_empty() && !files.is_empty() {
                scripts.push(files.remove(0));
            }
            for f in &script_files {
                let p = w.path(f);
                match w.files.get(&p.path).cloned().flatten() {
                    Some(text) => scripts.push(Exp::lit(&text)),
                    None => w.lower(Certainty::None, format!("the sed script in {}", f.text)),
                }
            }
            for sc in &scripts {
                if !sc.known() {
                    w.lower(Certainty::None, "a sed script with values that are not in the command");
                }
                let (runs, writes, reads) = sed_script(&sc.text);
                if runs {
                    w.lower(Certainty::None, "a sed script that runs commands");
                }
                for f in writes {
                    let p = w.path(&Exp::lit(&f));
                    w.write(p);
                }
                for f in reads {
                    w.read_file(&Exp::lit(&f));
                }
            }
            for f in &files {
                if in_place {
                    let p = w.path(f);
                    w.write(p);
                } else {
                    w.read_file(f);
                }
            }
            Input::Opaque("sed".to_owned())
        }
        "find" | "gfind" => find(w, args),
        "tar" | "bsdtar" | "gtar" => {
            let operation =
                args.iter().take(1).any(|a| !a.text.starts_with('-') && a.text.chars().any(|c| "cxtruAd".contains(c)))
                    || args.iter().any(|a| {
                        (a.text.starts_with('-')
                            && !a.text.starts_with("--")
                            && a.text.chars().any(|c| "cxtruAd".contains(c)))
                            || matches!(
                                a.text.as_str(),
                                "--create"
                                    | "--extract"
                                    | "--get"
                                    | "--list"
                                    | "--append"
                                    | "--update"
                                    | "--diff"
                                    | "--compare"
                                    | "--delete"
                                    | "--catenate"
                                    | "--concatenate"
                            )
                    });
            if !operation {
                w.lower(Certainty::None, "a tar command without an operation it knows");
            }
            // Options that run a program: not read.
            if args.iter().any(|a| {
                let t = a.text.as_str();
                [
                    "--to-command",
                    "--use-compress-program",
                    "--checkpoint-action",
                    "--info-script",
                    "--new-volume-script",
                    "--rsh-command",
                    "--rmt-command",
                ]
                .iter()
                .any(|o| t.starts_with(o))
                    || t == "-I"
                    || t == "-F"
            }) {
                w.lower(Certainty::None, "tar runs a program it is given");
            }
            let mut opts: Vec<Exp> = args.to_vec();
            // `tar czf x`: the first word is a group of flags without its dash.
            if let Some(first) = opts.first_mut()
                && !first.text.starts_with('-')
                && first.text.chars().all(|c| c.is_ascii_alphabetic())
            {
                first.text = format!("-{}", first.text);
            }
            known_options(
                w,
                &lower,
                &opts,
                "cxtrudAvzjJaZpkmOPShHlWBiGRMUw",
                "fCTXbLgNKVH",
                &[
                    "create",
                    "extract",
                    "get",
                    "list",
                    "append",
                    "update",
                    "diff",
                    "compare",
                    "delete",
                    "file",
                    "directory",
                    "verbose",
                    "gzip",
                    "gunzip",
                    "bzip2",
                    "xz",
                    "zstd",
                    "lzma",
                    "auto-compress",
                    "exclude",
                    "exclude-from",
                    "exclude-vcs",
                    "exclude-caches",
                    "strip-components",
                    "to-stdout",
                    "no-same-owner",
                    "same-owner",
                    "no-same-permissions",
                    "preserve-permissions",
                    "overwrite",
                    "keep-old-files",
                    "skip-old-files",
                    "wildcards",
                    "no-wildcards",
                    "transform",
                    "xform",
                    "null",
                    "files-from",
                    "owner",
                    "group",
                    "mtime",
                    "sort",
                    "numeric-owner",
                    "format",
                    "warning",
                    "totals",
                    "one-file-system",
                    "dereference",
                    "absolute-names",
                    "show-transformed-names",
                    "anchored",
                    "recursion",
                    "no-recursion",
                    "ignore-zeros",
                    "occurrence",
                    "mode",
                    "atime-preserve",
                    "touch",
                    "unlink-first",
                    "recursive-unlink",
                    "help",
                    "version",
                    "label",
                    "blocking-factor",
                    "record-size",
                    "hard-dereference",
                    "check-links",
                    "selinux",
                    "acls",
                    "xattrs",
                    "no-xattrs",
                    "no-acls",
                    "pax-option",
                    "posix",
                    "sparse",
                    "level",
                    "quiet",
                    "force-local",
                ],
            );
            let create = args.first().is_some_and(|a| !a.text.starts_with('-') && a.text.contains('c'))
                || short(args, 'c')
                || has(args, "--create");
            let extract = args.first().is_some_and(|a| !a.text.starts_with('-') && a.text.contains('x'))
                || short(args, 'x')
                || has(args, "--extract");
            if let Some(f) = opt_value(args, &["-f", "--file"]).filter(|f| f.text != "-") {
                if create {
                    let p = w.path(f);
                    w.write(p);
                } else {
                    w.read_file(f);
                }
            }
            if extract {
                let dir = opt_value(args, &["-C", "--directory"]).cloned().unwrap_or_else(|| Exp::lit("."));
                let p = w.path(&dir);
                w.write(p);
            }
            Input::Opaque("tar".to_owned())
        }
        "unzip" | "7z" | "7za" | "unrar" | "gunzip" | "unxz" | "bunzip2" | "zstd" | "unzstd" | "zip" | "gzip"
        | "xz" | "bzip2" => {
            if lower == "unzip" && short(args, 'l') {
                return Input::None;
            }
            if args.iter().any(|a| a.text == "-TT" || a.text.starts_with("--unzip-command")) {
                w.lower(Certainty::None, format!("{lower} runs a program it is given"));
            }
            if matches!(lower.as_str(), "zip" | "unzip" | "7z" | "7za" | "unrar")
                && !args.iter().any(|a| !a.text.starts_with('-'))
            {
                w.lower(Certainty::None, format!("{lower} without an archive"));
            }
            let dest = opt_value(args, &["-d"]).cloned().or_else(|| {
                if lower == "7z" || lower == "7za" {
                    args.iter().find_map(|a| a.text.strip_prefix("-o").filter(|d| !d.is_empty()).map(Exp::lit))
                } else {
                    None
                }
            });
            match dest {
                Some(d) => {
                    let p = w.path(&d);
                    w.write(p);
                }
                None => {
                    if lower == "zip" {
                        if let Some(z) = positional(args, &["-x", "-i"]).first() {
                            let p = w.path(z);
                            w.write(p);
                        }
                    } else {
                        let p = w.path(&Exp::lit("."));
                        w.write(p);
                    }
                }
            }
            Input::Opaque(lower)
        }

        // ---- git and forges
        "git" => super::git::shell_git(w, args),
        "gh" => super::git::shell_gh(w, args),

        // ---- network
        "curl" | "wget" | "http" | "https" | "xh" | "httpie" | "aria2c" => network(w, &lower, args),
        "ssh" | "mosh" | "autossh" => ssh(w, args, input),
        "nc" | "ncat" | "netcat" | "telnet" | "websocat" | "openssl"
            if lower != "openssl" || args.first().is_some_and(|a| a.text == "s_client") =>
        {
            let host = positional(args, &["-p", "-w", "-s", "-e", "-c", "-connect"])
                .first()
                .map_or_else(|| "a host".to_owned(), |a| a.text.clone());
            let method = if matches!(input, Input::None) {
                NetMethod::Connect
            } else {
                NetMethod::Write
            };
            w.effect(Effect::Network {
                host,
                method,
            });
            Input::Opaque(lower)
        }
        "openssl" => {
            let known = [
                "version", "rand", "dgst", "x509", "req", "verify", "genrsa", "genpkey", "rsa", "pkey", "ec",
                "ecparam", "enc", "base64", "smime", "cms", "pkeyutl", "rsautl", "pkcs12", "crl", "list", "help",
                "sha256", "sha1", "md5", "speed",
            ];
            if !args.first().is_some_and(|a| known.contains(&a.text.as_str())) {
                unmodelled(w, "openssl", args.first().map_or("", |a| a.text.as_str()));
                return Input::Opaque("openssl".to_owned());
            }
            if args
                .first()
                .is_some_and(|a| matches!(a.text.as_str(), "enc" | "base64" | "smime" | "cms" | "rsautl" | "pkeyutl"))
            {
                return Input::Decoded;
            }
            for f in args.iter().filter(|a| a.text.contains('/') || a.text.contains('.')) {
                if let FileKind::Secret(_) = file_kind(&f.text) {
                    w.effect(Effect::SecretUse {
                        secret: f.text.clone(),
                        local_only: true,
                    });
                }
            }
            Input::Opaque("openssl".to_owned())
        }

        // ---- databases and infrastructure
        "psql" | "mysql" | "mariadb" | "sqlite3" | "sqlite" | "duckdb" | "clickhouse-client" | "cockroach"
        | "mongosh" | "mongo" | "redis-cli" | "pg_dump" | "pg_restore" | "dropdb" | "createdb" | "mysqldump"
        | "mysqladmin" => super::infra::database(w, &lower, args, &input),
        // Clusters, clouds, containers and infrastructure as code are not modelled: they are asked (certainty
        // `None`), which is safe; a model is added when the replay shows it matters.
        p if SYSTEM_PACKAGES.contains(&p) => {
            let read_only = args.first().is_none_or(|a| {
                matches!(
                    a.text.as_str(),
                    "-Q" | "-Qi"
                        | "-Ql"
                        | "-Qs"
                        | "-Ss"
                        | "-Si"
                        | "search"
                        | "show"
                        | "list"
                        | "info"
                        | "query"
                        | "-q"
                        | "policy"
                        | "--version"
                ) || matches!(
                    a.text.as_str(),
                    "-Qe" | "-Qm" | "-Qdt" | "-Qqe" | "-Qo" | "-Qk" | "-Qu" | "-Sl" | "-Sg" | "-Qq" | "-V"
                )
            });
            if !read_only {
                w.effect(Effect::Infra {
                    target: "this computer's system packages".to_owned(),
                    change: InfraChange::Apply,
                    local: false,
                });
            }
            Input::Opaque(p.to_owned())
        }
        "useradd" | "usermod" | "userdel" | "groupadd" | "groupmod" | "passwd" | "chpasswd" | "visudo" | "adduser"
        | "deluser" | "gpasswd" | "setcap" | "chsh" => {
            if lower == "visudo" && short(args, 'c') {
                return Input::None;
            }
            w.effect(Effect::Permission {
                resource: "this computer's users".to_owned(),
                change: lower.clone(),
            });
            Input::None
        }

        // ---- package managers and builds
        "npm" | "pnpm" | "yarn" | "bun" => {
            let sub = args.iter().find(|a| !a.text.starts_with('-')).map(|a| a.text.as_str()).unwrap_or_default();
            match sub {
                "publish" | "unpublish" | "deprecate" | "dist-tag" => w.effect(Effect::Publish {
                    target: "an npm package".to_owned(),
                }),
                "owner" | "access" | "team" | "token" => w.effect(Effect::Permission {
                    resource: "an npm package".to_owned(),
                    change: sub.to_owned(),
                }),
                "npm" if has(args, "publish") => w.effect(Effect::Publish {
                    target: "an npm package".to_owned(),
                }),
                "login" | "adduser" => w.effect(Effect::SecretUse {
                    secret: "an npm token".to_owned(),
                    local_only: false,
                }),
                // The project's own scripts and builds, and installs from the registry.
                "install" | "i" | "ci" | "add" | "remove" | "rm" | "uninstall" | "update" | "up" | "run"
                | "run-script" | "test" | "t" | "start" | "build" | "lint" | "format" | "typecheck" | "check"
                | "dev" | "ls" | "list" | "why" | "outdated" | "audit" | "view" | "info" | "pack" | "version"
                | "--version" | "-v" | "help" | "prune" | "dedupe" | "config" | "get" | "bin" | "root" | "exec"
                | "cache" | "" => w.effect(Effect::Exec {
                    program: format!("{lower} {sub}").trim().to_owned(),
                    source: CodeSource::Workspace,
                }),
                // `yarn <script>`: a script of the project's package.json.
                script
                    if lower == "yarn"
                        && script.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_')) =>
                {
                    w.effect(Effect::Exec {
                        program: format!("yarn {script}"),
                        source: CodeSource::Workspace,
                    });
                }
                other => unmodelled(w, &lower, other),
            }
            Input::Opaque(lower)
        }
        "cargo" => {
            let sub = args
                .iter()
                .find(|a| !a.text.starts_with('-') && !a.text.starts_with('+'))
                .map(|a| a.text.as_str())
                .unwrap_or_default();
            match sub {
                "publish" | "yank" => w.effect(Effect::Publish {
                    target: "a crate".to_owned(),
                }),
                "owner" => w.effect(Effect::Permission {
                    resource: "a crate".to_owned(),
                    change: "owners".to_owned(),
                }),
                "login" => w.effect(Effect::SecretUse {
                    secret: "a crates.io token".to_owned(),
                    local_only: false,
                }),
                "clean" => {
                    let target = opt_value(args, &["--target-dir"])
                        .cloned()
                        .or_else(|| w.ctx.env.get("CARGO_TARGET_DIR").map(|t| Exp::lit(t)))
                        .unwrap_or_else(|| Exp::lit("target"));
                    let p = w.path(&target);
                    w.effect(Effect::FileDelete {
                        paths: vec![p],
                        recursive: true,
                    });
                }
                "build" | "b" | "test" | "t" | "check" | "c" | "clippy" | "fmt" | "run" | "r" | "doc" | "bench"
                | "tree" | "metadata" | "update" | "fetch" | "add" | "remove" | "rustc" | "rustdoc" | "nextest"
                | "llvm-cov" | "generate-lockfile" | "vendor" | "search" | "locate-project" | "pkgid"
                | "verify-project" | "version" | "--version" | "-V" | "help" | "fix" | "install" | "uninstall"
                | "init" | "new" | "package" | "" => w.effect(Effect::Exec {
                    program: format!("cargo {sub}").trim().to_owned(),
                    source: CodeSource::Workspace,
                }),
                // A third-party subcommand (`cargo foo` runs `cargo-foo`): not read.
                other => unmodelled(w, "cargo", other),
            }
            Input::Opaque("cargo".to_owned())
        }
        "pip" | "pip3" | "uv" | "mvn" | "gradle" | "gradlew" => {
            let publishes = args
                .iter()
                .any(|a| matches!(a.text.as_str(), "publish" | "upload" | "push" | "deploy" | "release" | "trunk"))
                && !(lower == "gradle" || lower == "gradlew")
                || args.iter().any(|a| {
                    a.text.starts_with("publish")
                        && (lower == "gradle" || lower == "gradlew")
                        && a.text.contains("Repository")
                        && !a.text.contains("MavenLocal")
                });
            if publishes {
                w.effect(Effect::Publish {
                    target: format!("a {lower} package or release"),
                });
            } else {
                toolchain(w, &lower, args);
            }
            Input::Opaque(lower)
        }
        "reins" => reins(w, args),
        "gpg" | "gpg2" => {
            if args.iter().any(|a| {
                matches!(
                    a.text.as_str(),
                    "--recv-keys" | "--send-keys" | "--refresh-keys" | "--search-keys" | "--keyserver"
                )
            }) {
                w.effect(Effect::Network {
                    host: "a key server".to_owned(),
                    method: NetMethod::Write,
                });
            }
            if args.iter().any(|a| {
                matches!(a.text.as_str(), "--export-secret-keys" | "--export-secret-subkeys" | "-d" | "--decrypt")
            }) {
                secret_to_agent(w, "a GPG secret");
            }
            Input::Decoded
        }
        "ssh-keygen" => {
            let reads = short(args, 'l')
                || short(args, 'y')
                || short(args, 'F')
                || short(args, 'R')
                || short(args, 'H')
                || short(args, 'Y')
                || short(args, 'L');
            match opt_value(args, &["-f"]) {
                Some(f) if reads => {
                    if short(args, 'y') || short(args, 'Y') {
                        w.read_file(f);
                    }
                }
                Some(f) => {
                    let p = w.path(f);
                    w.write(p);
                }
                // Without `-f`: a new key in ~/.ssh (asked for interactively), or the user's known hosts.
                None if !reads => {
                    let p = w.path(&Exp::lit("~/.ssh/id_new"));
                    w.write(p);
                }
                None => {}
            }
            if short(args, 'Y') && args.iter().any(|a| a.text == "sign") {
                w.effect(Effect::SecretUse {
                    secret: "an SSH key".to_owned(),
                    local_only: true,
                });
            }
            Input::Opaque("ssh-keygen".to_owned())
        }
        "ssh-add" => {
            if !(short(args, 'l') || short(args, 'L')) {
                w.effect(Effect::SecretUse {
                    secret: "an SSH key".to_owned(),
                    local_only: true,
                });
            }
            Input::None
        }
        "security" | "secret-tool" | "pass" | "gopass" | "keyring" | "kwallet-query" | "bw" | "lpass" | "vault" => {
            if args.iter().any(|a| {
                matches!(
                    a.text.as_str(),
                    "find-generic-password"
                        | "find-internet-password"
                        | "lookup"
                        | "show"
                        | "get"
                        | "read"
                        | "-w"
                        | "export"
                        | "kv"
                        | "unlock"
                )
            }) {
                secret_to_agent(w, &format!("a secret from {lower}"));
            } else if !args.is_empty() {
                unknown_program(w, &lower);
            }
            Input::Opaque("a secret".to_owned())
        }
        p if PATTERN_READERS.contains(&p) => {
            // Each program's options that take a value (`--name=value` is one word).
            let values: &[&str] = match p {
                "grep" | "egrep" | "fgrep" | "zgrep" => &[
                    "-e",
                    "-f",
                    "--regexp",
                    "--file",
                    "-A",
                    "-B",
                    "-C",
                    "-m",
                    "--max-count",
                    "-d",
                    "-D",
                    "--context",
                    "--after-context",
                    "--before-context",
                    "--include",
                    "--exclude",
                    "--exclude-dir",
                    "--exclude-from",
                    "--label",
                    "--binary-files",
                    "--directories",
                    "--devices",
                    "--color",
                    "--colour",
                ],
                "rg" | "ag" | "ack" => &[
                    "-e",
                    "-f",
                    "--regexp",
                    "--file",
                    "-A",
                    "-B",
                    "-C",
                    "-m",
                    "--max-count",
                    "-g",
                    "--glob",
                    "--iglob",
                    "-t",
                    "--type",
                    "-T",
                    "--type-not",
                    "-M",
                    "--max-columns",
                    "-j",
                    "--threads",
                    "--sort",
                    "--sortr",
                    "-r",
                    "--replace",
                    "-E",
                    "--encoding",
                    "--max-depth",
                    "-d",
                    "--max-filesize",
                    "--engine",
                    "--colors",
                    "--color",
                    "--path-separator",
                    "--context",
                    "--after-context",
                    "--before-context",
                ],
                "jq" | "yq" | "gojq" => {
                    &["-f", "--from-file", "--indent", "--arg", "--argjson", "--slurpfile", "--rawfile"]
                }
                _ => &["-F", "-v", "-f", "--field-separator", "--assign", "--file"],
            };
            let given = has(args, "-e")
                || has(args, "-f")
                || has(args, "--regexp")
                || has(args, "--file")
                || has(args, "--from-file");
            let mut files = positional(args, values);
            // `--arg name value`: two words.
            if matches!(p, "jq" | "yq" | "gojq") {
                for pair in args
                    .windows(3)
                    .filter(|w| matches!(w[0].text.as_str(), "--arg" | "--argjson" | "--slurpfile" | "--rawfile"))
                {
                    files.retain(|f| *f != &pair[2]);
                }
            }
            if !given
                && files.is_empty()
                && !args.iter().any(|a| {
                    a.text == "--files" || a.text == "--type-list" || a.text == "--version" || a.text == "--help"
                })
            {
                w.lower(Certainty::None, format!("{p} without a pattern or a program"));
            }
            if !given && !files.is_empty() {
                files.remove(0);
            }
            if matches!(p, "awk" | "gawk" | "mawk")
                && args.iter().any(|a| {
                    a.text.contains("system(")
                        || a.text.contains("getline")
                        || a.text.contains('>')
                        || a.text.contains('|')
                        || a.text.contains("inplace")
                })
                || (matches!(p, "awk" | "gawk" | "mawk") && has(args, "-f"))
            {
                w.lower(Certainty::None, "an awk program that runs commands or writes files");
            }
            if p == "rg" && args.iter().any(|a| a.text == "--pre" || a.text.starts_with("--pre=")) {
                w.lower(Certainty::None, "rg runs a preprocessor program on each file");
            }
            if matches!(p, "yq") && args.iter().any(|a| a.text == "-i" || a.text == "--inplace") {
                for f in positional(args, &["-e", "--expression"]).into_iter().skip(1) {
                    let path = w.path(f);
                    w.write(path);
                }
            }
            for f in files {
                w.read_file(f);
            }
            Input::Opaque(p.to_owned())
        }
        p if READERS.contains(&p) => {
            let values = reader_values(p);
            // `--files0-from F`: the names come from F, which is read (`du` prints its text back in errors).
            let mut lists: Vec<Exp> = Vec::new();
            for (k, a) in args.iter().enumerate() {
                if a.text == "--files0-from" {
                    lists.extend(args.get(k + 1).cloned());
                } else if let Some(f) = a.text.strip_prefix("--files0-from=") {
                    lists.push(Exp {
                        text: f.to_owned(),
                        ..a.clone()
                    });
                }
            }
            let list_words: Vec<&Exp> =
                args.windows(2).filter(|ab| ab[0].text == "--files0-from").map(|ab| &ab[1]).collect();
            let read: Vec<&Exp> = positional(args, values)
                .into_iter()
                .filter(|r| !list_words.iter().any(|l| std::ptr::eq(*l, *r)))
                .collect();
            for f in &lists {
                w.read_file(f);
                if !METADATA_READERS.contains(&p) {
                    w.read_file(&super::shell::unknown("the files a list names".to_owned()));
                }
            }
            for f in &read {
                if METADATA_READERS.contains(&p) {
                    w.read_meta(f);
                } else {
                    w.read_file(f);
                }
            }
            // Whatever else names a secret is read as one (the floor holds whatever the options are).
            for a in args {
                if !read.iter().any(|r| std::ptr::eq(*r, a)) && !a.text.starts_with('-') && a.known() {
                    let kind = file_kind(&w.path(a).path);
                    if matches!(kind, FileKind::Secret(_)) {
                        w.read_file(a);
                    }
                }
            }
            if (p == "sort" || p == "iconv")
                && let Some(o) = opt_value(args, &["-o", "--output"])
            {
                let path = w.path(o);
                w.write(path);
            }
            output(p, args)
        }
        "base64" | "base32" | "xxd" | "uudecode" => {
            for f in positional(args, &["-w", "-c", "-s", "-l"]) {
                w.read_file(f);
            }
            output(&lower, args)
        }
        p if HARMLESS.contains(&p) => {
            if matches!(p, "kill" | "pkill" | "killall") && args.iter().any(|a| a.text == "1" || a.text == "-1") {
                w.effect(Effect::Infra {
                    target: "every process".to_owned(),
                    change: InfraChange::Destroy,
                    local: true,
                });
            }
            output(p, args)
        }
        p if TOOLCHAIN.contains(&p) => {
            toolchain(w, p, args);
            Input::Opaque(p.to_owned())
        }
        _ => {
            let script = program
                .rsplit_once('.')
                .is_some_and(|(_, e)| e.eq_ignore_ascii_case("sh") || e.eq_ignore_ascii_case("py"));
            if runs_from_path || script {
                run_file(w, &words[0], args, Lang::Shell);
            } else {
                unknown_program(w, &program);
            }
            Input::Opaque(program)
        }
    }
}

/// `uv run`, `poetry run`, `bundle exec`, `pnpm exec`, `npx`: runners of another program.
fn is_runner(program: &str, args: &[Exp]) -> bool {
    match program {
        "npx" | "bunx" | "uvx" => true,
        "pipx" => args.first().is_some_and(|a| a.text == "run"),
        "bundle" => args.first().is_some_and(|a| a.text == "exec"),
        "pnpm" => args.first().is_some_and(|a| a.text == "exec" || a.text == "dlx"),
        _ => args.first().is_some_and(|a| a.text == "run"),
    }
}

fn runner_start(program: &str, args: &[Exp]) -> usize {
    let skip = usize::from(!matches!(program, "npx" | "bunx"));
    skip + after_options(
        &args[skip.min(args.len())..],
        "pw",
        &["with", "python", "package", "directory", "project", "env-file", "group", "extra"],
        0,
    )
}

/// A listener (a server, `socat TCP-LISTEN`): on this computer only when bound to it; else it makes something here
/// reachable from the network, which is publishing.
fn listener(w: &mut Walk<'_>, local: bool, what: &str) {
    if local {
        w.effect(Effect::Network {
            host: "localhost".to_owned(),
            method: NetMethod::Connect,
        });
    } else {
        w.effect(Effect::Publish {
            target: format!("{what} on this computer, reachable from the network"),
        });
    }
}

fn settings_write(w: &mut Walk<'_>, what: &str) {
    w.effect(Effect::DataWrite {
        service: "this computer".to_owned(),
        what: what.to_owned(),
        lost: true,
    });
}

fn system_change(w: &mut Walk<'_>, target: &str) {
    w.effect(Effect::Infra {
        target: target.to_owned(),
        change: InfraChange::Apply,
        local: false,
    });
}

/// A form of a program this engine does not model: certainty `None`, so it is asked (programs fail closed).
fn unmodelled(w: &mut Walk<'_>, program: &str, what: &str) {
    w.effect(Effect::Exec {
        program: program.to_owned(),
        source: CodeSource::Unknown,
    });
    w.lower(Certainty::None, format!("what `{program} {what}` does"));
}

/// Checks a program's options against the ones its model knows: short flags (`flags`), short options with a value
/// (`values`, attached or not) and long options (with or without `=value`). Any other option makes the request
/// unreadable: an option Reins doesn't know may change what the program does. Returns whether all were known.
fn known_options(w: &mut Walk<'_>, program: &str, args: &[Exp], flags: &str, values: &str, long: &[&str]) -> bool {
    let mut ok = true;
    let mut k = 0;
    while k < args.len() {
        let t = args[k].text.as_str();
        k += 1;
        if t == "--" {
            break;
        }
        if let Some(name) = t.strip_prefix("--") {
            let name = name.split_once('=').map_or(name, |(n, _)| n);
            if !long.contains(&name) {
                ok = false;
                w.lower(Certainty::None, format!("the option --{name} of {program}"));
            }
            continue;
        }
        let Some(short) = t.strip_prefix('-').filter(|s| !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit()))
        else {
            continue;
        };
        for (i, c) in short.char_indices() {
            if values.contains(c) {
                // The rest of the word, or the next word, is its value.
                if i + c.len_utf8() == short.len() {
                    k += 1;
                }
                break;
            }
            if !flags.contains(c) {
                ok = false;
                w.lower(Certainty::None, format!("the option -{c} of {program}"));
                break;
            }
        }
    }
    ok
}

/// The few programs outside the file, git, network and build families that the recorded history needs, each closed:
/// the forms it recognizes, and certainty `None` for anything else. `None`: not one of these.
#[allow(clippy::too_many_lines, reason = "one arm per program")]
fn special(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Option<Input> {
    let words: Vec<&str> = args.iter().filter(|a| !a.text.starts_with('-')).map(|a| a.text.as_str()).collect();
    let first = words.first().copied().unwrap_or_default();
    match program {
        "kill" | "pkill" | "killall" => {
            known_options(
                w,
                program,
                args,
                "9fxeinlqrvuowc",
                "sguUtPGFn",
                &[
                    "signal",
                    "full",
                    "exact",
                    "newest",
                    "oldest",
                    "list-name",
                    "list-full",
                    "echo",
                    "count",
                    "ignore-case",
                    "uid",
                    "euid",
                    "group",
                    "parent",
                    "session",
                    "terminal",
                    "pidfile",
                ],
            );
            if args.iter().any(|a| matches!(a.text.as_str(), "1" | "-1")) {
                w.effect(Effect::Infra {
                    target: "every process".to_owned(),
                    change: InfraChange::Destroy,
                    local: true,
                });
            } else {
                settings_write(w, "Stops processes");
            }
        }
        // `ps e`, `ps -E` show each process's environment.
        "ps" => {
            let bsd = args.first().filter(|a| !a.text.starts_with('-'));
            if let Some(b) = bsd {
                if !b.text.chars().all(|c| "auxwfjlvsrhTcgnmSU".contains(c) || c == 'e') {
                    w.lower(Certainty::None, format!("the ps form {}", b.text));
                } else if b.text.contains('e') {
                    secret_to_agent(w, "other processes' environments");
                }
            }
            let rest: Vec<Exp> = args.iter().skip(usize::from(bsd.is_some())).cloned().collect();
            if known_options(
                w,
                program,
                &rest,
                "AaefHjlLMTwxyFcNdr",
                "oOpuUgGtCqs",
                &[
                    "pid",
                    "ppid",
                    "user",
                    "format",
                    "sort",
                    "forest",
                    "no-headers",
                    "cols",
                    "columns",
                    "width",
                    "help",
                    "version",
                ],
            ) && has(&rest, "-E")
            {
                secret_to_agent(w, "other processes' environments");
            }
        }
        "date" => {
            known_options(
                w,
                program,
                args,
                "uRIn",
                "dfr",
                &["utc", "iso-8601", "rfc-3339", "rfc-email", "date", "reference", "universal", "set", "debug"],
            );
            if args.iter().any(|a| a.text == "-s" || a.text.starts_with("--set"))
                || words.iter().any(|x| {
                    !x.starts_with('+')
                        && !args.windows(2).any(|p| {
                            (p[0].text == "-d" || p[0].text == "-r" || p[0].text == "--date") && p[1].text == **x
                        })
                })
            {
                system_change(w, "the system clock");
            }
        }
        // `alias x='…'`: what it would run counts now; `alias x` only prints it.
        "alias" => {
            known_options(w, program, args, "p", "", &[]);
            for a in args {
                if let Some((_, body)) = a.text.split_once('=') {
                    let body = body.to_owned();
                    w.run_script(&body, Vec::new());
                }
            }
        }
        // The window manager: queries are nothing; `exec` (anywhere, also in `--batch` and after `;`) runs a program;
        // anything else is not read.
        "hyprctl" | "swaymsg" => {
            let raw = args.iter().map(|a| a.text.as_str()).collect::<Vec<_>>().join(" ");
            let commands: Vec<Vec<&str>> = raw.split(';').map(|c| c.split_whitespace().collect()).collect();
            let queries = [
                "clients",
                "monitors",
                "workspaces",
                "activewindow",
                "activeworkspace",
                "version",
                "devices",
                "layers",
                "binds",
                "splash",
                "systeminfo",
                "cursorpos",
                "instances",
                "globalshortcuts",
                "rollinglog",
                "animations",
                "decorations",
                "getoption",
                "workspacerules",
                "layouts",
                "configerrors",
                "-t",
                "get_tree",
                "get_outputs",
                "get_workspaces",
                "get_version",
                "-j",
            ];
            for cmd in commands.iter().filter(|c| !c.is_empty()) {
                let words: Vec<&str> = cmd
                    .iter()
                    .copied()
                    .filter(|x| !matches!(*x, "-j" | "--batch" | "-r" | "--instance" | "-q"))
                    .collect();
                if let Some(at) = words.iter().position(|x| matches!(*x, "exec" | "execr" | "exec-once")) {
                    let line = words[at + 1..].join(" ");
                    let known = args.iter().all(Exp::known);
                    w.run_script(&line, Vec::new());
                    if !known {
                        w.lower(
                            Certainty::None,
                            format!("a program {program} runs with values that are not in the command"),
                        );
                    }
                } else if words.first().is_none_or(|x| queries.contains(x)) {
                } else if words.first().is_some_and(|x| *x == "dispatch")
                    && words.len() <= 4
                    && args.iter().all(|a| !a.text.contains(['"', '\'', '(', '{']))
                {
                    // A window action (focus, move, resize): nothing lasting.
                } else if program == "hyprctl"
                    && words.first().is_some_and(|x| *x == "dispatch")
                    && args.iter().all(Exp::known)
                    && lua_window_action(&words[1..].join(" "))
                {
                    // The same, in the Lua form: one window call with literal values.
                } else if words.first().is_some_and(|x| matches!(*x, "keyword" | "reload" | "setcursor")) {
                    settings_write(w, "Changes the desktop's settings");
                } else {
                    unmodelled(w, program, &cmd.join(" "));
                }
            }
        }
        // Name lookups: they reach the network to resolve a name, nothing more.
        "dig" | "host" | "nslookup" => {
            known_options(w, program, args, "46vadrwsTCWNRlx", "pcqykt", &[]);
            for x in &words {
                let h = x.trim_start_matches('@');
                if !h.starts_with('+') {
                    w.effect(Effect::Network {
                        host: h.to_owned(),
                        method: NetMethod::Read,
                    });
                }
            }
        }
        // Read-only forms only: `-w`, `-p` and `name=value` set the kernel.
        "sysctl" => {
            known_options(
                w,
                program,
                args,
                "anNbeAqwp",
                "",
                &["all", "values", "names", "binary", "ignore", "write", "load", "system", "quiet"],
            );
            if has(args, "-w")
                || has(args, "-p")
                || has(args, "--write")
                || has(args, "--load")
                || has(args, "--system")
                || words.iter().any(|x| x.contains('='))
            {
                system_change(w, "kernel settings");
            }
        }
        // The connected Android device (an emulator or a phone).
        "adb" => {
            let device = |w: &mut Walk<'_>, change| {
                w.effect(Effect::Infra {
                    target: "the connected Android device".to_owned(),
                    change,
                    local: true,
                });
            };
            // `adb -s SERIAL …`: the device first.
            let mut words = words.clone();
            if let Some(k) = args.iter().position(|a| matches!(a.text.as_str(), "-s" | "-t" | "-H" | "-P" | "-L"))
                && let Some(v) = args.get(k + 1)
            {
                words.retain(|x| *x != v.text.as_str());
            }
            let first = words.first().copied().unwrap_or_default();
            match first {
                "devices" | "version" | "get-state" | "get-serialno" | "wait-for-device" | "logcat" | "help"
                | "start-server" | "kill-server" => {}
                "uninstall" => device(w, InfraChange::Destroy),
                "install" | "install-multiple" | "push" | "shell" | "emu" | "reboot" | "root" | "unroot"
                | "forward" | "reverse" | "exec-out" => device(w, InfraChange::Apply),
                "pull" => {
                    if let Some(d) = words.get(2) {
                        let p = w.path(&Exp::lit(d));
                        w.write(p);
                    } else {
                        let p = w.path(&Exp::lit("."));
                        w.write(p);
                    }
                }
                other => unmodelled(w, program, other),
            }
        }
        // Addresses by type: listeners publish (unless bound here), connections reach their host, files are read and
        // written, EXEC/SYSTEM run their command; any other address type is not read.
        "socat" => {
            known_options(w, program, args, "dDvxbslhuUgL46", "TtbL", &[]);
            for a in args.iter().filter(|a| !a.text.starts_with('-') || a.text == "-") {
                let text = a.text.as_str();
                let (kind, rest) = text.split_once(':').unwrap_or((text, ""));
                let kind = kind.to_ascii_uppercase();
                let options = rest.split(',').skip(1).map(str::to_ascii_lowercase).collect::<Vec<_>>();
                let bound_here = options.iter().any(|o| {
                    o.starts_with("bind=127.")
                        || o == "bind=localhost"
                        || o.starts_with("bind=[::1]")
                        || o.starts_with("range=127.")
                });
                let target = rest.split(',').next().unwrap_or_default();
                match kind.as_str() {
                    "-" | "STDIO" | "STDIN" | "STDOUT" | "STDERR" => {}
                    k if k.ends_with("-LISTEN")
                        || k.ends_with("-L")
                        || k.ends_with("-RECVFROM")
                        || k.ends_with("-RECV") =>
                    {
                        let local = bound_here || k.starts_with("UNIX") || k.starts_with("ABSTRACT");
                        listener(w, local, "a socat listener");
                    }
                    "TCP" | "TCP4" | "TCP6" | "UDP" | "UDP4" | "UDP6" | "OPENSSL" | "SSL" | "SCTP" | "SOCKS4"
                    | "SOCKS4A" | "SOCKS5" | "PROXY" | "UDP-SENDTO" | "UDP-DATAGRAM" => {
                        w.effect(Effect::Network {
                            host: target.split(':').next().unwrap_or(target).to_ascii_lowercase(),
                            method: NetMethod::Connect,
                        });
                    }
                    "OPEN" | "FILE" | "GOPEN" | "PIPE" | "CREATE" | "CREAT" => {
                        let e = Exp::lit(target);
                        if kind != "CREATE" && kind != "CREAT" {
                            w.read_file(&e);
                        }
                        let p = w.path(&e);
                        w.write(p);
                    }
                    "EXEC" | "SYSTEM" => {
                        if a.known() {
                            let cmd = target.to_owned();
                            w.run_script(&cmd, Vec::new());
                        } else {
                            w.lower(Certainty::None, "a program socat runs with values that are not in the command");
                        }
                    }
                    _ => unmodelled(w, program, text),
                }
            }
        }
        "tailscale" => match first {
            "status" | "ip" | "version" | "whois" | "netcheck" | "ping" | "" => {}
            other => unmodelled(w, program, other),
        },
        // Services: their status is nothing; anything else changes what runs on this computer.
        "systemctl" => {
            let reads = [
                "status",
                "is-active",
                "is-enabled",
                "is-failed",
                "list-units",
                "list-unit-files",
                "show",
                "cat",
                "list-timers",
                "list-sockets",
                "list-dependencies",
                "get-default",
                "help",
                "",
            ];
            known_options(
                w,
                program,
                args,
                "qalt",
                "pHMn",
                &[
                    "user",
                    "system",
                    "no-pager",
                    "all",
                    "type",
                    "state",
                    "quiet",
                    "now",
                    "plain",
                    "full",
                    "lines",
                    "output",
                    "property",
                    "value",
                    "no-legend",
                    "failed",
                    "machine",
                    "host",
                ],
            );
            if !reads.contains(&first) {
                w.effect(Effect::Infra {
                    target: format!("the service {}", words.get(1).copied().unwrap_or(first)),
                    change: InfraChange::Apply,
                    local: has(args, "--user"),
                });
            }
        }
        _ => return None,
    }
    Some(Input::Opaque(program.to_owned()))
}

/// What a sed script does beyond editing its input: whether it runs commands (`e`, `s///e`), and the files it writes
/// (`w file`, `W file`, `s///w file`) and reads (`r file`, `R file`). Read command by command: an `s` command's text is
/// skipped, so a `;` or a `w` inside it is text.
fn sed_script(script: &str) -> (bool, Vec<String>, Vec<String>) {
    let c: Vec<char> = script.chars().collect();
    let n = c.len();
    let mut runs = false;
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut i = 0;
    // Skips a delimited section starting after its opening `delim`; returns the index after the closing one.
    let section = |mut k: usize, delim: char| -> usize {
        while k < n {
            if c[k] == '\\' {
                k += 2;
                continue;
            }
            if c[k] == delim {
                return k + 1;
            }
            k += 1;
        }
        n
    };
    let address = |mut k: usize| -> usize {
        loop {
            while k < n && (c[k].is_ascii_digit() || matches!(c[k], '$' | '~' | '+')) {
                k += 1;
            }
            if k < n && c[k] == '/' {
                k = section(k + 1, '/');
                while k < n && matches!(c[k], 'I' | 'M') {
                    k += 1;
                }
            } else if k + 1 < n && c[k] == '\\' {
                k = section(k + 2, c[k + 1]);
            }
            if k < n && c[k] == ',' {
                k += 1;
                continue;
            }
            return k;
        }
    };
    // The rest of the line, trimmed (a file name).
    let line = |k: usize| -> (String, usize) {
        let end = (k..n).find(|&j| c[j] == '\n').unwrap_or(n);
        (c[k..end].iter().collect::<String>().trim().to_owned(), end)
    };
    while i < n {
        while i < n && (c[i].is_whitespace() || matches!(c[i], ';' | '{' | '}')) {
            i += 1;
        }
        if i >= n {
            break;
        }
        i = address(i);
        while i < n && (c[i] == ' ' || c[i] == '!') {
            i += 1;
        }
        let Some(&cmd) = c.get(i) else {
            break;
        };
        i += 1;
        match cmd {
            'e' => {
                runs = true;
                i = line(i).1;
            }
            'w' | 'W' | 'r' | 'R' => {
                let (file, end) = line(i);
                if matches!(cmd, 'w' | 'W') {
                    writes.push(file);
                } else {
                    reads.push(file);
                }
                i = end;
            }
            's' | 'y' => {
                let Some(&delim) = c.get(i) else {
                    break;
                };
                i = section(i + 1, delim);
                i = section(i, delim);
                if cmd == 's' {
                    let start = i;
                    while i < n && !matches!(c[i], ';' | '\n' | '}' | 'w') {
                        i += 1;
                    }
                    let flags: String = c[start..i].iter().collect();
                    runs |= flags.contains('e');
                    if i < n && c[i] == 'w' {
                        let (file, end) = line(i + 1);
                        writes.push(file);
                        i = end;
                    }
                }
            }
            // Text to the end of the line.
            'a' | 'i' | 'c' | ':' => i = line(i).1,
            'b' | 't' | 'T' => {
                while i < n && !matches!(c[i], ';' | '\n' | '}') {
                    i += 1;
                }
            }
            _ => {}
        }
    }
    (runs, writes, reads)
}

/// Wrappers in [`OPTIONS`]: only the options before the program they run are theirs.
const WRAPPERS: &[&str] = &["env", "nice", "timeout", "sudo", "xargs", "nohup", "stdbuf", "setsid"];

/// The options each modelled program has: (program, short flags, short options with a value, long options).
type Options = (&'static str, &'static str, &'static str, &'static [&'static str]);
const OPTIONS: &[Options] = &[
    (
        "rm",
        "rfRidIv",
        "",
        &[
            "recursive",
            "force",
            "interactive",
            "dir",
            "verbose",
            "one-file-system",
            "no-preserve-root",
            "preserve-root",
            "help",
            "version",
        ],
    ),
    ("rmdir", "pv", "", &["parents", "verbose", "ignore-fail-on-non-empty"]),
    ("unlink", "", "", &[]),
    (
        "cp",
        "rRaifnpuvLPHdlsxTb",
        "tS",
        &[
            "recursive",
            "archive",
            "force",
            "interactive",
            "no-clobber",
            "preserve",
            "no-preserve",
            "update",
            "verbose",
            "dereference",
            "no-dereference",
            "link",
            "symbolic-link",
            "target-directory",
            "no-target-directory",
            "parents",
            "backup",
            "suffix",
            "reflink",
            "sparse",
            "one-file-system",
            "remove-destination",
            "strip-trailing-slashes",
            "attributes-only",
            "copy-contents",
            "debug",
        ],
    ),
    (
        "mv",
        "finuvbT",
        "tS",
        &[
            "force",
            "interactive",
            "no-clobber",
            "update",
            "verbose",
            "backup",
            "suffix",
            "target-directory",
            "no-target-directory",
            "strip-trailing-slashes",
            "exchange",
            "no-copy",
            "debug",
        ],
    ),
    (
        "ln",
        "sfnvirTLPb",
        "tS",
        &[
            "symbolic",
            "force",
            "no-dereference",
            "verbose",
            "interactive",
            "relative",
            "target-directory",
            "no-target-directory",
            "logical",
            "physical",
            "backup",
            "suffix",
        ],
    ),
    (
        "install",
        "dDcCpsvbT",
        "mogtS",
        &[
            "directory",
            "mode",
            "owner",
            "group",
            "target-directory",
            "no-target-directory",
            "verbose",
            "compare",
            "preserve-timestamps",
            "strip",
            "backup",
            "suffix",
        ],
    ),
    ("mkdir", "pv", "m", &["parents", "verbose", "mode"]),
    ("touch", "acmhf", "dtr", &["date", "reference", "no-create", "time", "no-dereference"]),
    // Modes may start with a dash (`chmod -x f`).
    (
        "chmod",
        "RvcfHLPrwxXstugoa",
        "",
        &["recursive", "verbose", "changes", "silent", "quiet", "reference", "preserve-root", "no-preserve-root"],
    ),
    (
        "chown",
        "RvcfhHLP",
        "",
        &[
            "recursive",
            "verbose",
            "changes",
            "silent",
            "quiet",
            "reference",
            "dereference",
            "no-dereference",
            "from",
            "preserve-root",
        ],
    ),
    (
        "chgrp",
        "RvcfhHLP",
        "",
        &[
            "recursive",
            "verbose",
            "changes",
            "silent",
            "quiet",
            "reference",
            "dereference",
            "no-dereference",
            "preserve-root",
        ],
    ),
    ("truncate", "co", "sr", &["no-create", "io-blocks", "size", "reference"]),
    ("tee", "aip", "", &["append", "ignore-interrupts", "output-error"]),
    (
        "zip",
        "rqvjmTXDyuFgld0123456789AJ",
        "xinbtZsP",
        &[
            "recurse-paths",
            "quiet",
            "verbose",
            "junk-paths",
            "move",
            "exclude",
            "include",
            "symlinks",
            "test",
            "delete",
            "update",
            "freshen",
            "grow",
            "latest-time",
            "no-dir-entries",
        ],
    ),
    ("unzip", "lotqvjnoaCLXKZ", "dxP", &[]),
    (
        "gzip",
        "dcfklnNqrtv123456789",
        "S",
        &[
            "decompress",
            "stdout",
            "force",
            "keep",
            "list",
            "no-name",
            "name",
            "quiet",
            "recursive",
            "test",
            "verbose",
            "best",
            "fast",
            "suffix",
        ],
    ),
    ("awk", "", "Fv", &["field-separator", "assign"]),
    ("base64", "di", "w", &["decode", "ignore-garbage", "wrap", "help", "version"]),
    (
        "gpg",
        "dekKsbacvqnNy",
        "ruoRfi",
        &[
            "decrypt",
            "encrypt",
            "symmetric",
            "sign",
            "clearsign",
            "clear-sign",
            "detach-sign",
            "verify",
            "armor",
            "recipient",
            "local-user",
            "output",
            "batch",
            "yes",
            "quiet",
            "verbose",
            "list-keys",
            "list-secret-keys",
            "fingerprint",
            "import",
            "export",
            "export-secret-keys",
            "export-secret-subkeys",
            "homedir",
            "pinentry-mode",
            "passphrase-fd",
            "no-tty",
            "trust-model",
            "status-fd",
            "with-colons",
            "version",
            "help",
            "dearmor",
            "enarmor",
        ],
    ),
    (
        "grep",
        "rRinlLcvwxoqsEFGPHhabzZUITy",
        "efmABCdD",
        &[
            "recursive",
            "dereference-recursive",
            "ignore-case",
            "line-number",
            "files-with-matches",
            "files-without-match",
            "count",
            "invert-match",
            "word-regexp",
            "line-regexp",
            "only-matching",
            "quiet",
            "silent",
            "no-messages",
            "extended-regexp",
            "fixed-strings",
            "basic-regexp",
            "perl-regexp",
            "with-filename",
            "no-filename",
            "byte-offset",
            "text",
            "binary-files",
            "null",
            "null-data",
            "regexp",
            "file",
            "max-count",
            "after-context",
            "before-context",
            "context",
            "include",
            "exclude",
            "exclude-dir",
            "exclude-from",
            "color",
            "colour",
            "label",
            "line-buffered",
            "initial-tab",
            "devices",
            "directories",
            "help",
            "version",
        ],
    ),
    (
        "rg",
        "iSsFwxvcClLnNoqUuHz0.aIp",
        "eAfBCgmtTjMrEd",
        &[
            "ignore-case",
            "smart-case",
            "case-sensitive",
            "fixed-strings",
            "word-regexp",
            "line-regexp",
            "invert-match",
            "count",
            "count-matches",
            "files-with-matches",
            "files-without-match",
            "line-number",
            "no-line-number",
            "only-matching",
            "quiet",
            "multiline",
            "multiline-dotall",
            "unrestricted",
            "hidden",
            "no-ignore",
            "no-ignore-vcs",
            "with-filename",
            "no-filename",
            "null",
            "files",
            "type",
            "type-not",
            "glob",
            "iglob",
            "max-count",
            "max-depth",
            "max-filesize",
            "after-context",
            "before-context",
            "context",
            "regexp",
            "file",
            "threads",
            "replace",
            "json",
            "vimgrep",
            "heading",
            "no-heading",
            "color",
            "colors",
            "sort",
            "sortr",
            "follow",
            "pcre2",
            "trim",
            "column",
            "stats",
            "debug",
            "help",
            "version",
            "max-columns",
            "max-columns-preview",
            "no-messages",
            "passthru",
            "text",
            "search-zip",
            "pretty",
            "fixed-strings",
            "word-regexp",
            "engine",
            "path-separator",
            "field-match-separator",
            "field-context-separator",
            "byte-offset",
        ],
    ),
    (
        "jq",
        "rjcnseaSCMR",
        "f",
        &[
            "raw-output",
            "join-output",
            "compact-output",
            "null-input",
            "slurp",
            "exit-status",
            "ascii-output",
            "sort-keys",
            "color-output",
            "monochrome-output",
            "raw-input",
            "arg",
            "argjson",
            "slurpfile",
            "rawfile",
            "args",
            "jsonargs",
            "tab",
            "indent",
            "from-file",
            "seq",
            "stream",
            "help",
            "version",
            "raw-output0",
        ],
    ),
    ("ssh-keygen", "qyleEFRHcpBDkLuMa", "tbfNCYnIOVzmsrwP", &[]),
    ("ssh", "46AaCfGgKkMNnqsTtVvXxYy", "BbcDEeFIiJLlmOoPpRSWw", &[]),
    ("scp", "346ABCOpqRrsTv", "cDFiJlPSoX", &[]),
    (
        "rsync",
        "vqcarRbuLlHpEoXAgtDOJmSzhPnWxCiy08",
        "eBfTMs",
        &[
            "archive",
            "verbose",
            "quiet",
            "recursive",
            "delete",
            "delete-after",
            "delete-before",
            "delete-during",
            "delete-delay",
            "delete-excluded",
            "exclude",
            "include",
            "exclude-from",
            "include-from",
            "filter",
            "files-from",
            "progress",
            "partial",
            "partial-dir",
            "human-readable",
            "stats",
            "dry-run",
            "checksum",
            "compress",
            "compress-level",
            "links",
            "hard-links",
            "perms",
            "times",
            "owner",
            "group",
            "devices",
            "specials",
            "existing",
            "ignore-existing",
            "update",
            "inplace",
            "append",
            "append-verify",
            "size-only",
            "omit-dir-times",
            "no-perms",
            "no-owner",
            "no-group",
            "chmod",
            "chown",
            "rsh",
            "rsync-path",
            "itemize-changes",
            "info",
            "out-format",
            "log-file",
            "timeout",
            "contimeout",
            "bwlimit",
            "backup",
            "backup-dir",
            "suffix",
            "remove-source-files",
            "prune-empty-dirs",
            "relative",
            "no-relative",
            "mkpath",
            "one-file-system",
            "safe-links",
            "copy-links",
            "copy-unsafe-links",
            "keep-dirlinks",
            "max-size",
            "min-size",
            "max-delete",
            "force",
            "ignore-errors",
            "numeric-ids",
            "whole-file",
            "no-whole-file",
            "sparse",
            "temp-dir",
            "fuzzy",
            "compare-dest",
            "copy-dest",
            "link-dest",
            "protect-args",
            "secluded-args",
            "ipv4",
            "ipv6",
            "port",
            "password-file",
            "help",
            "version",
            "no-motd",
            "info",
            "debug",
            "executability",
            "acls",
            "xattrs",
        ],
    ),
    ("ssh-add", "lLdDDxXcqkKe", "Est", &[]),
    (
        "env",
        "i0v",
        "uCPS",
        &[
            "ignore-environment",
            "null",
            "unset",
            "chdir",
            "split-string",
            "debug",
            "help",
            "version",
            "block-signal",
            "default-signal",
            "ignore-signal",
            "list-signal-handling",
        ],
    ),
    ("nice", "", "n", &["adjustment"]),
    ("timeout", "v", "sk", &["preserve-status", "foreground", "verbose", "signal", "kill-after"]),
    (
        "sudo",
        "AbBEHiKklLnPSsVv",
        "ugChpUrtDRT",
        &[
            "preserve-env",
            "login",
            "shell",
            "non-interactive",
            "stdin",
            "reset-timestamp",
            "background",
            "set-home",
            "user",
            "group",
            "host",
            "prompt",
            "other-user",
            "role",
            "type",
            "chdir",
            "chroot",
            "close-from",
        ],
    ),
    (
        "xargs",
        "0prtx",
        "IinPLdEase",
        &[
            "null",
            "no-run-if-empty",
            "verbose",
            "interactive",
            "exit",
            "open-tty",
            "replace",
            "max-args",
            "max-procs",
            "max-lines",
            "delimiter",
            "eof",
            "arg-file",
            "max-chars",
        ],
    ),
];

/// A program with no model: not read, so it is asked (01 §2.4: certainty `None` names what couldn't be read).
fn unknown_program(w: &mut Walk<'_>, program: &str) {
    w.effect(Effect::Exec {
        program: program.to_owned(),
        source: CodeSource::Unknown,
    });
    w.lower(Certainty::None, format!("what `{program}` does (Reins has no model of it)"));
}

/// A secret printed into the agent's context (08 R4: a secret read).
/// A variable name that holds a credential by convention.
fn secret_name(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["TOKEN", "SECRET", "KEY", "PASSWORD", "PASSWD", "PASS", "CREDENTIAL", "AUTH", "COOKIE", "SESSION", "PRIVATE"]
        .iter()
        .any(|w| n.contains(w))
}

fn secret_to_agent(w: &mut Walk<'_>, what: &str) {
    w.effect(Effect::FileRead {
        path: super::model::PathRef {
            path: what.to_owned(),
            area: Area::Unknown,
            recoverable: false,
        },
        kind: FileKind::Secret(SecretKind::Token),
    });
}

/// A developer tool: its own run (L0), the files it writes with `-o`, and the secret files it uses locally.
fn toolchain(w: &mut Walk<'_>, program: &str, args: &[Exp]) {
    if toolchain_runs(w, program, args) {
        return;
    }
    // A build system's publishing targets (`make deploy`, `just publish`, `flutter pub publish`, `dotnet nuget push`).
    let words: Vec<&str> = args.iter().filter(|a| !a.text.starts_with('-')).map(|a| a.text.as_str()).collect();
    if let Some(target) = words.iter().find(|x| {
        matches!(**x, "publish" | "deploy" | "upload")
            || (**x == "push" && matches!(program, "dotnet" | "flutter" | "dart" | "swift" | "go"))
    }) {
        w.effect(Effect::Publish {
            target: format!("{program} {target}"),
        });
    }
    w.effect(Effect::Exec {
        program: program.to_owned(),
        source: CodeSource::Toolchain,
    });
    for (k, a) in args.iter().enumerate() {
        let value = a.text.split_once('=').map_or(a.text.as_str(), |(_, v)| v);
        if let FileKind::Secret(_) = file_kind(value) {
            w.effect(Effect::SecretUse {
                secret: value.to_owned(),
                local_only: true,
            });
        }
        if matches!(a.text.as_str(), "-o" | "--output" | "--out" | "--out-dir" | "-out" | "--output-dir" | "--outdir")
            && let Some(o) = args.get(k + 1)
        {
            let p = w.path(o);
            w.write(p);
        }
    }
    // ffmpeg, magick and the like write their last word.
    if matches!(program, "ffmpeg" | "magick" | "convert" | "montage" | "composite" | "rsvg-convert" | "pandoc" | "grim")
        && let Some(last) = args.last()
        && !last.text.starts_with('-')
    {
        let p = w.path(last);
        w.write(p);
    }
}

/// A developer tool's entries that run other code: the code is read as what it is. True when handled here.
fn toolchain_runs(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> bool {
    // A file of the tool's own commands: the project's is the project's code; any other is not read.
    let outside = |w: &mut Walk<'_>, file: Option<Exp>, what: &str| {
        if let Some(f) = file {
            let p = w.path(&f);
            if !f.known() || !p.area.in_workspace() {
                w.lower(Certainty::None, format!("{what} {}", p.path));
            }
        }
    };
    let attached = |names: &[&str], short: &str| -> Option<Exp> {
        args.iter().enumerate().find_map(|(k, a)| {
            if names.contains(&a.text.as_str()) || a.text == short {
                args.get(k + 1).cloned()
            } else {
                names
                    .iter()
                    .find_map(|n| a.text.strip_prefix(n).and_then(|r| r.strip_prefix('=')))
                    .or_else(|| a.text.strip_prefix(short).filter(|r| !r.is_empty() && !short.is_empty()))
                    .map(|r| Exp {
                        text: r.to_owned(),
                        ..a.clone()
                    })
            }
        })
    };
    match program {
        // `xcrun [-sdk S] tool …`: the tool runs.
        "xcrun" => {
            if args.iter().any(|a| {
                matches!(a.text.as_str(), "-f" | "--find" | "-l" | "--log" | "-k" | "--kill-cache")
                    || a.text.starts_with("--show-sdk")
            }) {
                return false;
            }
            let mut at = 0;
            while let Some(a) = args.get(at) {
                match a.text.as_str() {
                    "-sdk" | "--sdk" | "-toolchain" | "--toolchain" => at += 2,
                    t if t.starts_with('-') => at += 1,
                    _ => break,
                }
            }
            if at < args.len() {
                let _out: Input = run(w, &args[at..], Input::None);
                return true;
            }
            false
        }
        // `rustup run TOOLCHAIN cmd …`
        "rustup" => {
            let words: Vec<&Exp> = args.iter().filter(|a| !a.text.starts_with('-')).collect();
            if words.first().is_some_and(|a| a.text == "run")
                && let Some(at) = args.iter().position(|a| std::ptr::eq(a, words[0]))
                && at + 2 < args.len()
            {
                let _out: Input = run(w, &args[at + 2..], Input::None);
                return true;
            }
            false
        }
        // `cmake -E cmd …` is a small shell; `cmake -P file` runs a script.
        "cmake" => {
            if let Some(k) = args.iter().position(|a| a.text == "-E") {
                let rest = &args[k + 1..];
                let mapped: Option<Vec<Exp>> = rest.first().and_then(|c| {
                    let tail = rest[1..].iter().cloned();
                    let head: &[&str] = match c.text.as_str() {
                        "rm" => &["rm"],
                        "remove" => &["rm", "-f"],
                        "remove_directory" => &["rm", "-rf"],
                        "copy" | "copy_if_different" => &["cp"],
                        "copy_directory" | "copy_directory_if_different" => &["cp", "-r"],
                        "rename" => &["mv"],
                        "make_directory" => &["mkdir", "-p"],
                        "touch" | "touch_nocreate" => &["touch"],
                        "echo" | "echo_append" => &["echo"],
                        "cat" => &["cat"],
                        "sha256sum" | "sha1sum" | "md5sum" | "sha512sum" => &["sha256sum"],
                        "compare_files" => &["cmp"],
                        "capabilities" | "environment" | "true" | "false" | "sleep" => &["true"],
                        _ => return None,
                    };
                    Some(head.iter().map(|h| Exp::lit(h)).chain(tail).collect())
                });
                match mapped {
                    Some(words) => {
                        let _out: Input = run(w, &words, Input::None);
                    }
                    None => w.lower(Certainty::None, "a cmake -E command"),
                }
                return true;
            }
            if args.iter().any(|a| a.text == "-P" || (a.text.starts_with("-P") && a.text.len() > 2)) {
                w.effect(Effect::Exec {
                    program: "a cmake script".to_owned(),
                    source: CodeSource::Unknown,
                });
                w.lower(Certainty::None, "a cmake script");
                return true;
            }
            false
        }
        "just" => {
            outside(w, attached(&["--justfile"], "-f"), "a justfile");
            outside(w, attached(&["--working-directory"], "-d"), "a just folder");
            false
        }
        "make" | "gmake" => {
            outside(w, attached(&["--file", "--makefile"], "-f"), "a makefile");
            outside(w, attached(&["--directory"], "-C"), "a make folder");
            false
        }
        // `mvn exec:exec`/`exec:java` run what their properties name.
        "mvn" => {
            if args.iter().any(|a| a.text.starts_with("exec:") || a.text.contains(":exec-maven-plugin:")) {
                w.effect(Effect::Exec {
                    program: "a program maven runs".to_owned(),
                    source: CodeSource::Unknown,
                });
                w.lower(Certainty::None, "what mvn exec runs");
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Runs a script file: one the line wrote (read exactly), the project's own (its entry points: L0), or unknown.
fn run_file(w: &mut Walk<'_>, file: &Exp, args: &[Exp], lang: Lang) {
    let path = w.path(file);
    let written = w.files.get(&path.path).cloned();
    match written {
        Some(Some(text)) => {
            if w.maybe_unwritten.contains(&path.path) {
                w.lower(Certainty::Partial, format!("what {} held if this line did not write it", path.path));
            }
            let lang = match lang {
                Lang::Shell => inline::lang_of_script(&path.path, &text),
                l => l,
            };
            if lang == Lang::Shell {
                w.run_script(&text, args.to_vec());
            } else {
                inline::analyze_code(w, lang, &text, args);
            }
        }
        Some(None) => {
            w.effect(Effect::Exec {
                program: path.path.clone(),
                source: CodeSource::Generated,
            });
            w.lower(Certainty::None, format!("the script {} this command writes and runs", path.path));
        }
        None if path.area.in_workspace() => {
            let name = path.path.rsplit('/').next().unwrap_or(&path.path).to_owned();
            if TOOLCHAIN.contains(&name.as_str()) {
                toolchain(w, &name, args);
            } else {
                // The project's own script: what it does is not read here (01 §2.4).
                w.lower(Certainty::Partial, format!("what the project's {name} does"));
                w.effect(Effect::Exec {
                    program: name,
                    source: CodeSource::Workspace,
                });
            }
        }
        None => {
            let name = path.path.rsplit('/').next().unwrap_or(&path.path).to_owned();
            if TOOLCHAIN.contains(&name.as_str())
                || HARMLESS.contains(&name.as_str())
                || READERS.contains(&name.as_str())
            {
                let mut words = vec![Exp::lit(&name)];
                words.extend(args.iter().cloned());
                let _out: Input = run(w, &words, Input::None);
            } else {
                unknown_program(w, &path.path);
            }
        }
    }
}

/// A shell or interpreter reading the code it runs from its input.
fn run_input(w: &mut Walk<'_>, input: Input, lang: Lang, program: &str, args: &[Exp]) {
    match input {
        Input::Text(t) => {
            if lang == Lang::Shell {
                w.run_script(&t, Vec::new());
            } else {
                inline::analyze_code(w, lang, &t, args);
            }
        }
        Input::Downloaded(host) => {
            w.effect(Effect::Exec {
                program: host,
                source: CodeSource::Downloaded,
            });
        }
        Input::Uncertain(t) => {
            // Shown as far as it can be read; never certain.
            if lang == Lang::Shell {
                w.run_script(&t.replace(UNKNOWN, "\"$__reins_unknown__\""), Vec::new());
            } else {
                inline::analyze_code(w, lang, &t.replace(UNKNOWN, "__reins_unknown__"), args);
            }
            w.lower(Certainty::None, format!("{program} runs code with values that are not in the command"));
        }
        Input::Decoded => {
            w.effect(Effect::Exec {
                program: program.to_owned(),
                source: CodeSource::Generated,
            });
            w.lower(Certainty::None, format!("{program} runs code decoded while it runs"));
        }
        Input::Opaque(from) => {
            w.effect(Effect::Exec {
                program: program.to_owned(),
                source: CodeSource::Generated,
            });
            w.lower(Certainty::None, format!("{program} runs code from {from}"));
        }
        Input::None => {
            w.effect(Effect::Exec {
                program: program.to_owned(),
                source: CodeSource::Unknown,
            });
            w.lower(Certainty::None, format!("{program} reads the code it runs from its input"));
        }
    }
}

/// `host:path` or `user@host:path` (not `C:\x`, not a local path with a colon after a slash).
fn remote_spec(text: &str) -> Option<(String, String)> {
    let (host, path) = text.split_once(':')?;
    if host.is_empty() || host.contains('/') || host.len() == 1 {
        return None;
    }
    let host = host.rsplit('@').next()?.to_owned();
    Some((host, path.to_owned()))
}

fn copy_like(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Input {
    // The program that carries the copy (`rsync -e`, `--rsh`, `--rsync-path`, `scp -S`): ssh is read as ssh, any other
    // program is not read.
    let carrier = args
        .windows(2)
        .find(|p| {
            matches!(p[0].text.as_str(), "-e" | "--rsh" | "--rsync-path" | "-S")
                && (program == "rsync" || p[0].text == "-S")
        })
        .map(|p| p[1].text.clone())
        .or_else(|| {
            args.iter().find_map(|a| {
                a.text.strip_prefix("--rsh=").or_else(|| a.text.strip_prefix("--rsync-path=")).map(str::to_owned)
            })
        });
    if let Some(c) = carrier
        && !(c == "ssh" || c.starts_with("ssh ") || c == "rsync" || c.starts_with("sudo rsync"))
    {
        w.lower(Certainty::None, format!("{program} runs `{c}` to copy"));
    }
    if args.windows(2).any(|p| p[0].text == "-o" && p[1].text.to_ascii_lowercase().starts_with("proxycommand")) {
        w.lower(Certainty::None, format!("{program} runs a program on this computer that its options name"));
    }
    let value_opts: &[&str] = match program {
        "scp" => &["-P", "-i", "-o", "-F", "-J", "-l", "-c", "-S"],
        "rsync" => &[
            "-e",
            "--rsh",
            "--exclude",
            "--include",
            "--filter",
            "-f",
            "--files-from",
            "--exclude-from",
            "--include-from",
            "--chmod",
            "--chown",
            "--rsync-path",
            "--log-file",
            "--partial-dir",
            "--temp-dir",
            "-T",
            "--backup-dir",
            "--compare-dest",
            "--link-dest",
        ],
        "install" => &["-m", "--mode", "-o", "--owner", "-g", "--group", "-t", "--target-directory", "-S", "--suffix"],
        _ => &["-t", "--target-directory", "-S", "--suffix"],
    };
    let mut pos = positional(args, value_opts);
    let explicit = opt_value(args, &["-t", "--target-directory"]).cloned().or_else(|| {
        args.iter().find_map(|a| {
            a.text
                .strip_prefix("--target-directory=")
                .or_else(|| {
                    a.text.strip_prefix("-t").filter(|r| !r.is_empty() && program != "rsync" && program != "scp")
                })
                .map(|r| Exp {
                    text: r.to_owned(),
                    ..a.clone()
                })
        })
    });
    if program == "install" && short(args, 'd') {
        for p in pos {
            let path = w.path(p);
            if !matches!(path.area, Area::Generated | Area::Scratch | Area::Source | Area::Home) {
                w.write(path);
            }
        }
        return Input::None;
    }
    let dest = match explicit {
        Some(d) => Some(d),
        None if pos.len() >= 2 => pos.pop().cloned(),
        None => None,
    };
    let Some(dest) = dest else {
        return Input::None;
    };
    let secret_sources: Vec<String> =
        pos.iter().filter(|s| matches!(file_kind(&s.text), FileKind::Secret(_))).map(|s| s.text.clone()).collect();
    // Sources: read (a secret read is a secret read) and, for `mv`, removed from where they were.
    for s in &pos {
        if let Some((host, _)) = remote_spec(&s.text) {
            w.effect(Effect::Network {
                host,
                method: NetMethod::Read,
            });
            continue;
        }
        if program == "mv" {
            let p = w.path(s);
            w.write(p);
        } else {
            w.read_file(s);
        }
    }
    if let Some((host, path)) = remote_spec(&dest.text) {
        w.effect(Effect::Network {
            host: host.clone(),
            method: if pos.iter().any(|s| is_executable_name(&s.text)) {
                NetMethod::UploadExecutable
            } else {
                NetMethod::Write
            },
        });
        for s in secret_sources {
            w.effect(Effect::SecretOut {
                secret: s,
                destination: host.clone(),
            });
        }
        let e = Exp::lit(if path.is_empty() {
            "~"
        } else {
            &path
        });
        let saved = w.remote.replace(host);
        let p = w.path(&e);
        w.remote = saved;
        if program == "rsync" && args.iter().any(|a| a.text.starts_with("--delete")) {
            w.effect(Effect::FileDelete {
                paths: vec![p.clone()],
                recursive: true,
            });
        }
        w.write(p);
        return Input::None;
    }
    if program == "rsync" && has(args, "--remove-source-files") {
        let sources: Vec<_> = pos.iter().map(|s| w.path(s)).collect();
        w.effect(Effect::FileDelete {
            paths: sources,
            recursive: true,
        });
    }
    let p = w.path(&dest);
    if program == "rsync" && args.iter().any(|a| a.text.starts_with("--delete")) {
        w.effect(Effect::FileDelete {
            paths: vec![p.clone()],
            recursive: true,
        });
    }
    // Content of a copy into a file the line later runs: the source's, when the line wrote it.
    if program != "ln"
        && let [one] = pos.as_slice()
    {
        let src = w.path(one);
        if let Some(content) = w.files.get(&src.path).cloned() {
            w.files.insert(p.path.clone(), content);
        }
    }
    w.write(p);
    Input::None
}

fn is_executable_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        ".sh", ".py", ".rb", ".pl", ".js", ".exe", ".bin", ".apk", ".ipa", ".app", ".dmg", ".deb", ".rpm", ".jar",
        ".zip", ".tar", ".gz", ".tgz", ".xz",
    ]
    .iter()
    .any(|e| n.ends_with(e))
}

/// `find`'s tests, options and actions (with how many words follow each); any other `-word` is not read.
const FIND_WORDS: &[(&str, usize)] = &[
    ("-name", 1),
    ("-iname", 1),
    ("-path", 1),
    ("-ipath", 1),
    ("-wholename", 1),
    ("-iwholename", 1),
    ("-regex", 1),
    ("-iregex", 1),
    ("-regextype", 1),
    ("-type", 1),
    ("-xtype", 1),
    ("-size", 1),
    ("-mtime", 1),
    ("-mmin", 1),
    ("-atime", 1),
    ("-amin", 1),
    ("-ctime", 1),
    ("-cmin", 1),
    ("-newer", 1),
    ("-newermt", 1),
    ("-anewer", 1),
    ("-cnewer", 1),
    ("-maxdepth", 1),
    ("-mindepth", 1),
    ("-user", 1),
    ("-group", 1),
    ("-uid", 1),
    ("-gid", 1),
    ("-perm", 1),
    ("-links", 1),
    ("-inum", 1),
    ("-samefile", 1),
    ("-lname", 1),
    ("-ilname", 1),
    ("-used", 1),
    ("-printf", 1),
    ("-depth", 0),
    ("-prune", 0),
    ("-print", 0),
    ("-print0", 0),
    ("-ls", 0),
    ("-quit", 0),
    ("-empty", 0),
    ("-readable", 0),
    ("-writable", 0),
    ("-executable", 0),
    ("-not", 0),
    ("-a", 0),
    ("-and", 0),
    ("-o", 0),
    ("-or", 0),
    ("-true", 0),
    ("-false", 0),
    ("-follow", 0),
    ("-L", 0),
    ("-P", 0),
    ("-H", 0),
    ("-xdev", 0),
    ("-mount", 0),
    ("-daystart", 0),
    ("-noleaf", 0),
    ("-ignore_readdir_race", 0),
    ("-nouser", 0),
    ("-nogroup", 0),
    ("-delete", 0),
    ("-exec", 0),
    ("-execdir", 0),
    ("-ok", 0),
    ("-okdir", 0),
    ("-fprint", 1),
    ("-fprint0", 1),
    ("-fprintf", 2),
    ("-fls", 1),
];

fn find(w: &mut Walk<'_>, args: &[Exp]) -> Input {
    let mut k = 0;
    while k < args.len() {
        let t = args[k].text.as_str();
        if matches!(t, "-exec" | "-execdir" | "-ok" | "-okdir") {
            k = args[k + 1..].iter().position(|x| x.text == ";" || x.text == "+").map_or(args.len(), |p| k + 2 + p);
            continue;
        }
        if t.starts_with('-') && t.len() > 1 {
            match FIND_WORDS.iter().find(|(name, _)| *name == t) {
                Some((_, n)) => k += n,
                None => w.lower(Certainty::None, format!("the find expression {t}")),
            }
        }
        k += 1;
    }
    // Roots: the words before the first expression.
    let roots: Vec<Exp> = args
        .iter()
        .take_while(|a| !a.text.starts_with('-') && !matches!(a.text.as_str(), "!" | "(" | ")"))
        .cloned()
        .collect();
    let roots = if roots.is_empty() {
        vec![Exp::lit(".")]
    } else {
        roots
    };
    let matched: Vec<Exp> = roots
        .iter()
        .map(|r| Exp {
            text: format!("{}/*", r.text.trim_end_matches('/')),
            ..r.clone()
        })
        .collect();
    let mut k = 0;
    while k < args.len() {
        let a = args[k].text.as_str();
        match a {
            "-delete" => {
                let paths: Vec<_> = matched.iter().map(|m| w.path(m)).collect();
                w.effect(Effect::FileDelete {
                    paths,
                    recursive: true,
                });
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                let end =
                    args[k + 1..].iter().position(|x| x.text == ";" || x.text == "+").map_or(args.len(), |p| k + 1 + p);
                for m in &matched {
                    let cmd: Vec<Exp> = args[k + 1..end]
                        .iter()
                        .map(|x| {
                            if x.text == "{}" {
                                m.clone()
                            } else {
                                x.clone()
                            }
                        })
                        .collect();
                    if !cmd.is_empty() {
                        let _out: Input = run(w, &cmd, Input::None);
                    }
                }
                k = end;
            }
            "-fprint" | "-fprint0" | "-fprintf" | "-fls" => {
                if let Some(f) = args.get(k + 1) {
                    let p = w.path(f);
                    w.write(p);
                }
            }
            _ => {}
        }
        k += 1;
    }
    Input::Opaque("a list of files".to_owned())
}

/// curl's and wget's options this model reads (with or without a value); any other option is not read.
const CURL_LONG: &[&str] = &[
    "silent",
    "show-error",
    "location",
    "insecure",
    "fail",
    "fail-with-body",
    "include",
    "head",
    "verbose",
    "output",
    "remote-name",
    "remote-name-all",
    "data",
    "data-binary",
    "data-raw",
    "data-urlencode",
    "data-ascii",
    "json",
    "request",
    "header",
    "user",
    "form",
    "form-string",
    "upload-file",
    "user-agent",
    "referer",
    "cookie",
    "cookie-jar",
    "write-out",
    "max-time",
    "connect-timeout",
    "retry",
    "retry-delay",
    "retry-max-time",
    "retry-all-errors",
    "retry-connrefused",
    "proxy",
    "compressed",
    "get",
    "http1.1",
    "http2",
    "ipv4",
    "ipv6",
    "globoff",
    "no-progress-meter",
    "progress-bar",
    "create-dirs",
    "max-redirs",
    "oauth2-bearer",
    "cacert",
    "resolve",
    "url",
    "range",
    "continue-at",
    "limit-rate",
    "no-buffer",
    "keepalive-time",
    "tlsv1.2",
    "tlsv1.3",
    "unix-socket",
    "dump-header",
    "stderr",
    "trace-ascii",
    "netrc",
    "netrc-optional",
    "noproxy",
    "basic",
    "digest",
    "anyauth",
    "ssl-no-revoke",
    "path-as-is",
    "help",
    "version",
    "url-query",
    "variable",
    "expand-url",
    // wget
    "quiet",
    "output-document",
    "directory-prefix",
    "post-data",
    "post-file",
    "body-data",
    "body-file",
    "method",
    "no-check-certificate",
    "timeout",
    "tries",
    "continue",
    "no-verbose",
    "show-progress",
    "recursive",
    "level",
    "no-parent",
    "mirror",
    "page-requisites",
    "convert-links",
    "adjust-extension",
    "spider",
    "server-response",
    "no-clobber",
    "user-agent",
    "header",
    "content-disposition",
    "trust-server-names",
];

fn network(w: &mut Walk<'_>, program: &str, args: &[Exp]) -> Input {
    let (flags, values) = if program == "wget" {
        ("qvcrNkpmSnbi46xEKF", "OoaPtTUdlA")
    } else {
        ("sSLkfiIvOGgnqZNj46#lRJpBMa0123", "odXHuFTAebcwmxEKCrzYyUQDt")
    };
    known_options(w, program, args, flags, values, CURL_LONG);
    let hosts: Vec<String> = args.iter().filter_map(|a| url_host(&a.text)).collect();
    let hosts = if hosts.is_empty() {
        // `curl host/path` without a scheme.
        positional(
            args,
            &[
                "-o",
                "-O",
                "-d",
                "-X",
                "-H",
                "-u",
                "-F",
                "-T",
                "-A",
                "-e",
                "-b",
                "-c",
                "-w",
                "-m",
                "--output",
                "--data",
                "--header",
                "--user",
                "--form",
                "--upload-file",
                "--request",
                "--max-time",
                "--connect-timeout",
                "--retry",
                "-x",
                "--proxy",
                "--resolve",
                "--data-binary",
                "--data-raw",
                "--data-urlencode",
                "--json",
                "--cacert",
                "--cert",
                "--key",
                "-E",
                "--post-data",
                "--post-file",
                "--header",
                "-P",
                "--directory-prefix",
                "--user-agent",
                "-U",
            ],
        )
        .into_iter()
        .filter(|a| a.known() && a.text.contains('.'))
        .map(|a| a.text.split('/').next().unwrap_or_default().to_ascii_lowercase())
        .collect()
    } else {
        hosts
    };
    let hosts = if hosts.is_empty() {
        w.lower(Certainty::Partial, format!("the host {program} talks to"));
        vec!["a host not known".to_owned()]
    } else {
        hosts
    };
    let mut write = false;
    let mut uploads: Vec<Exp> = Vec::new();
    let mut k = 0;
    while k < args.len() {
        let t = args[k].text.as_str();
        let value = args.get(k + 1);
        let attached = |p: &str| t.strip_prefix(p).filter(|v| !v.is_empty());
        if matches!(
            t,
            "-d" | "--data"
                | "--data-binary"
                | "--data-raw"
                | "--data-urlencode"
                | "--json"
                | "-F"
                | "--form"
                | "--form-string"
                | "--post-data"
                | "--post-file"
                | "--body-data"
                | "--body-file"
        ) || t.starts_with("--data")
            || attached("-d").is_some()
            || attached("-F").is_some()
        {
            write = true;
            let v = attached("-d")
                .or_else(|| attached("-F"))
                .map(str::to_owned)
                .or_else(|| t.split_once('=').filter(|_| t.starts_with("--")).map(|(_, v)| v.to_owned()))
                .or_else(|| value.map(|v| v.text.clone()));
            if let Some(v) = v {
                // `@file`, `name=@file`, `name=<file`
                let file = v
                    .strip_prefix('@')
                    .or_else(|| v.split_once("=@").map(|(_, f)| f))
                    .or_else(|| v.split_once("=<").map(|(_, f)| f))
                    // `--data-urlencode name@file`
                    .or_else(|| {
                        v.split_once('@')
                            .filter(|(name, _)| t.starts_with("--data-urlencode") && !name.contains('='))
                            .map(|(_, f)| f)
                    });
                if let Some(f) = file.filter(|f| !f.is_empty() && *f != "-") {
                    uploads.push(Exp::lit(f.split(';').next().unwrap_or(f)));
                }
                if matches!(t, "--post-file" | "--body-file") {
                    uploads.push(Exp::lit(&v));
                }
            }
        }
        if matches!(t, "-T" | "--upload-file") {
            write = true;
            if let Some(v) = value {
                uploads.push(v.clone());
            }
        }
        if matches!(t, "-X" | "--request" | "--method")
            && value.is_some_and(|v| !v.text.eq_ignore_ascii_case("GET") && !v.text.eq_ignore_ascii_case("HEAD"))
        {
            write = true;
        }
        // The file it writes: `-o F`, `-oF`, `--output=F` (wget: `-O`, `--output-document`).
        let output_flags: &[&str] = if program == "wget" {
            &["-O", "--output-document", "-o", "--output-file", "-a", "--append-output"]
        } else {
            &["-o", "--output"]
        };
        let output = if output_flags.contains(&t) {
            value.cloned()
        } else {
            output_flags.iter().find_map(|f| {
                let rest = if f.starts_with("--") {
                    t.strip_prefix(f).and_then(|r| r.strip_prefix('='))
                } else {
                    t.strip_prefix(f).filter(|r| !r.is_empty())
                };
                rest.map(|r| Exp {
                    text: r.to_owned(),
                    ..args[k].clone()
                })
            })
        };
        if let Some(v) = output.filter(|v| v.text != "-" && !v.text.starts_with('-')) {
            let p = w.path(&v);
            w.write(p);
        }
        // A configuration file: its options are not read.
        if matches!(t, "-K" | "--config") || t.starts_with("--config=") || (t.starts_with("-K") && program != "wget") {
            w.lower(Certainty::None, format!("the options in {program}'s configuration file"));
        }
        if matches!(t, "-H" | "--header" | "-u" | "--user" | "-b" | "--cookie" | "--oauth2-bearer")
            && let Some(v) = value
        {
            let lower = v.text.to_ascii_lowercase();
            if t == "-u"
                || t == "--user"
                || t == "--oauth2-bearer"
                || lower.starts_with("authorization")
                || lower.contains("token")
                || lower.contains("api-key")
                || lower.contains("apikey")
                || t == "-b"
                || t == "--cookie"
            {
                for host in &hosts {
                    if !local_host(host) {
                        w.effect(Effect::SecretOut {
                            secret: "a credential in the request".to_owned(),
                            destination: host.clone(),
                        });
                    }
                }
            }
        }
        k += 1;
    }
    if program == "wget"
        && !has(args, "-O")
        && !args.iter().any(|a| {
            (a.text.starts_with("-O") && a.text.len() > 2)
                || a.text.starts_with("--output-document=")
                || (a.text.starts_with('-') && !a.text.starts_with("--") && a.text.ends_with("O-"))
        })
    {
        let dir = opt_value(args, &["-P", "--directory-prefix"])
            .cloned()
            .or_else(|| {
                args.iter().find_map(|a| {
                    a.text
                        .strip_prefix("--directory-prefix=")
                        .or_else(|| a.text.strip_prefix("-P").filter(|r| !r.is_empty()))
                        .map(|r| Exp {
                            text: r.to_owned(),
                            ..a.clone()
                        })
                })
            })
            .unwrap_or_else(|| Exp::lit("."));
        let p = w.path(&dir);
        w.write(p);
    }
    for u in &uploads {
        w.read_file(u);
        if let FileKind::Secret(_) | FileKind::Env = file_kind(&u.text) {
            for host in &hosts {
                if !local_host(host) {
                    w.effect(Effect::SecretOut {
                        secret: u.text.clone(),
                        destination: host.clone(),
                    });
                }
            }
        }
    }
    let executable = uploads.iter().any(|u| is_executable_name(&u.text));
    for host in &hosts {
        let method = if executable {
            NetMethod::UploadExecutable
        } else if write {
            NetMethod::Write
        } else {
            NetMethod::Read
        };
        w.effect(Effect::Network {
            host: host.clone(),
            method,
        });
    }
    let to_stdout = args.iter().any(|a| matches!(a.text.as_str(), "-O-" | "-qO-" | "-qO" | "--output-document=-"))
        || args
            .windows(2)
            .any(|p| (p[0].text == "-O" || p[0].text == "--output-document" || p[0].text == "-qO") && p[1].text == "-");
    if program == "wget" && !to_stdout {
        Input::Opaque("wget".to_owned())
    } else {
        Input::Downloaded(hosts[0].clone())
    }
}

fn ssh(w: &mut Walk<'_>, args: &[Exp], input: Input) -> Input {
    const VALUES: &str = "bcDEeFIiJLlmOopQRSWw";
    // Options that run a program on this computer (`ProxyCommand`, `LocalCommand`, a config file that may set them).
    let runs_local = args.windows(2).any(|p| {
        p[0].text == "-o" && {
            let o = p[1].text.to_ascii_lowercase();
            o.starts_with("proxycommand")
                || o.starts_with("localcommand")
                || o.starts_with("permitlocalcommand")
                || o.starts_with("knownhostscommand")
                || o.starts_with("match")
        }
    }) || args
        .iter()
        .any(|a| a.text == "-F" || a.text.starts_with("-oProxyCommand") || a.text.starts_with("-oLocalCommand"));
    if runs_local {
        w.lower(Certainty::None, "ssh runs a program on this computer that its options name");
    }
    let mut k = 0;
    let mut key = None;
    let mut tunnel = false;
    while let Some(a) = args.get(k) {
        let t = a.text.as_str();
        if !t.starts_with('-') || t.len() < 2 {
            break;
        }
        let last = t.chars().last().unwrap_or_default();
        if t == "-i" {
            key = args.get(k + 1).cloned();
        }
        if t.contains('N') || t.contains('L') || t.contains('R') || t.contains('D') || t.contains('W') {
            tunnel = true;
        }
        k += 1 + usize::from(VALUES.contains(last) && t.len() == 2);
    }
    if let Some(key) = key {
        w.effect(Effect::SecretUse {
            secret: key.text,
            local_only: true,
        });
    }
    let Some(dest) = args.get(k) else {
        return Input::None;
    };
    let host = dest.text.rsplit('@').next().unwrap_or(&dest.text).to_owned();
    let command: Vec<&Exp> = args[k + 1..].iter().collect();
    if command.is_empty() {
        match input {
            Input::Text(t) => {
                w.effect(Effect::Network {
                    host: host.clone(),
                    method: NetMethod::Read,
                });
                w.run_remote(&host, &t);
            }
            Input::Uncertain(t) => {
                w.effect(Effect::Network {
                    host: host.clone(),
                    method: NetMethod::Read,
                });
                w.run_remote(&host, &t.replace(UNKNOWN, "\"$__reins_unknown__\""));
                w.lower(Certainty::None, "a remote script with values that are not in the command");
            }
            _ => {
                w.effect(Effect::Network {
                    host,
                    method: NetMethod::Connect,
                });
                if !tunnel {
                    w.lower(Certainty::None, "what an interactive ssh session does");
                }
            }
        }
        return Input::Opaque("ssh".to_owned());
    }
    let uncertain = command.iter().any(|c| !c.known());
    let line = command.iter().map(|c| c.text.replace(UNKNOWN, "\u{2026}")).collect::<Vec<_>>().join(" ");
    w.effect(Effect::Network {
        host: host.clone(),
        method: NetMethod::Read,
    });
    w.run_remote(&host, &line);
    if uncertain {
        // Code for another computer with values that are not in the command: never read with certainty.
        w.lower(Certainty::None, "a remote command with values that are not in the command");
    }
    Input::Opaque("ssh".to_owned())
}

fn reins(w: &mut Walk<'_>, args: &[Exp]) -> Input {
    let sub: Vec<&str> = args.iter().map(|a| a.text.as_str()).take(2).collect();
    match sub.as_slice() {
        ["secrets", "scan" | "import" | "get", ..] => secret_to_agent(w, "secrets found on this computer"),
        ["payments-trust", ..] => w.effect(Effect::Permission {
            resource: "card details".to_owned(),
            change: "trust a payment key".to_owned(),
        }),
        ["run", ..] => {
            w.effect(Effect::SecretOut {
                secret: "secrets from the phone".to_owned(),
                destination: "a local program".to_owned(),
            });
            if let Some(k) = args.iter().position(|a| a.text == "--") {
                let _out: Input = run(w, &args[k + 1..], Input::None);
            }
        }
        // A question for the owner, and Reins's own status.
        ["ask" | "status" | "version" | "--version" | "help" | "--help" | "doctor", ..] | [] => {}
        [other, ..] => unmodelled(w, "reins", other),
    }
    Input::None
}
