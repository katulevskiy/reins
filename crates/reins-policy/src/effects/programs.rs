//! Program models: what one simple command does, from its program and its expanded words (01 §2.2).
//!
//! Each model reads the options the program really has and names effects on the paths, refs, hosts and people its
//! words name. A program with no model is `Exec{Unknown}` with partial certainty: never silently harmless.

use super::inline::{self, Lang};
use super::level::local_host;
use super::model::{Area, Certainty, CodeSource, Effect, FileKind, InfraChange, NetMethod, SecretKind};
use super::shell::{Exp, Input, UNKNOWN, Walk, file_kind};
use crate::shell::program_name;

/// Programs that only read (their file arguments are reads) and print.
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
    "tr",
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
    "pdftotext",
    "pdfinfo",
    "exiftool",
    "mediainfo",
    "ffprobe",
    "otool",
    "nm",
    "objdump",
    "readelf",
    "ldd",
    "lipo",
    "codesign",
    "plutil",
    "zipinfo",
    "unzip -l",
    "csplit",
    "expand",
    "unexpand",
    "look",
    "tsort",
];

/// Programs whose first argument is a pattern or a program text, the rest files they read.
const PATTERN_READERS: &[&str] =
    &["grep", "egrep", "fgrep", "rg", "ag", "ack", "zgrep", "jq", "yq", "gojq", "awk", "gawk", "mawk"];

/// Programs with no effect on anything a request cares about (their output goes to the agent).
const HARMLESS: &[&str] = &[
    "echo",
    "printf",
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
    "shift",
    "unset",
    "alias",
    "unalias",
    "hash",
    "type",
    "which",
    "whereis",
    "printenv",
    "id",
    "whoami",
    "groups",
    "date",
    "cal",
    "uname",
    "hostname",
    "hostnamectl",
    "uptime",
    "df",
    "free",
    "ps",
    "pgrep",
    "top",
    "htop",
    "lsof",
    "ss",
    "netstat",
    "ip",
    "ifconfig",
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
    "bc",
    "dc",
    "yes",
    "jobs",
    "disown",
    "kill",
    "pkill",
    "killall",
    "umask",
    "ulimit",
    "times",
    "history",
    "fc",
    "ls",
    "exa",
    "eza",
    "fd",
    "fdfind",
    "locate",
    "notify-send",
    "sysctl",
    "sw_vers",
    "system_profiler",
    "defaults",
    "scutil",
    "launchctl print",
    "wl-copy",
    "wl-paste",
    "xclip",
    "xsel",
    "pbcopy",
    "pbpaste",
    "open",
    "xdg-open",
    "tmux",
    "screen",
    "column",
    "env",
    "nice",
    "logger",
    "systemd-analyze",
    "journalctl",
    "dmesg",
    "loginctl",
    "busctl",
    "fc-list",
    "fc-match",
    "xrandr",
    "wlr-randr",
    "hyprctl",
    "swaymsg",
    "gsettings",
    "dconf",
    "pactl",
    "wpctl",
    "amixer",
    "playerctl",
    "brightnessctl",
    "nvidia-smi",
    "sensors",
    "vmstat",
    "iostat",
    "mpstat",
    "pidof",
    "pstree",
    "getent",
    "dig",
    "nslookup",
    "host",
    "ping",
    "traceroute",
    "mtr",
    "whois",
    "openssl version",
    "test",
    "cd",
    "pushd",
    "popd",
    "dirs",
    "export",
    "declare",
    "typeset",
    "local",
    "readonly",
    "read",
    "mapfile",
    "readarray",
    "getopts",
    "let",
    "trap",
    "source",
    ".",
    "eval",
    "exec",
    "command",
    "builtin",
    "caller",
    "complete",
    "compgen",
    "shellcheck",
    "tokei",
    "cloc",
    "scc",
];

