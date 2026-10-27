//! The admin console's HTML (O1, minimal): server-rendered, no JavaScript, every value escaped. The Next look
//! (`design/next`): paper and ink, one violet accent, Sora with system fallbacks, JetBrains Mono for record values,
//! short copy, real icons drawn inline (Material Symbols paths), never emoji.

#![allow(
    clippy::format_collect,
    clippy::format_in_format_args,
    clippy::format_push_string,
    reason = "small server-rendered pages: building them as readable format strings beats saving allocations"
)]

use reins_proto::org::{AdminAction, IdentityKind, Role, Subject, Tier};

use crate::api::reins::pages::escape_html as e;

const STYLE: &str = ":root{--paper:#f6f6f8;--raised:#fff;--sunken:#ececf0;--ink:#16151b;--ink2:#5f5e6b;--meta:#6b6a78;\
--rule:#e3e3ea;--violet:#5b43e8;--vwash:#ece9fd;--ok:#15803d;--bad:#b91c1c;--dwash:#fbe3e3;--warn:#a16207;\
--wwash:#fdf3d7;--sans:Sora,system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif;\
--mono:\"JetBrains Mono\",ui-monospace,SFMono-Regular,Menlo,monospace;color-scheme:light dark}\
@media (prefers-color-scheme:dark){:root{--paper:#121215;--raised:#1c1c21;--sunken:#26262c;--ink:#ededf0;\
--ink2:#a6a6b0;--meta:#8e8e99;--rule:#2a2a30;--violet:#a196ff;--vwash:rgba(161,150,255,.14);--ok:#34d399;\
--bad:#f87171;--dwash:rgba(248,113,113,.14);--warn:#facc15;--wwash:rgba(250,204,21,.12)}}\
*{box-sizing:border-box}body{font-family:var(--sans);margin:0;background:var(--paper);color:var(--ink);\
-webkit-font-smoothing:antialiased;font-size:14px;line-height:20px}\
header{display:flex;align-items:center;gap:12px;padding:14px 24px;border-bottom:1px solid var(--rule);\
background:var(--raised)}header .brand{font-weight:600;letter-spacing:-.01em}header nav{display:flex;gap:4px;flex:1}\
header nav a{padding:6px 10px;border-radius:8px;color:var(--ink2);text-decoration:none}\
header nav a.on{background:var(--sunken);color:var(--ink)}header .who{color:var(--meta);font-size:13px}\
main{max-width:960px;margin:0 auto;padding:24px;display:flex;flex-direction:column;gap:18px}\
h1{font-size:24px;line-height:30px;font-weight:600;letter-spacing:-.02em;margin:0}\
h2{font-size:13px;font-weight:600;text-transform:uppercase;letter-spacing:.06em;color:var(--meta);margin:0 0 8px}\
section{background:var(--raised);border-radius:14px;box-shadow:0 0 0 1px var(--rule);padding:16px 18px}\
table{width:100%;border-collapse:collapse}td,th{text-align:left;padding:9px 6px;border-top:1px solid var(--rule);\
vertical-align:top}th{font-weight:500;color:var(--meta);font-size:12px;border-top:0}\
.mono{font-family:var(--mono);font-size:12.5px}.meta{color:var(--meta);font-size:12.5px}\
.pill{display:inline-flex;align-items:center;gap:4px;padding:1px 8px;border-radius:999px;font-size:12px;\
background:var(--sunken);color:var(--ink2);margin:0 4px 2px 0}.pill.ok{color:var(--ok)}.pill.v{background:var(--vwash);\
color:var(--violet)}.pill.bad{background:var(--dwash);color:var(--bad)}\
form{display:inline-flex;gap:6px;align-items:center;flex-wrap:wrap;margin:0}\
input,select{height:34px;padding:0 10px;font:inherit;color:var(--ink);background:var(--paper);border:0;\
box-shadow:0 0 0 1px var(--rule);border-radius:9px}input:focus,select:focus{outline:2px solid var(--violet)}\
button{height:34px;padding:0 14px;border:0;border-radius:999px;background:var(--ink);color:var(--paper);font:inherit;\
font-weight:600;cursor:pointer;display:inline-flex;align-items:center;gap:6px}a.button{display:inline-flex;height:34px;align-items:center;padding:0 14px;border-radius:999px;\
background:var(--ink);color:var(--paper);font-weight:600;text-decoration:none}button.quiet{background:var(--sunken);\
color:var(--ink)}button.danger{background:var(--dwash);color:var(--bad)}\
.changed{color:var(--ink);font-weight:600}.flash{padding:10px 14px;border-radius:12px;background:var(--vwash);color:var(--violet)}\
.flash.bad{background:var(--dwash);color:var(--bad)}.banner{padding:12px 14px;border-radius:12px;\
background:var(--wwash);color:var(--warn);display:flex;gap:8px;align-items:center}\
.row{display:flex;gap:12px;align-items:center;justify-content:space-between;flex-wrap:wrap}\
.record{background:var(--sunken);border-radius:10px;padding:8px 10px;margin-top:6px;word-break:break-all}\
svg.i{width:18px;height:18px;fill:currentColor;flex:none}a{color:var(--violet)}";

