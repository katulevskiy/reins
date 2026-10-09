//! The phone: a coded recreation of the Reins app (Activity, the approval sheet, Keys), drawn in
//! iPhone points (390x844) and scaled. Requests buzz the phone, grow the island into a banner,
//! raise the sheet, get tapped and leave a line in Activity.

use fframes::{FFramesContext, Frame, Svgr};

use crate::kit::*;

pub const PW: f32 = 390.0;
pub const PH: f32 = 844.0;
const SHEET_TOP: f32 = 72.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    ForcePush,
    Push,
    RmRf,
    Email,
    Merge,
    Pay,
    /// The same shop asking again after the cart changed (banner only).
    PayAgain,
}

/// One request: the phone buzzes at `buzz`, the sheet is up at `up`, the finger taps at `tap`.
#[derive(Clone, Copy)]
pub struct Req {
    pub kind: Kind,
    pub buzz: f32,
    pub up: f32,
    pub tap: f32,
    pub approve: bool,
}

impl Req {
    /// When the decision is final: Face ID takes a second for payments.
    pub fn done(&self) -> f32 {
        if self.kind == Kind::Pay && self.approve {
            self.tap + 1.15
        } else {
            self.tap + 0.05
        }
    }
    fn gone(&self) -> f32 {
        self.done() + 0.5
    }
}

struct Info {
    icon: Icon,
    color: &'static str,
    title: &'static str,
    source: &'static str,
    source_icon: Icon,
    tags: [&'static str; 2],
    advice: (bool, Icon, &'static str, &'static str),
    banner: &'static str,
    approved: &'static str,
    denied: &'static str,
    row: &'static str,
}

fn info(k: Kind) -> Info {
    match k {
        Kind::ForcePush => Info {
            icon: Icon::Branch,
            color: c::DANGER,
            title: "Force push",
            source: "Agent on laptop",
            source_icon: Icon::Laptop,
            tags: ["GitHub", "octo/app"],
            advice: (false, Icon::Alert, "Rewrites history on main", "Always asks. Never automatic."),
            banner: "Force push to main",
            approved: "Pushed",
            denied: "Nothing was pushed",
            row: "Force push",
        },
        Kind::Push => Info {
            icon: Icon::Pencil,
            color: c::SEND,
            title: "Push with git",
            source: "Agent on laptop",
            source_icon: Icon::Laptop,
            tags: ["GitHub", "octo-cat"],
            advice: (true, Icon::Sparkles, "Autopilot would approve · 97%", "Like 4 pushes you approved"),
            banner: "Push with git",
            approved: "Pushed to octo/app",
            denied: "Nothing was pushed",
            row: "Push with git",
        },
        Kind::RmRf => Info {
            icon: Icon::Terminal,
            color: c::DANGER,
            title: "Run a command",
            source: "Agent on laptop",
            source_icon: Icon::Laptop,
            tags: ["Shell", "~/octo/app"],
            advice: (false, Icon::Alert, "Deletes files. Always asks.", "Destructive commands wait for you"),
            banner: "Run rm -rf /",
            approved: "Command ran",
            denied: "Nothing ran",
            row: "rm -rf /",
        },
        Kind::Email => Info {
            icon: Icon::Send,
            color: c::SEND,
            title: "Send email",
            source: "Agent in the cloud",
            source_icon: Icon::Bot,
            tags: ["Gmail", "you@acme.com"],
            advice: (false, Icon::Alert, "1,204 new recipients", "Risky sends always ask"),
            banner: "Send email",
            approved: "Email sent",
            denied: "Nothing was sent",
            row: "Send email",
        },
        Kind::Merge => Info {
            icon: Icon::Merge,
            color: c::ACCENT,
            title: "Merge pull request",
            source: "Agent on laptop",
            source_icon: Icon::Laptop,
            tags: ["GitHub", "octo/app"],
            advice: (true, Icon::Check, "Checks passed · 2 approvals", "Merged with your token, on the phone"),
            banner: "Merge pull request #42",
            approved: "Merged #42",
            denied: "Not merged",
            row: "Merge pull request",
        },
        Kind::PayAgain => Info {
            banner: "Pay $512.40 · cart changed",
            title: "Pay $512.40",
            ..info(Kind::Pay)
        },
        Kind::Pay => Info {
            icon: Icon::Card,
            color: c::SUCCESS,
            title: "Pay $47.18",
            source: "Shopping agent",
            source_icon: Icon::Bot,
            tags: ["Reins Pay", "Lumen Supply"],
            advice: (true, Icon::Lock, "One-time card for this cart", "Your card number stays here"),
            banner: "Pay $47.18 at Lumen Supply",
            approved: "Paid $47.18",
            denied: "Nothing was paid",
            row: "Pay $47.18",
        },
    }
}

pub struct Phone<'r> {
    /// Centre of the phone.
    pub x: f32,
    pub y: f32,
    /// Height on screen in design units.
    pub height: f32,
    pub reqs: &'r [Req],
    /// The Keys screen slides in at this second.
    pub keys_at: Option<f32>,
    /// Lights the screen up (0..1); the phone is dark before.
    pub wake: f32,
}