/// Developer tools doing their usual job: builds, tests, formatters, linters, emulators, media tools.
const TOOLCHAIN: &[&str] = &[
    "make",
    "gmake",
    "just",
    "ninja",
    "cmake",
    "meson",
    "bazel",
    "gradle",
    "gradlew",
    "mvn",
    "mvnw",
    "ant",
    "sbt",
    "xcodebuild",
    "xcrun",
    "swift",
    "swiftc",
    "swiftlint",
    "swiftformat",
    "xcodegen",
    "go",
    "gofmt",
    "rustc",
    "rustup",
    "rustfmt",
    "clippy-driver",
    "cargo-nextest",
    "gcc",
    "g++",
    "cc",
    "c++",
    "clang",
    "clang++",
    "ld",
    "javac",
    "java",
    "kotlinc",
    "kotlin",
    "dotnet",
    "flutter",
    "dart",
    "tsc",
    "eslint",
    "prettier",
    "biome",
    "vite",
    "webpack",
    "esbuild",
    "rollup",
    "jest",
    "vitest",
    "mocha",
    "playwright",
    "pytest",
    "tox",
    "nox",
    "ruff",
    "black",
    "isort",
    "mypy",
    "pyright",
    "pylint",
    "flake8",
    "adb",
    "emulator",
    "avdmanager",
    "sdkmanager",
    "apksigner",
    "aapt",
    "aapt2",
    "zipalign",
    "bundletool",
    "ffmpeg",
    "magick",
    "convert",
    "montage",
    "composite",
    "mogrify",
    "inkscape",
    "rsvg-convert",
    "grim",
    "slurp",
    "wf-recorder",
    "gamescope",
    "gamescopectl",
    "swaybg",
    "imv",
    "mpv",
    "vlc",
    "sox",
    "lame",
    "pandoc",
    "typst",
    "latexmk",
    "pdflatex",
    "xelatex",
    "uniffi-bindgen",
    "protoc",
    "buf",
    "sqlx",
    "diesel",
    "wasm-pack",
    "wasm-bindgen",
    "trunk",
    "hugo",
    "zola",
    "jekyll",
    "mdbook",
    "deno",
    "bun",
    "node-gyp",
    "pod",
    "xcpretty",
    "xcbeautify",
    "simctl",
    "instruments",
    "lldb",
    "gdb",
    "valgrind",
    "perf",
    "strace",
    "ltrace",
    "hyperfine",
    "criterion",
    "cargo-llvm-cov",
    "grcov",
    "llvm-cov",
    "lcov",
    "gcov",
    "docker-compose",
    "act",
    "actionlint",
    "zizmor",
    "hadolint",
    "typos",
    "codespell",
    "markdownlint",
    "yamllint",
    "taplo",
    "shfmt",
    "clang-format",
    "clang-tidy",
    "ktlint",
    "detekt",
    "spotless",
    "golangci-lint",
    "staticcheck",
    "nix",
    "nix-build",
    "direnv",
    "asdf",
    "mise",
    "rtx",
    "pyenv",
    "nvm",
    "fnm",
    "volta",
    "corepack",
    "pipx",
    "uv",
    "uvx",
    "rye",
    "pdm",
    "hatch",
    "socat",
    "tailscale",
    "wayvnc",
    "zeron",
    "claude",
    "codex",
    "gemini",
    "cursor-agent",
    "grok",
    "ollama",
    "vllm",
    "llama-server",
    "elevenlabs",
    "javap",
    "jar",
    "jdeps",
    "jlink",
    "jshell",
    "d8",
    "r8",
    "studio",
    "idea",
    "code",
    "xcode-select",
    "qemu-img",
    "caddy",
    "nginx",
    "http-server",
    "serve",
    "live-server",
    "stripe",
    "ngrok",
    "cloudflared",
    "uvicorn",
    "gunicorn",
    "flask",
    "django-admin",
    "rails",
    "rake",
    "mix",
    "elixir",
    "erl",
    "ghc",
    "cabal",
    "stack",
    "zig",
    "nim",
    "crystal",
    "dune",
    "opam",
    "swiftly",
    "tuist",
    "sccache",
    "ccache",
    "mold",
    "lld",
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
pub(super) fn run(w: &mut Walk<'_>, words: &[Exp], input: Input, piped: bool) -> Input {
    let program = program_name(&words[0].text);
    let lower = program.to_ascii_lowercase();
    let args = &words[1..];
    let runs_from_path = words[0].text.contains('/');
    match lower.as_str() {
        // ---- wrappers: what they run is read
        "sudo" | "doas" | "run0" | "pkexec" => {
            let at =
                after_options(args, "ugChpUrtDR", &["user", "group", "host", "prompt", "chdir", "role", "type"], 0);
            if at < args.len() {
                return run(w, &args[at..], input, piped);
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
                return run(w, &args[at..], input, piped);
            }
            Input::Opaque("the environment".to_owned())
        }
        "nice" | "nohup" | "time" | "command" | "builtin" | "exec" | "setsid" | "stdbuf" | "ionice" | "chrt"
        | "caffeinate" | "unbuffer" | "catchsegv" | "torsocks" | "proxychains" | "proxychains4" | "systemd-run"
        | "strace" | "ltrace" | "valgrind" | "rr" | "firejail" | "bwrap" => {
            if lower == "command" && (has(args, "-v") || has(args, "-V")) {
                return Input::Opaque("a lookup".to_owned());
            }
            let at = after_options(
                args,
                "nfoaioecpkSsEuUgGtT",
                &["adjustment", "format", "output", "unit", "uid", "gid", "property"],
                0,
            );
            if at < args.len() {
                return run(w, &args[at..], input, piped);
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
                return run(w, &args[at..], input, piped);
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
                return run(w, &args[at..], input, piped);
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
                Input::Text(t) => cmd.extend(t.split_whitespace().take(64).map(Exp::lit)),
                _ => cmd.push(super::shell::unknown("the words xargs reads from its input".to_owned())),
            }
            return run(w, &cmd, Input::None, piped);
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
                    return run(w, &args[k + 1..], input, piped);
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
        "uv" | "poetry" | "pipenv" | "bundle" | "pnpm" | "npx" | "bunx" | "pdm" | "hatch" | "rye"
            if is_runner(&lower, args) =>
        {
            let start = runner_start(&lower, args);
            if start < args.len() {
                return run(w, &args[start..], input, piped);
            }
            w.effect(Effect::Exec {
                program: lower.clone(),
                source: CodeSource::Toolchain,
            });
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
                    // Code with a value or two from the shell in it: read with those values unknown.
                    w.note(code);
                    let text = code.text.replace(UNKNOWN, "__reins_unknown__");
                    inline::analyze_code(w, lang, &text, &argv);
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
            let target = args.iter().find(|a| !a.text.starts_with('-') || a.text == "-");
            match target {
                Some(t) if t.text == "-" => w.cwd.clear(),
                Some(t) if t.known() => {
                    w.cwd = super::workspace::normalize(&t.text, &w.cwd.clone(), &w.ctx.workspace.home);
                }
                Some(t) => {
                    w.note(t);
                    w.cwd.clear();
                }
                None => w.cwd.clone_from(&w.ctx.workspace.home),
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
            let paths: Vec<_> = targets.iter().map(|t| w.path(t)).collect();
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
            if lower != "truncate" && !targets.is_empty() {
                targets.remove(0);
            }
            for t in targets {
                let p = w.path(t);
                w.write(p);
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
            let in_place = args.iter().any(|a| {
                a.text == "-i"
                    || a.text.starts_with("-i")
                    || a.text.starts_with("--in-place")
                    || (a.text.starts_with('-') && !a.text.starts_with("--") && a.text.contains('i'))
            });
            let script_given = has(args, "-e") || has(args, "-f") || has(args, "--expression");
            let mut files = positional(args, &["-e", "-f", "--expression", "--file", "-l"]);
            if !script_given && !files.is_empty() {
                files.remove(0);
            }
            for f in files {
                if in_place {
                    let p = w.path(f);
                    w.write(p);
                } else {
                    w.read_file(f);
                }
            }
            Input::Opaque("sed".to_owned())
        }
        "find" | "gfind" => find(w, args, piped),
        "tar" | "bsdtar" | "gtar" => {
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
        "glab" | "tea" | "hub" => {
            unknown_program(w, &lower);
            Input::None
        }

        // ---- network
        "curl" | "wget" | "http" | "https" | "xh" | "httpie" | "aria2c" => network(w, &lower, args, piped),
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
        | "mysqladmin" => super::infra::database(w, &lower, args, input),
        "kubectl" | "oc" | "k9s" | "helm" | "terraform" | "tofu" | "terragrunt" | "pulumi" | "cdk" | "aws"
        | "gcloud" | "gsutil" | "az" | "doctl" | "flyctl" | "fly" | "vercel" | "netlify" | "heroku" | "wrangler"
        | "firebase" | "eksctl" | "kind" | "minikube" | "ansible" | "ansible-playbook" | "serverless" | "sls"
        | "railway" | "render" | "supabase" => super::infra::cloud(w, &lower, args),
        "docker" | "podman" | "nerdctl" | "buildah" | "skopeo" => super::infra::containers(w, &lower, args),
        "systemctl" | "launchctl" | "service" | "rc-service" | "sv" | "crontab" | "at" => {
            super::infra::services(w, &lower, args)
        }
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
                ) || a.text.starts_with("-Q")
                    || a.text.starts_with("-Ss")
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
                _ => w.effect(Effect::Exec {
                    program: format!("{lower} {sub}").trim().to_owned(),
                    source: CodeSource::Workspace,
                }),
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
                _ => w.effect(Effect::Exec {
                    program: format!("cargo {sub}").trim().to_owned(),
                    source: CodeSource::Workspace,
                }),
            }
            Input::Opaque("cargo".to_owned())
        }
        "pip" | "pip3" | "twine" | "gem" | "poetry" | "flit" | "maturin" | "hatch" | "pdm" | "uv" | "rye"
        | "dotnet" | "mvn" | "gradle" | "gradlew" | "fastlane" | "pod" => {
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
            if publishes
                || (lower == "fastlane"
                    && args
                        .iter()
                        .any(|a| matches!(a.text.as_str(), "beta" | "release" | "deliver" | "pilot" | "supply")))
            {
                w.effect(Effect::Publish {
                    target: format!("a {lower} package or release"),
                });
            } else {
                toolchain(w, &lower, args, runs_from_path);
            }
            Input::Opaque(lower)
        }
        "reins" => reins(w, args),
        "gpg" | "gpg2" => {
            if args.iter().any(|a| {
                matches!(a.text.as_str(), "--export-secret-keys" | "--export-secret-subkeys" | "-d" | "--decrypt")
            }) {
                secret_to_agent(w, "a GPG secret");
            }
            Input::Decoded
        }
        "ssh-keygen" => {
            if let Some(f) = opt_value(args, &["-f"]) {
                let p = w.path(f);
                if !(short(args, 'l') || short(args, 'y') || short(args, 'F') || short(args, 'R')) {
                    w.write(p);
                } else if short(args, 'y') {
                    w.read_file(f);
                }
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
            let given = has(args, "-e") || has(args, "-f") || has(args, "--regexp") || has(args, "--file");
            let mut files = positional(
                args,
                &[
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
                    "-t",
                    "--type",
                    "-T",
                    "--type-not",
                    "--arg",
                    "--argjson",
                    "--slurpfile",
                    "--rawfile",
                    "-v",
                    "-F",
                    "--include",
                    "--exclude",
                    "--exclude-dir",
                    "--color",
                    "--colour",
                    "-d",
                    "-D",
                    "--context",
                    "--after-context",
                    "--before-context",
                    "-M",
                    "--max-columns",
                    "-j",
                    "--threads",
                    "--sort",
                    "--sortr",
                    "--field-match-separator",
                    "-r",
                    "--replace",
                ],
            );
            if !given && !files.is_empty() {
                files.remove(0);
            }
            if matches!(p, "awk" | "gawk" | "mawk")
                && args
                    .iter()
                    .any(|a| a.text.contains("system(") || a.text.contains("| getline") || a.text.contains("inplace"))
            {
                w.lower(Certainty::Partial, "an awk program that runs commands or writes files");
            }
            for f in files {
                w.read_file(f);
            }
            Input::Opaque(p.to_owned())
        }
        p if READERS.contains(&p) => {
            for f in positional(
                args,
                &["-n", "-c", "-k", "-t", "-d", "-f", "-s", "-w", "-L", "-I", "-o", "--max-depth", "-l"],
            ) {
                w.read_file(f);
            }
            if p == "sort"
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
        p if TOOLCHAIN.contains(&p) || p.starts_with("llvm-") || p.starts_with("clang-") || p.starts_with("gcc-") => {
            toolchain(w, p, args, runs_from_path);
            Input::Opaque(p.to_owned())
        }
        _ => {
            if runs_from_path || program.ends_with(".sh") || program.ends_with(".py") {
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
        "npx" | "bunx" => true,
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

fn unknown_program(w: &mut Walk<'_>, program: &str) {
    w.effect(Effect::Exec {
        program: program.to_owned(),
        source: CodeSource::Unknown,
    });
    w.lower(Certainty::Partial, format!("what `{program}` does"));
}

/// A secret printed into the agent's context (08 R4: a secret read).
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
fn toolchain(w: &mut Walk<'_>, program: &str, args: &[Exp], from_path: bool) {
    let _ = from_path;
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

/// Runs a script file: one the line wrote (read exactly), the project's own (its entry points: L0), or unknown.
fn run_file(w: &mut Walk<'_>, file: &Exp, args: &[Exp], lang: Lang) {
    let path = w.path(file);
    let written = w.files.get(&path.path).cloned();
    match written {
        Some(Some(text)) => {
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
            w.lower(Certainty::Partial, format!("the script {} this command writes", path.path));
        }
        None if path.area.in_workspace() => {
            let name = path.path.rsplit('/').next().unwrap_or(&path.path).to_owned();
            if TOOLCHAIN.contains(&name.as_str()) {
                toolchain(w, &name, args, true);
            } else {
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
                let _out: Input = run(w, &words, Input::None, false);
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
    let explicit = opt_value(args, &["-t", "--target-directory"]).cloned();
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

fn find(w: &mut Walk<'_>, args: &[Exp], piped: bool) -> Input {
    let _ = piped;
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
                        let _out: Input = run(w, &cmd, Input::None, false);
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

fn network(w: &mut Walk<'_>, program: &str, args: &[Exp], piped: bool) -> Input {
    let _ = piped;
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
                    .or_else(|| v.split_once("=<").map(|(_, f)| f));
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
        if matches!(t, "-o" | "--output" | "-O" | "--output-document")
            && let Some(v) = value.filter(|v| v.text != "-" && !v.text.starts_with('-'))
        {
            let p = w.path(v);
            w.write(p);
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
        && !has(args, "-qO-")
        && !args.iter().any(|a| a.text == "-O-" || a.text == "--output-document=-")
    {
        let dir = opt_value(args, &["-P", "--directory-prefix"]).cloned().unwrap_or_else(|| Exp::lit("."));
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
            _ => {
                w.effect(Effect::Network {
                    host,
                    method: NetMethod::Connect,
                });
                if !tunnel {
                    w.lower(Certainty::Partial, "what an interactive ssh session does");
                }
            }
        }
        return Input::Opaque("ssh".to_owned());
    }
    if command.iter().any(|c| !c.known()) {
        for c in &command {
            w.note(c);
        }
    }
    let line = command.iter().map(|c| c.text.replace(UNKNOWN, "\u{2026}")).collect::<Vec<_>>().join(" ");
    w.effect(Effect::Network {
        host: host.clone(),
        method: NetMethod::Read,
    });
    w.run_remote(&host, &line);
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
                let _out: Input = run(w, &args[k + 1..], Input::None, false);
            }
        }
        _ => {}
    }
    Input::None
}
