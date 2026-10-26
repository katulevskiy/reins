//! Keys & secrets: finding the secrets lying around this computer and moving them into the vault on the phone
//! (`docs/secrets.md`, 5; the approved design `desktop-secrets.html`). The hero on the page starts a scan; then the
//! scan's progress, the review (grouped findings with what makes them risky, pre-ticked by confidence, a file's
//! detail with masked values), the phone's approval with the batch's check digits, and what changed. Values are never
//! here: only names, places and masked previews.

use std::fmt::Write as _;

use gpui::prelude::FluentBuilder as _;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, FontWeight, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Window, div, list, px, relative,
};
use reins_desktop::secrets_import::{Clash, Existing};
use reins_desktop::secrets_scan::{Confidence, Detector, Finding, Group, Risk, Source, Stage, StageState};

use super::field::Field;
use super::hold::Held;
use super::icons::{Brand, Glyph, Mark, glyph};
use super::kit;
use super::parts::{
    page_title,
    Btn, btn, btn_icon, caption, card, check_circle, chip, chip_plain, empty, fine, help, help_with, link, logo,
    one_line, page_header, ring, row, row_text, sentence, small, stage, tile, well,
};
use super::story::{Words, plain};
use super::{Data, Root};
use crate::model::Model;
use crate::secrets::{self, Finished, Flow, How, Line, MoveState, Moving, Named, Naming, Review, Scanning, Tick};
use crate::theme::text::tracked;
use crate::theme::{MONO, NextText as _, Palette};

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
        Source::Dotenv | Source::Cargo => crate::theme::Field::Push.kind(&pal),
        Source::SshKey | Source::GitCredentials => crate::theme::Field::Pair.kind(&pal),
        Source::Aws => gpui::rgb(0x00FF_9900).into(),
        Source::Npm => gpui::rgb(0x00CB_3837).into(),
        Source::Kube | Source::Docker | Source::Gcp | Source::PyPi => crate::theme::Field::Read.kind(&pal),
        Source::GhCli | Source::Shell | Source::Netrc => pal.ink_2(),
    }
}

/// A source's icon.
fn source_glyph(source: Source) -> Glyph {
    match source {
        Source::GhCli | Source::GitCredentials => Glyph::GitBranch,
        Source::Dotenv => Glyph::FileText,
        Source::SshKey => Glyph::Terminal,
        Source::Npm | Source::PyPi | Source::Cargo => Glyph::Package,
        Source::Docker | Source::Aws | Source::Gcp | Source::Kube => Glyph::Cloud,
        Source::Shell => Glyph::Terminal,
        Source::Netrc => Glyph::Globe,
    }
}

/// A source's square (`.tile2`): its icon on `paper-sunken`, the GitHub CLI's mark on its plate.
fn source_tile(source: Source, pal: Palette, side: f32) -> Div {
    let _ = side;
    let i = match source {
        Source::GhCli => return kit::pane::avatar(Mark::Brand(Brand::GitHub), 34.0, pal),
        Source::Dotenv => Glyph::FileLock,
        Source::SshKey => Glyph::Key,
        Source::GitCredentials => Glyph::GitBranch,
        Source::Npm | Source::PyPi | Source::Cargo => Glyph::Package,
        Source::Docker | Source::Aws | Source::Gcp | Source::Kube => Glyph::Cloud,
        Source::Shell => Glyph::Terminal,
        Source::Netrc => Glyph::Globe,
    };
    kit::pane::tile(i, pal)
}

/// What makes a finding risky, as a small chip: red for a danger, yellow for plain text, grey for the keychain.
fn risk_chip(risk: Risk, pal: Palette) -> Div {
    let (i, color) = match risk {
        Risk::ReadableByAgents | Risk::WorldReadable => (Glyph::Eye, pal.danger),
        Risk::InGitHistory => (Glyph::GitCommit, pal.danger),
        Risk::NoPassphrase => (Glyph::LockOpen, pal.danger),
        Risk::PlainText => (Glyph::FileText, pal.warning),
        Risk::InKeychain => return chip_plain(Some(Glyph::Lock), risk.label(), pal).h(px(22.0)),
    };
    chip(Some(i), risk.label(), color).h(px(22.0))
}

/// A tick box that stands for several items: all ticked, some, or none.
fn tick_box(t: Tick, pal: Palette) -> Div {
    let frame = div().flex().flex_none().items_center().justify_center().size(px(22.0)).rounded(px(7.0));
    match t {
        Tick::All => frame.bg(pal.violet_fill()).child(glyph(Glyph::Check, 14.0, pal.on_field())),
        Tick::Some => frame.bg(pal.violet_fill()).child(glyph(Glyph::Minus, 14.0, pal.on_field())),
        Tick::None => frame.border(px(2.0)).border_color(pal.ink_faint()),
    }
}

/// A tick box.
fn tick(on: bool, pal: Palette) -> Div {
    tick_box(
        if on {
            Tick::All
        } else {
            Tick::None
        },
        pal,
    )
}