pub fn phone(frame: &mut Frame, ctx: &FFramesContext, t: f32, o: &Phone) -> Svgr<'static> {
    let s = o.height / PH;
    // Buzz: a short rattle and haptic rings for each request.
    let mut rot = 0.0;
    let mut dx = 0.0;
    let mut rings = vec![];
    for r in o.reqs {
        let dt = t - r.buzz;
        if (0.0..0.5).contains(&dt) {
            let k = (1.0 - dt / 0.5).powf(1.5);
            rot += (dt * 75.0).sin() * 2.2 * k;
            dx += (dt * 90.0).sin() * 5.0 * k;
        }
        for i in 0..2 {
            let p = (dt - i as f32 * 0.14) / 0.7;
            if (0.0..1.0).contains(&p) {
                let g = 14.0 + p * 70.0;
                rings.push(stroke_rect(
                    -g,
                    -g,
                    PW + g * 2.0,
                    PH + g * 2.0,
                    64.0 + g,
                    c::LILAC,
                    3.0 / s.max(0.3),
                    (1.0 - p) * 0.5,
                ));
            }
        }
    }

    let keys = o.keys_at.map(|k| sp(t, k, M::Snappy)).unwrap_or(0.0);
    let screen = vec![
        group(tr(-keys * 150.0, 0.0), 1.0 - clamp01(keys), vec![activity(t, o.reqs)]),
        if keys > 0.001 {
            group(tr((1.0 - keys) * 370.0, 0.0), 1.0, vec![keys_screen(t, o.keys_at.unwrap_or(0.0))])
        } else {
            Svgr::empty()
        },
    ];
    let mut sheets = vec![];
    let mut scrim: f32 = 0.0;
    for r in o.reqs {
        let up = sp(t, r.up, M::Heavy) - ease(t, r.gone(), 0.35, E::In);
        if up > 0.001 {
            scrim = scrim.max(clamp01(up));
            sheets.push(sheet(frame, ctx, t, r, up));
        }
    }
    let island = island(t, o.reqs);
    let wake = clamp01(o.wake);
    let transform = format!(
        "translate({:.2} {:.2}) rotate({rot:.3}) scale({s:.4}) translate({} {})",
        o.x + dx,
        o.y,
        -PW / 2.0,
        -PH / 2.0
    );
    fframes::svgr!(<g transform={transform}>
        <defs>
            <linearGradient id="phone-frame" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" stop-color="#4A4A52" />
                <stop offset="0.4" stop-color="#18181B" />
                <stop offset="1" stop-color="#2A2A30" />
            </linearGradient>
            <clipPath id="phone-screen">
                <rect x="10" y="10" width="370" height="824" rx="55" />
            </clipPath>
            <radialGradient id="phone-shadow">
                <stop offset="0" stop-color="#000" stop-opacity="0.7" />
                <stop offset="1" stop-color="#000" stop-opacity="0" />
            </radialGradient>
        </defs>
        {rings}
        <ellipse cx="195" cy="470" rx="330" ry="560" fill="url(#phone-shadow)" />
        <rect x="-3" y="150" width="4" height="60" rx="2" fill="#2A2A30" />
        <rect x="389" y="190" width="4" height="90" rx="2" fill="#2A2A30" />
        <rect width="390" height="844" rx="64" fill="url(#phone-frame)" />
        <rect x="0.75" y="0.75" width="388.5" height="842.5" rx="63.5" fill="none" stroke="#FFFFFF" stroke-opacity="0.16" stroke-width="1.5" />
        <rect x="10" y="10" width="370" height="824" rx="55" fill="#000" />
        <g clip-path="url(#phone-screen)" opacity={wake}>
            {screen}
            {rect_o(10.0, 10.0, 370.0, 824.0, 0.0, "#000000", scrim * 0.55)}
            {sheets}
            {status_bar()}
            {taps(t, o.reqs)}
        </g>
        {island}
    </g>)
}

