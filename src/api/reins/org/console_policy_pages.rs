#![allow(clippy::format_collect, reason = "pages are built from rows, as in console_pages")]

//! The console's O4 pages (HTML): policy drafts and their simulations, the staged rollout, retained samples, and the
//! policy repo. Same look and rules as [`super::console_pages`]: every value escaped, no script, short words.

use reins_policy::{
    simulate::{ChangedRef, SimulationReport},
    source::SourceError,
};

use super::console_pages::{Frame, hidden_csrf, icon, page, svg};
use crate::api::reins::pages::escape_html as e;

fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

fn org_base(frame: &Frame<'_>) -> String {
    frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default()
}

fn nav(frame: &Frame<'_>, here: &str) -> String {
    let org = org_base(frame);
    let link = |path: &str, label: &str, ic: &str| {
        if path == here {
            format!("<span class=\"pill\">{}{label}</span>", svg(ic))
        } else {
            format!("<a class=\"pill\" href=\"{org}/policy{path}\">{}{label}</a>", svg(ic))
        }
    };
    format!(
        "<p>{} {} {} {}</p>",
        link("", "Policy", icon::SHIELD),
        link("/drafts", "Drafts", icon::LIST),
        link("/rollout", "Rollout", icon::GROUP),
        link("/repo", "Repo", icon::LIST)
    )
}

/// One draft row.
pub struct DraftItem {
    pub revision: i64,
    pub hash: String,
    pub state: String,
    pub source: String,
    /// The newest simulation: its report hash and line, whether it is fresh (≤ a day, no drift).
    pub simulation: Option<(String, String, bool)>,
}

pub struct DraftsView {
    pub drafts: Vec<DraftItem>,
    pub can_author: bool,
    pub can_activate: bool,
}

pub fn drafts(frame: &Frame<'_>, v: &DraftsView) -> String {
    let org = org_base(frame);
    let csrf = hidden_csrf(frame.csrf);
    let rows: String = v
        .drafts
        .iter()
        .map(|d| {
            let sim = d.simulation.as_ref().map_or_else(
                || "<span class=\"meta\">Not simulated</span>".to_owned(),
                |(hash, line, _)| format!("<a href=\"{org}/policy/simulations/{}\">{}</a>", e(hash), e(line)),
            );
            let simulate = if v.can_author && d.state == "draft" {
                format!(
                    "<form method=\"post\" action=\"{org}/policy/drafts/{}/simulate\">{csrf}\
<button class=\"quiet\" type=\"submit\">Simulate</button></form>",
                    d.revision
                )
            } else {
                String::new()
            };
            let fresh = d.simulation.as_ref().filter(|(_, _, fresh)| *fresh);
            let propose = match (v.can_activate && d.state == "draft", fresh) {
                (true, Some((report, _, _))) => format!(
                    "<form method=\"post\" action=\"{org}/policy/activate\">{csrf}\
<input type=\"hidden\" name=\"revision\" value=\"{}\"><input type=\"hidden\" name=\"hash\" value=\"{}\">\
<input type=\"hidden\" name=\"report\" value=\"{}\"><button type=\"submit\">Propose</button></form>",
                    d.revision,
                    e(&d.hash),
                    e(report)
                ),
                (true, None) => "<button type=\"button\" disabled title=\"Simulate first\">Propose</button>".to_owned(),
                _ => String::new(),
            };
            format!(
                "<tr><td class=\"mono\">{}</td><td class=\"mono\">{}</td><td>{}</td><td class=\"mono\">{}</td>\
<td>{sim}</td><td>{simulate}</td><td>{propose}</td></tr>",
                d.revision,
                e(short(&d.hash)),
                e(&d.state),
                e(&d.source)
            )
        })
        .collect();
    let new = if v.can_author {
        format!("<p><a class=\"pill\" href=\"{org}/policy/drafts/new\">New draft</a></p>")
    } else {
        String::new()
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Drafts</h1></div>{}<section><table><tr><th>#</th><th>Hash</th><th>State</th>\
<th>Source</th><th>Simulation</th><th></th><th></th></tr>{rows}</table>{new}</section>",
        svg(icon::LIST),
        nav(frame, "/drafts")
    );
    page("Drafts", Some(frame), &body, frame.base)
}