/// A step of the review, as a section line (`.sec`): a glyph and its name.
fn step(n: u8, label: &str, pal: Palette) -> Div {
    let g = match n {
        1 => Glyph::Check,
        2 => Glyph::Vault,
        _ => Glyph::Phone,
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .min_h(px(30.0))
        .child(glyph(g, 15.0, pal.ink_2()))
        .child(tracked(label.to_owned()).text_color(pal.ink_2()))
}

/// What a risk looks like: its icon and colour.
fn risk_look(risk: Risk, pal: Palette) -> (Glyph, Hsla) {
    match risk {
        Risk::ReadableByAgents | Risk::WorldReadable => (Glyph::Eye, pal.danger),
        Risk::InGitHistory => (Glyph::GitCommit, pal.danger),
        Risk::NoPassphrase => (Glyph::LockOpen, pal.danger),
        Risk::PlainText => (Glyph::FileText, pal.warning),
        Risk::InKeychain => (Glyph::Lock, pal.ink_2()),
    }
}

/// The (?) beside "N of M selected".
const COUNT_HELP: &str = "secrets-count";

/// What the one total leaves out or counts without ticking, in a quiet line (`None`: nothing to say).
fn count_note(r: &Review) -> Option<String> {
    let (unsure, out) = (r.unsure(), r.left_out());
    let mut parts = vec!["Every count here is of values that may be secrets.".to_owned()];
    if unsure > 0 {
        parts.push(format!("{unsure} the scan is unsure of are counted but not ticked."));
    }
    if out > 0 {
        parts.push(format!(
            "{} that are not secrets (LOG_LEVEL, PORT…) are left out; open a file to move one anyway.",
            plural(out, "setting", "settings")
        ));
    }
    (unsure + out > 0).then(|| parts.join(" "))
}

/// A danger as a pill in its words and colour (red: AI agents can read it; amber: the rest).
fn risk_mark(risk: Risk, pal: Palette) -> Div {
    let color = match risk {
        Risk::ReadableByAgents | Risk::WorldReadable => pal.danger,
        Risk::InKeychain => pal.ink_2(),
        _ => pal.warning,
    };
    kit::status_pill(None, risk.label(), Some(color), pal)
}

/// How many of a file's (or a repository's) keys are ticked: `4/7`, quiet.
fn count(on: usize, all: usize, pal: Palette) -> Div {
    div()
        .flex_none()
        .font_family(MONO)
        .text_size(px(12.0))
        .text_color(if on == 0 {
            pal.ink_faint()
        } else {
            pal.ink_meta()
        })
        .child(format!("{on}/{all}"))
}

/// A row that opens what is folded under it ("More"): a chevron, a few words, how many.
fn fold(
    id: &'static str,
    label: &'static str,
    n: String,
    open: bool,
    pal: Palette,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(34.0))
        .border_t_1()
        .border_color(pal.rule())
        .cursor_pointer()
        .hover(|s| s.opacity(0.8))
        .child(glyph(
            if open {
                Glyph::ChevronDown
            } else {
                Glyph::ChevronRight
            },
            14.0,
            pal.ink_meta(),
        ))
        .child(div().flex_1().text_size(px(13.0)).text_color(pal.ink_2()).child(label))
        .child(div().font_family(MONO).text_size(px(12.0)).text_color(pal.ink_meta()).child(n))
        .on_click(on_click)
}

/// A project's name in the vault: its own name in ink, what was added to keep it apart quieter; its keys.
fn name_row(p: &Named, pal: Palette) -> Div {
    let text = format!("{}{}{}", p.before, p.base, p.after);
    let quiet = HighlightStyle {
        color: Some(pal.ink_faint()),
        ..HighlightStyle::default()
    };
    let mut marks = Vec::new();
    if !p.before.is_empty() {
        marks.push((0..p.before.len(), quiet));
    }
    if !p.after.is_empty() {
        marks.push((text.len() - p.after.len()..text.len(), quiet));
    }
    let tag = match p.how {
        How::Merged => Some("merged"),
        How::Renamed => Some("kept apart"),
        How::AsIs | How::Suffixed => None,
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(28.0))
        .py(px(3.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .font_family(MONO)
                .text_size(px(12.0))
                .line_height(px(17.0))
                .text_color(pal.ink())
                .child(StyledText::new(text).with_highlights(marks)),
        )
        .when_some(tag, |r, t| r.child(fine(t, pal)))
        .child(
            div()
                .flex_none()
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(pal.ink_meta())
                .child(p.keys.to_string()),
        )
}

/// A place in the vault that is not a project (this computer's own keys, SSH keys, git hosts): its keys.
fn plain_row(i: Glyph, label: String, n: usize, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(28.0))
        .py(px(3.0))
        .child(glyph(i, 13.0, pal.ink_meta()))
        .child(div().flex_1().min_w(px(0.0)).truncate().text_size(px(12.5)).text_color(pal.ink_2()).child(label))
        .child(div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(pal.ink_meta()).child(n.to_string()))
}

fn mono(text: impl Into<SharedString>, pal: Palette) -> Div {
    div().font_family(MONO).text_size(px(12.5)).text_color(pal.ink_meta()).min_w(px(0.0)).truncate().child(text.into())
}

/// A section line with a count on its right.
fn heading(title: &str, right: Option<String>, pal: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(6.0))
        .pt(px(6.0))
        .label()
        .text_color(pal.ink_2())
        .child(div().flex_1().child(title.to_owned()))
        .when_some(right, |h, r| h.child(div().font_weight(FontWeight::NORMAL).child(r)))
}