fn status_bar() -> Svgr<'static> {
    fframes::svgr!(<g>
        <text x="50" y="45" font-family={SANS} font-size="16" font-weight="600" fill={c::TEXT}>"9:41"</text>
        <rect x="298" y="36" width="4" height="7" rx="1.5" fill={c::TEXT} />
        <rect x="304" y="33" width="4" height="10" rx="1.5" fill={c::TEXT} />
        <rect x="310" y="30" width="4" height="13" rx="1.5" fill={c::TEXT} />
        <rect x="324" y="31" width="26" height="12" rx="4" fill="none" stroke={c::TEXT} stroke-width="1.5" />
        <rect x="327" y="34" width="17" height="6" rx="1.5" fill={c::TEXT} />
    </g>)
}

// ---------------------------------------------------------------- island and banners

fn island(t: f32, reqs: &[Req]) -> Svgr<'static> {
    // (from, until, icon, colour, title, subtitle)
    let mut banners: Vec<(f32, f32, Icon, &str, &str, &str)> = vec![];
    for r in reqs {
        let i = info(r.kind);
        banners.push((r.buzz, r.up + 0.05, i.icon, i.color, i.banner, "Waiting for you"));
        let (ic, col, sub) = if r.approve {
            (Icon::Check, c::SUCCESS, i.approved)
        } else {
            (Icon::X, c::DANGER, i.denied)
        };
        banners.push((
            r.done() + 0.4,
            r.done() + 1.7,
            ic,
            col,
            if r.approve {
                "Approved"
            } else {
                "Denied"
            },
            sub,
        ));
    }
    let mut grow: f32 = 0.0;
    let mut content = vec![];
    for &(from, until, ic, col, title, sub) in &banners {
        let k = (sp(t, from, M::Snappy) - sp(t, until, M::Snappy)).max(0.0);
        grow = grow.max(k);
        let v = clamp01((t - from - 0.08) / 0.2) * (1.0 - clamp01((t - until + 0.12) / 0.15));
        if v > 0.01 && t >= from {
            content.push(group(
                String::new(),
                v,
                vec![
                    rect_o(37.0, 25.0, 46.0, 46.0, 23.0, col, 0.18),
                    icon(ic, 60.0, 48.0, 24.0, col, 2.4),
                    text(95.0, 44.0, title, SANS, 16.5, 600, c::TEXT, "start"),
                    text(95.0, 63.0, sub, SANS, 13.5, 400, c::SECONDARY, "start"),
                    crate::ui::mark(334.0, 48.0, 32.0, 1.0, 1.0),
                ],
            ));
        }
    }
    let w = 118.0 + grow * (356.0 - 118.0);
    let h = 34.0 + grow * (80.0 - 34.0);
    fframes::svgr!(<g>
        <defs>
            <clipPath id="island-clip">
                <rect x={195.0 - w / 2.0} y="21" width={w} height={h} rx={h.min(40.0)} />
            </clipPath>
        </defs>
        <rect x={195.0 - w / 2.0} y="21" width={w} height={h} rx={h.min(40.0)} fill="#000" />
        <g clip-path="url(#island-clip)">{content}</g>
    </g>)
}

// ---------------------------------------------------------------- base screens

fn activity(t: f32, reqs: &[Req]) -> Svgr<'static> {
    let mut rows: Vec<(Icon, &str, &str, &str, bool, f32)> = vec![];
    for r in reqs.iter().rev() {
        let i = info(r.kind);
        rows.push((i.icon, i.color, i.row, "Just now", r.approve, r.gone()));
    }
    rows.extend([
        (Icon::Mail, "#2DD4BF", "Read email", "2 min ago", true, -1.0),
        (Icon::Merge, c::ACCENT, "Merge pull request", "9 min ago", true, -1.0),
        (Icon::Terminal, c::DANGER, "terraform destroy", "1 h ago", false, -1.0),
        (Icon::Send, c::SEND, "Send email", "3 h ago", true, -1.0),
        (Icon::Key, c::WARNING, "Use GitHub token", "5 h ago", true, -1.0),
    ]);
    let mut y = 160.0;
    let mut items = vec![];
    for (ic, col, title, sub, ok, at) in rows {
        let k = if at < 0.0 {
            1.0
        } else {
            sp(t, at, M::Pop)
        };
        if k <= 0.001 {
            continue;
        }
        let h = 70.0 * clamp01(k);
        let verdict = if ok {
            "Approved"
        } else {
            "Denied"
        };
        items.push(group(
            trs(195.0, y + 31.0, lerp(0.85, 1.0, clamp01(k))),
            clamp01(k * 1.5),
            vec![
                rect(-173.0, -31.0, 346.0, 62.0, 18.0, c::ELEVATED),
                icon_tile(ic, -141.0, 0.0, 38.0, col),
                text(-110.0, -3.0, title, SANS, 15.5, 600, c::TEXT, "start"),
                text(-110.0, 15.0, sub, SANS, 12.5, 400, c::TERTIARY, "start"),
                text(
                    160.0,
                    5.0,
                    verdict,
                    SANS,
                    12.5,
                    600,
                    if ok {
                        c::SUCCESS
                    } else {
                        c::DANGER
                    },
                    "end",
                ),
            ],
        ));
        y += h;
    }
    fframes::svgr!(<g>
        <rect x="10" y="10" width="370" height="824" fill="#000" />
        <text x="32" y="112" font-family={SANS} font-size="34" font-weight="700" letter-spacing="-1" fill={c::TEXT}>"Activity"</text>
        <rect x="248" y="86" width="112" height="34" rx="17" fill={c::SUCCESS} fill-opacity="0.13" />
        {icon(Icon::Sparkles, 270.0, 103.0, 15.0, c::SUCCESS, 2.0)}
        <text x="284" y="108" font-family={SANS} font-size="14" font-weight="600" fill={c::SUCCESS}>"Assisted"</text>
        <text x="32" y="148" font-family={SANS} font-size="12.5" font-weight="500" letter-spacing="1.2" fill={c::TERTIARY}>"RECENT"</text>
        {items}
        {nav_bar()}
    </g>)
}

