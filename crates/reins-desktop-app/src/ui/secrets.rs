//! Keys & secrets: finding the secrets lying around this computer and moving them into the vault on the phone
//! (`docs/secrets.md`, 5; the approved design `desktop-secrets.html`). The hero on the page starts a scan; then the
//! scan's progress, the review (grouped findings with what makes them risky, pre-ticked by confidence, a file's
//! detail with masked values), the phone's approval with the batch's check digits, and what changed. Values are never
//! here: only names, places and masked previews.

use std::fmt::Write as _;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
};
use reins_desktop::secrets_scan::{Confidence, Detector, Finding, Group, Risk, Source, Stage, StageState};

use super::field::Field;
use super::hold::Held;
use super::icons::{Brand, Icon, Mark, icon};
use super::parts::{
    Btn, big, btn, btn_icon, caption, card, check_circle, chip, chip_plain, empty, fine, help, help_with, link, logo,
    one_line, page_header, ring, row, row_text, sentence, small, stage, tile, well,
};
use super::story::Words;
use super::{Data, Root};
use crate::model::Model;
use crate::secrets::{self, Finished, Flow, Moving, PhoneKey, Review, Scanning};
use crate::theme::{MONO, Palette, SERIF};

fn words(parts: &[(&str, bool)]) -> Vec<Words> {
    parts
        .iter()
        .map(|(t, b)| Words {
            text: (*t).to_owned(),
            bold: *b,
        })
        .collect()
}

/// The colour a kind of source is shown in.
fn source_color(source: Source, pal: Palette) -> Hsla {
    match source {
        Source::Dotenv | Source::Cargo => pal.orange(),
        Source::SshKey | Source::GitCredentials => pal.teal(),
        Source::Aws => gpui::rgb(0x00FF_9900).into(),
        Source::Npm => gpui::rgb(0x00CB_3837).into(),
        Source::Kube | Source::Docker | Source::Gcp | Source::PyPi => pal.blue(),
        Source::GhCli | Source::Shell | Source::Netrc => pal.secondary,
    }
}

/// A source's square: its logo or icon in its colour.
fn source_tile(source: Source, pal: Palette, side: f32) -> Div {
    let i = match source {
        Source::GhCli => return logo(Mark::Brand(Brand::GitHub), pal).size(px(side)),
        Source::Dotenv => Icon::FileLock,
        Source::SshKey => Icon::Terminal,
        Source::GitCredentials => Icon::GitBranch,
        Source::Npm | Source::PyPi | Source::Cargo => Icon::Package,
        Source::Docker | Source::Aws | Source::Gcp | Source::Kube => Icon::Cloud,
        Source::Shell => Icon::SquareTerminal,
        Source::Netrc => Icon::Globe,
    };
    tile(i, source_color(source, pal), side)
}

/// What makes a finding risky, as a small chip: red for a danger, yellow for plain text, grey for the keychain.
fn risk_chip(risk: Risk, pal: Palette) -> Div {
    let (i, color) = match risk {
        Risk::ReadableByAgents | Risk::WorldReadable => (Icon::Eye, pal.danger),
        Risk::InGitHistory => (Icon::GitCommit, pal.danger),
        Risk::NoPassphrase => (Icon::LockOpen, pal.danger),
        Risk::PlainText => (Icon::FileText, pal.warning),
        Risk::InKeychain => return chip_plain(Some(Icon::Lock), risk.label(), pal).h(px(22.0)),
    };
    chip(Some(i), risk.label(), color).h(px(22.0))
}

/// A tick box.
fn tick(on: bool, pal: Palette) -> Div {
    div().flex().flex_none().items_center().justify_center().size(px(18.0)).rounded(px(5.0)).when_else(
        on,
        |b| b.bg(pal.violet_fill()).child(icon(Icon::Check, 12.0, pal.on_accent)),
        |b| b.border(px(1.5)).border_color(pal.tile_border().blend(pal.tertiary.opacity(0.4))),
    )
}

/// A radio choice's circle.
fn radio(on: bool, pal: Palette) -> Div {
    div().flex_none().mt(px(1.0)).size(px(18.0)).rounded_full().when_else(
        on,
        |r| r.border(px(5.0)).border_color(pal.violet_fill()),
        |r| r.border(px(1.5)).border_color(pal.tertiary),
    )
}