/// Material Symbols paths (24 px grid), drawn inline.
pub mod icon {
    pub const SHIELD: &str =
        "M12 22q-3.48-.88-5.74-3.99Q4 14.9 4 11.1V5l8-3 8 3v6.1q0 3.8-2.26 6.91Q15.48 21.12 12 22Z";
    pub const CHECK: &str = "m9.55 18-5.7-5.7 1.43-1.43 4.27 4.28 9.18-9.18 1.42 1.42Z";
    pub const WARNING: &str = "M1 21 12 2l11 19Zm11-3q.43 0 .71-.29.29-.28.29-.71t-.29-.71Q12.43 16 12 16t-.71.29Q11 \
16.57 11 17t.29.71q.28.29.71.29Zm-1-3h2v-5h-2Z";
    pub const GROUP: &str = "M1 20v-2.8q0-.85.44-1.56.43-.71 1.16-1.09 1.55-.78 3.15-1.16Q7.35 13 9 13t3.25.39q1.6.38 \
3.15 1.16.73.38 1.16 1.09.44.71.44 1.56V20Zm8-8q-1.65 0-2.82-1.18Q5 9.65 5 8t1.18-2.83Q7.35 4 9 4t2.83 1.17Q13 \
6.35 13 8t-1.17 2.82Q10.65 12 9 12Z";
    pub const LIST: &str = "M8 19v-2h13v2Zm0-6v-2h13v2Zm0-6V5h13v2ZM4 20q-.83 0-1.41-.59Q2 18.83 2 18t.59-1.41Q3.17 \
16 4 16t1.41.59Q6 17.17 6 18t-.59 1.41Q4.83 20 4 20Zm0-6q-.83 0-1.41-.59Q2 12.83 2 12t.59-1.41Q3.17 10 4 10t1.41.59\
Q6 11.17 6 12t-.59 1.41Q4.83 14 4 14Zm0-6q-.83 0-1.41-.59Q2 6.83 2 6t.59-1.41Q3.17 4 4 4t1.41.59Q6 5.17 6 6t-.59 \
1.41Q4.83 8 4 8Z";
}

fn svg(path: &str) -> String {
    format!("<svg class=\"i\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"{path}\"/></svg>")
}

/// Which tab is open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Overview,
    Members,
    Events,
}

/// What every signed-in page shows in its header.
pub struct Frame<'a> {
    /// The console's absolute base path, ending in `/` (`{domain path}/reins/org/`).
    pub base: &'a str,
    pub email: &'a str,
    pub csrf: &'a str,
    /// `(org id, org name)` when an org is open.
    pub org: Option<(&'a str, &'a str)>,
    pub tab: Tab,
    pub flash: Option<(&'a str, bool)>,
}

/// A whole page. `up`: the console's absolute base path (links are absolute: Rocket has no trailing-slash routes).
pub fn page(title: &str, frame: Option<&Frame<'_>>, body: &str, up: &str) -> String {
    let header = frame.map_or_else(String::new, |f| {
        let tab = |t: Tab, href: String, label: &str| {
            let on = if f.tab == t {
                " class=\"on\""
            } else {
                ""
            };
            format!("<a{on} href=\"{href}\">{label}</a>")
        };
        let nav = match f.org {
            Some((id, _)) => [
                tab(Tab::Home, up.to_owned(), "Orgs"),
                tab(Tab::Overview, format!("{up}{}", e(id)), "Overview"),
                tab(Tab::Members, format!("{up}{}/members", e(id)), "Members"),
                tab(Tab::Events, format!("{up}{}/events", e(id)), "Events"),
            ]
            .concat(),
            None => tab(Tab::Home, up.to_owned(), "Orgs"),
        };
        format!(
            "<header>{shield}<span class=\"brand\">{brand}</span><nav>{nav}</nav><span class=\"who\">{email}</span>\
<form method=\"post\" action=\"{up}logout\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
<button class=\"quiet\" type=\"submit\">Sign out</button></form></header>",
            shield = svg(icon::SHIELD),
            brand = f.org.map_or_else(|| "Reins for organizations".to_owned(), |(_, name)| e(name)),
            email = e(f.email),
            csrf = e(f.csrf),
        )
    });
    let flash = frame
        .and_then(|f| f.flash)
        .map(|(text, bad)| {
            format!(
                "<p class=\"flash{}\" role=\"status\">{}</p>",
                if bad {
                    " bad"
                } else {
                    ""
                },
                e(text)
            )
        })
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex,nofollow\">\
<title>{title}</title><style>{STYLE}</style></head><body>{header}<main>{flash}{body}</main></body></html>",
        title = e(title),
    )
}