fn nav_bar() -> Svgr<'static> {
    fframes::svgr!(<g>
        <rect x="70" y="752" width="250" height="60" rx="30" fill="#1C1C1F" />
        <rect x="70" y="752" width="250" height="60" rx="30" fill="none" stroke="#FFFFFF" stroke-opacity="0.07" />
        <rect x="82" y="759" width="72" height="46" rx="23" fill="#FFFFFF" fill-opacity="0.08" />
        {icon(Icon::Shield, 118.0, 782.0, 22.0, c::TEXT, 2.0)}
        {icon(Icon::Key, 195.0, 782.0, 22.0, c::TERTIARY, 2.0)}
        {icon(Icon::Sparkles, 272.0, 782.0, 22.0, c::TERTIARY, 2.0)}
    </g>)
}

fn keys_screen(t: f32, at: f32) -> Svgr<'static> {
    let keys = [
        ("GitHub token", "git push, pull requests", c::ACCENT),
        ("Gmail", "2 accounts", c::SEND),
        ("SSH key", "ed25519 · prod servers", c::SUCCESS),
        ("Visa •••• 4242", "Reins Pay", c::SEARCH),
        ("OpenAI API key", "released per run", c::PAIR),
    ];
    let mut rows = vec![];
    for (i, (name, sub, col)) in keys.iter().enumerate() {
        let p = sp(t, at + 0.3 + i as f32 * 0.09, M::Pop);
        let locked = sp(t, at + 1.0 + i as f32 * 0.07, M::Bouncy);
        let y = 190.0 + i as f32 * 76.0;
        rows.push(group(
            format!("translate(0 {:.2})", (1.0 - p) * 40.0),
            clamp01(p * 1.5),
            vec![
                rect(22.0, y, 346.0, 66.0, 18.0, c::ELEVATED),
                icon_tile(Icon::Key, 54.0, y + 33.0, 40.0, col),
                text(86.0, y + 30.0, *name, SANS, 15.5, 600, c::TEXT, "start"),
                text(86.0, y + 49.0, *sub, SANS, 12.5, 400, c::TERTIARY, "start"),
                group(
                    trs(340.0, y + 33.0, clamp01(locked) * lerp(0.4, 1.0, clamp01(locked))),
                    clamp01(locked * 2.0),
                    vec![icon(Icon::Lock, 0.0, 0.0, 20.0, c::WARNING, 2.2)],
                ),
            ],
        ));
    }
    fframes::svgr!(<g>
        <rect x="10" y="10" width="370" height="824" fill="#000" />
        <text x="32" y="112" font-family={SANS} font-size="34" font-weight="700" letter-spacing="-1" fill={c::TEXT}>"Keys"</text>
        <text x="32" y="146" font-family={SANS} font-size="14.5" font-weight="400" fill={c::SECONDARY}>"Stored on this phone. Never sent to an AI."</text>
        {rows}
        {nav_bar()}
    </g>)
}

// ---------------------------------------------------------------- the approval sheet