fn mono(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().font_family(MONO).text_size(px(11.5)).text_color(pal.tertiary).min_w(px(0.0)).truncate().child(text.into())
}

/// A small capitals heading with a count on its right.
fn heading(title: &str, right: Option<String>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(4.0))
        .pt(px(6.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.tertiary)
        .child(div().flex_1().child(title.to_uppercase()))
        .when_some(right, |h, r| h.child(div().font_weight(FontWeight::NORMAL).child(r)))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    crate::format::count(n as u64, one, many)
}

/// The detector's mark beside a value: a brand where there is one.
fn detector_mark(d: Option<Detector>, pal: Palette) -> Option<AnyElement> {
    let i = match d? {
        Detector::GithubPat | Detector::GithubFineGrained | Detector::GithubApp | Detector::GithubOauth => {
            return Some(super::icons::avatar(Brand::GitHub, 18.0, pal).into_any_element());
        }
        Detector::Postgres | Detector::MySql | Detector::MongoDb | Detector::Redis => Icon::Database,
        Detector::AwsAccessKeyId | Detector::AwsSecretKey | Detector::GoogleApiKey => Icon::Cloud,
        Detector::StripeLive | Detector::StripeTest => Icon::CreditCard,
        Detector::SupabaseService | Detector::Jwt => Icon::Ticket,
        _ => Icon::KeyRound,
    };
    Some(icon(i, 13.0, pal.secondary).into_any_element())
}