/// The editor: one textarea per file, and the errors of the last compile.
pub fn new_draft(frame: &Frame<'_>, files: &[(String, String)], errors: &[SourceError]) -> String {
    let org = org_base(frame);
    let csrf = hidden_csrf(frame.csrf);
    let errs = if errors.is_empty() {
        String::new()
    } else {
        let items: String = errors.iter().map(|x| format!("<li class=\"mono\">{}</li>", e(&x.to_string()))).collect();
        format!("<section><h2>{}Errors</h2><ul>{items}</ul></section>", svg(icon::WARNING))
    };
    let areas: String = files
        .iter()
        .map(|(path, text)| {
            let rows = text.lines().count().clamp(4, 30);
            format!(
                "<p class=\"mono\">{}</p><textarea name=\"files[{}]\" rows=\"{rows}\" spellcheck=\"false\" class=\"mono\" \
style=\"width:100%;border-radius:9px;padding:10px;border:0;box-shadow:0 0 0 1px var(--rule);background:var(--paper)\">{}\
</textarea>",
                e(path),
                e(path),
                e(text)
            )
        })
        .collect();
    let body = format!(
        "<div class=\"row\"><h1>{}New draft</h1></div>{}{errs}<section><form method=\"post\" \
action=\"{org}/policy/drafts/new\" style=\"display:block\">{csrf}{areas}<p class=\"mono\">New rules file</p>\
<input name=\"new_path\" placeholder=\"rules/team.toml\"><textarea name=\"new_text\" rows=\"4\" class=\"mono\" \
style=\"width:100%\"></textarea><p><button type=\"submit\">Compile and save</button></p></form></section>",
        svg(icon::LIST),
        nav(frame, "/drafts")
    );
    page("New draft", Some(frame), &body, frame.base)
}

fn sample_rows(org: &str, samples: &[ChangedRef]) -> String {
    samples
        .iter()
        .map(|s| {
            format!(
                "<tr><td class=\"mono\"><a href=\"{org}/policy/samples/{}\">{}</a></td><td>{} → {}</td><td class=\"mono\">{}\
</td></tr>",
                e(&s.id),
                e(&s.id),
                e(s.before.as_str()),
                e(s.after.as_str()),
                e(&s.rules.join(", "))
            )
        })
        .collect()
}

pub fn simulation(frame: &Frame<'_>, hash: &str, revision: i64, ran_at: &str, r: &SimulationReport) -> String {
    let org = org_base(frame);
    let top = |v: &[(String, u32)]| -> String {
        v.iter().map(|(k, n)| format!("<tr><td>{}</td><td>{n}</td></tr>", e(k))).collect()
    };
    let drift = if r.drift > 0 {
        format!("<p><span class=\"pill bad\">{}Drift {}</span></p>", svg(icon::WARNING), r.drift)
    } else {
        String::new()
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Simulation</h1></div>{}<section><h2>{}</h2>{drift}<p class=\"meta\">Revision {revision} \
· <span class=\"mono\">{}</span> · {} cases, {} skipped · {}</p><table><tr><td>Auto → Ask</td><td>{}</td></tr>\
<tr><td>Ask → Auto</td><td>{}</td></tr><tr><td>→ Deny</td><td>{}</td></tr><tr><td>Deny →</td><td>{}</td></tr></table>\
</section><section><h2>Groups</h2><table>{}</table></section><section><h2>Rules</h2><table>{}</table></section>\
<section><h2>Changed</h2><table><tr><th>Request</th><th>Outcome</th><th>Rules</th></tr>{}</table></section>",
        svg(icon::LIST),
        nav(frame, "/drafts"),
        e(&r.line()),
        e(short(hash)),
        r.cases,
        r.skipped,
        e(ran_at),
        r.auto_to_review,
        r.review_to_auto,
        r.to_deny,
        r.from_deny,
        top(&r.top_groups),
        top(&r.top_rules),
        sample_rows(&org, &r.samples)
    );
    page("Simulation", Some(frame), &body, frame.base)
}