fn sheet(frame: &mut Frame, ctx: &FFramesContext, t: f32, r: &Req, up: f32) -> Svgr<'static> {
    let i = info(r.kind);
    let y0 = SHEET_TOP + (1.0 - up) * 760.0;
    let left = (40.0 - (t - r.buzz).max(0.0)).floor().max(1.0);
    let waiting = format!("{} is waiting · {left:.0} s left", i.source);
    let (good, aic, atitle, asub) = i.advice;
    let acol = if good {
        c::SUCCESS
    } else {
        c::DANGER
    };
    let tag0 = measure(frame, ctx, SANS, 600, 14.5, i.tags[0]) + 24.0;
    let tag1 = measure(frame, ctx, MONO, 400, 14.0, i.tags[1]) + 24.0;
    fframes::svgr!(<g transform={tr(0.0, y0)}>
        <rect x="10" y="0" width="370" height="900" rx="34" fill={c::ELEVATED} />
        <rect x="10" y="0" width="370" height="900" rx="34" fill="none" stroke="#FFFFFF" stroke-opacity="0.08" />
        <rect x="176" y="10" width="38" height="5" rx="2.5" fill="#3A3A40" />
        <circle cx="51" cy="52" r="21" fill={c::CONTROL} />
        {icon(i.source_icon, 51.0, 52.0, 21.0, c::LILAC, 2.0)}
        {text(84.0, 48.0, i.source, SANS, 16.0, 600, c::TEXT, "start")}
        {text(84.0, 67.0, "Just now", SANS, 13.0, 400, c::TERTIARY, "start")}
        <circle cx="344" cy="52" r="18" fill={c::CONTROL} />
        {icon(Icon::X, 344.0, 52.0, 15.0, c::SECONDARY, 2.2)}
        {icon_tile(i.icon, 57.0, 116.0, 54.0, i.color)}
        {text_ls(98.0, 126.0, i.title, SANS, 28.0, 700, c::TEXT, "start", -0.7)}
        {rect_o(30.0, 156.0, tag0, 30.0, 10.0, c::ACCENT, 0.14)}
        {text(30.0 + tag0 / 2.0, 176.0, i.tags[0], SANS, 14.5, 600, c::ACCENT, "middle")}
        {rect(38.0 + tag0, 156.0, tag1, 30.0, 10.0, c::CONTROL)}
        {text(38.0 + tag0 + tag1 / 2.0, 176.0, i.tags[1], MONO, 14.0, 400, c::TEXT, "middle")}
        {text(30.0, 214.0, waiting, SANS, 14.0, 400, c::SECONDARY, "start")}
        {rect_o(26.0, 230.0, 338.0, 62.0, 18.0, acol, 0.11)}
        {stroke_rect(26.0, 230.0, 338.0, 62.0, 18.0, acol, 1.2, 0.3)}
        {icon(aic, 52.0, 261.0, 20.0, acol, 2.2)}
        {text(76.0, 256.0, atitle, SANS, 14.5, 600, acol, "start")}
        {text(76.0, 275.0, asub, SANS, 12.5, 400, c::SECONDARY, "start")}
        {preview(t, r)}
        {buttons(t, r)}
    </g>)
}