fn hidden_csrf(csrf: &str) -> String {
    format!("<input type=\"hidden\" name=\"csrf\" value=\"{}\">", e(csrf))
}

pub fn signed_out(base: &str, error: Option<&str>) -> String {
    let error = error.map(|m| format!("<p class=\"flash bad\" role=\"alert\">{}</p>", e(m))).unwrap_or_default();
    let body = format!(
        "{error}<section><h1>Reins for organizations</h1><p class=\"meta\">Sign in with your work account.</p>\
<p><a class=\"button\" href=\"{b}login\">Sign in</a></p></section>",
        b = e(base)
    );
    page("Reins for organizations", None, &body, base)
}

pub struct HomeView<'a> {
    pub orgs: Vec<(String, String, Tier)>,
    /// The domain a new org would claim, when this account may create one.
    pub can_create: Option<&'a str>,
}

pub fn home(frame: &Frame<'_>, view: &HomeView<'_>) -> String {
    let orgs: String = if view.orgs.is_empty() {
        "<p class=\"meta\">No organizations yet.</p>".to_owned()
    } else {
        view.orgs
            .iter()
            .map(|(id, name, tier)| {
                format!(
                    "<tr><td><a href=\"{}{}\">{}</a></td><td><span class=\"pill\">{}</span></td></tr>",
                    e(frame.base),
                    e(id),
                    e(name),
                    tier.label()
                )
            })
            .collect::<String>()
    };
    let orgs = if view.orgs.is_empty() {
        orgs
    } else {
        format!("<table>{orgs}</table>")
    };
    let create = view.can_create.map_or_else(String::new, |domain| {
        format!(
            "<section><h2>New organization</h2><p class=\"meta\">Claims <span class=\"mono\">{d}</span>. \
This account becomes its Org owner and a work account.</p><form method=\"post\" action=\"{base}new\">{csrf}\
<input name=\"name\" placeholder=\"Name\" maxlength=\"64\" required><select name=\"tier\">\
<option value=\"team\">Team</option><option value=\"business\" selected>Business</option>\
<option value=\"enterprise\">Enterprise</option></select><button type=\"submit\">Create</button></form></section>",
            d = e(domain),
            base = e(frame.base),
            csrf = hidden_csrf(frame.csrf),
        )
    });
    page("Organizations", Some(frame), &format!("<h1>Organizations</h1><section>{orgs}</section>{create}"), frame.base)
}

pub struct DomainView {
    pub domain: String,
    pub verified: bool,
    pub record_name: String,
    pub record_value: String,
}

pub struct ChangeView {
    pub id: String,
    pub action: AdminAction,
    pub proposed_by: String,
    pub can_approve: bool,
    pub can_reject: bool,
}

#[allow(clippy::struct_excessive_bools, reason = "independent facts the page shows")]
pub struct OverviewView {
    pub org_id: String,
    pub name: String,
    pub tier: Tier,
    pub identity: IdentityKind,
    pub workos_org_id: Option<String>,
    pub directory_id: Option<String>,
    pub github_org: Option<String>,
    pub two_person: bool,
    pub owner_recovery: bool,
    pub domains: Vec<DomainView>,
    pub changes: Vec<ChangeView>,
    pub can_set_up: bool,
    /// Names for the members and groups pending changes mention.
    pub names: std::collections::HashMap<String, String>,
    /// The roles members (by membership) and groups hold now.
    pub roles: std::collections::HashMap<String, Vec<Role>>,
    /// Members the directory sync held back (how many, and the digest a confirmation names).
    pub held: Option<(usize, String)>,
    /// The viewer may propose removing them.
    pub can_confirm_held: bool,
}

fn name_of<'a>(names: &'a std::collections::HashMap<String, String>, id: &'a str) -> &'a str {
    names.get(id).map_or(id, String::as_str)
}

fn or_dash(v: Option<&str>) -> String {
    v.filter(|v| !v.is_empty()).map_or_else(|| "—".to_owned(), e)
}

/// One field of a change: its value now, and what the change makes it (all escaped by the caller).
fn field(label: &str, old: &str, new: &str) -> String {
    let changed = if old == new {
        ""
    } else {
        " changed"
    };
    format!(
        "<div class=\"meta{changed}\">{label}: <span class=\"mono\">{old}</span> &rarr; <span class=\"mono\">{new}</span></div>"
    )
}

fn roles_text(roles: Option<&Vec<Role>>) -> String {
    match roles {
        Some(r) if !r.is_empty() => r.iter().map(|r| r.label()).collect::<Vec<_>>().join(", "),
        _ => "none".to_owned(),
    }
}

