//! Windows: what Linux and macOS give this app through file modes, unix sockets, signals and service managers, done with
//! what every Windows ships (`icacls`, `reg`, `powershell`, named pipes) and without unsafe code (the workspace forbids
//! it, so no direct Win32 calls).
//!
//! The pure helpers here (paths, quoting, the PE header, `reg` output) compile everywhere, so their tests run on every
//! platform; what starts Windows programs is behind `cfg(windows)`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// `CREATE_NO_WINDOW`: a console program started with it gets a hidden console instead of a new window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `CREATE_NEW_PROCESS_GROUP`: Ctrl-C and Ctrl-Break in the terminal that started it do not reach it.
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
/// `CREATE_BREAKAWAY_FROM_JOB`: it outlives a job object the starting terminal kills on close (when the job allows).
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// A program in `%SystemRoot%\System32` by its full path, so a same-named program earlier on `PATH` (or in the current
/// directory) is never run instead; just the name when `SystemRoot` is not set.
#[must_use]
pub fn system_program(system_root: Option<&OsStr>, name: &str) -> PathBuf {
    match system_root.filter(|r| !r.is_empty()) {
        Some(root) => Path::new(root).join("System32").join(name),
        None => PathBuf::from(name),
    }
}

/// [`system_program`] for this process's `SystemRoot`.
#[must_use]
pub fn system32(name: &str) -> PathBuf {
    system_program(std::env::var_os("SystemRoot").as_deref(), name)
}

/// `C:\x` for `\\?\C:\x` and `\\server\share\x` for `\\?\UNC\server\share\x` (what `canonicalize` returns on Windows);
/// other paths as they are. Harnesses, `reg` and shells take the plain form.
#[must_use]
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() >= 2 && rest.as_bytes()[1] == b':' && rest.as_bytes()[0].is_ascii_alphabetic() => {
            PathBuf::from(rest)
        }
        _ => path.to_path_buf(),
    }
}

/// How a harness hook runs this program on Windows. Claude Code runs hook commands with Git Bash (PowerShell without
/// it), other harnesses with PowerShell or cmd: an unquoted path with forward slashes is the one form all of them run.
/// A path that needs quoting (a space in the user name) has no such form: then the bare `reins` when this program is
/// the one on `PATH` (`on_path`), else the path in double quotes, which Git Bash and cmd run (PowerShell would need
/// `& "…"`).
#[must_use]
pub fn hook_program(exe: &str, on_path: bool) -> String {
    let forward = exe.replace('\\', "/");
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+=:@".contains(c);
    if !forward.is_empty() && forward.chars().all(plain) {
        forward
    } else if on_path {
        "reins".to_owned()
    } else {
        format!("\"{forward}\"")
    }
}

/// Whether the directory of `exe` is one of `path_dirs` (compared as Windows does: ignoring case and a trailing
/// separator), so the bare program name runs `exe`.
#[must_use]
pub fn dir_on_path(exe: &Path, path_dirs: &[PathBuf]) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    let exe = norm(exe);
    let Some((dir, _)) = exe.rsplit_once('\\') else {
        return false;
    };
    path_dirs.iter().any(|d| norm(d) == dir)
}

/// The program a bare name like `npm` runs on Windows: the first directory of `path_dirs` holding `name` with one of
/// the `PATHEXT` endings (`.COM;.EXE;.BAT;.CMD` by default). Rust's `Command` itself only looks for `name.exe`, so
/// `reins run -- npm test` would not find `npm.cmd`. A name with a directory or an ending of its own is left alone.
#[must_use]
pub fn resolve_program(
    name: &OsStr,
    path_dirs: &[PathBuf],
    pathext: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let name = name.to_str()?;
    if name.is_empty() || name.contains(['/', '\\', ':']) || Path::new(name).extension().is_some() {
        return None;
    }
    let pathext = pathext.filter(|e| !e.trim().is_empty()).unwrap_or(".COM;.EXE;.BAT;.CMD");
    let exts: Vec<&str> = pathext.split(';').map(str::trim).filter(|e| e.starts_with('.') && e.len() > 1).collect();
    path_dirs.iter().find_map(|dir| {
        exts.iter().map(|ext| dir.join(format!("{name}{}", ext.to_ascii_lowercase()))).find(|p| exists(p))
    })
}