/// A row of the review's list drawn as part of its group's pane (the list is drawn a row at a time): raised paper
/// with the pane's sides, its top corners on the group's first row (`top`) and its bottom corners on the last
/// (`bottom`), a hairline after the tile under every other row.
fn slice(row: impl IntoElement, top: bool, bottom: bool, pal: Palette) -> Div {
    let r = kit::pane::RADIUS;
    div()
        .flex()
        .flex_col()
        .bg(pal.paper_raised())
        .when(!pal.dark, |d| d.border_l_1().border_r_1().border_color(pal.rule()))
        .when(top, |d| d.rounded_tl(px(r)).rounded_tr(px(r)).when(!pal.dark, |d| d.border_t_1()))
        .when(bottom, |d| d.rounded_bl(px(r)).rounded_br(px(r)).when(!pal.dark, |d| d.border_b_1()))
        .overflow_hidden()
        .child(row)
        .when(!bottom, |d| d.child(div().h(px(1.0)).ml(px(kit::pane::INSET)).bg(pal.rule())))
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
        Detector::Postgres | Detector::MySql | Detector::MongoDb | Detector::Redis => Glyph::Database,
        Detector::AwsAccessKeyId | Detector::AwsSecretKey | Detector::GoogleApiKey => Glyph::Cloud,
        Detector::StripeLive | Detector::StripeTest => Glyph::Card,
        Detector::SupabaseService | Detector::Jwt => Glyph::Token,
        _ => Glyph::Key,
    };
    Some(glyph(i, 13.0, pal.ink_2()).into_any_element())
}

impl Root {
    /// The Keys & secrets page's hero: what a scan does, and Scan.
    pub(super) fn secrets_hero(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let _ = d;
        let label = if cfg!(target_os = "macos") {
            "Scan this Mac"
        } else {
            "Scan this computer"
        };
        let row = div()
            .flex()
            .items_center()
            .gap(px(18.0))
            .px(px(20.0))
            .py(px(18.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(34.0))
                    .rounded(px(10.0))
                    .bg(pal.violet().opacity(0.15))
                    .child(glyph(Glyph::ScanSearch, 17.0, pal.violet())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .line_height(px(21.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.ink())
                            .child(format!("Find the secrets lying around {}", super::parts::this_computer())),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(13.5))
                            .line_height(px(18.0))
                            .text_color(pal.ink_2())
                            .child("Scans here, sends nothing. You approve each move on your phone."),
                    ),
            )
            .child(
                kit::button("secrets-scan-start", kit::Look::Inverse, kit::Size::Regular, Some(Glyph::ScanSearch), label, pal)
                    .on_click(Self::on_model(cx, Model::start_scan)),
            );
        kit::pane(row, kit::Glass::Solid, kit::pane::RADIUS, pal)
    }

