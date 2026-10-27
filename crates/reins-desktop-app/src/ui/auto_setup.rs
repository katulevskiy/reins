//! Auto on this computer, the first time it is picked (Decisions v2, `docs/design/decisions-v2/01-decision-engine.md`
//! §3 and §9.1, `08-review-resolutions.md` R1): a sheet in two steps before the mode change goes to the phone.
//!
//! - **This computer**: the zones this computer decides hook requests with (`[auto] facts`): the GitHub owners its
//!   projects push to (proposed from their `origin`, never zoned until the owner picks), the owners, repositories and
//!   hosts already zoned, and hosts the owner adds. The workspace folders are Mine, read-only: they are the workspace.
//! - **Finish on your phone**: Done writes `[auto]` (`setup_done`, the facts) and asks the phone for Auto; the phone
//!   guides the rest (its own zones, the matrix, the hold).
//!
//! Only what the owner picks is written: a proposal left alone stays unknown, which is Outside.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::config::Config;
use reins_policy::context::{FactKind, Facts, Zone};

use super::field::Field;
use super::icons::{Brand, Glyph, Mark, glyph};
use super::{Data, Root, kit, parts};
use crate::theme::motion::{Entrance, enter};
use crate::theme::{MONO, NextText as _, Palette};

/// Which step shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Computer,
    Phone,
}

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum What {
    /// A workspace folder: Mine, not to be changed here.
    Root,
    Fact(FactKind),
}

/// One row of "This computer".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub what: What,
    pub key: String,
    /// `None`: proposed, not zoned (unknown, so Outside, until picked).
    pub zone: Option<Zone>,
}

impl Row {
    #[must_use]
    pub fn read_only(&self) -> bool {
        self.what == What::Root
    }
}

/// The setup as it is being filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    pub step: Step,
    pub rows: Vec<Row>,
    /// "Add a host", as typed, and whether the last one was refused.
    pub host: String,
    pub host_refused: bool,
    /// `[auto] facts` when it opened: kinds the sheet does not show are written back as they were.
    base: Facts,
}

impl Setup {
    /// The rows for `config` and the owners `proposed` (`consequences::proposals`): the workspace folders (Mine), then
    /// owners, repositories and hosts, each once, a zoned one with its zone.
    #[must_use]
    pub fn new(config: &Config, proposed: &[(FactKind, String)]) -> Self {
        let facts = &config.auto.facts;
        let mut rows: Vec<Row> = config
            .workspace
            .roots
            .iter()
            .map(|r| Row {
                what: What::Root,
                key: r.clone(),
                zone: Some(Zone::Mine),
            })
            .collect();
        let mut zoned: std::collections::BTreeMap<(What, String), Option<Zone>> = std::collections::BTreeMap::new();
        for (kind, map) in
            [(FactKind::Owner, &facts.owners), (FactKind::Repo, &facts.repos), (FactKind::Host, &facts.hosts)]
        {
            for (k, z) in map {
                zoned.insert((What::Fact(kind), k.clone()), Some(*z));
            }
        }
        for (kind, key) in proposed {
            let Some(key) = reins_policy::context::key(key) else {
                continue;
            };
            let zone = facts.get(*kind, &key);
            zoned.entry((What::Fact(*kind), key)).or_insert(zone);
        }
        rows.extend(zoned.into_iter().map(|((what, key), zone)| Row {
            what,
            key,
            zone,
        }));
        Self {
            step: Step::Computer,
            rows,
            host: String::new(),
            host_refused: false,
            base: facts.clone(),
        }
    }

    /// For the window: this computer's proposals, and only the workspace folders that are there (`--demo`: made-up
    /// owners, every folder).
    #[must_use]
    pub fn open(config: &Config, demo: bool) -> Self {
        if demo {
            let mut s = Self::new(config, &crate::demo::Demo::auto_proposals());
            let mut first = true;
            s.rows.retain(|r| !r.read_only() || std::mem::take(&mut first));
            return s;
        }
        let mut s = Self::new(config, &reins_desktop::consequences::proposals(config));
        let home = reins_desktop::config::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
        s.rows.retain(|r| {
            !r.read_only() || std::path::Path::new(&reins_policy::effects::normalize(&r.key, &home, &home)).is_dir()
        });
        s
    }

    /// Puts row `i` in `zone` (a workspace folder stays Mine).
    pub fn pick(&mut self, i: usize, zone: Zone) {
        if let Some(r) = self.rows.get_mut(i).filter(|r| !r.read_only()) {
            r.zone = Some(zone);
        }
    }