/// The named pipe the SSH agent listens on for the state directory `state_dir`: one per state directory (and so per
/// user), `\\.\pipe\reins-ssh-agent-<16 hex digits>`.
#[must_use]
pub fn ssh_pipe(state_dir: &Path) -> PathBuf {
    use sha2::Digest as _;
    let key = state_dir.to_string_lossy().to_lowercase();
    let digest = sha2::Sha256::digest(key.as_bytes());
    PathBuf::from(format!(r"\\.\pipe\reins-ssh-agent-{}", data_encoding::HEXLOWER.encode(&digest[..8])))
}

/// A named pipe path: `\\.\pipe\name` (also written with forward slashes).
#[must_use]
pub fn is_pipe_path(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    s.strip_prefix(r"\\.\pipe\").is_some_and(|name| !name.is_empty() && !name.contains('\\'))
}

/// `//./pipe/name` for `\\.\pipe\name`: the form `IdentityAgent` takes in `~/.ssh/config`, where newer OpenSSH reads a
/// backslash as an escape and older OpenSSH does not; Windows opens both forms alike.
#[must_use]
pub fn forward_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

/// The `IMAGE_SUBSYSTEM_*` values this module deals with.
pub const SUBSYSTEM_GUI: u16 = 2;
pub const SUBSYSTEM_CONSOLE: u16 = 3;

/// Where the PE optional header starts, after checking the DOS stub, the `PE\0\0` signature and the header's magic.
fn optional_header(pe: &[u8]) -> Result<usize, String> {
    let bad = |what: &str| Err(format!("not a Windows program ({what})"));
    if pe.len() < 0x40 || &pe[..2] != b"MZ" {
        return bad("no MZ header");
    }
    let lfanew = u32::from_le_bytes([pe[0x3c], pe[0x3d], pe[0x3e], pe[0x3f]]) as usize;
    let coff = lfanew.checked_add(4).ok_or("bad header offset")?;
    if pe.get(lfanew..coff) != Some(b"PE\0\0".as_slice()) {
        return bad("no PE signature");
    }
    let size = pe.get(coff + 16..coff + 18).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    let opt = coff + 20;
    if size < 72 || pe.len() < opt + 72 {
        return bad("short optional header");
    }
    match u16::from_le_bytes([pe[opt], pe[opt + 1]]) {
        0x10b | 0x20b => Ok(opt),
        _ => bad("unknown optional header"),
    }
}

/// The subsystem of a PE file (`SUBSYSTEM_CONSOLE` for `reins.exe`).
#[must_use]
pub fn pe_subsystem(pe: &[u8]) -> Option<u16> {
    let opt = optional_header(pe).ok()?;
    Some(u16::from_le_bytes([pe[opt + 68], pe[opt + 69]]))
}

/// A copy of the console program `pe` that Windows starts without a console window: the subsystem field of the PE
/// header set to GUI, which is what `editbin /SUBSYSTEM:WINDOWS` does (the entry point stays `mainCRTStartup`, as with
/// Rust's `windows_subsystem = "windows"`). The header checksum is cleared; Windows checks it only for drivers and
/// boot-time libraries. Everything else is byte for byte the same program.
pub fn gui_copy(pe: &[u8]) -> Result<Vec<u8>, String> {
    let opt = optional_header(pe)?;
    let current = u16::from_le_bytes([pe[opt + 68], pe[opt + 69]]);
    if current != SUBSYSTEM_CONSOLE && current != SUBSYSTEM_GUI {
        return Err(format!("not a console program (subsystem {current})"));
    }
    let mut out = pe.to_vec();
    out[opt + 64..opt + 68].copy_from_slice(&[0; 4]);
    out[opt + 68..opt + 70].copy_from_slice(&SUBSYSTEM_GUI.to_le_bytes());
    Ok(out)
}

/// The data of value `name` in the output of `reg query <key> /v <name>` (`    Reins    REG_SZ    "C:\…" daemon`).
#[must_use]
pub fn reg_value(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(name)?.strip_prefix("    ")?;
        let (kind, data) = rest.split_once("    ").unwrap_or((rest, ""));
        kind.starts_with("REG_").then(|| data.trim_end_matches(['\r', '\n']).to_owned())
    })
}