/// What exactly will happen, per kind. Drawn from y = 306 (sheet coordinates) down to ~640.
fn preview(t: f32, r: &Req) -> Svgr<'static> {
    match r.kind {
        Kind::Push | Kind::ForcePush => {
            let force = r.kind == Kind::ForcePush;
            let commits: [(&str, &str); 4] = if force {
                [
                    ("9f3c2a1", "Passkey login"),
                    ("4be81d0", "Session refresh"),
                    ("c07a9e3", "Billing page"),
                    ("17d0f5b", "Fix flaky tests"),
                ]
            } else {
                [
                    ("9f3c2a1", "Log in with a passkey"),
                    ("4be81d0", "Remove the old login form"),
                    ("c07a9e3", "Keep the session for 30 days"),
                    ("17d0f5b", "Document the login flow"),
                ]
            };
            let mut rows = vec![];
            for (n, (h, m)) in commits.iter().enumerate() {
                let y = 418.0 + n as f32 * 46.0;
                let col = if force {
                    c::DANGER
                } else {
                    c::TEXT
                };
                rows.push(text(
                    44.0,
                    y,
                    *h,
                    MONO,
                    13.5,
                    400,
                    if force {
                        c::DANGER
                    } else {
                        c::TERTIARY
                    },
                    "start",
                ));
                rows.push(text(116.0, y, *m, SANS, 14.5, 500, col, "start"));
                rows.push(text(
                    116.0,
                    y + 18.0,
                    if n % 2 == 0 {
                        "Ada Lovelace"
                    } else {
                        "Grace Hopper"
                    },
                    SANS,
                    12.0,
                    400,
                    c::TERTIARY,
                    "start",
                ));
                if force {
                    let w = 14.5 * 0.47 * m.chars().count() as f32;
                    rows.push(rect_o(116.0, y - 5.0, w, 1.6, 0.0, c::DANGER, 0.9));
                }
            }
            let branch = if force {
                "main"
            } else {
                "feature/passkeys"
            };
            let chip = if force {
                "Overwrite"
            } else {
                "Update"
            };
            let chip_col = if force {
                c::DANGER
            } else {
                c::SECONDARY
            };
            let note = if force {
                "8 commits on main will be lost"
            } else {
                "5 commits · +412 −96"
            };
            let bw = 9.9 * branch.chars().count() as f32 + 4.0;
            fframes::svgr!(<g>
                {text(30.0, 322.0, "octo/app", MONO, 16.0, 600, c::TEXT, "start")}
                {text(360.0, 322.0, "47.1 KB", SANS, 13.0, 400, c::TERTIARY, "end")}
                {rect(26.0, 336.0, 338.0, 290.0, 20.0, c::RAISED)}
                {text(44.0, 368.0, branch, MONO, 16.5, 600, c::TEXT, "start")}
                {rect_o(56.0 + bw, 351.0, 86.0, 26.0, 13.0, chip_col, 0.16)}
                {text(99.0 + bw, 369.0, chip, SANS, 13.0, 600, chip_col, "middle")}
                {text(44.0, 392.0, note, SANS, 13.0, 500, if force { c::DANGER } else { c::TERTIARY }, "start")}
                {rows}
            </g>)
        }
        Kind::RmRf => {
            let glow = 0.5 + 0.5 * (t * 5.0).sin();
            fframes::svgr!(<g>
                {text(30.0, 322.0, "Command", SANS, 13.0, 500, c::TERTIARY, "start")}
                {rect(26.0, 336.0, 338.0, 120.0, 20.0, "#0C0C0E")}
                {stroke_rect(26.0, 336.0, 338.0, 120.0, 20.0, c::DANGER, 1.5, 0.3 + glow * 0.4)}
                {text(48.0, 410.0, "$ rm -rf /", MONO, 34.0, 600, c::DANGER, "start")}
                {rect(26.0, 470.0, 338.0, 150.0, 20.0, c::RAISED)}
                {icon(Icon::Trash, 52.0, 503.0, 18.0, c::DANGER, 2.2)}
                {text(74.0, 509.0, "Deletes everything it can reach", SANS, 14.5, 500, c::TEXT, "start")}
                {icon(Icon::Alert, 52.0, 547.0, 18.0, c::WARNING, 2.2)}
                {text(74.0, 553.0, "Can't be undone", SANS, 14.5, 500, c::TEXT, "start")}
                {icon(Icon::Laptop, 52.0, 591.0, 18.0, c::SECONDARY, 2.0)}
                {text(74.0, 597.0, "In ~/octo/app on your laptop", SANS, 14.5, 500, c::TEXT, "start")}
            </g>)
        }
        Kind::Email => fframes::svgr!(<g>
            {rect(26.0, 310.0, 338.0, 316.0, 20.0, c::RAISED)}
            {text(44.0, 342.0, "To", SANS, 13.0, 500, c::TERTIARY, "start")}
            {text(104.0, 342.0, "all-staff@acme.com", SANS, 14.5, 600, c::DANGER, "start")}
            {text(104.0, 362.0, "1,204 people", SANS, 12.5, 500, c::DANGER, "start")}
            {rect(44.0, 378.0, 302.0, 1.0, 0.0, c::HAIRLINE)}
            {text(44.0, 404.0, "Subject", SANS, 13.0, 500, c::TERTIARY, "start")}
            {text(104.0, 404.0, "Q3 numbers (draft)", SANS, 14.5, 600, c::TEXT, "start")}
            {rect(44.0, 420.0, 302.0, 1.0, 0.0, c::HAIRLINE)}
            {text(44.0, 450.0, "Hi all, attached are the", SANS, 14.5, 400, c::SECONDARY, "start")}
            {text(44.0, 472.0, "salary numbers for next year.", SANS, 14.5, 400, c::SECONDARY, "start")}
            {text(44.0, 494.0, "Please keep this internal.", SANS, 14.5, 400, c::SECONDARY, "start")}
            {rect(44.0, 520.0, 302.0, 56.0, 14.0, c::CONTROL)}
            {icon(Icon::Card, 72.0, 548.0, 20.0, c::SEND, 2.0)}
            {text(94.0, 553.0, "payroll-2026.xlsx", MONO, 14.0, 500, c::TEXT, "start")}
        </g>),
        Kind::Merge => fframes::svgr!(<g>
            {text(30.0, 322.0, "octo/app", MONO, 16.0, 600, c::TEXT, "start")}
            {rect(26.0, 336.0, 338.0, 290.0, 20.0, c::RAISED)}
            {text(44.0, 372.0, "#42  Add passkey login", SANS, 16.5, 600, c::TEXT, "start")}
            {text(44.0, 396.0, "feature/passkeys → main", MONO, 13.0, 400, c::TERTIARY, "start")}
            {text(44.0, 432.0, "+412", MONO, 16.0, 600, c::SUCCESS, "start")}
            {text(98.0, 432.0, "−96", MONO, 16.0, 600, c::DANGER, "start")}
            {text(146.0, 432.0, "12 files", SANS, 13.5, 500, c::TERTIARY, "start")}
            {icon(Icon::Check, 52.0, 470.0, 16.0, c::SUCCESS, 2.6)}
            {text(70.0, 476.0, "CI passed", SANS, 14.0, 500, c::TEXT, "start")}
            {icon(Icon::Check, 52.0, 502.0, 16.0, c::SUCCESS, 2.6)}
            {text(70.0, 508.0, "Reviewed by Grace Hopper", SANS, 14.0, 500, c::TEXT, "start")}
            {icon(Icon::Key, 52.0, 534.0, 16.0, c::WARNING, 2.2)}
            {text(70.0, 540.0, "Uses your GitHub token, on this phone", SANS, 14.0, 500, c::TEXT, "start")}
        </g>),
        Kind::Pay | Kind::PayAgain => pay_preview(),
    }
}