/// A retained case, after its read was recorded.
pub fn sample(frame: &Frame<'_>, id: &str, json: &str) -> String {
    let body = format!(
        "<div class=\"row\"><h1>{}Sample</h1></div>{}<section><h2 class=\"mono\">{}</h2><p class=\"meta\">This read is \
in the audit log.</p><pre class=\"mono\">{}</pre></section>",
        svg(icon::LIST),
        nav(frame, "/drafts"),
        e(id),
        e(json)
    );
    page("Sample", Some(frame), &body, frame.base)
}

/// One cohort's numbers.
pub struct CohortView {
    pub decisions: u32,
    pub ask_rate: f64,
    pub median_ttd: u64,
}

pub struct RolloutView {
    pub id: String,
    pub state: String,
    pub revision: i64,
    pub stable: String,
    pub candidate: String,
    /// Stage labels, and how many are published.
    pub stages: Vec<String>,
    pub published: usize,
    pub since: String,
    pub candidate_numbers: CohortView,
    pub stable_numbers: CohortView,
    pub min_decisions: u32,
    pub reason: Option<String>,
    pub can_control: bool,
}

pub fn rollout(frame: &Frame<'_>, active: Option<(i64, &str)>, r: Option<&RolloutView>) -> String {
    let org = org_base(frame);
    let csrf = hidden_csrf(frame.csrf);
    let active = active.map_or_else(
        || "<p class=\"meta\">No active policy.</p>".to_owned(),
        |(rev, hash)| format!("<p>Active: revision {rev} <span class=\"mono\">{}</span></p>", e(short(hash))),
    );
    let section = r.map_or_else(
        || "<section><p class=\"meta\">No rollout.</p></section>".to_owned(),
        |r| {
            let stages: String = r
                .stages
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let class = if i < r.published { "pill ok" } else { "pill" };
                    format!("<span class=\"{class}\">{}</span> ", e(s))
                })
                .collect();
            let wait = r.min_decisions.saturating_sub(r.candidate_numbers.decisions.min(r.stable_numbers.decisions));
            let waiting = if r.state == "running" && wait > 0 {
                format!("<p class=\"meta\">Waiting for {wait} more decisions</p>")
            } else {
                String::new()
            };
            let reason = r.reason.as_deref().map_or_else(String::new, |why| {
                format!("<p><span class=\"pill bad\">{}Rolled back</span> <span class=\"mono\">{}</span></p>", svg(icon::WARNING), e(why))
            });
            let button = |what: &str, label: &str| {
                format!(
                    "<form method=\"post\" action=\"{org}/rollouts/{}/{what}\">{csrf}<button class=\"quiet\" \
type=\"submit\">{label}</button></form>",
                    e(&r.id)
                )
            };
            let controls = if r.can_control {
                match r.state.as_str() {
                    "running" => format!(
                        "<p>{}{}{}</p>",
                        button("pause", "Pause"),
                        button("advance", "Advance"),
                        button("rollback", "Roll back")
                    ),
                    "paused" => format!(
                        "<p>{}{}</p>",
                        button("resume", "Resume"),
                        button("rollback", "Roll back")
                    ),
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            let n = |c: &CohortView| {
                format!("<td>{}</td><td>{:.1}%</td><td>{} s</td>", c.decisions, c.ask_rate, c.median_ttd)
            };
            format!(
                "<section><h2>Revision {} <span class=\"mono\">{}</span></h2><p class=\"meta\">{} · stage since {} · stable \
<span class=\"mono\">{}</span></p><p>{stages}</p>{waiting}{reason}<table><tr><th></th><th>Decisions</th><th>Ask rate</th>\
<th>Median time</th></tr><tr><td>Candidate</td>{}</tr><tr><td>Stable</td>{}</tr></table>{controls}</section>",
                r.revision,
                e(short(&r.candidate)),
                e(&r.state),
                e(&r.since),
                e(short(&r.stable)),
                n(&r.candidate_numbers),
                n(&r.stable_numbers)
            )
        },
    );
    let body = format!(
        "<div class=\"row\"><h1>{}Rollout</h1></div>{}<section>{active}</section>{section}",
        svg(icon::GROUP),
        nav(frame, "/rollout")
    );
    page("Rollout", Some(frame), &body, frame.base)
}

