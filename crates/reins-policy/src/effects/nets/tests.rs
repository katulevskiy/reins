use super::*;
use crate::effects::Workspace;

const REINS: &str = "/usr/local/bin/reins";
const PLAN: &str = "plan01";
const TRASH: &str = "/home/me/.local/share/reins/nets";

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
        trash: TRASH.to_owned(),
    }
}

fn plan(command: &str) -> Option<NetPlan> {
    plan_command(command, &ctx("/home/me/Documents/dev/notes"), &disk(), REINS, PLAN)
}

fn pre() -> String {
    prefix(REINS, PLAN)
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
        "rm -rf \"$(pwd)/old\"",
        "X=1 rm -rf old >/dev/null 2>&1",
        "{ rm -rf old; }",
        "(rm -rf old)",
        "if true; then rm -rf old; fi",
        "x=$(rm -rfv old)",
    ] {
        let p = plan(c).unwrap_or_else(|| panic!("a net for {c}"));
        let at = c.find("rm").unwrap();
        assert_eq!(p.rewritten, format!("{}{}{}", &c[..at], pre(), &c[at..]), "{c}");
        // The rewrite is the original with the prefix in front of the program, nothing else.
        assert_eq!(p.rewritten.replace(&pre(), ""), c);
    }
}

#[test]
fn loops_and_functions_are_rewritten_once() {
    let c = "for d in a b c; do rm -rf \"$d\"; done";
    let p = plan(c).expect("a net");
    assert_eq!(p.rewritten.matches("net run").count(), 1);
    assert_eq!(p.netted_paths.len(), 3, "{p:?}");
    let c = "clean() { rm -rf \"$1\"; }; clean a; clean b";
    let p = plan(c).expect("a net");
    assert_eq!(p.rewritten.matches("net run").count(), 1);
    assert_eq!(p.after.level, Level::L1, "{p:?}");
}

#[test]
fn masking_attributes_netted_deletes() {
    let p = plan("rm -rf old; sh -c 'rm -rf older'").expect("a net for the first");
    assert_eq!(p.netted_paths, vec!["/home/me/Documents/dev/notes/old".to_owned()]);
    assert_eq!(p.after.level, Level::L3, "the sh -c delete keeps its level: {p:?}");
    // The same path deleted another way too: not netted.
    let p = plan("rm -rf old; find old -delete");
    assert!(p.is_none_or(|p| p.after.level == Level::L3));
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
    let p = plan_command("rm -rf src/x", &ctx("/home/me/Documents/dev/app"), &disk(), REINS, PLAN).expect("a net");
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
    assert!(plan_command("rm -rf ~/mnt/usb/photos", &c, &Elsewhere, REINS, PLAN).is_none());
    assert!(plan_command("rm -rf old ~/mnt/usb/photos", &c, &Elsewhere, REINS, PLAN).is_none(), "all or nothing");
    assert!(plan_command("rm -rf old", &c, &NoFacts, REINS, PLAN).is_none());
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
    let p = plan_command("git worktree remove --force ../app-wt", &c, &disk(), REINS, PLAN).expect("a net");
    assert_eq!(p.rewritten, format!("{}git worktree remove --force ../app-wt", pre()));
    assert_eq!((p.before.level, p.after.level), (Level::L3, Level::L1), "{p:?}");
    let p = plan_command("git branch -D feat/x", &c, &disk(), REINS, PLAN).expect("a net");
    assert_eq!(p.kinds(), vec![NetKind::GitBackup]);
    assert_eq!(p.netted_refs.len(), 1);
    let p = plan_command("git status && git reset --hard origin/x", &c, &disk(), REINS, PLAN).expect("a net");
    assert_eq!(p.rewritten, format!("git status && {}git reset --hard origin/x", pre()));
    // The replay's own shape: a loop of worktree removals, branch deletes, and scratch deletes.
    let cmd = "cd ~/Documents/dev/app && for w in a b; do git worktree remove --force ~/Documents/dev/$w && echo ok; done; git branch -D x y 2>&1 | grep -c Deleted; rm -rf ~/Documents/dev/tmp/a";
    let p = plan_command(cmd, &c, &disk(), REINS, PLAN).expect("a net");
    assert_eq!(p.rewritten.matches("net run").count(), 2, "{}", p.rewritten);
    assert!(p.before.level == Level::L3 && p.after.level <= Level::L1, "{p:?}");
    // Not a shape: untouched.
    assert!(plan_command("git worktree remove ../w", &c, &disk(), REINS, PLAN).is_none());
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
    let (text, _) = rewrite("rm -rf old", "rm", odd, PLAN, &rm_wanted).expect("a rewrite");
    let parsed = ast::parse(&text);
    let Command::Simple(s) = &parsed.script.0[0].pipelines[0].commands[0] else {
        panic!()
    };
    assert_eq!(s.words[0].literal().as_deref(), Some(odd));
    assert!(rewrite("rm -rf old", "rm", REINS, "bad id!", &rm_wanted).is_none());
    assert!(rewrite("rm -rf 'old", "rm", REINS, PLAN, &rm_wanted).is_none(), "incomplete input");
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
        // The agent calling reins itself.
        "reins net list; rm -rf old",
    ] {
        // No plan: the request is decided on its own analysis, at the level it has without a net.
        let p = plan(c);
        assert!(p.is_none(), "{c}: {p:?}");
    }
}