/// One argument of a command line Windows splits with the usual rules (`CommandLineToArgvW`): in double quotes when it
/// has a space, a tab or nothing. Paths cannot contain `"`, and backslashes are literal unless they precede a quote, so
/// a trailing backslash is doubled.
#[must_use]
pub fn command_line_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The PowerShell that shows a Yes/No message box on top of the other windows, No as the default, and prints `Yes` or
/// `No`. The title and text come from environment variables, never from the script itself.
pub const MESSAGE_BOX_SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms; \
     $r = [System.Windows.Forms.MessageBox]::Show($env:REINS_PROMPT_TEXT, $env:REINS_PROMPT_TITLE, 'YesNo', \
     'Question', 'Button2', 'DefaultDesktopOnly'); [Console]::Out.Write([string]$r)";

/// The answer [`MESSAGE_BOX_SCRIPT`] printed: `Some(true)` Yes, `Some(false)` No, `None` nothing (killed, failed).
#[must_use]
pub fn message_box_answer(stdout: &str) -> Option<bool> {
    match stdout.trim() {
        "Yes" => Some(true),
        "No" => Some(false),
        _ => None,
    }
}

/// A native Yes/No message box (PowerShell and Windows Forms, both part of Windows). Killing the process (dropping
/// the future with `kill_on_drop`) takes the box down, so the callers' timeouts work as with the other platforms'
/// dialogs. `None` on other systems.
#[must_use]
pub fn message_box(title: &str, text: &str) -> Option<tokio::process::Command> {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new(system32(r"WindowsPowerShell\v1.0\powershell.exe"));
        c.args(["-NoProfile", "-NonInteractive", "-Command", MESSAGE_BOX_SCRIPT])
            .env("REINS_PROMPT_TITLE", title)
            .env("REINS_PROMPT_TEXT", text);
        hidden_async(&mut c);
        Some(c)
    }
    #[cfg(not(windows))]
    {
        let _ = (title, text);
        None
    }
}

/// This user's security identifier (`S-1-5-21-…`), from `whoami /user`; asked once per process.
#[cfg(windows)]
fn user_sid() -> Option<&'static str> {
    static SID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SID.get_or_init(|| {
        let out = hidden(std::process::Command::new(system32("whoami.exe")).args(["/user", "/fo", "csv", "/nh"]))
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        parse_whoami_sid(&String::from_utf8_lossy(&out.stdout))
    })
    .as_deref()
}

/// The SID in `whoami /user /fo csv /nh` output: `"desktop\me","S-1-5-21-1-2-3-1001"`.
#[must_use]
pub fn parse_whoami_sid(output: &str) -> Option<String> {
    let sid = output.trim().rsplit(',').next()?.trim().trim_matches('"');
    (sid.starts_with("S-1-") && sid.len() > 4 && sid.bytes().all(|b| b.is_ascii_digit() || b == b'-' || b == b'S'))
        .then(|| sid.to_owned())
}

/// The `icacls` arguments that leave `path` to this user alone: inherited entries removed, full control for the user
/// (`*SID`), inherited by what a directory holds.
#[must_use]
pub fn icacls_private_args(path: &Path, sid: &str, dir: bool) -> Vec<std::ffi::OsString> {
    let grant = if dir {
        format!("*{sid}:(OI)(CI)F")
    } else {
        format!("*{sid}:F")
    };
    vec![path.as_os_str().to_owned(), "/inheritance:r".into(), "/grant:r".into(), grant.into(), "/q".into()]
}

/// Makes `path` readable and writable by this user only (the Windows counterpart of mode 0600 / 0700). Under the user's
/// profile (`%APPDATA%`, `%LOCALAPPDATA%`) files are private to the user already (and to the administrators and
/// SYSTEM, as everything is); this also takes the administrators' and SYSTEM's inherited entries off.
#[cfg(windows)]
pub fn make_private(path: &Path, dir: bool) -> std::io::Result<()> {
    let Some(sid) = user_sid() else {
        return Err(std::io::Error::other("cannot find this user's SID (whoami /user)"));
    };
    let status = hidden(std::process::Command::new(system32("icacls.exe")).args(icacls_private_args(path, sid, dir)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("icacls could not restrict {} to this user ({status})", path.display())))
    }
}

/// Starts `cmd` without a console window of its own (it shares the hidden console of the daemon, or none).
#[cfg(windows)]
pub fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    std::os::windows::process::CommandExt::creation_flags(cmd, CREATE_NO_WINDOW)
}