fn pay_preview() -> Svgr<'static> {
    let items = [("Single-origin beans, 1 kg", "×2", "$34.00"), ("USB-C cable, 2 m", "×1", "$9.99")];
    let mut rows = vec![];
    for (n, (name, qty, price)) in items.iter().enumerate() {
        let y = 342.0 + n as f32 * 34.0;
        rows.push(text(44.0, y, *name, SANS, 14.5, 500, c::TEXT, "start"));
        rows.push(text(286.0, y, *qty, SANS, 14.0, 500, c::TERTIARY, "end"));
        rows.push(text(346.0, y, *price, MONO, 14.0, 500, c::TEXT, "end"));
    }
    fframes::svgr!(<g>
        {rect(26.0, 310.0, 338.0, 316.0, 20.0, c::RAISED)}
        {rows}
        {rect(44.0, 398.0, 302.0, 1.0, 0.0, c::HAIRLINE)}
        {text(44.0, 424.0, "Shipping and tax", SANS, 13.5, 400, c::SECONDARY, "start")}
        {text(346.0, 424.0, "$3.19", MONO, 13.5, 400, c::SECONDARY, "end")}
        {text(44.0, 458.0, "Total", SANS, 18.0, 700, c::TEXT, "start")}
        {text(346.0, 458.0, "$47.18", MONO, 18.0, 700, c::TEXT, "end")}
        {rect(44.0, 476.0, 302.0, 1.0, 0.0, c::HAIRLINE)}
        {icon(Icon::Pin, 54.0, 506.0, 18.0, c::SECONDARY, 2.0)}
        {text(76.0, 503.0, "Ada Lovelace", SANS, 14.0, 600, c::TEXT, "start")}
        {text(76.0, 522.0, "42 Market St, San Francisco", SANS, 12.5, 400, c::TERTIARY, "start")}
        {icon(Icon::Card, 54.0, 566.0, 18.0, c::SECONDARY, 2.0)}
        {text(76.0, 563.0, "Visa •••• 4242", SANS, 14.0, 600, c::TEXT, "start")}
        {text(76.0, 582.0, "One-time number for this cart", SANS, 12.5, 400, c::TERTIARY, "start")}
    </g>)
}

fn buttons(t: f32, r: &Req) -> Svgr<'static> {
    let press = sp(t, r.tap, M::Pop) - sp(t, r.tap + 0.12, M::Snappy);
    let done = clamp01(sp(t, r.done(), M::Snappy));
    let pay = r.kind == Kind::Pay;
    let approve_label = if pay {
        "Pay"
    } else {
        "Approve"
    };
    let (deny_s, appr_s) = if r.approve {
        (1.0, 1.0 - press * 0.06)
    } else {
        (1.0 - press * 0.06, 1.0)
    };
    let deny_bg = if r.approve {
        c::CONTROL
    } else {
        c::DANGER_DEEP
    };
    let deny_done = if r.approve {
        0.0
    } else {
        done
    };
    let appr_done = if r.approve {
        done
    } else {
        0.0
    };
    let pay_icon = if pay {
        icon(Icon::FaceId, 238.0, 0.0, 20.0, "#111113", 2.2)
    } else {
        Svgr::empty()
    };
    let appr_text_x = if pay {
        290.0
    } else {
        280.0
    };
    fframes::svgr!(<g transform="translate(0 656)">
        <g transform={format!("translate(106 26) scale({deny_s:.4}) translate(-106 -26)")}>
            {rect(26.0, 0.0, 160.0, 52.0, 26.0, c::CONTROL)}
            {rect_o(26.0, 0.0, 160.0, 52.0, 26.0, deny_bg, deny_done)}
            {group(String::new(), 1.0 - deny_done, vec![text(106.0, 32.0, "Deny", SANS, 17.0, 600, c::TEXT, "middle")])}
            {group(String::new(), deny_done, vec![
                icon(Icon::X, 72.0, 26.0, 18.0, "#FFFFFF", 2.8),
                text(116.0, 32.0, "Denied", SANS, 17.0, 600, "#FFFFFF", "middle"),
            ])}
        </g>
        <g transform={format!("translate(280 26) scale({appr_s:.4}) translate(-280 -26)")}>
            {rect(196.0, 0.0, 168.0, 52.0, 26.0, c::TEXT)}
            {rect_o(196.0, 0.0, 168.0, 52.0, 26.0, "#10B981", appr_done)}
            {group(String::new(), 1.0 - appr_done, vec![
                group(tr(0.0, 26.0), 1.0, vec![pay_icon]),
                text(appr_text_x, 32.0, approve_label, SANS, 17.0, 600, "#111113", "middle"),
            ])}
            {group(String::new(), appr_done, vec![
                icon(Icon::Check, 238.0, 26.0, 18.0, "#FFFFFF", 2.8),
                text(292.0, 32.0, if pay { "Paid" } else { "Approved" }, SANS, 17.0, 600, "#FFFFFF", "middle"),
            ])}
        </g>
        {face_id(t, r)}
    </g>)
}

