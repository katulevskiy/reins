//! The local policy: rules from `config.toml` applied to a read or a push.
//!
//! For each kind of request (read, push, risky push) the first rule that matches and says something about that kind
//! decides; when none does, the top-level default does. A rule's `repo` pattern is matched against the repository path
//! (`owner/name`, on GitLab `group/sub/name`) and its `host` pattern, when set, against the git host, both ignoring
//! case (the hosts do, so `Me/Secret` must not slip past a rule for `me/secret`); its `branch` pattern, when
//! set, must match every ref the push changes (a tag or another ref never matches a branch pattern, and a read never
//! matches a rule with a branch).

use reins_proto::desktop::{PushSummary, RefUpdate};

use crate::config::{PolicyConfig, PolicyRule, Rule};

/// Whether `value` matches `pattern`: `*` matches any run of characters within a `/`-separated part, `**` any run
/// across parts, everything else itself. Linear in `pattern.len() * value.len()` (no backtracking blow-up).
#[must_use]
pub fn matches(pattern: &str, value: &str) -> bool {
    enum Tok {
        Lit(char),
        Star,
        DoubleStar,
    }
    let mut toks = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '*' {
            if chars.peek() == Some(&'*') {
                while chars.peek() == Some(&'*') {
                    chars.next();
                }
                toks.push(Tok::DoubleStar);
            } else {
                toks.push(Tok::Star);
            }
        } else {
            toks.push(Tok::Lit(c));
        }
    }
    let value: Vec<char> = value.chars().collect();
    // reach[j]: the tokens so far can match exactly value[..j].
    let mut reach = vec![false; value.len() + 1];
    reach[0] = true;
    for tok in &toks {
        let mut next = vec![false; value.len() + 1];
        match tok {
            Tok::Lit(c) => {
                for j in 0..value.len() {
                    if reach[j] && value[j] == *c {
                        next[j + 1] = true;
                    }
                }
            }
            Tok::Star | Tok::DoubleStar => {
                let across = matches!(tok, Tok::DoubleStar);
                let mut open = false;
                for j in 0..=value.len() {
                    open |= reach[j];
                    next[j] = open;
                    if j < value.len() && value[j] == '/' && !across {
                        open = false;
                    }
                }
            }
        }
        reach = next;
    }
    reach[value.len()]
}

fn repo_matches(rule: &PolicyRule, host: &str, repo: &str) -> bool {
    rule.host.as_ref().is_none_or(|h| matches(&h.to_ascii_lowercase(), &host.to_ascii_lowercase()))
        && matches(&rule.repo.to_ascii_lowercase(), &repo.to_ascii_lowercase())
}

fn branches_match(rule: &PolicyRule, updates: &[RefUpdate]) -> bool {
    match &rule.branch {
        None => true,
        Some(pattern) => !updates.is_empty() && updates.iter().all(|u| u.branch().is_some_and(|b| matches(pattern, b))),
    }
}

/// A push is risky when it is asked for every time on the phone: a force push, a deletion, a moved tag, several
/// branches, an unusual ref, or ancestry that could not be checked.
#[must_use]
pub fn is_risky(summary: &PushSummary) -> bool {
    summary.once_only() || summary.updates.iter().any(RefUpdate::is_risky)
}

/// What the policy says about reading `repo` (`owner/name`, `group/sub/name`) on `host`.
#[must_use]
pub fn decide_read(policy: &PolicyConfig, host: &str, repo: &str) -> Rule {
    policy
        .rules
        .iter()
        .filter(|r| r.branch.is_none() && repo_matches(r, host, repo))
        .find_map(|r| r.read)
        .unwrap_or(policy.read)
}

/// What the policy says about this push to `repo` on `host`; risky pushes use the `risky` rules, never `push`.
#[must_use]
pub fn decide_push(policy: &PolicyConfig, host: &str, repo: &str, summary: &PushSummary) -> Rule {
    let risky = is_risky(summary);
    policy
        .rules
        .iter()
        .filter(|r| repo_matches(r, host, repo) && branches_match(r, &summary.updates))
        .find_map(|r| {
            if risky {
                r.risky
            } else {
                r.push
            }
        })
        .unwrap_or(if risky {
            policy.risky
        } else {
            policy.push
        })
}

#[cfg(test)]
mod tests {
    use reins_proto::desktop::{RefChange, ZERO_OID};

    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn update(name: &str, old: &str, new: &str, ff: Option<bool>) -> RefUpdate {
        RefUpdate {
            name: name.to_owned(),
            change: match (old == ZERO_OID, new == ZERO_OID) {
                (true, _) => RefChange::Create,
                (_, true) => RefChange::Delete,
                _ => RefChange::Update,
            },
            old: old.to_owned(),
            new: new.to_owned(),
            fast_forward: ff,
            commit_count: 1,
            commits: vec![],
            files_changed: 0,
            files: vec![],
            additions: None,
            deletions: None,
        }
    }

    fn push(updates: Vec<RefUpdate>) -> PushSummary {
        PushSummary {
            updates,
            ..PushSummary::default()
        }
    }

    fn rule(repo: &str, branch: Option<&str>) -> PolicyRule {
        PolicyRule {
            host: None,
            repo: repo.to_owned(),
            branch: branch.map(str::to_owned),
            read: None,
            push: None,
            risky: None,
        }
    }