/// A change as its approver must see it: what it is, and every field it sets, from its value now to the new one.
pub fn describe(action: &AdminAction, v: &OverviewView) -> String {
    let names = &v.names;
    let subject = |s: &Subject| match s {
        Subject::Member(id) => (e(name_of(names, id)), v.roles.get(id)),
        Subject::Group(id) => (format!("group {}", e(name_of(names, id))), v.roles.get(id)),
    };
    let (title, fields) = match action {
        AdminAction::GrantRole {
            subject: s,
            role,
        }
        | AdminAction::RevokeRole {
            subject: s,
            role,
        } => {
            let (who, roles) = subject(s);
            let now = roles_text(roles);
            let mut after: Vec<Role> = roles.cloned().unwrap_or_default();
            let grant = matches!(action, AdminAction::GrantRole { .. });
            if grant {
                after.push(*role);
            } else {
                after.retain(|r| r != role);
            }
            after.sort();
            after.dedup();
            let title = if grant {
                format!("Give {} to {who}", role.label())
            } else {
                format!("Take {} from {who}", role.label())
            };
            (title, field("Roles", &e(&now), &e(&roles_text(Some(&after)))))
        }
        AdminAction::SetIdentity {
            identity,
            workos_org_id,
            directory_id,
            github_org,
        } => (
            "Change how members sign in".to_owned(),
            [
                field("Sign-in", v.identity.label(), identity.label()),
                field("WorkOS organization", &or_dash(v.workos_org_id.as_deref()), &or_dash(workos_org_id.as_deref())),
                field("Directory", &or_dash(v.directory_id.as_deref()), &or_dash(directory_id.as_deref())),
                field("GitHub organization", &or_dash(v.github_org.as_deref()), &or_dash(github_org.as_deref())),
            ]
            .concat(),
        ),
        AdminAction::SetTier {
            tier,
        } => ("Change the plan".to_owned(), field("Plan", v.tier.label(), tier.label())),
        AdminAction::RemoveDomain {
            domain,
        } => {
            let verified = v.domains.iter().find(|d| d.domain == *domain).is_some_and(|d| d.verified);
            (
                format!("Remove the domain {}", e(domain)),
                field(
                    "Domain",
                    &format!(
                        "{} ({})",
                        e(domain),
                        if verified {
                            "verified"
                        } else {
                            "pending"
                        }
                    ),
                    "removed",
                ),
            )
        }
        AdminAction::RemoveMember {
            member,
        } => (
            format!("Remove {}", e(name_of(names, member))),
            field("Roles", &e(&roles_text(v.roles.get(member))), "none, signed out everywhere"),
        ),
        AdminAction::ConfirmDirectoryRemovals {
            held,
        } => {
            let now = v
                .held
                .as_ref()
                .filter(|(_, d)| d == held)
                .map_or_else(|| "changed since".to_owned(), |(n, _)| format!("{n} not in the directory"));
            (
                "Remove members the directory dropped".to_owned(),
                field("Members", &e(&now), "removed, signed out everywhere"),
            )
        }
        AdminAction::SetTwoPerson {
            on,
        } => (
            "Two-person approval".to_owned(),
            field(
                "Two-person",
                if v.two_person {
                    "on"
                } else {
                    "off"
                },
                if *on {
                    "on"
                } else {
                    "off"
                },
            ),
        ),
        AdminAction::ActivatePolicy {
            revision,
            hash,
        } => (
            format!("Activate policy revision {revision}"),
            field("Policy", "current", &e(&format!("revision {revision} · {}", hash.get(..12).unwrap_or(hash)))),
        ),
        AdminAction::SetRetention {
            days,
        } => ("Request body retention".to_owned(), field("Bodies kept", "current", &retention_text(*days))),
        AdminAction::RegisterExecutor {
            executor_id,
            mode,
            public_key,
        } => (
            format!("Register executor {}", e(executor_id)),
            [field("Mode", "—", mode.as_str()), field("Key", "—", &e(public_key.get(..16).unwrap_or(public_key)))]
                .concat(),
        ),
        AdminAction::RemoveExecutor {
            executor_id,
        } => (format!("Remove executor {}", e(executor_id)), field("Executor", "registered", "removed")),
    };
    format!("<div>{title}</div>{fields}")
}

/// "none", "30 days", "1 year".
#[must_use]
pub fn retention_text(days: u32) -> String {
    match days {
        0 => "none".to_owned(),
        365 => "1 year".to_owned(),
        d => format!("{d} days"),
    }
}