/// [`hidden`] for tokio's `Command`.
#[cfg(windows)]
pub fn hidden_async(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_programs_come_from_system32() {
        assert_eq!(
            system_program(Some(OsStr::new(r"C:\Windows")), "reg.exe"),
            Path::new(r"C:\Windows").join("System32").join("reg.exe")
        );
        assert_eq!(system_program(None, "reg.exe"), PathBuf::from("reg.exe"));
        assert_eq!(system_program(Some(OsStr::new("")), "reg.exe"), PathBuf::from("reg.exe"));
    }

    #[test]
    fn verbatim_prefixes_are_dropped() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\Users\me\reins.exe")), PathBuf::from(r"C:\Users\me\reins.exe"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\srv\share\r.exe")), PathBuf::from(r"\\srv\share\r.exe"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\Volume{x}\r.exe")), PathBuf::from(r"\\?\Volume{x}\r.exe"));
        assert_eq!(strip_verbatim(Path::new("/usr/bin/reins")), PathBuf::from("/usr/bin/reins"));
    }

    #[test]
    fn hook_programs_run_in_every_windows_shell() {
        let exe = r"C:\Users\me\AppData\Local\Programs\Reins\reins.exe";
        assert_eq!(hook_program(exe, false), "C:/Users/me/AppData/Local/Programs/Reins/reins.exe");
        assert_eq!(hook_program(exe, true), "C:/Users/me/AppData/Local/Programs/Reins/reins.exe");
        let spaced = r"C:\Users\Jo Doe\AppData\Local\Programs\Reins\reins.exe";
        assert_eq!(hook_program(spaced, true), "reins");
        assert_eq!(hook_program(spaced, false), "\"C:/Users/Jo Doe/AppData/Local/Programs/Reins/reins.exe\"");
        assert_eq!(hook_program(r"C:\100%\reins.exe", false), "\"C:/100%/reins.exe\"");
    }

    #[test]
    fn the_program_directory_is_found_on_path_ignoring_case() {
        let exe = Path::new(r"C:\Users\Jo Doe\AppData\Local\Programs\Reins\reins.exe");
        let dirs = |d: &[&str]| d.iter().map(PathBuf::from).collect::<Vec<_>>();
        assert!(dir_on_path(exe, &dirs(&[r"C:\Windows", r"c:\users\jo doe\appdata\local\programs\reins\"])));
        assert!(dir_on_path(exe, &dirs(&["C:/Users/Jo Doe/AppData/Local/Programs/Reins"])));
        assert!(!dir_on_path(exe, &dirs(&[r"C:\Users\Jo Doe\AppData\Local\Programs"])));
        assert!(!dir_on_path(exe, &[]));
    }

    #[test]
    fn bare_names_find_cmd_and_bat_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("npm.cmd"), "").unwrap();
        std::fs::write(dir.path().join("tool.exe"), "").unwrap();
        let dirs = [PathBuf::from("/nowhere"), dir.path().to_path_buf()];
        let exists = |p: &Path| p.is_file();
        assert_eq!(resolve_program(OsStr::new("npm"), &dirs, None, exists), Some(dir.path().join("npm.cmd")));
        assert_eq!(
            resolve_program(OsStr::new("tool"), &dirs, Some(".EXE;.CMD"), exists),
            Some(dir.path().join("tool.exe"))
        );
        assert_eq!(resolve_program(OsStr::new("npm"), &dirs, Some(".EXE"), exists), None);
        assert_eq!(resolve_program(OsStr::new("npm.cmd"), &dirs, None, exists), None, "has an ending");
        assert_eq!(resolve_program(OsStr::new(r"C:\x\npm"), &dirs, None, exists), None, "has a directory");
        assert_eq!(resolve_program(OsStr::new("missing"), &dirs, None, exists), None);
    }

    #[test]
    fn the_ssh_pipe_is_per_state_directory() {
        let a = ssh_pipe(Path::new(r"C:\Users\me\AppData\Local\reins"));
        assert!(a.to_string_lossy().starts_with(r"\\.\pipe\reins-ssh-agent-"), "{}", a.display());
        assert_eq!(a.to_string_lossy().len(), r"\\.\pipe\reins-ssh-agent-".len() + 16);
        assert_eq!(a, ssh_pipe(Path::new(r"c:\users\me\appdata\local\reins")));
        assert_ne!(a, ssh_pipe(Path::new(r"C:\Users\other\AppData\Local\reins")));
        assert!(is_pipe_path(&a));
        assert!(is_pipe_path(Path::new("//./pipe/openssh-ssh-agent")));
        assert!(!is_pipe_path(Path::new(r"C:\pipe\x")));
        assert!(!is_pipe_path(Path::new(r"\\.\pipe\")));
        assert_eq!(forward_slashes(r"\\.\pipe\x"), "//./pipe/x");
    }

    /// A minimal PE32+ header: DOS stub, signature at 0x80, COFF header, 240-byte optional header.
    fn pe(subsystem: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84 + 16..0x84 + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = 0x84 + 20;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        b[opt + 64..opt + 68].copy_from_slice(&0x1234u32.to_le_bytes());
        b[opt + 68..opt + 70].copy_from_slice(&subsystem.to_le_bytes());
        b[0x1ff] = 0xaa;
        b
    }

    #[test]
    fn the_gui_copy_changes_only_the_subsystem_and_the_checksum() {
        let console = pe(SUBSYSTEM_CONSOLE);
        assert_eq!(pe_subsystem(&console), Some(SUBSYSTEM_CONSOLE));
        let gui = gui_copy(&console).unwrap();
        assert_eq!(pe_subsystem(&gui), Some(SUBSYSTEM_GUI));
        let opt = 0x84 + 20;
        let changed: Vec<usize> = (0..console.len()).filter(|&i| console[i] != gui[i]).collect();
        assert_eq!(changed, vec![opt + 64, opt + 65, opt + 68]);
        assert_eq!(gui_copy(&gui).unwrap(), gui, "already a GUI program");
        assert!(gui_copy(&pe(1)).is_err(), "a driver");
        assert!(gui_copy(b"#!/bin/sh\n").is_err());
        let mut no_sig = console.clone();
        no_sig[0x80] = b'X';
        assert!(gui_copy(&no_sig).is_err());
        let mut far = console;
        far[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(gui_copy(&far).is_err());
    }

    #[test]
    fn registry_values_are_read_from_reg_query() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    Reins    REG_SZ    \"C:\\Users\\me\\reins-daemon.exe\" daemon\r\n\r\n";
        assert_eq!(reg_value(out, "Reins").as_deref(), Some("\"C:\\Users\\me\\reins-daemon.exe\" daemon"));
        assert_eq!(reg_value(out, "Other"), None);
        assert_eq!(reg_value("    ReinsX    REG_SZ    x\r\n", "Reins"), None);
        assert_eq!(
            reg_value("ERROR: The system was unable to find the specified registry key or value.\r\n", "Reins"),
            None
        );
    }

    #[test]
    fn command_line_arguments_are_quoted_for_windows() {
        assert_eq!(command_line_arg(r"C:\x\reins.exe"), r"C:\x\reins.exe");
        assert_eq!(command_line_arg(r"C:\Jo Doe\reins.exe"), r#""C:\Jo Doe\reins.exe""#);
        assert_eq!(command_line_arg(r"C:\Jo Doe\"), r#""C:\Jo Doe\\""#);
        assert_eq!(command_line_arg(""), r#""""#);
        assert_eq!(command_line_arg(r#"a"b"#), r#""a\"b""#);
    }

    #[test]
    fn the_message_box_takes_its_text_from_the_environment_only() {
        assert!(MESSAGE_BOX_SCRIPT.contains("$env:REINS_PROMPT_TEXT, $env:REINS_PROMPT_TITLE"));
        assert!(MESSAGE_BOX_SCRIPT.contains("'Button2'"), "No is the default");
        assert_eq!(message_box_answer("Yes"), Some(true));
        assert_eq!(message_box_answer("No\r\n"), Some(false));
        assert_eq!(message_box_answer(""), None);
        assert_eq!(message_box_answer("Cancel"), None);
    }

    #[test]
    fn the_sid_comes_from_whoami_and_icacls_gets_it() {
        assert_eq!(
            parse_whoami_sid("\"desktop-1\\jo doe\",\"S-1-5-21-111-222-333-1001\"\r\n").as_deref(),
            Some("S-1-5-21-111-222-333-1001")
        );
        assert_eq!(parse_whoami_sid("ERROR: access denied"), None);
        assert_eq!(parse_whoami_sid(""), None);
        let args = icacls_private_args(Path::new("state"), "S-1-5-21-1", true);
        assert_eq!(args, ["state", "/inheritance:r", "/grant:r", "*S-1-5-21-1:(OI)(CI)F", "/q"]);
        assert_eq!(icacls_private_args(Path::new("f"), "S-1-5-21-1", false)[3], "*S-1-5-21-1:F");
    }
}