    #[test]
    fn single_stars_stay_within_a_part_and_double_stars_cross_parts() {
        assert!(matches("me/*", "me/app"));
        assert!(!matches("me/*", "you/app"));
        assert!(matches("*/*", "me/app"));
        assert!(!matches("*", "me/app"));
        assert!(matches("**", "me/app"));
        assert!(matches("feature/**", "feature/a/b"));
        assert!(!matches("feature/*", "feature/a/b"));
        assert!(matches("feature/*", "feature/"));
        assert!(matches("rel-*-x", "rel-1.2-x"));
        assert!(!matches("main", "main2"));
        assert!(matches("", ""));
        assert!(!matches("a", ""));
        let long = "a".repeat(5_000);
        assert!(!matches(&"*a".repeat(200), &format!("{long}b")));
    }

    #[test]
    fn reads_use_the_first_rule_with_an_opinion_ignoring_case() {
        let mut p = PolicyConfig::default();
        assert_eq!(decide_read(&p, "github.com", "me/app"), Rule::Allow);
        p.rules.push(PolicyRule {
            push: Some(Rule::Allow),
            ..rule("me/*", None)
        });
        p.rules.push(PolicyRule {
            read: Some(Rule::Deny),
            ..rule("me/secret", None)
        });
        p.rules.push(PolicyRule {
            read: Some(Rule::Ask),
            ..rule("**", None)
        });
        p.rules.push(PolicyRule {
            read: Some(Rule::Allow),
            ..rule("**", Some("main"))
        });
        assert_eq!(decide_read(&p, "github.com", "Me/Secret"), Rule::Deny);
        assert_eq!(decide_read(&p, "github.com", "me/app"), Rule::Ask);
        assert_eq!(decide_read(&p, "github.com", "other/x"), Rule::Ask);
    }

    #[test]
    fn rules_with_a_host_match_only_that_host_and_nested_paths_need_double_stars() {
        let mut p = PolicyConfig::default();
        p.rules.push(PolicyRule {
            host: Some("GitLab.com".to_owned()),
            read: Some(Rule::Deny),
            push: Some(Rule::Allow),
            ..rule("group/**", None)
        });
        p.rules.push(PolicyRule {
            host: Some("*.example.com".to_owned()),
            read: Some(Rule::Ask),
            ..rule("**", None)
        });
        p.rules.push(PolicyRule {
            read: Some(Rule::Ask),
            ..rule("group/*", None)
        });
        assert_eq!(decide_read(&p, "gitlab.com", "Group/Sub/App"), Rule::Deny);
        assert_eq!(decide_read(&p, "github.com", "group/sub/app"), Rule::Allow, "no rule for it on GitHub");
        assert_eq!(decide_read(&p, "github.com", "group/app"), Rule::Ask, "a rule without a host matches every host");
        assert_eq!(decide_read(&p, "git.example.com", "a/b"), Rule::Ask);
        let ff = push(vec![update("refs/heads/main", A, B, Some(true))]);
        assert_eq!(decide_push(&p, "gitlab.com", "group/sub/app", &ff), Rule::Allow);
        assert_eq!(decide_push(&p, "codeberg.org", "group/sub/app", &ff), Rule::Ask);
    }

    #[test]
    fn pushes_match_branches_for_every_ref_and_risky_pushes_use_the_risky_rule() {
        let mut p = PolicyConfig::default();
        p.rules.push(PolicyRule {
            push: Some(Rule::Allow),
            risky: Some(Rule::Deny),
            ..rule("me/app", Some("feature/**"))
        });
        p.rules.push(PolicyRule {
            push: Some(Rule::Deny),
            ..rule("me/*", Some("main"))
        });
        let ff = push(vec![update("refs/heads/feature/x", A, B, Some(true))]);
        assert_eq!(decide_push(&p, "github.com", "me/app", &ff), Rule::Allow);
        assert_eq!(decide_push(&p, "github.com", "me/other", &ff), Rule::Ask);
        let main = push(vec![update("refs/heads/main", A, B, Some(true))]);
        assert_eq!(decide_push(&p, "github.com", "me/app", &main), Rule::Deny);
        let force = push(vec![update("refs/heads/feature/x", A, B, Some(false))]);
        assert!(is_risky(&force));
        assert_eq!(decide_push(&p, "github.com", "me/app", &force), Rule::Deny);
        let unknown = push(vec![update("refs/heads/feature/x", A, B, None)]);
        assert_eq!(decide_push(&p, "github.com", "me/app", &unknown), Rule::Deny);
        // Main is not a feature branch, so the mixed push matches no branch rule and is risky (two branches).
        let mixed =
            push(vec![update("refs/heads/feature/x", A, B, Some(true)), update("refs/heads/main", A, B, Some(true))]);
        assert!(is_risky(&mixed));
        assert_eq!(decide_push(&p, "github.com", "me/app", &mixed), Rule::Ask);
        let tag = push(vec![update("refs/tags/v1", ZERO_OID, B, None)]);
        assert!(!is_risky(&tag));
        assert_eq!(decide_push(&p, "github.com", "me/app", &tag), Rule::Ask);
        let delete = push(vec![update("refs/heads/feature/x", A, ZERO_OID, None)]);
        assert_eq!(decide_push(&p, "github.com", "me/app", &delete), Rule::Deny);
    }
}
