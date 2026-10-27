//! `rm` with a net: GNU rm's behaviour for the options the planner nets (`-r -R -f -d -v`, `--recursive`, `--force`,
//! `--dir`, `--verbose`, `--interactive=never`, `--preserve-root`, `--`), except that what it removes goes into the
//! Reins trash. Scratch (the OS temp folders, `~/.cache`, configured scratch folders) is deleted for real: nothing is
//! lost there.
//!
//! It never deletes what it could not keep: an option it does not model refuses the whole command before anything,
//! and a path it can't move into the trash (another disk, a full disk, the trash itself) is left where it is, with an
//! error, and the failure is recorded so the same command is asked without the net next time.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use reins_policy::effects::nets::rm_args;

use super::store::{NetError, Store};

/// What deciding a path's area at run time needs.
pub struct Scratch<'a>(pub &'a dyn Fn(&Path) -> bool);

/// The message when a path could not be kept.
fn not_kept(op: &str, why: &dyn std::fmt::Display) -> String {
    format!("reins: couldn't keep a copy of '{op}' ({why}), so it was not deleted. Run the command again to be asked.")
}

/// Runs `rm ARGS` in `cwd` with the trash net of `plan`. Returns the exit code (0, or 1 when anything failed).
pub fn rm(
    store: &Store,
    plan: &str,
    args: &[OsString],
    cwd: &Path,
    scratch: &Scratch<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let Some(texts) = args.iter().map(|a| a.to_str().map(str::to_owned)).collect::<Option<Vec<String>>>() else {
        let _said = writeln!(err, "reins: rm: a name that is not text can't be netted; nothing was deleted.");
        store.fail_plan(plan, "a name that is not text");
        return 1;
    };
    let a = match rm_args(&texts) {
        Ok(a) => a,
        Err(what) => {
            let _said =
                writeln!(err, "reins: {what} can't be netted; nothing was deleted. Run the command again to be asked.");
            store.fail_plan(plan, &what);
            return 1;
        }
    };
    if a.operands.is_empty() {
        if a.force {
            return 0;
        }
        let _said = writeln!(err, "rm: missing operand");
        return 1;
    }
    let mut status = 0;
    for op in &a.operands {
        if let Err(message) = one(store, plan, op, &a, cwd, scratch, out) {
            let _said = writeln!(err, "{message}");
            status = 1;
        }
    }
    status
}

fn one(
    store: &Store,
    plan: &str,
    op: &str,
    a: &reins_policy::effects::nets::RmArgs,
    cwd: &Path,
    scratch: &Scratch<'_>,
    out: &mut dyn Write,
) -> Result<(), String> {
    // An empty name names nothing: as GNU rm, not there (and fine with -f).
    if op.is_empty() {
        return if a.force {
            Ok(())
        } else {
            Err("rm: cannot remove '': No such file or directory".to_owned())
        };
    }
    let trimmed = op.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if last == "." || last == ".." {
        return Err(format!("rm: refusing to remove '.' or '..' directory: skipping '{op}'"));
    }
    if trimmed.is_empty() {
        return Err("rm: it is dangerous to operate recursively on '/'".to_owned());
    }
    let path: PathBuf = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        cwd.join(trimmed)
    };
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return if a.force {
                Ok(())
            } else {
                Err(format!("rm: cannot remove '{op}': No such file or directory"))
            };
        }
        Err(e) => return Err(format!("rm: cannot remove '{op}': {e}")),
    };
    if op.ends_with('/') && meta.file_type().is_symlink() {
        // GNU rm would act on what the link points to: the net does not guess.
        return Err(format!(
            "reins: '{op}' goes through a symlink; nothing was deleted. Remove the link without the trailing /."
        ));
    }
    let dir = meta.is_dir();
    // `file/`: a trailing slash names a folder, and this is not one.
    if op.ends_with('/') && !dir {
        return Err(format!("rm: cannot remove '{op}': Not a directory"));
    }
    if dir && !a.recursive {
        if !a.dir {
            return Err(format!("rm: cannot remove '{op}': Is a directory"));
        }
        if std::fs::read_dir(&path).map_or(true, |mut d| d.next().is_some()) {
            return Err(format!("rm: cannot remove '{op}': Directory not empty"));
        }
    }
    // The real place decides whether it is scratch (a symlinked folder counts where it leads).
    let real = path
        .parent()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map_or_else(|| path.clone(), |p| p.join(path.file_name().unwrap_or_default()));
    if (scratch.0)(&real) {
        let gone = if dir {
            std::fs::remove_dir_all(&real)
        } else {
            std::fs::remove_file(&real)
        };
        gone.map_err(|e| format!("rm: cannot remove '{op}': {e}"))?;
    } else {
        match store.trash(plan, &path, None) {
            Ok(_) => {}
            Err(NetError::Missing) if a.force => return Ok(()),
            Err(NetError::Missing) => return Err(format!("rm: cannot remove '{op}': No such file or directory")),
            Err(e) => {
                store.fail_plan(plan, &e.to_string());
                return Err(not_kept(op, &e));
            }
        }
    }
    if a.verbose {
        let what = if dir {
            "removed directory"
        } else {
            "removed"
        };
        let _said = writeln!(out, "{what} '{op}'");
    }
    Ok(())
}