    /// "Add a host": the host typed (or a URL's), Outside until picked otherwise. `false` (kept to fix) when it is
    /// not a host name.
    pub fn add_host(&mut self) -> bool {
        let typed = self.host.trim();
        let host = url::Url::parse(typed)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| typed.to_owned());
        let ok = !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
            && Facts::default().set(FactKind::Host, &host, Zone::Outside);
        self.host_refused = !ok;
        if !ok {
            return false;
        }
        let key = reins_policy::context::key(&host).unwrap_or(host);
        if !self.rows.iter().any(|r| r.what == What::Fact(FactKind::Host) && r.key == key) {
            self.rows.push(Row {
                what: What::Fact(FactKind::Host),
                key,
                zone: Some(Zone::Outside),
            });
        }
        self.host.clear();
        true
    }

    /// The facts to write: those there were, with every zone picked here.
    #[must_use]
    pub fn facts(&self) -> Facts {
        let mut facts = self.base.clone();
        for r in &self.rows {
            if let (What::Fact(kind), Some(zone)) = (r.what, r.zone) {
                facts.set(kind, &r.key, zone);
            }
        }
        facts
    }
}

/// What a row is, in a word, and its mark.
fn kind_of(what: What) -> (&'static str, Mark) {
    match what {
        What::Root => ("Workspace", Mark::Glyph(Glyph::Laptop)),
        What::Fact(FactKind::Owner) => ("GitHub", Mark::Brand(Brand::GitHub)),
        What::Fact(FactKind::Repo) => ("Repository", Mark::Glyph(Glyph::GitBranch)),
        What::Fact(_) => ("Host", Mark::Glyph(Glyph::Globe)),
    }
}

impl Root {
    /// Choosing Auto: this computer's setup first (once), else the change asked of the phone.
    pub(super) fn choose_auto(&mut self, cx: &mut Context<'_, Self>) {
        let needs = self.model.read(cx).needs_auto_setup();
        if !needs {
            self.model.update(cx, |m, cx| m.change_mode(reins_desktop::autopilot::ModeChoice::Auto, cx));
            return;
        }
        let _opened = self.open_auto_setup(cx);
    }

    /// Opens the setup (`false`: the settings are not read yet, or could not be).
    pub(super) fn open_auto_setup(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let m = self.model.read(cx);
        let demo = m.demo();
        let Some(config) = m.snapshot.as_ref().and_then(|s| s.config.as_ref().ok()).cloned() else {
            return false;
        };
        let mut setup = Setup::open(&config, demo);
        if demo && std::env::var("REINS_DEMO_AUTO_SETUP").is_ok_and(|v| v.trim() == "phone") {
            setup.step = Step::Phone;
        }
        self.auto_setup = Some(setup);
        self.while_open = false;
        cx.notify();
        true
    }