impl Root {
    /// The Keys & secrets page's hero: what a scan does, and Scan.
    pub(super) fn secrets_hero(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let computer = super::parts::this_computer();
        let label = if cfg!(target_os = "macos") {
            "Scan this Mac"
        } else {
            "Scan this computer"
        };
        let top = div()
            .flex()
            .items_center()
            .gap(px(18.0))
            .child(stage(Mark::Icon(Icon::Laptop), Mark::Icon(Icon::Vault), None, pal.accent, 42.0, pal))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(16.0))
                    .line_height(px(22.0))
                    .text_color(pal.secondary)
                    .child(sentence(
                        &words(&[
                            ("Find the secrets lying around ", false),
                            (computer, true),
                            (" and keep them in ", false),
                            ("your vault", true),
                        ]),
                        pal,
                    )),
            )
            .child(
                btn_icon("secrets-scan-start", Icon::ScanSearch, label, Btn::Primary, pal)
                    .h(px(36.0))
                    .on_click(Self::on_model(cx, Model::start_scan)),
            );
        // Under the sentence, where it starts (after the two tiles and the dots).
        let chips = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .pl(px(42.0 * 2.0 + 24.0 + 12.0 * 2.0 + 18.0))
            .child(
                chip_plain(Some(Icon::ShieldCheck), "Scans here, sends nothing", pal).h(px(24.0)).text_size(px(11.5)),
            )
            .child(chip_plain(Some(Icon::Smartphone), "You approve each move", pal).h(px(24.0)).text_size(px(11.5)))
            .child(help(
                "secrets-scan",
                "Reins looks for .env files, SSH keys, the GitHub CLI's token, git credentials, cloud and registry \
                 tokens and shell exports on this computer. Only names, places and masked values are shown; nothing \
                 leaves it until you approve a move on your phone. Only you can start a scan, from this window or your \
                 own terminal.",
                d,
                pal,
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(20.0))
            .py(px(18.0))
            .rounded(px(20.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .child(top)
            .child(chips)
            .into_any_element()
    }

    /// The step of the flow on its own page, when one is under way: `(page, whether it fills the window)`.
    pub(super) fn secrets_page(
        &self,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<(AnyElement, bool)> {
        match &d.secrets {
            Flow::Idle => None,
            Flow::Scanning(s) => Some((Self::scanning(s, pal, cx), false)),
            Flow::Review(r) => match r.open.as_ref().and_then(|id| r.report.finding(id)) {
                Some(f) => Some((Self::finding_detail(r, f, d, pal, cx), false)),
                None => Some((self.review_page(r, d, pal, window, cx), true)),
            },
            Flow::Moving(m) => Some((Self::moving(m, d, pal), false)),
            Flow::Done(done) => Some((self.secrets_done(done, d, pal, cx), false)),
        }
    }

    fn scanning(s: &Scanning, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let p = &s.progress;
        let title = if cfg!(target_os = "macos") {
            "Scanning this Mac"
        } else {
            "Scanning this computer"
        };
        let head = page_header(title, None)
            .child(btn("secrets-scan-stop", "Stop", Btn::Plain, pal).on_click(Self::on_model(cx, Model::stop_scan)));
        let summary = card(pal).px(px(20.0)).py(px(18.0)).child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(ring(p.fraction.max(0.04), pal.accent, pal.tile_border(), 44.0, 4.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{} found so far", p.found)),
                        )
                        .child(mono(p.at.clone(), pal).text_size(px(12.0))),
                ),
        );
        let bar = div()
            .mt(px(14.0))
            .h(px(6.0))
            .w_full()
            .rounded(px(3.0))
            .bg(pal.tile_border())
            .overflow_hidden()
            .child(div().h_full().w(relative(p.fraction.clamp(0.0, 1.0))).bg(pal.violet_fill()));
        let mut stages = card(pal);
        for (i, stage) in Stage::ALL.iter().enumerate() {
            let state = p.stages.get(stage).copied().unwrap_or_default();
            let mark: Div = match stage {
                Stage::Github => logo(Mark::Brand(Brand::GitHub), pal),
                Stage::Projects => tile(Icon::FolderSearch, pal.secondary, 30.0).bg(pal.tile()),
                Stage::Ssh => tile(Icon::Terminal, pal.secondary, 30.0).bg(pal.tile()),
                Stage::Cloud => tile(Icon::Cloud, pal.secondary, 30.0).bg(pal.tile()),
                Stage::Registries => tile(Icon::Package, pal.secondary, 30.0).bg(pal.tile()),
                Stage::Shell => tile(Icon::SquareTerminal, pal.secondary, 30.0).bg(pal.tile()),
            };
            let right: AnyElement = match state {
                StageState::Done(n) => div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(div().text_size(px(12.5)).text_color(pal.secondary).child(format!("{n} found")))
                    .child(icon(Icon::CircleCheck, 16.0, pal.success))
                    .into_any_element(),
                StageState::Running => ring(0.35, pal.accent, pal.tile_border(), 16.0, 2.0).into_any_element(),
                StageState::Waiting => {
                    div().text_size(px(12.5)).text_color(pal.tertiary).child("waiting").into_any_element()
                }
            };
            stages = stages.child(
                row(pal, i == 0)
                    .child(mark)
                    .child(row_text(stage.label(), Some(mono(stage.places(), pal).into_any_element()), pal))
                    .child(right),
            );
        }
        let computer = super::parts::this_computer();
        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(head)
            .child(summary.child(bar))
            .child(stages)
            .child(div().flex().justify_center().child(fine(
                format!("Only names and places are shown until you open one. Nothing leaves {computer}."),
                pal,
            )))
            .into_any_element()
    }

    /// One finding's row in the review: the tick, its kind, its name with what makes it risky, its place, its size.
    fn finding_row(r: &Review, f: &Finding, first: bool, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let on = r.ticked_in(f) > 0;
        let id = f.id.clone();
        let open = f.id.clone();
        // The most telling risk first: plain text only when it is all there is.
        let first_risk = f.risks.iter().find(|r| **r != Risk::PlainText).or(f.risks.first());
        // The name on its own line; under it the place and what makes it risky.
        let title = one_line(f.title.clone()).font_weight(FontWeight::MEDIUM);
        let under = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w(px(0.0))
            .child(mono(f.place.clone(), pal).min_w(px(64.0)))
            .when_some(first_risk, |t, risk| {
                t.child(risk_chip(*risk, pal).h(px(20.0)).text_size(px(11.0)).pl(px(7.0)).pr(px(8.0)))
            });
        row(pal, first)
            .id(SharedString::from(format!("finding-{}", f.id)))
            .cursor_pointer()
            .hover(|s| s.bg(pal.control_fill.opacity(0.5)))
            .child(div().id(SharedString::from(format!("tick-{}", f.id))).p(px(4.0)).child(tick(on, pal)).on_click(
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let id = id.clone();
                    this.model.update(cx, |m, cx| m.review(cx, |r| r.toggle_finding(&id)));
                }),
            ))
            .child(source_tile(f.source, pal, 30.0))
            .child(div().flex().flex_col().flex_1().min_w(px(0.0)).gap(px(3.0)).child(title).child(under))
            .child(
                div()
                    .flex_none()
                    .max_w(px(130.0))
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(pal.secondary)
                    .child(f.summary.clone()),
            )
            .child(icon(Icon::ChevronRight, 15.0, pal.tertiary))
            .on_click(Self::on_model(cx, move |m, cx| {
                let open = open.clone();
                m.review(cx, |r| r.open = Some(open));
            }))
            .into_any_element()
    }

    fn review_page(
        &self,
        r: &Review,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let top = if cfg!(target_os = "macos") {
            40.0
        } else {
            30.0
        };
        let report = &r.report;
        let secrets = report.secrets();
        let places = report.findings.len();
        let mut list = div().flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .min_h(px(32.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(32.0))
                        .line_height(px(36.0))
                        .font_family(SERIF)
                        .font_weight(FontWeight::NORMAL)
                        .child(format!(
                            "{} in {}",
                            plural(secrets, "secret", "secrets"),
                            plural(places, "place", "places")
                        )),
                )
                .child(
                    btn_icon("secrets-rescan", Icon::RefreshCw, "Scan again", Btn::Plain, pal)
                        .on_click(Self::on_model(cx, Model::start_scan)),
                ),
        );
        let chips = secrets::risk_counts(report);
        if !chips.is_empty() {
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            for (risk, n) in chips {
                let (i, color, words) = match risk {
                    Risk::ReadableByAgents => (Icon::Eye, pal.danger, format!("{n} readable by AI agents")),
                    Risk::InGitHistory => (Icon::GitCommit, pal.danger, format!("{n} in git history")),
                    _ => (Icon::LockOpen, pal.warning, plural(n, "key without passphrase", "keys without passphrase")),
                };
                row = row.child(chip(Some(i), words, color));
            }
            list = list.child(row);
        }
        if report.findings.is_empty() {
            list = list.child(card(pal).child(empty("No secrets found on this computer.", pal)));
        }
        for group in Group::ALL {
            let found: Vec<&Finding> = r.in_group(group).collect();
            if found.is_empty() {
                continue;
            }
            let n: usize = found.iter().map(|f| f.secrets()).sum();
            let mut c = card(pal);
            for (i, f) in found.iter().enumerate() {
                c = c.child(Self::finding_row(r, f, i == 0, pal, cx));
            }
            list = list.child(heading(group.label(), Some(plural(n, "secret", "secrets")), pal)).child(c);
        }
        if report.budget.is_some() || report.skipped_total > 0 {
            let mut note = format!(
                "{} skipped (big, binary, links out of your home folder, caches).",
                plural(report.skipped_total, "place", "places")
            );
            if let Some(b) = &report.budget {
                let _ = write!(note, " The scan stopped at its {} limit at {}.", b.limit, b.at);
            }
            list = list.child(fine(note, pal).px(px(4.0)));
        }
        let left = div()
            .id("secrets-review")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_y_scroll()
            .child(div().px(px(34.0)).pt(px(top)).pb(px(34.0)).child(list));
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(left)
            .child(self.review_panel(r, d, pal, top, window, cx))
            .into_any_element()
    }

    /// The review's right panel: how many move, into which folder, what happens after, and Move.
    fn review_panel(
        &self,
        r: &Review,
        d: &Data,
        pal: Palette,
        top: f32,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let n = r.count();
        let plan = r.plan(&d.computer, &d.machine);
        let computer = d.computer.clone();
        let choice = |id: &'static str,
                      on: bool,
                      title: &'static str,
                      under: &'static str,
                      keep: bool,
                      first: bool,
                      cx: &mut Context<'_, Self>| {
            row(pal, first)
                .id(id)
                .items_start()
                .cursor_pointer()
                .child(radio(on, pal))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(div().text_size(px(12.0)).line_height(px(16.0)).text_color(pal.tertiary).child(under)),
                )
                .on_click(Self::on_model(cx, move |m, cx| m.review(cx, |r| r.keep_local = keep)))
        };
        let move_label = format!("Move {n} · approve on phone");
        let mv = big(btn_icon("secrets-move", Icon::Smartphone, move_label, Btn::Primary, pal)).w_full().h(px(42.0));
        let mv = if n == 0 || r.key.is_some() {
            mv.opacity(0.45).cursor_default()
        } else {
            mv.on_click(Self::on_model(cx, Model::move_secrets))
        };
        let small_caps = |t: &str| {
            div().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(pal.tertiary).child(t.to_uppercase())
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(290.0))
            .h_full()
            .border_l_1()
            .border_color(pal.hairline)
            .bg(pal.sidebar)
            .px(px(24.0))
            .pt(px(top))
            .pb(px(24.0))
            .child(small_caps("Move to your vault"))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(6.0))
                    .mt(px(8.0))
                    .child(
                        div()
                            .text_size(px(40.0))
                            .line_height(px(40.0))
                            .font_family(SERIF).font_weight(FontWeight::NORMAL)
                            .child(n.to_string()),
                    )
                    .child(
                        div()
                            .pb(px(4.0))
                            .text_size(px(15.0))
                            .text_color(pal.tertiary)
                            .child(format!("of {}", r.total())),
                    ),
            )
            .child(
                caption(format!("into your vault; this computer's own under \"{computer}\""), pal)
                    .text_color(pal.tertiary)
                    .mt(px(2.0)),
            )
            .child(small_caps("Then").mt(px(22.0)))
            .child(
                card(pal)
                    .mt(px(8.0))
                    .child(choice(
                        "secrets-then-reins",
                        !r.keep_local,
                        "Use them through Reins",
                        "Local copies go to the Trash, readable until you shred them. Apps get them from your vault when you approve.",
                        false,
                        true,
                        cx,
                    ))
                    .child(choice(
                        "secrets-then-keep",
                        r.keep_local,
                        "Keep local copies too",
                        "A backup in your vault only; nothing here changes.",
                        true,
                        false,
                        cx,
                    )),
            )
            .when(!plan.clashes.is_empty() || !plan.conflicts.is_empty(), |p| {
                p.child(small_caps("Names").mt(px(18.0))).child(Self::names_card(&plan, pal, cx))
            })
            .when_some(r.key.as_ref(), |p, key| {
                p.child(small_caps("Your phone's key").mt(px(18.0))).child(self.phone_key_card(key, pal, window, cx))
            })
            .when_some(r.error.clone(), |p, e| p.child(caption(e, pal).mt(px(12.0)).text_color(pal.danger)))
            .child(div().flex_1())
            .child(mv)
            .into_any_element()
    }

    /// The first time only: the twelve digits the phone shows for its key, typed here, then checked with the phone.
    fn phone_key_card(&self, key: &PhoneKey, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        let check = small(btn(
            "secrets-key-check",
            if key.checking {
                "Approve on phone"
            } else {
                "Check"
            },
            Btn::Plain,
            pal,
        ));
        let check = if key.checking || key.digits().is_none() {
            check.opacity(0.45).cursor_default()
        } else {
            check.on_click(Self::on_model(cx, Model::check_phone_key))
        };
        card(pal)
            .mt(px(8.0))
            .p(px(12.0))
            .gap(px(8.0))
            .child(caption("12 digits: Reins on your phone, Settings › Devices", pal).text_color(pal.tertiary))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(self.text_field(Field::PhoneKey, "0000-0000-0000", pal, window, cx))
                    .child(check),
            )
    }

    /// What the review says about names: a project the vault already has from another folder (a new name, or merge
    /// into it on purpose), and a variable two files give the same environment (the more specific file wins).
    fn names_card(plan: &reins_desktop::secrets_import::Plan, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mut c = card(pal).mt(px(8.0));
        let mut first = true;
        for clash in &plan.clashes {
            let name = clash.name.clone();
            let label = if clash.merged() {
                "Keep apart"
            } else {
                "Merge into it"
            };
            c = c.child(
                row(pal, first).items_start().child(icon(Icon::TriangleAlert, 14.0, pal.warning).mt(px(2.0))).child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(6.0))
                        .child(div().text_size(px(12.0)).line_height(px(16.0)).child(clash.line()))
                        .child(div().flex().child(
                            small(btn(SharedString::from(format!("merge-{name}")), label, Btn::Plain, pal)).on_click(
                                Self::on_model(cx, move |m, cx| {
                                    let name = name.clone();
                                    m.review(cx, |r| r.toggle_merge(&name));
                                }),
                            ),
                        )),
                ),
            );
            first = false;
        }
        for conflict in &plan.conflicts {
            c = c.child(
                row(pal, first).items_start().child(icon(Icon::CircleAlert, 14.0, pal.secondary).mt(px(2.0))).child(
                    div().flex_1().min_w(px(0.0)).text_size(px(12.0)).line_height(px(16.0)).child(conflict.line()),
                ),
            );
            first = false;
        }
        c
    }

    fn finding_detail(r: &Review, f: &Finding, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let back = link("secrets-back", "", pal)
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(icon(Icon::ChevronLeft, 16.0, pal.accent))
            .child(format!("{} found", r.report.secrets()))
            .on_click(Self::on_model(cx, |m, cx| m.review(cx, |r| r.open = None)));
        let mut place = f.place.clone();
        if let Some(c) = &f.git_commit {
            let _ = write!(place, " · committed in {c}");
        }
        let mut head = div().flex().items_center().gap(px(12.0)).child(source_tile(f.source, pal, 40.0)).child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(20.0))
                        .line_height(px(26.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(f.title.clone()),
                )
                .child(mono(place, pal).text_size(px(12.0))),
        );
        for risk in f.risks.iter().filter(|r| r.severe()) {
            head = head.child(risk_chip(*risk, pal));
        }
        let mut values = card(pal);
        for (i, item) in f.items.iter().enumerate() {
            let on = r.ticked.contains(&item.id);
            let id = item.id.clone();
            let kind: AnyElement = if item.confidence == Confidence::None {
                // A setting .env.reins cannot hold as it is stays where it is.
                let file = f.abs_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let words = if item.stays_in_file && !on {
                    format!("stays in {file}")
                } else {
                    "not a secret".to_owned()
                };
                div().text_color(pal.tertiary).child(words).into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_color(pal.secondary)
                    .when_some(detector_mark(item.detector, pal), gpui::ParentElement::child)
                    .child(item.label.clone())
                    .into_any_element()
            };
            let name = match &item.group {
                Some(g) => format!("{g} · {}", item.name),
                None => item.name.clone(),
            };
            values = values.child(
                row(pal, i == 0)
                    .id(SharedString::from(format!("item-{}", item.id)))
                    .min_h(px(44.0))
                    .cursor_pointer()
                    .child(tick(on, pal))
                    .child(div().flex_none().w(px(200.0)).font_family(MONO).text_size(px(12.5)).truncate().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .text_color(pal.tertiary)
                            .truncate()
                            .child(item.preview.clone()),
                    )
                    .child(div().flex_none().text_size(px(12.0)).child(kind))
                    .on_click(Self::on_model(cx, move |m, cx| {
                        let id = id.clone();
                        m.review(cx, |r| r.toggle_item(&id));
                    })),
            );
        }
        let explain = secrets::after_move(f.source);
        let mut after = card(pal).flex_row().items_center().gap(px(16.0)).px(px(16.0)).py(px(14.0)).child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(13.0))
                .line_height(px(19.0))
                .text_color(pal.secondary)
                .child(explain),
        );
        if f.source == Source::Dotenv {
            let mut w = well(pal).w(px(300.0)).flex_none().text_size(px(11.5)).line_height(px(18.0));
            let project = secrets::project_in(&r.plan(&d.computer, &d.machine), f);
            for (line, reference) in secrets::env_reins_preview(f, &r.ticked, &project).into_iter().take(8) {
                w = w.child(div().truncate().when(!reference, |l| l.text_color(pal.tertiary)).child(line));
            }
            after = after.child(w);
        }
        let mut page = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(div().flex().child(back))
            .child(head)
            .child(values)
            .child(heading("After the move", None, pal))
            .child(after);
        if f.risks.contains(&Risk::InGitHistory) {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .rounded(px(14.0))
                    .bg(pal.danger.opacity(0.08))
                    .border_1()
                    .border_color(pal.danger.opacity(0.25))
                    .child(icon(Icon::GitCommit, 16.0, pal.danger))
                    .child(div().flex_1().text_size(px(13.0)).child("This file is in git history. Moving it doesn't unpublish it: rotate these keys."))
                    .child(help_with(
                        "secrets-rotate",
                        || {
                            "At each service (OpenAI, Stripe, your database…), make a new key and delete the old one. The \
                             new one goes into your vault with reins secrets set; the old one stays in the repository's \
                             history, where it no longer works."
                                .into_any_element()
                        },
                        300.0,
                        d,
                        pal,
                        cx,
                    )),
            );
        }
        page.into_any_element()
    }

    fn moving(m: &Moving, d: &Data, pal: Palette) -> AnyElement {
        let phone = d.phone();
        let digits = m.check.clone().unwrap_or_else(|| "····".to_owned());
        let mut boxes = div().flex().gap(px(10.0)).mt(px(18.0));
        for c in digits.chars() {
            boxes = boxes.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(52.0))
                    .h(px(62.0))
                    .rounded(px(14.0))
                    .bg(pal.elevated)
                    .border_1()
                    .border_color(pal.tile_border())
                    .font_family(MONO)
                    .text_size(px(28.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(c.to_string()),
            );
        }
        let mut list = card(pal).w(px(520.0)).mt(px(26.0));
        for (i, (title, under, source, _)) in m.targets.iter().take(2).enumerate() {
            list = list.child(
                row(pal, i == 0)
                    .child(source_tile(*source, pal, 30.0))
                    .child(row_text(title.clone(), Some(under.clone().into_any_element()), pal))
                    .child(ring(1.0, pal.accent, pal.tile_border(), 16.0, 2.0)),
            );
        }
        let more: usize = m.targets.iter().skip(2).count();
        if more > 0 {
            let secrets_left: usize = m.count.saturating_sub(m.targets.iter().take(2).map(|t| t.3).sum());
            list = list.child(
                row(pal, false)
                    .child(tile(Icon::Ellipsis, pal.secondary, 30.0).bg(pal.tile()))
                    .child(row_text(format!("{secrets_left} more"), None, pal))
                    .child(div().text_size(px(12.5)).text_color(pal.secondary).child("waiting for your OK")),
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(30.0))
            .child(stage(Mark::Icon(Icon::Laptop), Mark::Icon(Icon::Smartphone), None, pal.accent, 56.0, pal))
            .child(
                div()
                    .mt(px(16.0))
                    .text_size(px(24.0))
                    .line_height(px(31.0))
                    .text_color(pal.secondary)
                    .child(sentence(&words(&[("Approve on ", false), (&phone, true)]), pal)),
            )
            .child(caption("Your phone shows the same check digits", pal).mt(px(6.0)).text_color(pal.tertiary))
            .child(boxes)
            .child(list)
            .child(fine("The server only relays sealed boxes; it can't read them.", pal).mt(px(14.0)))
            .into_any_element()
    }

    fn secrets_done(&self, done: &Finished, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let receipt = &done.receipt;
        let off = receipt.off_disk();
        let in_trash = receipt.moved.iter().filter(|m| m.in_trash).count();
        let n = receipt.moved.len();
        let title = match n {
            0 => "Nothing reached your vault".to_owned(),
            1 => "1 secret is in your vault".to_owned(),
            n => format!("{n} secrets are in your vault"),
        };
        let mut under = format!("In your vault · from \"{}\"", receipt.computer);
        if receipt.keep_local {
            under.push_str(" · local copies kept");
        } else if in_trash > 0 {
            under.push_str(" · local copies in the Trash until shredded");
        } else if receipt.purged_at.is_some() {
            under.push_str(" · local copies shredded");
        }
        if n > off + in_trash && !receipt.keep_local {
            let _ = write!(under, " · {} still on disk", n - off - in_trash);
        }
        let head = div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .child(check_circle(52.0, pal.success))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(30.0))
                            .line_height(px(34.0))
                            .font_family(SERIF)
                            .font_weight(FontWeight::NORMAL)
                            .child(title),
                    )
                    .child(caption(under, pal).text_color(pal.tertiary)),
            )
            .child(
                btn("secrets-done-close", "Back", Btn::Plain, pal).on_click(Self::on_model(cx, Model::secrets_home)),
            );
        let mut changed = card(pal);
        for (i, line) in receipt.done.iter().enumerate() {
            changed = changed.child(
                row(pal, i == 0)
                    .child(source_tile(line.source, pal, 30.0))
                    .child(row_text(line.what.clone(), Some(mono(line.how.clone(), pal).into_any_element()), pal))
                    .child(if line.ok {
                        icon(Icon::CircleCheck, 16.0, pal.success)
                    } else {
                        icon(Icon::TriangleAlert, 16.0, pal.warning)
                    }),
            );
        }
        if receipt.done.is_empty() {
            changed = changed.child(empty(
                if receipt.keep_local {
                    "Nothing on this computer changed: the vault keeps a backup."
                } else {
                    "Nothing on this computer changed."
                },
                pal,
            ));
        }
        let mut page =
            div().flex().flex_col().gap(px(14.0)).child(head).child(heading("What changed", None, pal)).child(changed);
        for note in &receipt.notes {
            page = page.child(caption(note.clone(), pal).px(px(4.0)).text_color(pal.warning));
        }
        // The originals in the Trash still hold the secrets in plain text: once all works, shred them (held to confirm).
        let trashed = receipt.trashed().len();
        if trashed > 0 && receipt.purged_at.is_none() && receipt.undone_at.is_none() {
            page = page.child(
                card(pal).child(
                    row(pal, true)
                        .child(tile(Icon::Trash, pal.danger, 30.0))
                        .child(row_text(
                            "Still readable in the Trash",
                            Some(
                                format!(
                                    "{}. Shred once all works; undo can't bring them back.",
                                    plural(trashed, "file", "files")
                                )
                                .into_any_element(),
                            ),
                            pal,
                        ))
                        .child(div().w(px(220.0)).flex_none().child(self.hold_button(
                            "secrets-purge",
                            Held::PurgeOriginals,
                            Icon::Trash,
                            "Shred",
                            "Shredded",
                            pal,
                            cx,
                        ))),
                ),
            );
        }
        if let Some(after) = &done.after {
            page = page.child(caption(after.clone(), pal).px(px(4.0)));
        }
        let pair = card(pal)
            .p(px(16.0))
            .flex_1()
            .gap(px(6.0))
            .child(
                div().flex().items_center().gap(px(10.0)).child(icon(Icon::MonitorSmartphone, 20.0, pal.blue())).child(
                    div().text_size(px(14.5)).font_weight(FontWeight::SEMIBOLD).child("Use them on another computer"),
                ),
            )
            .child(caption("Pair it with your phone; the same secrets are there.", pal).text_color(pal.tertiary))
            .child(div().flex().pt(px(6.0)).child(
                small(btn_icon("secrets-pair", Icon::QrCode, "Pair a computer", Btn::Plain, pal)).on_click(
                    |_, _, cx| cx.open_url("https://github.com/katulevskiy/reins/blob/main/docs/quick-start.md"),
                ),
            ));
        let ci = card(pal)
            .p(px(16.0))
            .flex_1()
            .gap(px(6.0))
            .opacity(0.75)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(Icon::CloudUpload, 20.0, pal.secondary))
                    .child(div().flex_1().text_size(px(14.5)).font_weight(FontWeight::SEMIBOLD).child("CI and servers"))
                    .child(chip_plain(None, "Soon", pal).h(px(22.0))),
            )
            .child(
                caption("Short-lived tokens for GitHub Actions and your servers, approved like everything else.", pal)
                    .text_color(pal.tertiary),
            );
        page = page.child(div().flex().gap(px(12.0)).mt(px(4.0)).child(pair).child(ci));
        if receipt.undone_at.is_none() && !receipt.changes.is_empty() {
            page = page.child(
                div().flex().justify_end().child(
                    small(btn("secrets-undo", "Undo the move", Btn::Plain, pal))
                        .on_click(Self::on_model(cx, Model::undo_import)),
                ),
            );
        }
        let _ = d;
        page.into_any_element()
    }
}