/// Face ID for payments: a dim screen, the glyph drawing its corners, then a check.
fn face_id(t: f32, r: &Req) -> Svgr<'static> {
    if r.kind != Kind::Pay || !r.approve {
        return Svgr::empty();
    }
    let v = life(t, r.tap + 0.08, r.done() + 0.15, M::Snappy);
    if v <= 0.001 {
        return Svgr::empty();
    }
    let scan = ease(t, r.tap + 0.15, 0.6, E::InOut);
    let ok = sp(t, r.done() - 0.2, M::Bouncy);
    let corners = format!("{:.1} 400", 120.0 * scan);
    fframes::svgr!(<g transform={format!("translate(195 -290) scale({:.4})", lerp(1.0, 1.35, v))} opacity={v}>
        <rect x="-80" y="-80" width="160" height="160" rx="36" fill="#1C1C1F" />
        <rect x="-80" y="-80" width="160" height="160" rx="36" fill="none" stroke="#FFFFFF" stroke-opacity="0.1" />
        <g transform="translate(-36 -36) scale(3)" fill="none" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 7V5a2 2 0 0 1 2-2h2 M17 3h2a2 2 0 0 1 2 2v2 M21 17v2a2 2 0 0 1-2 2h-2 M7 21H5a2 2 0 0 1-2-2v-2" stroke={c::LILAC} stroke-width="1.6" stroke-dasharray={corners} opacity={1.0 - clamp01(ok)} />
            <path d="M8 14s1.5 2 4 2 4-2 4-2 M9 9h.01 M15 9h.01" stroke={c::LILAC} stroke-width="1.6" opacity={scan * (1.0 - clamp01(ok))} />
            <path d="M6 12.5l4 4 8-9" stroke={c::SUCCESS} stroke-width="2" stroke-dasharray={format!("{:.1} 40", 18.0 * clamp01(ok))} />
        </g>
    </g>)
}

/// The finger: a soft dot that lands on a button and ripples when it presses.
fn taps(t: f32, reqs: &[Req]) -> Svgr<'static> {
    let mut out = vec![];
    for r in reqs {
        let v = life(t, r.tap - 0.35, r.tap + 0.3, M::Snappy);
        if v <= 0.01 {
            continue;
        }
        let (x, y) = if r.approve {
            (280.0, SHEET_TOP + 682.0)
        } else {
            (106.0, SHEET_TOP + 682.0)
        };
        let press = sp(t, r.tap, M::Pop) - sp(t, r.tap + 0.12, M::Snappy);
        let ring = clamp01(sp(t, r.tap, M::Soft));
        out.push(circle(x, y, 30.0 * v * (1.0 - 0.25 * press), "#FFFFFF", 0.35 * v));
        out.push(fframes::svgr!(<circle cx={x} cy={y} r={30.0 * v * (1.0 - 0.25 * press)} fill="none" stroke="#FFFFFF" stroke-opacity="0.6" stroke-width="2" />));
        if ring > 0.001 && ring < 0.999 {
            out.push(fframes::svgr!(<circle cx={x} cy={y} r={30.0 * (1.0 + ring * 2.2)} fill="none" stroke="#FFFFFF" stroke-width="2" opacity={(1.0 - ring) * 0.7} />));
        }
    }
    fframes::svgr!(<g>{out}</g>)
}
