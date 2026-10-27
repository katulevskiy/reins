use super::*;
use crate::effects::Workspace;

const REINS: &str = "/usr/local/bin/reins";
const PLAN: &str = "plan01";
const TRASH: &str = "/home/me/.local/share/reins/nets";
const RUNNER: Runner<'static> = Runner {
    reins: REINS,
    plan: PLAN,
    store: TRASH,
};

fn ctx(cwd: &str) -> Context {
    let ws = Workspace {
        worktrees: vec!["/home/me/Documents/dev/app".to_owned()],
        scratch: vec!["~/Documents/dev/tmp".to_owned()],
        ..Workspace::standard("/home/me")
    };
    let mut c = Context::new(cwd, ws);
    c.default_branch = Some("main".to_owned());
    c
}

fn disk() -> SameDisk {
    SameDisk {
        worktrees: vec![
            "/home/me/Documents/dev/app-wt".to_owned(),
            "/home/me/Documents/dev/a".to_owned(),
            "/home/me/Documents/dev/b".to_owned(),
        ],
        ..SameDisk::new(TRASH)
    }
}

fn plan(command: &str) -> Option<NetPlan> {
    plan_command(command, &ctx("/home/me/Documents/dev/notes"), &disk(), RUNNER)
}

fn pre() -> String {
    prefix(RUNNER)
}

#[test]
fn rewrite_keeps_the_tree() {
    let p = plan("rm -rf old").expect("a net");
    assert_eq!(p.rewritten, format!("{}rm -rf old", pre()));
    assert_eq!(p.before.level, Level::L3, "untracked source, no net: {:?}", p.before);
    assert_eq!(p.after.level, Level::L1);
    assert_eq!(p.kinds(), vec![NetKind::Trash]);
    assert_eq!(p.netted_paths, vec!["/home/me/Documents/dev/notes/old".to_owned()]);
    let p = plan("cd sub && rm -r a b; ls").expect("a net");
    assert_eq!(p.rewritten, format!("cd sub && {}rm -r a b; ls", pre()));
}

#[test]
fn only_command_words_are_rewritten() {
    for c in [
        "echo rm -rf old",
        "echo \"a; rm -rf old\"",
        "printf 'rm -rf old'",
        "cat <<EOF\nrm -rf old\nEOF",
        "\\rm -rf old",
        "command rm -rf old",
        "sudo rm -rf old",
        "ls | xargs rm -rf",
        "R=rm; $R -rf old",
        "sh -c 'rm -rf old'",
        "find . -name x -exec rm -rf {} +",
        "grep -r rm .",
        "rm() { echo kept; }; rm -rf old",
        "function rm { echo kept; }; rm -rf old",
        "alias rm='echo'; rm -rf old",
        "enable -n echo; rm -rf old",
    ] {
        let p = plan(c);
        assert!(p.as_ref().is_none_or(|p| !p.rewritten.contains("-- rm -rf old") || c.starts_with("rm")), "{c}: {p:?}");
        assert!(p.is_none(), "nothing to net in {c}: {p:?}");
    }
}

#[test]
fn quoting_survives_the_rewrite() {
    for c in [
        "rm -rf 'my dir' \"other dir\" plain",
        "rm -f $'a\\tb'",
        "rm -rf ./*.log",
        "rm -rf old >/dev/null 2>&1",
        "ls && rm -rf old || echo failed",
        "rm -rfv old | tail -1",
        "rm -rf ~/Documents/dev/notes/old",
    ] {
        let p = plan(c).unwrap_or_else(|| panic!("a net for {c}"));
        let at = c.find("rm").unwrap();
        assert_eq!(p.rewritten, format!("{}{}{}", &c[..at], pre(), &c[at..]), "{c}");
        // The rewrite is the original with the prefix in front of the program, nothing else.
        assert_eq!(p.rewritten.replace(&pre(), ""), c);
    }
}