    /// The step of the flow on its own page, when one is under way: `(page, whether it fills the window)`.
    pub(super) fn secrets_page(
        &mut self,
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
        let stop = kit::button("secrets-scan-stop", kit::Look::Soft, kit::Size::Regular, None, "Stop", pal)
            .on_click(Self::on_model(cx, Model::stop_scan));
        let head = page_title(
            &[plain(title)],
            Some("Only names and places leave this screen, nothing else."),
            Some(stop.into_any_element()),
            pal,
        );
        let summary = div()
            .flex()
            .items_center()
            .gap(px(18.0))
            .px(px(20.0))
            .py(px(18.0))
            .child(ring(p.fraction.max(0.04), pal.violet(), pal.rule_2(), 40.0, 6.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .line_height(px(21.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} found so far", p.found)),
                    )
                    .child(mono(p.at.clone(), pal))
                    .child(
                        div()
                            .mt(px(12.0))
                            .h(px(6.0))
                            .w_full()
                            .rounded(px(3.0))
                            .bg(pal.rule_2())
                            .overflow_hidden()
                            .child(div().h_full().w(relative(p.fraction.clamp(0.0, 1.0))).rounded(px(3.0)).bg(pal.violet())),
                    ),
            );
        let mut rows: Vec<AnyElement> = Vec::new();
        for stage in Stage::ALL {
            let state = p.stages.get(&stage).copied().unwrap_or_default();
            let g = match stage {
                Stage::Github => Glyph::GitBranch,
                Stage::Projects => Glyph::FolderSearch,
                Stage::Ssh => Glyph::Terminal,
                Stage::Cloud => Glyph::Cloud,
                Stage::Registries => Glyph::Package,
                Stage::Shell => Glyph::FileText,
            };
            let right: AnyElement = match state {
                StageState::Done(n) => div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(kit::pane::row_value(format!("{n} found"), pal))
                    .child(glyph(Glyph::Check, 16.0, pal.success))
                    .into_any_element(),
                StageState::Running => kit::pane::row_value("scanning", pal).into_any_element(),
                StageState::Waiting => kit::pane::row_value("waiting", pal).into_any_element(),
            };
            rows.push(
                kit::pane_row()
                    .child(kit::pane::tile(g, pal))
                    .child(kit::title_sub(stage.label(), Some(mono(stage.places(), pal).into_any_element()), pal))
                    .child(right)
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(head)
            .child(div().mt(px(6.0)).child(kit::pane(summary, kit::Glass::Solid, kit::pane::RADIUS, pal)))
            .child(kit::group(rows, kit::pane::INSET, kit::Glass::Solid, pal))
            .into_any_element()
    }

    /// The review: what was found and what is ticked on the left (step 1, only the rows in view drawn), how each
    /// will be named and the move on the right (steps 2 and 3).
    fn review_page(
        &mut self,
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
        // A new scan: a new list (its rows do not change with the ticks).
        let key = Arc::as_ptr(&r.lines) as usize;
        if self.scan_list_of != key {
            self.scan_list_of = key;
            self.scan_list.reset(r.lines.len());
        }
        let rows = list(
            self.scan_list.clone(),
            cx.processor(move |this: &mut Self, ix: usize, _window, cx| {
                let model = this.model.clone();
                let m = model.read(cx);
                let Flow::Review(r) = &m.secrets else {
                    return div().into_any_element();
                };
                let count_help = this.help == Some(COUNT_HELP);
                Self::review_line(r, ix, top, count_help, pal, cx)
            }),
        )
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(div().id("secrets-review").flex_1().min_w(px(0.0)).h_full().child(rows))
            .child(self.review_panel(r, d, pal, top, window, cx))
            .into_any_element()
    }

    /// One row of the review's left side.
    fn review_line(
        r: &Review,
        ix: usize,
        top: f32,
        count_help: bool,
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(line) = r.lines.get(ix) else {
            return div().into_any_element();
        };
        let inner: AnyElement = match line {
            Line::Head => Self::review_head(r, top, count_help, pal, cx),
            Line::Group {
                group,
                findings,
                shared,
            } => Self::group_line(r, *group, findings, *shared, pal, cx),
            Line::Repo {
                name,
                place,
                findings,
                ..
            } => Self::repo_line(r, name, place, findings, pal, cx),
            Line::File {
                id,
                nested,
                last,
                shared,
            } => match r.report.finding(id) {
                Some(f) => Self::file_line(r, f, *nested, *last, *shared, pal, cx),
                None => div().into_any_element(),
            },
            Line::Foot => {
                let report = &r.report;
                let mut note = format!(
                    "{} skipped (big, binary, links out of your home folder, caches).",
                    plural(report.skipped_total, "place", "places")
                );
                if let Some(b) = &report.budget {
                    let _ = write!(note, " The scan stopped at its {} limit at {}.", b.limit, b.at);
                }
                fine(note, pal).pt(px(14.0)).pb(px(34.0)).into_any_element()
            }
        };
        div().px(px(40.0)).child(inner).into_any_element()
    }

    /// The title, the risks found, step 1 and the whole selection's tick.
    fn review_head(r: &Review, top: f32, count_help: bool, pal: Palette, cx: &Context<'_, Self>) -> AnyElement {
        let report = &r.report;
        let title = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .title()
                    .child(format!(
                        "{} in {}",
                        plural(r.total(), "secret", "secrets"),
                        plural(report.findings.len(), "place", "places")
                    )),
            )
            // Scan again: the icon alone, so the title keeps its line.
            .child(
                kit::icon_button("secrets-rescan", Glyph::Refresh, 34.0, pal)
                    .on_click(Self::on_model(cx, Model::start_scan)),
            );
        let mut chips = div().flex().flex_wrap().gap(px(6.0));
        for (risk, n) in secrets::risk_counts(report) {
            let (color, words) = match risk {
                Risk::ReadableByAgents => (pal.danger, format!("{n} AI agents can read")),
                Risk::InGitHistory => (pal.warning, format!("{n} in git history")),
                _ => (pal.warning, plural(n, "key without passphrase", "keys without passphrase")),
            };
            chips = chips.child(kit::status_pill(None, words, Some(color), pal));
        }
        let all = r.tick_all();
        let select = div()
            .id("secrets-select-all")
            .flex()
            .items_center()
            .gap(px(10.0))
            .min_h(px(40.0))
            .cursor_pointer()
            .child(tick_box(all, pal))
            .child(div().label().text_color(pal.ink_2()).child(format!(
                "{} of {} selected",
                r.count(),
                r.total()
            )))
            .when_some(count_note(r), |row, note| {
                row.child(super::parts::help_open(COUNT_HELP, note, count_help, pal, cx))
            })
            .on_click(Self::on_model(cx, |m, cx| m.review(cx, Review::toggle_all)));
        let mut head = div().flex().flex_col().gap(px(10.0)).pt(px(top)).child(title);
        if report.findings.is_empty() {
            head = head.child(card(pal).child(empty("No secrets found on this computer.", pal)));
        } else {
            head = head.child(chips).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt(px(8.0))
                    .px(px(6.0))
                    .child(step(1, "Choose", pal))
                    .child(select),
            );
        }
        head.into_any_element()
    }

    /// A group's heading: its tick, its name, the risk nearly all of it shares, how many secrets.
    fn group_line(
        r: &Review,
        group: Group,
        findings: &[String],
        shared: Option<Risk>,
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let ids = findings.to_vec();
        let n: usize = findings.iter().filter_map(|id| r.report.finding(id)).map(|f| r.total_in(f)).sum();
        let with = |risk: Risk| {
            findings.iter().filter_map(|id| r.report.finding(id)).filter(|f| f.risks.contains(&risk)).count()
        };
        let head = div()
            .id(SharedString::from(format!("group-{}", group.label())))
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .pt(px(12.0))
            .pb(px(8.0))
            .cursor_pointer()
            .child(tick_box(r.tick_of(findings), pal))
            .child(tracked(group.label()).text_color(pal.ink_2()).flex_1().min_w(px(0.0)).overflow_hidden())
            .when_some(shared, |h, risk| {
                let color = match risk {
                    Risk::ReadableByAgents | Risk::WorldReadable => pal.danger,
                    _ => pal.warning,
                };
                h.child(kit::status_pill(None, format!("{} of {} · {}", with(risk), findings.len(), risk.label()), Some(color), pal))
            })
            .child(div().flex_none().label().text_color(pal.ink_2()).child(plural(n, "secret", "secrets")))
            .on_click(Self::on_model(cx, move |m, cx| {
                let ids = ids.clone();
                m.review(cx, |r| r.toggle_findings(&ids));
            }));
        div().pt(px(18.0)).child(slice(head, true, findings.is_empty(), pal)).into_any_element()
    }

    /// A repository with several files: its tick, its name and folder, how many of its keys are ticked.
    fn repo_line(
        r: &Review,
        name: &str,
        place: &str,
        findings: &[String],
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let ids = findings.to_vec();
        let files: Vec<&Finding> = findings.iter().filter_map(|id| r.report.finding(id)).collect();
        let on: usize = files.iter().map(|f| r.ticked_in(f)).sum();
        let all: usize = files.iter().map(|f| r.total_in(f)).sum();
        let row = div()
            .id(SharedString::from(format!("repo-{place}")))
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_h(px(54.0))
            .px(px(14.0))
            .py(px(8.0))
            .cursor_pointer()
            .child(tick_box(r.tick_of(findings), pal))
            .child(source_tile(Source::Dotenv, pal, 34.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(1.0))
                    .child(one_line(name.to_owned()).row())
                    .child(mono(place.to_owned(), pal)),
            )
            .child(count(on, all, pal))
            // Where a file's chevron sits, so the counts line up.
            .child(div().flex_none().w(px(15.0)))
            .on_click(Self::on_model(cx, move |m, cx| {
                let ids = ids.clone();
                m.review(cx, |r| r.toggle_findings(&ids));
            }));
        slice(row, false, false, pal).into_any_element()
    }

    /// A file: its tick, its name (under its repository's row: its path there), the risks it has that its group does
    /// not say, how many of its keys are ticked; a click opens it.
    fn file_line(
        r: &Review,
        f: &Finding,
        nested: bool,
        last: bool,
        shared: Option<Risk>,
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = f.id.clone();
        let open = f.id.clone();
        // Its dangers as icons (the chips over the list name them), but the one its group says for all.
        let marks = f
            .risks
            .iter()
            .filter(|k| k.severe() && Some(**k) != shared)
            .fold(div().flex().flex_none().items_center().gap(px(4.0)), |m, k| m.child(risk_mark(*k, pal)));
        let title = if nested {
            secrets::file_in_repo(f)
        } else {
            f.title.clone()
        };
        let text = if nested {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_1()
                .min_w(px(0.0))
                .child(mono(title, pal).text_size(px(12.5)).text_color(pal.ink()))
                .child(marks)
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(one_line(title).row())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .child(mono(f.place.clone(), pal).min_w(px(48.0)))
                        .child(marks),
                )
        };
        let right: AnyElement = if f.source == Source::Dotenv {
            count(r.ticked_in(f), r.total_in(f), pal).into_any_element()
        } else {
            div()
                .flex_none()
                .max_w(px(130.0))
                .truncate()
                .text_size(px(12.0))
                .text_color(pal.ink_meta())
                .child(f.summary.clone())
                .into_any_element()
        };
        let row = div()
            .id(SharedString::from(format!("finding-{}", f.id)))
            .flex()
            .items_center()
            .gap(px(12.0))
            .min_h(px(if nested {
                42.0
            } else {
                54.0
            }))
            .px(px(14.0))
            .py(px(6.0))
            .when(nested, |row| row.pl(px(56.0)))
            .cursor_pointer()
            .hover(|s| s.bg(pal.ink().opacity(0.035)))
            .child(
                div()
                    .id(SharedString::from(format!("tick-{}", f.id)))
                    .child(tick(r.ticked_in(f) > 0, pal))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        let id = id.clone();
                        this.model.update(cx, |m, cx| m.review(cx, |r| r.toggle_finding(&id)));
                    })),
            )
            .when(!nested, |row| row.child(source_tile(f.source, pal, 34.0)))
            .child(text)
            .child(right)
            .child(glyph(Glyph::ChevronRight, 14.0, pal.ink_faint()))
            .on_click(Self::on_model(cx, move |m, cx| {
                let open = open.clone();
                m.review(cx, |r| r.open = Some(open));
            }));
        slice(row, false, last, pal).into_any_element()
    }

    /// The review's right side: how many move, step 2 (the names, scrolling on its own) and step 3 (the move, always
    /// in view).
    fn review_panel(
        &self,
        r: &Review,
        d: &Data,
        pal: Palette,
        top: f32,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let plan = r.plan(&d.computer, &d.machine);
        let naming = Naming::of(&plan);
        let width = (window.viewport_size().width.as_f32() * 0.34).clamp(300.0, 380.0);
        let summary =
            div().flex().flex_col().gap(px(6.0)).child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(pal.ink_meta()).child("Move to your vault")).child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(6.0))
                    .child(
                        div().ask_xl().child(r.count().to_string()),
                    )
                    .child(
                        div()
                            .pb(px(4.0))
                            .text_size(px(14.0))
                            .text_color(pal.ink_meta())
                            .child(format!("of {}", r.total())),
                    ),
            );
        let body = div().id("secrets-panel").flex_1().min_h(px(0.0)).overflow_y_scroll().child(
            div()
                .flex()
                .flex_col()
                .gap(px(18.0))
                .px(px(22.0))
                .pt(px(top))
                .pb(px(18.0))
                .child(summary)
                .child(Self::names_step(r, &naming, d, pal, cx)),
        );
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(width))
            .h_full()
            .border_l_1()
            .border_color(pal.rule())
            .bg(pal.paper_raised())
            .child(body)
            .child(self.move_step(r, d, pal, window, cx))
            .into_any_element()
    }

    /// Step 2: what needs a choice (a project the vault already has from elsewhere), the vault not checked (once),
    /// and, folded, every project's name and the variables two files set.
    fn names_step(r: &Review, n: &Naming, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mut col = div().flex().flex_col().gap(px(10.0)).child(
            div().flex().items_center().gap(px(6.0)).child(step(2, "Names", pal)).child(help(
                "secrets-names",
                format!(
                    "Each project goes to your vault under its name (from its git origin, else its folder); this \
                     computer's own keys under \"{}\". Only what needs you shows here; the rest is under More.",
                    d.computer
                ),
                d,
                pal,
                cx,
            )),
        );
        if let Some(notice) = Self::vault_notice(r, d, pal, cx) {
            col = col.child(notice);
        }
        if !n.decisions.is_empty() {
            let mut rows = div().flex().flex_col();
            for c in &n.decisions {
                rows = rows.child(Self::decision_row(c, pal, cx));
            }
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(glyph(Glyph::CircleAlert, 14.0, pal.warning))
                            .child(div().flex_1().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).child(plural(
                                n.decisions.len(),
                                "name already in your vault",
                                "names already in your vault",
                            )))
                            .child(help(
                                "secrets-clash",
                                "Your vault has a project of this name from another folder. Keep both: this one gets \
                                 the name shown. Merge: its keys go into the vault's project, replacing any of the \
                                 same name (the old values stay in its history).",
                                d,
                                pal,
                                cx,
                            )),
                    )
                    .child(rows),
            );
        }
        // Every project's name, folded: quiet, the added parts quieter.
        let places =
            n.projects.len() + usize::from(n.machine.0 > 0) + usize::from(n.ssh > 0) + usize::from(n.tokens > 0);
        col = col.child(fold(
            "secrets-more-names",
            "Where they go",
            places.to_string(),
            r.open_names,
            pal,
            Self::on_model(cx, |m, cx| m.review(cx, |r| r.open_names = !r.open_names)),
        ));
        if r.open_names {
            let mut list = div().flex().flex_col().pl(px(22.0));
            for p in &n.projects {
                list = list.child(name_row(p, pal));
            }
            if n.machine.0 > 0 {
                list = list.child(plain_row(Glyph::Laptop, d.computer.clone(), n.machine.1, pal));
            }
            if n.ssh > 0 {
                list = list.child(plain_row(Glyph::Key, "Reins/ssh".to_owned(), n.ssh, pal));
            }
            if n.tokens > 0 {
                list = list.child(plain_row(Glyph::GitBranch, "git through Reins".to_owned(), n.tokens, pal));
            }
            col = col.child(list);
        }
        if !n.duplicates.is_empty() {
            col = col.child(fold(
                "secrets-more-dups",
                "Set in two files",
                n.duplicate_names().to_string(),
                r.open_dups,
                pal,
                Self::on_model(cx, |m, cx| m.review(cx, |r| r.open_dups = !r.open_dups)),
            ));
            if r.open_dups {
                let mut list = div().flex().flex_col().pl(px(22.0));
                for g in &n.duplicates {
                    let under = if g.same_file {
                        format!("twice in {}: the last one goes", g.kept)
                    } else {
                        format!("{} wins over {}", g.kept, g.dropped)
                    };
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .py(px(7.0))
                            .border_t_1()
                            .border_color(pal.rule())
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(12.0))
                                    .line_height(px(17.0))
                                    .text_color(pal.ink())
                                    .child(g.names.join(", ")),
                            )
                            .child(fine(under, pal)),
                    );
                }
                col = col.child(list);
            }
        }
        col
    }

    /// The vault's projects are not known: said once, why behind the (?), and Retry when asking again can help.
    fn vault_notice(r: &Review, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Option<Div> {
        let why = match &r.existing {
            Existing::Unknown(why) => why.clone(),
            Existing::Known(_) => return None,
        };
        let frame = || {
            div().flex().items_center().gap(px(10.0)).px(px(12.0)).py(px(10.0)).rounded(px(12.0)).bg(pal.paper_sunken())
        };
        let words = |title: &str, under: &str| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).child(title.to_owned()))
                .child(fine(under.to_owned(), pal))
        };
        if r.listing {
            return Some(
                frame()
                    .child(super::parts::waiting_dot("secrets-vault-wait", pal.violet(), 8.0))
                    .child(words("Checking your vault", "Approve on your phone")),
            );
        }
        if r.key.is_some() {
            return Some(
                frame().child(glyph(Glyph::Lock, 16.0, pal.ink_meta())).child(words("Vault not checked yet", "After your phone key")).child(help(
                    "secrets-unchecked",
                    "Your vault's projects can be listed once your phone's key is checked (step 3). Until then each \
                     project shows a short suffix, so nothing merges into one of the same name by mistake; the names \
                     settle after the check.",
                    d,
                    pal,
                    cx,
                )),
            );
        }
        Some(
            frame()
                .child(glyph(Glyph::Warning, 16.0, pal.warning))
                .child(words("Vault not checked", "Names kept apart"))
                .child(help(
                    "secrets-unchecked",
                    format!(
                        "Your vault's projects could not be listed ({why}). Each project shows a short suffix so \
                         nothing merges into one of the same name by mistake. Retry asks your phone again."
                    ),
                    d,
                    pal,
                    cx,
                ))
                .child(
                    kit::button(
                        "secrets-vault-retry",
                        kit::Look::Soft,
                        kit::Size::Small,
                        Some(Glyph::Refresh),
                        "Retry",
                        pal,
                    )
                    .on_click(Self::on_model(cx, Model::list_vault)),
                ),
        )
    }

    /// A project the vault already has from elsewhere: its name, Merge or Keep both, and what it becomes.
    fn decision_row(c: &Clash, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let merged = c.merged();
        let choice = |label: &'static str, merge: bool, cx: &mut Context<'_, Self>| {
            let name = c.name.clone();
            kit::segment(SharedString::from(format!("merge-{}-{merge}", c.name)), label, merged == merge, pal)
                .h(px(26.0))
                .px(px(10.0))
                .text_size(px(12.5))
                .on_click(Self::on_model(cx, move |m, cx| {
                    let name = name.clone();
                    m.review(cx, |r| r.set_merge(&name, merge));
                }))
        };
        let from = c.existing_origin.clone().unwrap_or_else(|| "no git origin".to_owned());
        let under = if merged {
            format!("into the vault's {} · {from}", c.name)
        } else {
            format!("→ {} · vault's: {from}", c.chosen)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(pal.rule())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(80.0))
                            .font_family(MONO)
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(c.name.clone()),
                    )
                    .child(kit::segmented(pal).p(px(2.0)).child(choice("Merge", true, cx)).child(choice(
                        "Keep both",
                        false,
                        cx,
                    ))),
            )
            .child(fine(under, pal).font_family(MONO))
    }

    /// Step 3, always in view: what happens to the local copies, the phone's key the first time, and the button that
    /// says what it does, or what it waits for.
    fn move_step(&self, r: &Review, d: &Data, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        let after = |label: &'static str, keep: bool, cx: &mut Context<'_, Self>| {
            kit::segment(SharedString::from(format!("secrets-then-{keep}")), label, r.keep_local == keep, pal)
                .h(px(28.0))
                .px(px(10.0))
                .text_size(px(12.5))
                .on_click(Self::on_model(cx, move |m, cx| m.review(cx, |r| r.keep_local = keep)))
        };
        let mut foot = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(10.0))
            .px(px(22.0))
            .pt(px(14.0))
            .pb(px(18.0))
            .border_t_1()
            .border_color(pal.rule())
            .child(step(3, "Move", pal))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        kit::segmented(pal)
                            .flex_1()
                            .min_w(px(0.0))
                            .child(after("Use via Reins", false, cx))
                            .child(after("Keep copies", true, cx)),
                    )
                    .child(help(
                        "secrets-after",
                        "Use via Reins: .env files become .env.reins (references only) and the originals go to the \
                         Trash, readable until you shred them; apps get the secrets from your vault when you approve. \
                         Keep copies: a backup in your vault only; nothing on this computer changes.",
                        d,
                        pal,
                        cx,
                    )),
            );
        if let Some(key) = &r.key {
            foot = foot.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(div().text_size(px(12.5)).text_color(pal.ink_2()).child("Phone key"))
                            .child(help(
                                "secrets-key",
                                "The twelve digits your phone shows for its key: Reins on your phone, Settings › \
                                 Devices. Checked once on this computer (your phone approves), so only your phone can \
                                 read what is sent.",
                                d,
                                pal,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .opacity(if key.checking {
                                0.55
                            } else {
                                1.0
                            })
                            .child(self.text_field(Field::PhoneKey, "0000-0000-0000", pal, window, cx)),
                    ),
            );
        }
        foot.child(Self::move_button(r, pal, cx))
            .when_some(r.error.clone(), |f, e| f.child(caption(e, pal).text_color(pal.danger)))
    }

    /// The button: what it does when it can (Move, or check the phone's key), else what it waits for, in a few words.
    fn move_button(r: &Review, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let (look, lead, label, live) = match r.move_state() {
            MoveState::Ready(n) => {
                (kit::Look::Inverse, Some(Glyph::Phone), format!("Move {n} · approve on phone"), true)
            }
            MoveState::CheckKey => {
                (kit::Look::Inverse, Some(Glyph::Key), "Check key · approve on phone".to_owned(), true)
            }
            MoveState::EnterKey => (kit::Look::Soft, Some(Glyph::Key), "Enter phone key".to_owned(), false),
            MoveState::NothingSelected => (kit::Look::Soft, None, "Select something to move".to_owned(), false),
            MoveState::CheckingKey => (kit::Look::Soft, None, "Approve on your phone".to_owned(), false),
            MoveState::CheckingVault => (kit::Look::Soft, None, "Checking your vault".to_owned(), false),
        };
        let waiting = matches!(r.move_state(), MoveState::CheckingKey | MoveState::CheckingVault);
        let b = kit::button("secrets-move", look, kit::Size::Regular, lead, label, pal).w_full().h(px(42.0));
        if live {
            b.on_click(Self::on_model(cx, Model::move_secrets)).into_any_element()
        } else if waiting {
            b.cursor_default()
                .child(div().absolute().left(px(16.0)).child(super::parts::waiting_dot(
                    "secrets-move-wait",
                    pal.violet(),
                    8.0,
                )))
                .into_any_element()
        } else {
            // Disabled, and it says why.
            kit::disabled(b).into_any_element()
        }
    }

    fn finding_detail(r: &Review, f: &Finding, d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> AnyElement {
        let back = link("secrets-back", "", pal)
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(glyph(Glyph::ChevronLeft, 16.0, pal.violet()))
            .child(format!("{} found", r.total()))
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
                div().text_color(pal.ink_meta()).child(words).into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_color(pal.ink_2())
                    .when_some(detector_mark(item.detector, pal), ParentElement::child)
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
                            .text_color(pal.ink_meta())
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
                .text_color(pal.ink_2())
                .child(explain),
        );
        if f.source == Source::Dotenv {
            let mut w = well(pal).w(px(300.0)).flex_none().text_size(px(11.5)).line_height(px(18.0));
            let project = secrets::project_in(&r.plan(&d.computer, &d.machine), f);
            for (line, reference) in secrets::env_reins_preview(f, &r.ticked, &project).into_iter().take(8) {
                w = w.child(div().truncate().when(!reference, |l| l.text_color(pal.ink_meta())).child(line));
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
                    .child(glyph(Glyph::GitCommit, 16.0, pal.danger))
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
                    .bg(pal.paper_raised())
                    .border_1()
                    .border_color(pal.rule())
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
                    .child(ring(1.0, pal.violet(), pal.rule(), 16.0, 2.0)),
            );
        }
        let more: usize = m.targets.iter().skip(2).count();
        if more > 0 {
            let secrets_left: usize = m.count.saturating_sub(m.targets.iter().take(2).map(|t| t.3).sum());
            list = list.child(
                row(pal, false)
                    .child(tile(Glyph::More, pal.ink_2(), 30.0).bg(pal.paper_sunken()))
                    .child(row_text(format!("{secrets_left} more"), None, pal))
                    .child(div().text_size(px(12.5)).text_color(pal.ink_2()).child("waiting for your OK")),
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(30.0))
            .child(stage(Mark::Glyph(Glyph::Laptop), Mark::Glyph(Glyph::Phone), None, pal.violet(), 56.0, pal))
            .child(
                div()
                    .mt(px(16.0))
                    .text_size(px(24.0))
                    .line_height(px(31.0))
                    .text_color(pal.ink_2())
                    .child(sentence(&words(&[("Approve on ", false), (&phone, true)]), pal)),
            )
            .child(caption("Your phone shows the same check digits", pal).mt(px(6.0)).text_color(pal.ink_meta()))
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
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(56.0))
                    .rounded_full()
                    .bg(pal.success.opacity(0.16))
                    .child(glyph(Glyph::Check, 26.0, pal.success)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(div().title().child(title))
                    .child(div().mt(px(4.0)).mood().text_color(pal.ink_2()).child(under)),
            )
            .child(
                kit::button("secrets-done-close", kit::Look::Inverse, kit::Size::Regular, None, "Done", pal)
                    .on_click(Self::on_model(cx, Model::secrets_home)),
            );
        let glass = kit::Glass::Solid;
        let mut changed: Vec<AnyElement> = Vec::new();
        for line in &receipt.done {
            let (g, color) = if line.ok { (Glyph::Check, pal.success) } else { (Glyph::Warning, pal.warning) };
            changed.push(
                kit::pane_row()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(34.0))
                            .rounded(px(10.0))
                            .bg(pal.success.opacity(0.14))
                            .child(glyph(source_glyph(line.source), 17.0, pal.success)),
                    )
                    .child(kit::title_sub(line.what.clone(), Some(div().child(line.how.clone()).into_any_element()), pal))
                    .child(glyph(g, 17.0, color))
                    .into_any_element(),
            );
        }
        if receipt.done.is_empty() {
            changed.push(
                empty(
                    if receipt.keep_local {
                        "Nothing on this computer changed: the vault keeps a backup."
                    } else {
                        "Nothing on this computer changed."
                    },
                    pal,
                )
                .into_any_element(),
            );
        }
        let mut page = div()
            .flex()
            .flex_col()
            .child(head)
            .child(kit::section(None, "What changed", None, pal).pt(px(26.0)))
            .child(kit::group(changed, kit::pane::INSET, glass, pal));
        for note in &receipt.notes {
            page = page.child(caption(note.clone(), pal).px(px(6.0)).pt(px(8.0)).text_color(pal.warning));
        }
        if let Some(after) = &done.after {
            page = page.child(caption(after.clone(), pal).px(px(6.0)).pt(px(8.0)));
        }
        let pair = kit::group(
            [kit::pane::clickable(kit::pane_row(), "secrets-pair", pal)
                .child(kit::pane::tile(Glyph::Laptop, pal))
                .child(kit::title_sub(
                    "Use them on another computer",
                    Some(div().child("Pair it with your phone; the same names work").into_any_element()),
                    pal,
                ))
                .child(kit::pane::chevron(pal))
                .on_click(|_, _, cx| cx.open_url("https://github.com/katulevskiy/reins/blob/main/docs/quick-start.md"))
                .into_any_element()],
            kit::pane::INSET,
            glass,
            pal,
        );
        let ci = kit::group(
            [kit::pane_row()
                .child(kit::pane::tile(Glyph::Server, pal))
                .child(kit::title_sub(
                    "CI and servers",
                    Some(div().child("Short-lived tokens, approved as they run").into_any_element()),
                    pal,
                ))
                .child(kit::pane::row_value("Soon", pal))
                .into_any_element()],
            kit::pane::INSET,
            glass,
            pal,
        );
        page = page.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(16.0))
                .mt(px(16.0))
                .child(div().flex_1().min_w(px(300.0)).child(pair))
                .child(div().flex_1().min_w(px(300.0)).child(ci)),
        );
        // The originals in the Trash still hold the secrets in plain text: once all works, shred them (held).
        let trashed = receipt.trashed().len();
        if trashed > 0 && receipt.purged_at.is_none() && receipt.undone_at.is_none() {
            page = page.child(div().mt(px(16.0)).child(self.hold_row(
                "secrets-purge",
                Held::PurgeOriginals,
                Glyph::Trash,
                "Shred the originals",
                format!("{} still readable in the Trash. Shred cannot be undone.", plural(trashed, "file", "files")),
                pal,
                cx,
            )));
        }
        if receipt.undone_at.is_none() && !receipt.changes.is_empty() {
            page = page.child(
                div().flex().justify_center().mt(px(18.0)).child(
                    div()
                        .id("secrets-undo")
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(40.0))
                        .px(px(14.0))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(pal.ink_2())
                        .cursor_pointer()
                        .hover(|s| s.text_color(pal.ink()))
                        .child(glyph(Glyph::Undo, 16.0, pal.ink_2()))
                        .child("Undo the move")
                        .on_click(Self::on_model(cx, Model::undo_import)),
                ),
            );
        }
        let _ = d;
        page.into_any_element()
    }
}