pub struct RepoView {
    /// App id and installation id, and whether its key and webhook secret are posted.
    pub reader: Option<(i64, i64, bool)>,
    /// Bound repo and default branch.
    pub repo: Option<(String, String)>,
    pub can_setup: bool,
    pub can_connect: bool,
}

pub fn repo(frame: &Frame<'_>, v: &RepoView) -> String {
    let org = org_base(frame);
    let csrf = hidden_csrf(frame.csrf);
    let reader = match v.reader {
        Some((app, inst, true)) => {
            format!("<p><span class=\"pill ok\">{}App {app} · installation {inst}</span></p>", svg(icon::CHECK))
        }
        Some((app, inst, false)) => {
            format!("<p>App {app} · installation {inst} <span class=\"meta\">Key not set</span></p>")
        }
        None => "<p class=\"meta\">Not set up.</p>".to_owned(),
    };
    let reader_forms = if v.can_connect {
        format!(
            "<form method=\"post\" action=\"{org}/policy/reader\">{csrf}<input name=\"app_id\" inputmode=\"numeric\" \
placeholder=\"App id\"><input name=\"installation_id\" inputmode=\"numeric\" placeholder=\"Installation id\">\
<button class=\"quiet\" type=\"submit\">Set</button></form><form method=\"post\" action=\"{org}/policy/reader/secrets\" \
style=\"display:block\">{csrf}<textarea name=\"pem\" rows=\"4\" class=\"mono\" placeholder=\"Private key (PEM)\" \
style=\"width:100%\"></textarea><input name=\"webhook_secret\" type=\"password\" placeholder=\"Webhook secret\">\
<button class=\"quiet\" type=\"submit\">Save</button></form>"
        )
    } else {
        String::new()
    };
    let bound = v.repo.as_ref().map_or_else(
        || "<p class=\"meta\">No repo.</p>".to_owned(),
        |(r, b)| format!("<p><span class=\"mono\">{}</span> · {}</p>", e(r), e(b)),
    );
    let repo_forms = if v.can_setup {
        let unbind = if v.repo.is_some() {
            format!(
                "<form method=\"post\" action=\"{org}/policy/repo/unbind\">{csrf}<button class=\"quiet\" type=\"submit\">\
Unbind</button></form>"
            )
        } else {
            String::new()
        };
        format!(
            "<form method=\"post\" action=\"{org}/policy/repo\">{csrf}<input name=\"repo\" placeholder=\"acme/reins-policy\">\
<button type=\"submit\">Bind</button></form>{unbind}"
        )
    } else {
        String::new()
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Repo</h1></div>{}<section><h2>GitHub reader</h2>{reader}{reader_forms}</section>\
<section><h2>Policy repo</h2>{bound}{repo_forms}<p class=\"meta\">A merge only proposes the change. Two admins \
activate it.</p></section>",
        svg(icon::LIST),
        nav(frame, "/repo")
    );
    page("Repo", Some(frame), &body, frame.base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::reins::org::console_pages::Tab;

    fn frame() -> Frame<'static> {
        Frame {
            base: "/reins/org/",
            email: "dana@acme.com",
            csrf: "tok",
            org: Some(("o1", "Acme")),
            tab: Tab::Policy,
            flash: None,
        }
    }

    #[test]
    fn values_are_escaped() {
        let evil = "<script>alert(1)</script>";
        let d = DraftItem {
            revision: 3,
            hash: evil.into(),
            state: "draft".into(),
            source: evil.into(),
            simulation: Some((evil.into(), evil.into(), true)),
        };
        let views = [
            drafts(
                &frame(),
                &DraftsView {
                    drafts: vec![d],
                    can_author: true,
                    can_activate: true,
                },
            ),
            new_draft(
                &frame(),
                &[(evil.into(), evil.into())],
                &[SourceError {
                    file: evil.into(),
                    line: 1,
                    message: evil.into(),
                }],
            ),
            sample(&frame(), evil, evil),
            repo(
                &frame(),
                &RepoView {
                    reader: Some((1, 2, true)),
                    repo: Some((evil.into(), evil.into())),
                    can_setup: true,
                    can_connect: true,
                },
            ),
            rollout(
                &frame(),
                Some((1, evil)),
                Some(&RolloutView {
                    id: evil.into(),
                    state: "running".into(),
                    revision: 2,
                    stable: evil.into(),
                    candidate: evil.into(),
                    stages: vec![evil.into()],
                    published: 1,
                    since: evil.into(),
                    candidate_numbers: CohortView {
                        decisions: 1,
                        ask_rate: 2.0,
                        median_ttd: 3,
                    },
                    stable_numbers: CohortView {
                        decisions: 1,
                        ask_rate: 2.0,
                        median_ttd: 3,
                    },
                    min_decisions: 50,
                    reason: Some(evil.into()),
                    can_control: true,
                }),
            ),
        ];
        for html in views {
            assert!(!html.contains("<script>"), "{html}");
            assert!(html.contains("&lt;script&gt;"));
            assert!(!html.contains("javascript:") && !html.contains(" onclick"));
        }
    }

    #[test]
    fn propose_is_disabled_without_a_fresh_simulation() {
        let item = |sim: Option<(String, String, bool)>| DraftItem {
            revision: 4,
            hash: "h".repeat(64),
            state: "draft".into(),
            source: "pr#3@abc".into(),
            simulation: sim,
        };
        let render = |d: DraftItem| {
            drafts(
                &frame(),
                &DraftsView {
                    drafts: vec![d],
                    can_author: true,
                    can_activate: true,
                },
            )
        };
        for none in [None, Some(("r1".into(), "Ask rate +1.0%".into(), false))] {
            let html = render(item(none));
            assert!(html.contains("disabled title=\"Simulate first\">Propose"), "{html}");
            assert!(!html.contains("/policy/activate"));
        }
        let html = render(item(Some(("r1".into(), "Ask rate +1.0%".into(), true))));
        assert!(html.contains("/policy/activate") && html.contains("name=\"report\" value=\"r1\""), "{html}");
        // Waiting for decisions is shown while a stage can't be judged.
        let r = RolloutView {
            id: "ro_1".into(),
            state: "running".into(),
            revision: 2,
            stable: "s".into(),
            candidate: "c".into(),
            stages: vec!["Pilot".into(), "25%".into(), "Everyone".into()],
            published: 1,
            since: "now".into(),
            candidate_numbers: CohortView {
                decisions: 12,
                ask_rate: 3.0,
                median_ttd: 0,
            },
            stable_numbers: CohortView {
                decisions: 80,
                ask_rate: 2.0,
                median_ttd: 0,
            },
            min_decisions: 50,
            reason: None,
            can_control: true,
        };
        assert!(rollout(&frame(), None, Some(&r)).contains("Waiting for 38 more decisions"));
    }
}