/// Fail closed by design: only scripts of plain simple commands from the allowlist are netted. Everything else keeps its
/// level (each form here was a way around a denylist).
#[test]
fn only_plain_allowlisted_scripts_are_netted() {
    for c in [
        // computed words, assignments, arithmetic
        "rm -rf \"$(pwd)/old\"",
        "rm -rf \"$D\"",
        "X=1 rm -rf old",
        "let HOME=1; rm -rf old",
        // assignments to names longer than a letter, of values that are computed, or that may not run
        "xy=1; rm -rf old",
        "PATH=/x; rm -rf old",
        "IFS=/; rm -rf old",
        "x=$(pwd); rm -rf \"$x\"/old",
        "x=$HOME; rm -rf \"$x\"/old",
        "x=*; rm -rf $x",
        "x='a b'; rm -rf $x",
        "cd sub && x=old; rm -rf \"$x\"",
        "true | x=old; rm -rf \"$x\"",
        "x=old & rm -rf \"$x\"",
        "rm -rf \"$x\"; x=old",
        "(( HOME = 1 )); rm -rf old",
        "rm -rf old$((1))",
        // compound commands, functions, subshells, substitutions
        "{ rm -rf old; }",
        "(rm -rf old)",
        "if true; then rm -rf old; fi",
        "for d in *; do rm -rf \"$d\"; done",
        "for d in \"$@\"; do rm -rf \"$d\"; done",
        "for d; do rm -rf \"$d\"; done",
        "for dd in a b; do rm -rf \"$dd\"; done",
        "for HOME in a b; do rm -rf old; done",
        "for d in a b; do rm -rf \"$d\"; rm() { :; }; done",
        "for d in a b; do x=$d; rm -rf \"$x\"; done",
        "x=a; for d in b c; do rm -rf \"$x\"; x=$d; done",
        "while true; do rm -rf old; done",
        "clean() { rm -rf \"$1\"; }; clean a",
        "x=$(rm -rfv old)",
        "rm -rf old <(true)",
        // programs off the list, and the ones that run code or set variables
        "mapfile -C 'rm() { :; }' -c 1 a < f; rm -rf old",
        "printf -v x %s; rm -rf old",
        "printf -vHOME %s; rm -rf old",
        "printf \"$f\" x; rm -rf old",
        "set -- \"$x\"; rm -rf old",
        "read x; rm -rf old",
        "eval x; rm -rf old",
        "export X; rm -rf old",
        "declare -x X; rm -rf old",
        "set -a; rm -rf old",
        "shopt -s expand_aliases; rm -rf old",
        "trap x EXIT; rm -rf old",
        "alias rm=x; rm -rf old",
        "hash -r; rm -rf old",
        "enable -n echo; rm -rf old",
        "builtin cd x; rm -rf old",
        "command rm -rf old",
        "exec 3>f; rm -rf old",
        "let x=1; rm -rf old",
        "source ./x; rm -rf old",
        ". ./x; rm -rf old",
        "pushd x; rm -rf old",
        "umask 077; rm -rf old",
        "unset x; rm -rf old",
        "type rm; rm -rf old",
        "time rm -rf old",
        "coproc x; rm -rf old",
        "kill -0 1; rm -rf old",
        "~/bin/tool; rm -rf old",
        "./*.sh; rm -rf old",
        // a heredoc
        "cat <<EOF > f\nx\nEOF\nrm -rf old",
        // globs outside rm and git (`printf -*` could expand to `-v NAME`), and printf at all
        "printf -* x; rm -rf old",
        "printf '%s' done; rm -rf old",
        "ls -*; rm -rf old",
        "echo * > out; rm -rf old",
        "rm -rf old > *.log",
    ] {
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
    assert!(plan("echo done; rm -rf old ./*.tmp; ls ~/x").is_some(), "plain words, and globs in rm, are fine");
    // A one-letter variable the script surely set, and a loop over plain values.
    for c in [
        "x=old; rm -rf \"$x\"",
        "B=~/Documents/dev/notes; rm -rf $B/old; mkdir -p $B/old",
        "x=old && rm -rf ${x}",
        "for d in a b c; do rm -rf \"$d\"; done",
        "for d in a b; do [ -d \"$d\" ] && rm -rf \"$d\" && echo \"removed $d\"; done; ls",
    ] {
        let p = plan(c);
        assert!(p.as_ref().is_some_and(|p| p.after.level == Level::L1), "{c}: {p:?}");
    }
    // Any literal external program, with plain words.
    assert!(plan("cargo build -p x && rm -rf old && python3 tools/x.py --flag ~/y && /usr/bin/du -sh out").is_some());
}

#[test]
fn masking_attributes_netted_deletes() {
    // Two deletes, each by an `rm` of its own: both netted, and nothing else is.
    let p = plan("rm -rf old && rm -f notes.md").expect("a net");
    assert_eq!(p.netted_paths.len(), 2, "{p:?}");
    assert_eq!(p.after.level, Level::L1);
    // A delete another program does (`sh -c`) is not netted and keeps its level; the plain `rm` beside it is netted.
    let p = plan("sh -c 'rm -rf older'; rm -rf old").expect("a net for the plain rm");
    assert_eq!(p.netted_paths, vec!["/home/me/Documents/dev/notes/old".to_owned()]);
    assert_eq!(p.after.level, Level::L3, "the sh -c delete keeps its level: {p:?}");
    let p = plan("find older -delete; rm -rf old").expect("a net for the plain rm");
    assert_eq!(p.after.level, Level::L3, "find's delete keeps its level: {p:?}");
    // A delete the net doesn't run (git without a nettable shape) keeps its level.
    let p = plan("rm -rf old; git rm -rq --cached x");
    assert!(p.is_none_or(|p| p.netted_paths == vec!["/home/me/Documents/dev/notes/old".to_owned()]));
}

#[test]
fn levels_with_nets() {
    // Home outside the workspace: recoverable with the net.
    let p = plan("rm -rf ~/notes/journal.md").expect("a net");
    assert_eq!((p.before.level, p.after.level), (Level::L3, Level::L1));
    // Scratch only: nothing to net.
    assert!(plan("rm -rf /tmp/x ~/.cache/y").is_none());
    // Mixed scratch and source: netted, scratch deleted for real at run time.
    let p = plan("rm -rf /tmp/x old").expect("a net");
    assert_eq!(p.after.level, Level::L1);
    // In a known worktree it was recoverable already, the net keeps a copy anyway.
    let p = plan_command("rm -rf src/x", &ctx("/home/me/Documents/dev/app"), &disk(), RUNNER).expect("a net");
    assert_eq!((p.before.level, p.after.level), (Level::L1, Level::L1));
}

#[test]
fn system_remote_unknown_are_not_lowered() {
    let p = plan("rm -rf old /etc/thing").expect("a net for old");
    assert_eq!(p.after.level, Level::L3, "a system path stays L3: {p:?}");
    assert!(plan("ssh host rm -rf /var/www").is_none());
    let p = plan("rm -rf \"$UNSET\"");
    assert!(p.is_none_or(|p| p.after.level >= Level::L1));
}

#[test]
fn the_trash_and_its_ancestors_are_never_netted() {
    assert!(!trash_safe("/home/me", TRASH));
    assert!(!trash_safe("/home/me/.local/share/reins/nets", TRASH));
    assert!(!trash_safe("/home/me/.local/share/reins/nets/x/items", TRASH));
    assert!(!trash_safe("/", TRASH));
    assert!(!trash_safe("relative", TRASH));
    assert!(!trash_safe("/home/me/a/../b", TRASH));
    assert!(trash_safe("/home/me/.local/share/reins/netsy", TRASH));
    for c in ["rm -rf ~", "rm -rf ~/.local/share/reins", "rm -rf ~/.local/share/reins/nets/plan01", "rm -rf /"] {
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
}

#[test]
fn another_disk_is_not_netted() {
    struct Elsewhere;
    impl NetFacts for Elsewhere {
        fn trashable(&self, abs: &str) -> bool {
            !abs.starts_with("/home/me/mnt/")
        }
    }
    let c = ctx("/home/me/Documents/dev/notes");
    assert!(plan_command("rm -rf ~/mnt/usb/photos", &c, &Elsewhere, RUNNER).is_none());
    assert!(plan_command("rm -rf old ~/mnt/usb/photos", &c, &Elsewhere, RUNNER).is_none(), "all or nothing");
    assert!(plan_command("rm -rf old", &c, &NoFacts, RUNNER).is_none());
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn git_shapes() {
    let shape = |s: &str| git_shape(&words(s)).map(|g| g.net);
    assert_eq!(shape("branch -D a b"), Some(GitNet::BranchDelete));
    assert_eq!(shape("-C /x branch --delete --force a"), Some(GitNet::BranchDelete));
    assert_eq!(shape("reset --hard"), Some(GitNet::ResetHard));
    assert_eq!(shape("reset --hard HEAD~2"), Some(GitNet::ResetHard));
    assert_eq!(shape("worktree remove --force ../w"), Some(GitNet::WorktreeRemove));
    assert_eq!(shape("worktree remove -f -f ../w"), Some(GitNet::WorktreeRemove));
    assert_eq!(shape("clean -fdx"), Some(GitNet::Clean));
    assert_eq!(shape("clean -f -d -e keep -- src"), Some(GitNet::Clean));
    assert_eq!(shape("push --force origin feat/x"), Some(GitNet::ForcePush));
    assert_eq!(shape("push origin +feat/x"), Some(GitNet::ForcePush));
    assert_eq!(shape("push -f origin HEAD:feat/x"), Some(GitNet::ForcePush));
    for no in [
        "branch -m a b",
        "branch a",
        "reset --soft HEAD~1",
        "reset --hard HEAD -- path",
        "worktree remove ../w",
        "worktree remove --force a b",
        "worktree prune",
        "clean -n",
        "clean -fi",
        "clean -f :/magic",
        "push origin feat/x",
        "push --mirror origin",
        "push --force origin :feat/x",
        "push --force --delete origin x",
        "push --force origin 'refs/*:refs/*'",
        "push --force origin a:refs/tags/x",
        "--git-dir=/x branch -D a",
        "-c",
    ] {
        assert_eq!(shape(no), None, "{no}");
    }
}

#[test]
fn git_nets_are_planned() {
    let c = ctx("/home/me/Documents/dev/app");
    let p = plan_command("git worktree remove --force ../app-wt", &c, &disk(), RUNNER).expect("a net");
    assert_eq!(p.rewritten, format!("{}git worktree remove --force ../app-wt", pre()));
    assert_eq!((p.before.level, p.after.level), (Level::L3, Level::L1), "{p:?}");
    let p = plan_command("git branch -D feat/x", &c, &disk(), RUNNER).expect("a net");
    assert_eq!(p.kinds(), vec![NetKind::GitBackup]);
    assert_eq!(p.netted_refs.len(), 1);
    let p = plan_command("git status && git reset --hard origin/x", &c, &disk(), RUNNER).expect("a net");
    assert_eq!(p.rewritten, format!("git status && {}git reset --hard origin/x", pre()));
    // The history's shape, written out: worktree removals, a branch delete and a scratch delete.
    let cmd = "cd ~/Documents/dev/app && git worktree remove --force ~/Documents/dev/a && git worktree remove --force ~/Documents/dev/b; git branch -D x y 2>&1 | grep -c Deleted; rm -rf ~/Documents/dev/tmp/a";
    let p = plan_command(cmd, &c, &disk(), RUNNER).expect("a net");
    assert_eq!(p.rewritten.matches("net run").count(), 3, "{}", p.rewritten);
    assert!(p.before.level == Level::L3 && p.after.level <= Level::L1, "{p:?}");
    // The same in a loop (`$w`) over plain names: netted when each is a linked worktree.
    let looped = "for w in a b; do git worktree remove --force ~/Documents/dev/$w; done";
    let p = plan_command(looped, &c, &disk(), RUNNER).expect("a net");
    assert_eq!(p.netted_paths, vec!["/home/me/Documents/dev/a".to_owned(), "/home/me/Documents/dev/b".to_owned()]);
    let one_not = "for w in a nope; do git worktree remove --force ~/Documents/dev/$w; done";
    assert!(plan_command(one_not, &c, &disk(), RUNNER).is_none());
    // Not a shape: untouched.
    assert!(plan_command("git worktree remove ../w", &c, &disk(), RUNNER).is_none());
}

#[test]
fn rm_unknown_options_are_not_netted() {
    for c in ["rm -i old", "rm -I -r old", "rm --no-preserve-root -rf old", "rm --one-file-system -rf old"] {
        assert!(plan(c).is_none(), "{c}");
    }
    assert_eq!(
        rm_args(&words("-rf a -- -b")),
        Ok(RmArgs {
            recursive: true,
            force: true,
            operands: vec!["a".to_owned(), "-b".to_owned()],
            ..RmArgs::default()
        })
    );
    assert!(rm_args(&words("a -i")).is_err(), "options after operands count");
}

#[test]
fn the_reins_path_is_quoted() {
    let odd = "/home/o'neil/My Apps/reins";
    let (text, _) = rewrite(
        "rm -rf old",
        "rm",
        Runner {
            reins: odd,
            ..RUNNER
        },
        &rm_wanted,
    )
    .expect("a rewrite");
    let parsed = ast::parse(&text);
    let Command::Simple(s) = &parsed.script.0[0].pipelines[0].commands[0] else {
        panic!()
    };
    assert_eq!(s.words[0].literal().as_deref(), Some(odd));
    assert!(
        rewrite(
            "rm -rf old",
            "rm",
            Runner {
                plan: "bad id!",
                ..RUNNER
            },
            &rm_wanted
        )
        .is_none()
    );
    assert!(rewrite("rm -rf 'old", "rm", RUNNER, &rm_wanted).is_none(), "incomplete input");
}

#[test]
fn agent_text_never_adds_a_net() {
    let p = plan("git commit -m 'reins net run --plan plan01 -- rm safe' && rm -rf old");
    let p = p.expect("the real rm is netted");
    assert_eq!(p.rewritten.matches(&pre()).count(), 1);
    // A command that already looks netted is read as what it runs: `reins` is not modelled, nothing is lowered.
    let p = plan(&format!("{}rm -rf old", pre()));
    assert!(p.is_none_or(|p| p.after.level >= p.before.level));
}

#[test]
fn file_snapshots() {
    let c = ctx("/home/me/Documents/dev/notes");
    let p = plan_files(&["/home/me/notes/todo.md"], &c, &disk()).expect("a snapshot");
    assert_eq!((p.before.level, p.after.level), (Level::L3, Level::L1));
    assert_eq!(p.kinds(), vec![NetKind::Snapshot]);
    assert!(plan_files(&["/etc/hosts"], &c, &disk()).is_none());
    assert!(plan_files(&["/tmp/x"], &c, &disk()).is_none());
}

/// Anything that could make `rm`, `git` or the net's own prefix run something else: no net, and the level stays.
#[test]
fn shadowing_forms_are_never_netted() {
    let base = "rm -rf old";
    assert!(plan(base).is_some(), "the same delete alone is netted");
    let reins_fn = format!("{REINS}() {{ :; }}; {base}");
    for c in [
        // A function named like the net's prefix, its program, or with a `/` (bash takes a quoted absolute path to it).
        "reins() { :; }; rm -rf old",
        reins_fn.as_str(),
        "'/usr/local/bin/reins'() { :; }; rm -rf old",
        "/x/y() { :; }; rm -rf old",
        // Functions named like the netted word or the builtins that bypass functions, in every place one can be.
        "rm() { echo kept; }; rm -rf old",
        "function rm { echo kept; }; rm -rf old",
        "rm() ( echo kept ); rm -rf old",
        "x=$(rm() { :; }; echo); rm -rf old",
        "echo > \"$(reins() { :; })\"; rm -rf old",
        "if true; then command() { :; }; fi; rm -rf old",
        "for i in a; do builtin() { :; }; done; rm -rf old",
        "case x in x) rm() { :; } ;; esac; rm -rf old",
        "f() { g() { :; }; reins() { :; }; }; f; rm -rf old",
        // Code run as text, aliases, builtins switched or bypassed.
        "eval 'rm() { :; }'; rm -rf old",
        "source ./defs.sh; rm -rf old",
        ". ./defs.sh; rm -rf old",
        "alias rm='echo'; rm -rf old",
        "unalias rm; rm -rf old",
        "enable -n echo; rm -rf old",
        "hash -p /bin/echo rm; rm -rf old",
        "builtin cd . && rm -rf old",
        "command -v rm && rm -rf old",
        "trap 'rm() { :; }' DEBUG; rm -rf old",
        // A command word that could be any of them.
        "E=eval; $E 'rm() { :; }'; rm -rf old",
        "$(printf eval) x; rm -rf old",
    ] {
        // No plan: the request is decided on its own analysis, at the level it has without a net.
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
}

/// git resolves `worktree remove NAME` by path, else by a unique ending of a worktree's path: only a path that is a
/// linked worktree now is netted.
#[test]
fn only_a_worktree_path_is_netted() {
    let c = ctx("/home/me/Documents/dev/app");
    assert!(plan_command("git worktree remove --force ../app-wt", &c, &disk(), RUNNER).is_some());
    for not in
        ["git worktree remove --force app-wt", "git worktree remove --force ../nope", "git worktree remove --force wt"]
    {
        let p = plan_command(not, &c, &disk(), RUNNER);
        assert!(p.is_none(), "{not}: {p:?}");
    }
}

/// What finds `reins`, its store, or what git and rm read, set by the command itself: no net.
#[test]
fn environment_the_command_sets_is_never_trusted() {
    for c in [
        "HOME=/x rm -rf old",
        "REINS_NETS_DIR=/x rm -rf old",
        "export REINS_NETS_DIR=/x; rm -rf old",
        "env XDG_STATE_HOME=/x rm -rf old",
        "PATH=/x:$PATH rm -rf old",
        "GIT_DIR=/x git branch -D a",
        "local HOME=/x; rm -rf old",
        "for HOME in /x; do rm -rf old; done",
        "for GIT_DIR in /x; do git branch -D a; done",
        "read HOME < f; rm -rf old",
        "mapfile -t PATH < f; rm -rf old",
        "read -r \"$v\" < f; rm -rf old",
        "printf -v HOME %s /x; rm -rf old",
        "unset HOME; rm -rf old",
        "set -a; . ./env; rm -rf old",
        "set -ea; rm -rf old",
        "set -o allexport; rm -rf old",
        "printf -vHOME %s /x; rm -rf old",
        "printf -v x %s; rm -rf old",
        "read -raHOME < f; rm -rf old",
        "read -r -a GIT_DIR < f; rm -rf old",
        "readarray -tPATH < f; rm -rf old",
        "wait -p HOME; rm -rf old",
        "wait -pXDG_STATE_HOME; rm -rf old",
        "declare -n ref=HOME; ref=/x; rm -rf old",
        "local -n r=GIT_DIR; rm -rf old",
        "typeset -n r; rm -rf old",
        "declare \"$n\"=/x; rm -rf old",
        "export -n PATH; rm -rf old",
        "readonly REINS_NETS_DIR; rm -rf old",
    ] {
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
    assert!(plan("FOO=1 rm -rf old").is_none(), "no assignment at all");
}

#[test]
fn a_plan_names_its_programs_and_folders() {
    let p = plan("cd sub && rm -rf a; cd ../other && rm -rf b; git -C x status").expect("a net");
    assert_eq!(p.programs, vec!["rm".to_owned()]);
    for d in ["/home/me/Documents/dev/notes", "/home/me/Documents/dev/notes/sub", "/home/me/Documents/dev/notes/other"]
    {
        assert!(p.cwds.contains(&d.to_owned()), "{d} in {:?}", p.cwds);
    }
}

#[test]
fn only_files_a_snapshot_can_copy_are_netted() {
    struct NoCopies;
    impl NetFacts for NoCopies {
        fn trashable(&self, _: &str) -> bool {
            true
        }
    }
    let c = ctx("/home/me/Documents/dev/notes");
    assert!(plan_files(&["/home/me/notes/big.iso"], &c, &NoCopies).is_none());
}

/// Only the leading top-level commands that pass the allowlist are netted. What follows them runs once they are done
/// (or, after `&`, in a shell they no longer share), so it can't change what they run: anything may be there, nothing
/// in it is netted, and it keeps its own level.
#[test]
fn only_the_leading_plain_commands_are_netted() {
    let p = plan("rm -rf old; export PATH=$HOME/bin:$PATH && cargo fmt -- --check").expect("a net");
    assert_eq!(p.rewritten, format!("{}rm -rf old; export PATH=$HOME/bin:$PATH && cargo fmt -- --check", pre()));
    assert_eq!(p.after.level, Level::L1, "{p:?}");
    // A function defined after the delete can't reach it.
    assert!(plan("rm -rf old; rm() { :; }; time make").is_some());
    // A delete after the allowlisted part is not netted, and keeps its level.
    let p = plan("rm -rf old; eval \"$X\"; rm -rf other").expect("a net");
    assert_eq!(p.rewritten.matches("net run").count(), 1, "{}", p.rewritten);
    assert_eq!(p.netted_paths, vec!["/home/me/Documents/dev/notes/old".to_owned()]);
    let p = plan("rm -rf old; f() { :; }; rm -rf other").expect("a net");
    assert_eq!(p.after.level, Level::L3, "{p:?}");
    // Anything off the allowlist before or beside the delete: no net at all.
    for c in [
        "export X=1; rm -rf old",
        "rm() { :; }; rm -rf old",
        "rm -rf old && eval x",
        "time rm -rf old; ls",
        "for d in a; do rm() { :; }; done; rm -rf old",
    ] {
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
}

/// A `cd` to a folder under `~` is where the plan's commands may run.
#[test]
fn a_cd_under_home_is_a_plan_folder() {
    let p = plan("cd ~/Documents/dev/notes/sub && rm -rf a").expect("a net");
    assert!(p.cwds.contains(&"/home/me/Documents/dev/notes/sub".to_owned()), "{:?}", p.cwds);
    // A quoted or escaped `~` is a folder named `~`, not the home folder.
    for c in ["cd '~/Documents/dev/notes/sub' && rm -rf a", "cd \\~/Documents/dev/notes/sub && rm -rf a"] {
        let p = plan(c).expect("a net");
        assert!(!p.cwds.contains(&"/home/me/Documents/dev/notes/sub".to_owned()), "{c}: {:?}", p.cwds);
    }
}