pub fn overview(frame: &Frame<'_>, v: &OverviewView) -> String {
    let csrf = hidden_csrf(frame.csrf);
    let recovery = if v.owner_recovery {
        format!(
            "<p class=\"banner\" role=\"alert\">{}The directory removed the last Org owner. Two Security admins can \
make a new one.</p>",
            svg(icon::WARNING)
        )
    } else {
        String::new()
    };
    let held = match &v.held {
        Some((n, digest)) => {
            let button = if v.can_confirm_held {
                format!(
                    "<form method=\"post\" action=\"{}{}/directory/confirm\">{csrf}<input type=\"hidden\" \
name=\"held\" value=\"{}\"><button type=\"submit\">Remove them</button></form>",
                    e(frame.base),
                    e(&v.org_id),
                    e(digest)
                )
            } else {
                String::new()
            };
            format!(
                "<div class=\"banner\" role=\"alert\">{}<p>The directory no longer lists {n} members. Kept until an \
admin confirms.</p>{button}</div>",
                svg(icon::WARNING)
            )
        }
        None => String::new(),
    };
    let recovery = format!("{recovery}{held}");
    let domains: String = v
        .domains
        .iter()
        .map(|d| {
            let state = if d.verified {
                format!("<span class=\"pill ok\">{}Verified</span>", svg(icon::CHECK))
            } else {
                format!(
                    "<span class=\"pill\">Pending</span><div class=\"meta\">Add this TXT record, then Verify.</div>\
<div class=\"record mono\">{}<br>{}</div>",
                    e(&d.record_name),
                    e(&d.record_value)
                )
            };
            let actions = if v.can_set_up {
                let verify = if d.verified {
                    String::new()
                } else {
                    format!(
                        "<form method=\"post\" action=\"{id}/domains/verify\">{csrf}<input type=\"hidden\" name=\"domain\" \
value=\"{d}\"><button type=\"submit\">Verify</button></form>",
                        id = format!("{}{}", e(frame.base), e(&v.org_id)),
                        d = e(&d.domain)
                    )
                };
                format!(
                    "{verify}<form method=\"post\" action=\"{id}/domains/remove\">{csrf}<input type=\"hidden\" \
name=\"domain\" value=\"{d}\"><button class=\"quiet\" type=\"submit\">Remove</button></form>",
                    id = format!("{}{}", e(frame.base), e(&v.org_id)),
                    d = e(&d.domain)
                )
            } else {
                String::new()
            };
            format!("<tr><td class=\"mono\">{}</td><td>{state}</td><td>{actions}</td></tr>", e(&d.domain))
        })
        .collect();
    let add_domain = if v.can_set_up {
        format!(
            "<form method=\"post\" action=\"{}{}/domains\">{csrf}<input name=\"domain\" placeholder=\"acme.com\" \
required><button class=\"quiet\" type=\"submit\">Add domain</button></form>",
            e(frame.base),
            e(&v.org_id)
        )
    } else {
        String::new()
    };
    let opt = |id: IdentityKind| {
        let sel = if id == v.identity {
            " selected"
        } else {
            ""
        };
        format!("<option value=\"{}\"{sel}>{}</option>", id.as_str(), id.label())
    };
    let identity_form = if v.can_set_up {
        format!(
            "<form method=\"post\" action=\"{id}/identity\">{csrf}<select name=\"identity\">{opts}</select>\
<input name=\"workos_org_id\" placeholder=\"WorkOS org (org_…)\" value=\"{w}\">\
<input name=\"directory_id\" placeholder=\"Directory (directory_…)\" value=\"{dir}\">\
<input name=\"github_org\" placeholder=\"GitHub org\" value=\"{gh}\"><button type=\"submit\">Save</button></form>",
            id = format!("{}{}", e(frame.base), e(&v.org_id)),
            opts = IdentityKind::ALL.into_iter().filter(|i| v.tier.allows(*i)).map(opt).collect::<String>(),
            w = e(v.workos_org_id.as_deref().unwrap_or_default()),
            dir = e(v.directory_id.as_deref().unwrap_or_default()),
            gh = e(v.github_org.as_deref().unwrap_or_default()),
        )
    } else {
        String::new()
    };
    let tier_form = if v.can_set_up {
        let opts: String = Tier::ALL
            .into_iter()
            .map(|t| {
                let sel = if t == v.tier {
                    " selected"
                } else {
                    ""
                };
                format!("<option value=\"{}\"{sel}>{}</option>", t.as_str(), t.label())
            })
            .collect();
        format!(
            "<form method=\"post\" action=\"{id}/tier\">{csrf}<select name=\"tier\">{opts}</select>\
<button class=\"quiet\" type=\"submit\">Change plan</button></form>\
<form method=\"post\" action=\"{id}/two-person\">{csrf}<input type=\"hidden\" name=\"on\" value=\"{flip}\">\
<button class=\"quiet\" type=\"submit\">{label}</button></form>",
            id = format!("{}{}", e(frame.base), e(&v.org_id)),
            flip = !v.two_person,
            label = if v.two_person {
                "Turn two-person off"
            } else {
                "Turn two-person on"
            },
        )
    } else {
        String::new()
    };
    let changes: String = v
        .changes
        .iter()
        .map(|ch| {
            let mut buttons = String::new();
            if ch.can_approve {
                buttons.push_str(&format!(
                    "<form method=\"post\" action=\"{id}/changes/approve\">{csrf}<input type=\"hidden\" name=\"change\" \
value=\"{c}\"><button type=\"submit\">{}Approve</button></form>",
                    svg(icon::CHECK),
                    id = format!("{}{}", e(frame.base), e(&v.org_id)),
                    c = e(&ch.id)
                ));
            }
            if ch.can_reject {
                buttons.push_str(&format!(
                    "<form method=\"post\" action=\"{id}/changes/reject\">{csrf}<input type=\"hidden\" name=\"change\" \
value=\"{c}\"><button class=\"quiet\" type=\"submit\">Reject</button></form>",
                    id = format!("{}{}", e(frame.base), e(&v.org_id)),
                    c = e(&ch.id)
                ));
            }
            if !ch.can_approve && ch.can_reject {
                buttons.push_str("<span class=\"meta\">Waiting for a second admin</span>");
            }
            format!(
                "<tr><td>{}</td><td class=\"meta\">{}</td><td>{buttons}</td></tr>",
                describe(&ch.action, v),
                e(&ch.proposed_by)
            )
        })
        .collect();
    let changes = if changes.is_empty() {
        "<p class=\"meta\">Nothing waiting.</p>".to_owned()
    } else {
        format!("<table><tr><th>Change</th><th>Proposed by</th><th></th></tr>{changes}</table>")
    };
    let bound = match v.identity {
        IdentityKind::None => String::new(),
        _ => format!(
            "<p class=\"meta mono\">{}{}</p>",
            e(v.workos_org_id.as_deref().unwrap_or_default()),
            v.directory_id.as_deref().map(|d| format!(" · {}", e(d))).unwrap_or_default()
        ),
    };
    let body = format!(
        "{recovery}<div class=\"row\"><h1>{name}</h1><span><span class=\"pill v\">{tier}</span>\
<span class=\"pill\">{two}</span></span></div>\
<section><h2>Waiting for approval</h2>{changes}</section>\
<section><h2>Domains</h2><table>{domains}</table>{add_domain}</section>\
<section><h2>Sign-in</h2><p><span class=\"pill v\">{idl}</span></p>{bound}{identity_form}</section>\
<section><h2>Plan</h2>{tier_form}</section>",
        name = e(&v.name),
        tier = v.tier.label(),
        two = if v.two_person {
            "Two-person on"
        } else {
            "Two-person off"
        },
        idl = v.identity.label(),
    );
    page(&v.name, Some(frame), &body, frame.base)
}