    /// The setup's sheet, over a dimmed window.
    pub(super) fn auto_setup_sheet(
        &self,
        s: &Setup,
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body = match s.step {
            Step::Computer => self.computer_step(s, d, pal, window, cx),
            Step::Phone => Self::phone_step(d, pal, cx),
        };
        let sheet = div()
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .w(px(600.0))
            .px(px(24.0))
            .py(px(22.0))
            .rounded(px(22.0))
            .bg(pal.paper_raised())
            .border_1()
            .border_color(pal.rule())
            .shadow_lg()
            .child(body);
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("auto-setup-scrim")
                    .absolute()
                    .inset_0()
                    .bg(pal.scrim())
                    .on_click(Self::on_view(cx, |this, _| this.auto_setup = None)),
            )
            .child(div().absolute().inset_0().flex().justify_center().items_start().pt(px(90.0)).child(enter(
                sheet,
                "auto-setup-sheet",
                Entrance::Sheet,
            )))
            .into_any_element()
    }

    /// Step 1: the rows, "Add a host", Cancel and Next.
    fn computer_step(&self, s: &Setup, d: &Data, pal: Palette, window: &Window, cx: &mut Context<'_, Self>) -> Div {
        let final_row = s.rows.len().saturating_sub(1);
        let mut list = div().flex().flex_col();
        for (i, r) in s.rows.iter().enumerate() {
            list = list.child(Self::zone_row(i, r, i == final_row, pal, cx));
        }
        let add = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mt(px(12.0))
            .child(self.text_field(Field::AutoHost, "Add a host", pal, window, cx))
            .child(
                kit::button("auto-add-host", kit::Look::Soft, kit::Size::Regular, Some(Glyph::Plus), "Add", pal)
                    .on_click(Self::on_view(cx, |this, _| {
                        if let Some(s) = this.auto_setup.as_mut() {
                            s.add_host();
                        }
                    })),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().signal(crate::theme::text::SHEET_TITLE).text_color(pal.ink()).child("This computer"))
                    .child(parts::help(
                        "auto-setup",
                        "Auto on this computer uses these to tell what's yours. Anything not here counts as Outside.",
                        d,
                        pal,
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(div().text_size(px(12.5)).text_color(pal.ink_meta()).font_family(MONO).child("1 / 2")),
            )
            .child(div().id("auto-setup-rows").mt(px(14.0)).max_h(px(330.0)).overflow_y_scroll().child(list))
            .child(add)
            .when(s.host_refused, |c| {
                c.child(div().mt(px(6.0)).text_size(px(12.5)).text_color(pal.danger).child("Not a host name"))
            })
            .child(Self::sheet_buttons(
                ("auto-cancel", "Cancel", None),
                ("auto-next", "Next", Some(Glyph::ArrowForward), kit::Look::Inverse),
                Self::on_view(cx, |this, _| this.auto_setup = None),
                Self::on_view(cx, |this, _| {
                    if let Some(s) = this.auto_setup.as_mut() {
                        s.step = Step::Phone;
                    }
                }),
                true,
                pal,
            ))
    }

    /// One row: its mark, its key and kind, and the zone picker (a workspace folder's, Mine and still).
    fn zone_row(i: usize, r: &Row, last: bool, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let (kind, mark) = kind_of(r.what);
        let row = kit::hairline_row(pal, last).child(kit::pane::avatar(mark, 28.0, pal)).child(kit::row_lines(
            r.key.clone(),
            Some(kind.into()),
            pal,
        ));
        if r.read_only() {
            // The workspace is Mine: said, not offered.
            return row.child(kit::status_pill(Some(Glyph::Lock), Zone::Mine.label(), None, pal));
        }
        let mut picker =
            div().flex().flex_none().items_center().p(px(3.0)).gap(px(2.0)).rounded_full().bg(pal.paper_sunken());
        for zone in Zone::ALL {
            let on = r.zone == Some(zone);
            let seg = div()
                .id(SharedString::from(format!("auto-zone-{i}-{}", zone.label())))
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .px(px(10.0))
                .rounded_full()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .when_else(
                    on,
                    |s| s.bg(pal.paper_raised()).border_1().border_color(pal.rule()).text_color(pal.ink()).shadow_xs(),
                    |s| s.text_color(pal.ink_2()),
                )
                .child(zone.label());
            picker = picker.child(
                seg.cursor_pointer()
                    .hover(|h| h.text_color(pal.ink()))
                    .active(|s| s.opacity(kit::button::PRESSED))
                    .on_click(Self::on_view(cx, move |this, _| {
                        if let Some(s) = this.auto_setup.as_mut() {
                            s.pick(i, zone);
                        }
                    })),
            );
        }
        row.child(picker)
    }

    /// Step 2: one line, the phone, Back and Done.
    fn phone_step(d: &Data, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let sending = d.mode_asked.is_some();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .signal(crate::theme::text::SHEET_TITLE)
                            .text_color(pal.ink())
                            .child("Finish on your phone"),
                    )
                    .child(div().flex_1())
                    .child(div().text_size(px(12.5)).text_color(pal.ink_meta()).font_family(MONO).child("2 / 2")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(14.0))
                    .py(px(30.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(64.0))
                            .rounded(px(20.0))
                            .bg(pal.violet().opacity(0.12))
                            .child(glyph(Glyph::Phone, 30.0, pal.violet())),
                    )
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(px(21.0))
                            .text_color(pal.ink_2())
                            .child("Your phone sets up the rest"),
                    ),
            )
            .child(Self::sheet_buttons(
                ("auto-back", "Back", Some(Glyph::ArrowBack)),
                ("auto-done", "Done", Some(Glyph::Check), kit::Look::Violet),
                Self::on_view(cx, |this, _| {
                    if let Some(s) = this.auto_setup.as_mut() {
                        s.step = Step::Computer;
                    }
                }),
                cx.listener(|this, _, _, cx| {
                    let Some(s) = this.auto_setup.take() else {
                        return;
                    };
                    this.model.update(cx, |m, cx| m.finish_auto_setup(s.facts(), cx));
                    cx.notify();
                }),
                !sending && d.paired,
                pal,
            ))
    }

    /// The sheet's two buttons, 1 : 1.4, as the pause sheet's.
    fn sheet_buttons(
        back: (&'static str, &'static str, Option<Glyph>),
        go: (&'static str, &'static str, Option<Glyph>, kit::Look),
        on_back: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
        on_go: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
        enabled: bool,
        pal: Palette,
    ) -> Div {
        let go_button = kit::button(go.0, go.3, kit::Size::Regular, go.2, go.1, pal).w_full();
        div()
            .flex()
            .gap(px(8.0))
            .mt(px(18.0))
            .child(
                div().flex().flex_none().w(gpui::relative(0.41)).child(
                    kit::button(back.0, kit::Look::Soft, kit::Size::Regular, back.2, back.1, pal)
                        .w_full()
                        .on_click(on_back),
                ),
            )
            .child(div().flex().flex_1().min_w(px(0.0)).child(if enabled {
                go_button.on_click(on_go).into_any_element()
            } else {
                go_button.opacity(kit::button::DISABLED).into_any_element()
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut c = Config::default();
        c.workspace.roots = vec!["~/code".to_owned()];
        assert!(c.auto.facts.set(FactKind::Owner, "acme", Zone::Work));
        assert!(c.auto.facts.set(FactKind::Repo, "octo/tools", Zone::Friends));
        assert!(c.auto.facts.set(FactKind::Host, "build.acme.io", Zone::Work));
        assert!(c.auto.facts.set(FactKind::Domain, "acme.io", Zone::Work));
        c
    }

    #[test]
    fn setup_rows_from_proposals() {
        let proposed = vec![(FactKind::Owner, "dkat".to_owned()), (FactKind::Owner, "ACME".to_owned())];
        let s = Setup::new(&config(), &proposed);
        let rows: Vec<(What, &str, Option<Zone>, bool)> =
            s.rows.iter().map(|r| (r.what, r.key.as_str(), r.zone, r.read_only())).collect();
        assert_eq!(
            rows,
            vec![
                (What::Root, "~/code", Some(Zone::Mine), true),
                (What::Fact(FactKind::Owner), "acme", Some(Zone::Work), false),
                (What::Fact(FactKind::Owner), "dkat", None, false),
                (What::Fact(FactKind::Repo), "octo/tools", Some(Zone::Friends), false),
                (What::Fact(FactKind::Host), "build.acme.io", Some(Zone::Work), false),
            ]
        );
        assert_eq!(s.step, Step::Computer);
    }

    #[test]
    fn picks_and_hosts() {
        let mut s = Setup::new(&config(), &[(FactKind::Owner, "dkat".to_owned())]);
        s.pick(0, Zone::Outside);
        assert_eq!(s.rows[0].zone, Some(Zone::Mine), "a workspace folder stays Mine");
        s.host = "https://Deploy.Example.com/path".to_owned();
        assert!(s.add_host());
        assert!(s.host.is_empty());
        let added = s.rows.last().unwrap();
        assert_eq!((added.key.as_str(), added.zone), ("deploy.example.com", Some(Zone::Outside)));
        s.host = "dеploy.example.com".to_owned(); // a Cyrillic е
        assert!(!s.add_host() && s.host_refused);
        s.host = "two words".to_owned();
        assert!(!s.add_host());
        s.host = "deploy.example.com".to_owned();
        let before = s.rows.len();
        assert!(s.add_host());
        assert_eq!(s.rows.len(), before, "once");
    }

    #[test]
    fn saving_the_setup_writes_auto() {
        let dir = tempfile::tempdir().unwrap();
        let paths = reins_desktop::config::Paths::under(dir.path());
        let mut s =
            Setup::new(&config(), &[(FactKind::Owner, "dkat".to_owned()), (FactKind::Owner, "octo".to_owned())]);
        let dkat = s.rows.iter().position(|r| r.key == "dkat").unwrap();
        s.pick(dkat, Zone::Mine);
        s.host = "staging.acme.io".to_owned();
        assert!(s.add_host());
        let i = s.rows.len() - 1;
        s.pick(i, Zone::Work);
        reins_desktop::settings::set_auto_setup(&paths, &s.facts()).unwrap();
        let c = Config::load(&paths).unwrap();
        assert!(c.auto.setup_done);
        assert!(!c.auto.matrix_decides, "the switch is not the setup's");
        let f = &c.auto.facts;
        assert_eq!(f.get(FactKind::Owner, "dkat"), Some(Zone::Mine));
        assert_eq!(f.get(FactKind::Owner, "octo"), None, "a proposal left alone stays unknown");
        assert_eq!(f.get(FactKind::Owner, "acme"), Some(Zone::Work));
        assert_eq!(f.get(FactKind::Repo, "octo/tools"), Some(Zone::Friends));
        assert_eq!(f.get(FactKind::Host, "staging.acme.io"), Some(Zone::Work));
        assert_eq!(f.get(FactKind::Domain, "acme.io"), Some(Zone::Work), "kinds not shown are kept");
    }
}