pub struct MemberRow {
    pub membership: String,
    pub email: String,
    pub active: bool,
    pub from_directory: bool,
    pub roles: Vec<Role>,
    pub groups: Vec<String>,
    /// One line per enrolled device.
    pub devices: Vec<String>,
}

pub struct GroupRow {
    pub id: String,
    pub name: String,
    pub from_directory: bool,
    pub members: usize,
    pub roles: Vec<Role>,
}

pub struct MembersView {
    pub org_id: String,
    pub members: Vec<MemberRow>,
    pub groups: Vec<GroupRow>,
    pub can_assign: bool,
    pub can_remove: bool,
}

fn roles_html(roles: &[Role]) -> String {
    roles.iter().map(|r| format!("<span class=\"pill v\">{}</span>", r.label())).collect()
}

pub fn members(frame: &Frame<'_>, v: &MembersView) -> String {
    let csrf = hidden_csrf(frame.csrf);
    let org = format!("{}{}", e(frame.base), e(&v.org_id));
    let rows: String = v
        .members
        .iter()
        .map(|m| {
            let state = if m.active {
                ""
            } else {
                "<span class=\"pill bad\">Removed</span>"
            };
            let dir = if m.from_directory {
                "<span class=\"pill\">Directory</span>"
            } else {
                ""
            };
            let devices = if m.devices.is_empty() {
                "<span class=\"meta\">No enrolled device</span>".to_owned()
            } else {
                m.devices.iter().map(|d| format!("<div class=\"meta\">{}</div>", e(d))).collect()
            };
            let remove = if v.can_remove && m.active {
                format!(
                    "<form method=\"post\" action=\"{org}/members/remove\">{csrf}<input type=\"hidden\" name=\"member\" \
value=\"{}\"><button class=\"danger\" type=\"submit\">Remove</button></form>",
                    e(&m.membership),
                    org = org,
                )
            } else {
                String::new()
            };
            format!(
                "<tr><td>{email} {state}{dir}<div class=\"meta\">{groups}</div></td><td>{roles}</td><td>{devices}</td>\
<td>{remove}</td></tr>",
                email = e(&m.email),
                groups = e(&m.groups.join(", ")),
                roles = roles_html(&m.roles),
            )
        })
        .collect();
    let groups: String = v
        .groups
        .iter()
        .map(|g| {
            format!(
                "<tr><td>{}{}</td><td>{}</td><td class=\"meta\">{} members</td></tr>",
                e(&g.name),
                if g.from_directory {
                    " <span class=\"pill\">Directory</span>"
                } else {
                    ""
                },
                roles_html(&g.roles),
                g.members
            )
        })
        .collect();
    let assign = if v.can_assign {
        let subjects: String = v
            .members
            .iter()
            .filter(|m| m.active)
            .map(|m| format!("<option value=\"member:{}\">{}</option>", e(&m.membership), e(&m.email)))
            .chain(
                v.groups.iter().map(|g| format!("<option value=\"group:{}\">Group: {}</option>", e(&g.id), e(&g.name))),
            )
            .collect();
        let roles: String =
            Role::ALL.iter().map(|r| format!("<option value=\"{}\">{}</option>", r.as_str(), r.label())).collect();
        format!(
            "<section><h2>Roles</h2><form method=\"post\" action=\"{org}/roles\">{csrf}<select name=\"subject\">{subjects}\
</select><select name=\"role\">{roles}</select><select name=\"op\"><option value=\"grant\">Give</option>\
<option value=\"revoke\">Take away</option></select><button type=\"submit\">Propose</button></form></section>"
        )
    } else {
        String::new()
    };
    let body = format!(
        "<h1>Members</h1><section><table><tr><th>Member</th><th>Roles</th><th>Devices</th><th></th></tr>{rows}\
</table></section>{assign}<section><h2>{}Groups</h2><table>{groups}</table></section>",
        svg(icon::GROUP)
    );
    page("Members", Some(frame), &body, frame.base)
}

pub struct EventRow {
    pub seq: i64,
    pub at: String,
    pub kind: String,
    pub actor: Option<String>,
    pub subject: Option<String>,
    pub detail: String,
}

pub fn events(frame: &Frame<'_>, rows: &[EventRow], older: Option<i64>, chain_ok: Option<bool>) -> String {
    let list: String = rows
        .iter()
        .map(|r| {
            format!(
                "<tr><td class=\"mono\">{}</td><td class=\"meta\">{}</td><td>{}</td><td>{}</td><td class=\"mono\">{}</td>\
<td class=\"mono meta\">{}</td></tr>",
                r.seq,
                e(&r.at),
                e(&r.kind),
                e(r.actor.as_deref().unwrap_or("Directory")),
                e(r.subject.as_deref().unwrap_or_default()),
                e(&r.detail)
            )
        })
        .collect();
    let org = frame.org.map(|(id, _)| format!("{}{}", e(frame.base), e(id))).unwrap_or_default();
    let more = older.map(|b| format!("<p><a href=\"{org}/events?before={b}\">Older</a></p>")).unwrap_or_default();
    let chain = match chain_ok {
        Some(true) => format!("<span class=\"pill ok\">{}Chain intact</span>", svg(icon::CHECK)),
        Some(false) => format!("<span class=\"pill bad\">{}Chain broken</span>", svg(icon::WARNING)),
        None => String::new(),
    };
    let body = format!(
        "<div class=\"row\"><h1>{}Events</h1>{chain}</div><section><table><tr><th>#</th><th>When</th><th>What</th>\
<th>Who</th><th>About</th><th>Detail</th></tr>{list}</table>{more}</section>",
        svg(icon::LIST)
    );
    page("Events", Some(frame), &body, frame.base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame<'static> {
        Frame {
            base: "/reins/org/",
            email: "dana@acme.com",
            csrf: "tok",
            org: Some(("o1", "Acme")),
            tab: Tab::Overview,
            flash: None,
        }
    }

    fn view() -> OverviewView {
        OverviewView {
            org_id: "o1".to_owned(),
            name: "<Acme>".to_owned(),
            tier: Tier::Business,
            identity: IdentityKind::None,
            workos_org_id: None,
            directory_id: None,
            github_org: None,
            two_person: true,
            owner_recovery: false,
            domains: vec![DomainView {
                domain: "acme.com".to_owned(),
                verified: false,
                record_name: "_reins-challenge.acme.com".to_owned(),
                record_value: "reins-domain-verification=abc".to_owned(),
            }],
            changes: vec![ChangeView {
                id: "ch1".to_owned(),
                action: AdminAction::RemoveMember {
                    member: "m2".to_owned(),
                },
                proposed_by: "dana@acme.com".to_owned(),
                can_approve: false,
                can_reject: true,
            }],
            can_set_up: true,
            names: std::iter::once(("m2".to_owned(), "<script>lee</script>".to_owned())).collect(),
            roles: std::iter::once(("m2".to_owned(), vec![Role::SecurityAdmin])).collect(),
            held: None,
            can_confirm_held: false,
        }
    }

    #[test]
    fn held_directory_removals_are_shown_and_confirmed_by_digest() {
        let mut v = view();
        assert!(!overview(&frame(), &v).contains("no longer lists"));
        v.held = Some((7, "abc123".to_owned()));
        let html = overview(&frame(), &v);
        assert!(html.contains("no longer lists 7 members"));
        assert!(!html.contains("directory/confirm"), "only for whoever may remove members");
        v.can_confirm_held = true;
        let html = overview(&frame(), &v);
        assert!(html.contains("/reins/org/o1/directory/confirm") && html.contains("value=\"abc123\""));
        let d = describe(
            &AdminAction::ConfirmDirectoryRemovals {
                held: "abc123".to_owned(),
            },
            &v,
        );
        assert!(d.contains("7 not in the directory"), "{d}");
        let stale = describe(
            &AdminAction::ConfirmDirectoryRemovals {
                held: "old".to_owned(),
            },
            &v,
        );
        assert!(stale.contains("changed since"), "{stale}");
    }

    #[test]
    fn the_approver_sees_every_field_of_the_change() {
        let mut v = view();
        v.identity = IdentityKind::Sso;
        v.workos_org_id = Some("org_OLD".to_owned());
        v.changes[0].action = AdminAction::SetIdentity {
            identity: IdentityKind::Sso,
            workos_org_id: Some("org_NEW<x>".to_owned()),
            directory_id: Some("directory_NEW".to_owned()),
            github_org: None,
        };
        let html = overview(&frame(), &v);
        for part in [
            "Sign-in",
            "SSO (SAML or OIDC)",
            "org_OLD",
            "org_NEW&lt;x&gt;",
            "Directory",
            "directory_NEW",
            "GitHub organization",
        ] {
            assert!(html.contains(part), "{part}");
        }
        assert!(!html.contains("org_NEW<x>"));
        // A removal shows what the member holds now.
        assert!(overview(&frame(), &view()).contains("Security admin"));
        // Every kind of action shows its fields.
        for action in [
            AdminAction::SetTier {
                tier: Tier::Team,
            },
            AdminAction::SetTwoPerson {
                on: false,
            },
            AdminAction::ConfirmDirectoryRemovals {
                held: "d1".to_owned(),
            },
            AdminAction::RemoveDomain {
                domain: "acme.com".to_owned(),
            },
            AdminAction::GrantRole {
                subject: Subject::Member("m2".to_owned()),
                role: Role::Auditor,
            },
            AdminAction::RevokeRole {
                subject: Subject::Member("m2".to_owned()),
                role: Role::SecurityAdmin,
            },
        ] {
            let d = describe(&action, &view());
            assert!(d.contains("&rarr;"), "{d}");
        }
        assert!(
            describe(
                &AdminAction::SetTier {
                    tier: Tier::Team
                },
                &view()
            )
            .contains("Business")
        );
        let grant = AdminAction::GrantRole {
            subject: Subject::Member("m2".to_owned()),
            role: Role::Auditor,
        };
        assert!(describe(&grant, &view()).contains("Security admin, Auditor"));
    }

    #[test]
    fn values_are_escaped() {
        let html = overview(&frame(), &view());
        assert!(html.contains("&lt;Acme&gt;"));
        assert!(!html.contains("<Acme>"));
        assert!(html.contains("&lt;script&gt;lee"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn pending_changes_hide_approve_from_their_proposer() {
        let html = overview(&frame(), &view());
        assert!(!html.contains("changes/approve"));
        assert!(html.contains("changes/reject"));
        assert!(html.contains("Waiting for a second admin"));
        let mut v = view();
        v.changes[0].can_approve = true;
        assert!(overview(&frame(), &v).contains("changes/approve"));
    }

    #[test]
    fn the_txt_record_is_shown_exactly() {
        let html = overview(&frame(), &view());
        assert!(html.contains("_reins-challenge.acme.com<br>reins-domain-verification=abc"));
        assert!(html.contains("name=\"csrf\" value=\"tok\""));
        assert!(html.contains("action=\"/reins/org/o1/domains/verify\""), "links are absolute");
    }

    #[test]
    fn the_owner_recovery_banner_shows() {
        let mut v = view();
        v.owner_recovery = true;
        assert!(overview(&frame(), &v).contains("removed the last Org owner"));
        assert!(!overview(&frame(), &view()).contains("removed the last Org owner"));
    }

    #[test]
    fn pages_carry_no_script() {
        for html in [
            overview(&frame(), &view()),
            signed_out("/reins/org/", Some("<x>")),
            events(&frame(), &[], None, Some(true)),
        ] {
            assert!(!html.contains("<script"), "{html}");
        }
    }
}
