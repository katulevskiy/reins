//! The component gallery (`--demo`, the sidebar's "Gallery", or `REINS_DEMO_SECTION=gallery`): every piece of the
//! kit, every field with its grain, the type scale, the colours and the glyphs, in the window's theme. For comparing
//! against `design/signal/screens/png/{Components,Color,Type}.png`; nothing here does anything real.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::hold::Held;
use super::icons::{Brand, Glyph, Illustration, avatar, glyph, illustration};
use super::kit::{self, HoldLook, Look, Mode, PillLook, Size};
use super::{Data, Root};
use crate::theme::text::{SCALE, tracked};
use crate::theme::{Field, MONO, Palette, NextText as _, Tone, tokens};

/// What the gallery's controls are set to.
#[derive(Clone, Copy, Debug)]
pub struct State {
    switch: bool,
    segment: usize,
    filter: usize,
    menu: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            switch: true,
            segment: 0,
            filter: 1,
            menu: false,
        }
    }
}

/// A panel of the gallery: a raised plate with its label.
fn panel(label: &str, pal: Palette) -> Div {
    kit::plate(pal).gap(px(16.0)).p(px(22.0)).rounded(px(24.0)).child(kit::section_label(label.to_owned(), pal))
}

fn row_of() -> Div {
    div().flex().flex_wrap().items_center().gap(px(10.0))
}

impl Root {
    pub(super) fn gallery(
        &mut self,
        d: &Data,
        pal: Palette,
        _window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let _ = d;
        let s = self.gallery_state;
        // `REINS_DEMO_GALLERY=<part>`: one part alone, for a screenshot of each (`fields`, `actions`, `controls`,
        // `type`, `colours`, `glyphs`).
        let part = std::env::var("REINS_DEMO_GALLERY").ok().filter(|p| !p.is_empty());
        let shows = |name: &str| part.as_deref().is_none_or(|p| p == name);
        let column = || div().flex().flex_col().flex_1().min_w(px(300.0)).gap(px(18.0));
        let two = || div().flex().flex_wrap().items_start().gap(px(18.0));
        div()
            .flex()
            .flex_col()
            .gap(px(26.0))
            .when(part.is_none(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(div().title().child(kit::ask_line("Signal, ", "the kit")))
                        .child(div().mood().text_color(pal.ink_2()).child("Every piece, in this window's theme.")),
                )
            })
            .when(shows("fields"), |page| page.child(Self::gallery_fields(pal)))
            .when(shows("actions"), |page| {
                page.child(
                    two()
                        .child(
                            column()
                                .child(Self::gallery_buttons(pal))
                                .child(self.gallery_hold(pal, cx))
                                .child(Self::gallery_timers(pal)),
                        )
                        .child(column().child(Self::gallery_chooser(s, pal, cx)).child(Self::gallery_menu(s, pal, cx))),
                )
            })
            .when(shows("controls"), |page| {
                page.child(
                    two()
                        .child(column().child(Self::gallery_controls(s, pal, cx)).child(Self::gallery_rows(s, pal, cx)))
                        .child(column().child(Self::gallery_calm(pal))),
                )
            })
            .when(shows("type"), |page| page.child(Self::gallery_type(pal)))
            .when(shows("colours"), |page| page.child(Self::gallery_colours(pal)))
            .when(shows("glyphs"), |page| page.child(Self::gallery_glyphs(pal)))
            .into_any_element()
    }

    /// The seven fields with their grain, and a field header as an ask shows one.
    fn gallery_fields(pal: Palette) -> Div {
        let mut swatches = div().flex().flex_wrap().gap(px(12.0));
        for f in Field::ALL {
            swatches = swatches.child(
                kit::field_surface(f, 20.0, 20.0)
                    .flex_1()
                    .min_w(px(112.0))
                    .h(px(118.0))
                    .p(px(14.0))
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(kit::field_label(f.label(), pal).relative())
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .text_color(pal.on_field())
                            .child(div().font_family(MONO).text_size(px(11.5)).child(format!("#{:06X}", f.rgb())))
                            .child(div().text_size(px(12.0)).text_color(pal.on_field_2()).child(f.means())),
                    ),
            );
        }
        let header = kit::field_surface(Field::Push, 0.0, kit::field::FIELD_RADIUS)
            .flex()
            .flex_col()
            .gap(px(12.0))
            .px(px(28.0))
            .pt(px(22.0))
            .pb(px(26.0))
            .text_color(pal.on_field())
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(avatar(Brand::Claude, 24.0, pal))
                    .child(div().text_size(px(13.5)).text_color(pal.on_field_2()).child("Claude Code · MacBook Pro"))
                    .child(div().flex_1())
                    .child(kit::timer(40, 120, pal.on_field())),
            )
            .child(kit::field_label("GitHub · Push", pal).relative())
            .child(div().relative().ask_xl().child(kit::ask_line("Push 8 commits to ", "octo/app")))
            .child(
                div()
                    .relative()
                    .text_size(px(14.0))
                    .text_color(pal.on_field_2())
                    .child("Autopilot would allow · 97%, like 11 pushes you approved"),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .gap(px(8.0))
                    .pt(px(4.0))
                    .child(kit::button("g-review", Look::OnField, Size::Regular, None, "Review", pal).w(px(160.0)))
                    .child(kit::button("g-wash", Look::Wash, Size::Regular, None, "Deny", pal).w(px(120.0)))
                    .child(kit::icon_button("g-close", Glyph::Close, 38.0, pal).bg(pal.on_field_wash()).border_0()),
            );
        div().flex().flex_col().gap(px(14.0)).child(header).child(swatches)
    }

    fn gallery_buttons(pal: Palette) -> Div {
        let b = |id: &'static str, look, label: &'static str| kit::button(id, look, Size::Regular, None, label, pal);
        panel("Buttons", pal)
            .child(
                div().flex().gap(px(10.0)).child(b("g-deny", Look::Outline, "Deny").flex_1()).child(
                    kit::button("g-approve", Look::Inverse, Size::Regular, Some(Glyph::FaceId), "Approve", pal)
                        .flex_1()
                        .flex_grow(1.35),
                ),
            )
            .child(
                row_of()
                    .child(kit::button("g-wrong", Look::Soft, Size::Small, Some(Glyph::Undo), "This was wrong", pal))
                    .child(kit::button("g-end", Look::DangerWash, Size::Small, None, "End now", pal))
                    .child(kit::button("g-allow", Look::Violet, Size::Small, None, "Allow", pal))
                    .child(
                        b("g-pressed", Look::Inverse, "Pressed")
                            .h(px(30.0))
                            .text_size(px(13.0))
                            .opacity(kit::button::PRESSED),
                    )
                    .child(kit::disabled(kit::button("g-disabled", Look::Inverse, Size::Small, None, "Disabled", pal))),
            )
            .child(
                row_of()
                    .child(kit::focused(
                        kit::button("g-focus", Look::Inverse, Size::Small, None, "Focused", pal),
                        30.0,
                        false,
                        pal,
                    ))
                    .child(div().w(px(6.0)))
                    .child(kit::icon_button("g-add", Glyph::Plus, 36.0, pal))
                    .child(kit::icon_button("g-filter", Glyph::Filter, 36.0, pal))
                    .child(kit::icon_button("g-back", Glyph::ChevronLeft, 36.0, pal)),
            )
            .child(
                kit::field_surface(Field::Push, 20.0, 20.0).flex().gap(px(8.0)).p(px(12.0)).children([
                    kit::button("g-f-review", Look::OnField, Size::Small, None, "Review", pal)
                        .flex_1()
                        .into_any_element(),
                    kit::focused(kit::button("g-f-deny", Look::Wash, Size::Small, None, "Deny", pal), 30.0, true, pal)
                        .flex_1()
                        .into_any_element(),
                ]),
            )
    }

    fn gallery_hold(&self, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let live = if self.gallery_held {
            kit::hold_button("g-hold-live", Glyph::Trash, "", "Deleted", HoldLook::Done, 38.0, pal)
                .on_click(Self::on_view(cx, |this, _| this.gallery_held = false))
        } else {
            self.hold_button("g-hold-live", Held::Gallery, Glyph::Trash, "Hold to delete", "Deleted", pal, cx)
        };
        panel("Hold · irreversible", pal)
            .child(live)
            .child(kit::hold_button("g-hold-mid", Glyph::Trash, "", "", HoldLook::Filling(0.58), 38.0, pal))
            .child(kit::hold_button("g-hold-done", Glyph::Trash, "", "Deleted", HoldLook::Done, 38.0, pal))
            .child(
                div()
                    .meta()
                    .text_color(pal.ink_meta())
                    .child("1200 ms, linear. Letting go drains in 180 ms. Reduce Motion never shortens it."),
            )
    }

    fn gallery_timers(pal: Palette) -> Div {
        panel("Timer", pal).child(
            row_of()
                .child(
                    kit::field_surface(Field::Read, 16.0, 16.0).p(px(10.0)).child(div().relative().child(kit::timer(
                        72,
                        120,
                        pal.on_field(),
                    ))),
                )
                .child(
                    kit::field_surface(Field::Read, 16.0, 16.0).p(px(10.0)).child(div().relative().child(kit::timer(
                        12,
                        120,
                        pal.on_field(),
                    ))),
                )
                .child(kit::timer(291, 300, pal.ink_2())),
        )
    }

    fn gallery_chooser(s: State, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mut seg = kit::segmented(pal);
        for (i, label) in ["Once", "1 hour", "1 day", "Custom"].into_iter().enumerate() {
            seg = seg.child(
                kit::segment(SharedString::from(format!("g-seg-{i}")), label, s.segment == i, pal)
                    .on_click(Self::on_view(cx, move |this, _| this.gallery_state.segment = i)),
            );
        }
        panel("Allow for · the chooser", pal).child(seg).child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .h(px(42.0))
                .rounded_full()
                .bg(pal.paper_sunken())
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(pal.ink_2())
                .child(glyph(Glyph::Lock, 15.0, pal.ink_2()))
                .child("Asked every time"),
        )
    }

    fn gallery_menu(s: State, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let items = |prefix: &'static str| {
            kit::menu(pal)
                .child(kit::menu_item(
                    SharedString::from(format!("{prefix}-manual")),
                    Glyph::Person,
                    (pal.paper_sunken(), pal.ink()),
                    "Manual",
                    Some("You decide, with a hint".into()),
                    false,
                    false,
                    pal,
                ))
                .child(kit::menu_item(
                    SharedString::from(format!("{prefix}-auto")),
                    Glyph::Autopilot,
                    (pal.violet_fill(), pal.on_field()),
                    "Auto",
                    Some("Decides the easy ones".into()),
                    true,
                    false,
                    pal,
                ))
                .child(kit::menu_item(
                    SharedString::from(format!("{prefix}-lockdown")),
                    Glyph::Lock,
                    (Field::Warn.tile(&pal), Field::Warn.kind(&pal)),
                    "Lockdown",
                    Some("Deny everything now".into()),
                    false,
                    false,
                    pal,
                ))
                .child(kit::menu_rule(pal))
                .child(kit::menu_item(
                    SharedString::from(format!("{prefix}-bypass")),
                    Glyph::Autopilot,
                    (pal.danger_wash(), pal.danger),
                    "Bypass…",
                    Some("Approve everything for a while".into()),
                    false,
                    true,
                    pal,
                ))
        };
        let trigger = div()
            .id("g-menu-trigger")
            .relative()
            .cursor_pointer()
            .active(|st| st.opacity(kit::button::PRESSED))
            .child(kit::pill(Some(Glyph::Autopilot), "Auto", PillLook::Violet, pal).child(glyph(
                Glyph::ChevronDown,
                12.0,
                pal.on_field(),
            )))
            .on_click(Self::on_view(cx, |this, _| this.gallery_state.menu = !this.gallery_state.menu))
            .when(s.menu, |t| t.child(kit::open_menu("g-menu-open", items("g-menu-live"), Anchor::TopRight)));
        panel("Menus · open from their trigger", pal)
            .child(div().flex().justify_end().child(trigger))
            .child(div().flex().justify_end().child(items("g-menu")))
            .child(
                div()
                    .meta()
                    .text_color(pal.ink_meta())
                    .child("Fades in and slides 4 px from the trigger in 180 ms; GPUI cannot scale it from 0.92."),
            )
    }

    fn gallery_controls(s: State, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mut filters = row_of();
        for (i, label) in ["All", "Autopilot", "Denied", "Grants"].into_iter().enumerate() {
            let look = if s.filter == i {
                PillLook::Selected
            } else {
                PillLook::Plain
            };
            filters = filters.child(
                div()
                    .id(SharedString::from(format!("g-filter-{i}")))
                    .cursor_pointer()
                    .active(|st| st.opacity(kit::button::PRESSED))
                    .child(kit::pill(None, label, look, pal))
                    .on_click(Self::on_view(cx, move |this, _| this.gallery_state.filter = i)),
            );
        }
        let mut tiles = row_of();
        for f in Field::ALL {
            let g = match f {
                Field::Push => Glyph::Send,
                Field::Read => Glyph::Search,
                Field::Delete => Glyph::Trash,
                Field::Grant => Glyph::Key,
                Field::Pair => Glyph::Link,
                Field::Accounts => Glyph::Person,
                Field::Warn => Glyph::Warning,
            };
            tiles = tiles.child(kit::kind_tile(g, f.into(), 32.0, pal));
        }
        tiles = tiles.child(kit::kind_tile(Glyph::Laptop, Tone::Neutral, 32.0, pal)).child(kit::kind_tile(
            Glyph::Check,
            Tone::Success,
            32.0,
            pal,
        ));
        panel("Controls", pal)
            .child(
                row_of()
                    .child(kit::switch("g-switch", s.switch, pal).on_click(Self::on_view(cx, |this, _| {
                        this.gallery_state.switch = !this.gallery_state.switch;
                    })))
                    .child(kit::switch("g-switch-off", false, pal))
                    .child(kit::kbd("Ctrl+1", pal))
                    .child(kit::kbd("⌘R", pal)),
            )
            .child(filters)
            .child(
                row_of()
                    .child(kit::mode_chip("g-mode-manual", Mode::Manual, pal))
                    .child(kit::mode_chip("g-mode-auto", Mode::Auto, pal))
                    .child(kit::mode_chip("g-mode-lockdown", Mode::Lockdown, pal))
                    .child(kit::mode_chip("g-mode-bypass", Mode::Bypass("14:05"), pal)),
            )
            .child(
                row_of()
                    .child(kit::pill(Some(Glyph::Check), "4 AI tools", PillLook::Soft, pal))
                    .child(kit::pill(None, "git through Reins", PillLook::Soft, pal))
                    .child(kit::tag("New", pal.success))
                    .child(kit::tag("Auto", pal.violet())),
            )
            .child(tiles)
    }

    fn gallery_rows(s: State, pal: Palette, cx: &mut Context<'_, Self>) -> Div {
        let mono =
            |t: &'static str| div().font_family(MONO).text_size(px(13.5)).font_weight(FontWeight::NORMAL).child(t);
        let request = |g: Glyph, tone: Tone, title: &'static str, meta: &'static str, end: AnyElement, last: bool| {
            kit::hairline_row(pal, last)
                .child(kit::kind_tile(g, tone, 32.0, pal))
                .child(kit::row_lines(title, Some(meta.into()), pal))
                .child(end)
        };
        let word = |t: &'static str, color| {
            div().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(t).into_any_element()
        };
        panel("Rows · hairlines, never cards", pal).child(
            div()
                .flex()
                .flex_col()
                .child(kit::fact_row(
                    "Branch",
                    div().flex().gap(px(8.0)).child(mono("feature/passkeys")).child(kit::tag("New", pal.success)),
                    pal,
                    false,
                ))
                .child(kit::fact_row(
                    "8 commits",
                    div()
                        .flex()
                        .gap(px(6.0))
                        .font_family(MONO)
                        .text_size(px(13.0))
                        .child(div().text_color(pal.success).child("+412"))
                        .child(div().text_color(pal.danger).child("−96"))
                        .child(glyph(Glyph::ChevronRight, 14.0, pal.ink_faint())),
                    pal,
                    false,
                ))
                .child(request(
                    Glyph::GitBranch,
                    Field::Push.into(),
                    "Pushed 8 commits to octo/app",
                    "Claude Code · 4 min",
                    word("Auto", pal.violet()),
                    false,
                ))
                .child(request(
                    Glyph::Mail,
                    Field::Read.into(),
                    "Read 2 emails from Stripe",
                    "ChatGPT · 18 min",
                    glyph(Glyph::Check, 16.0, pal.success).into_any_element(),
                    false,
                ))
                .child(request(
                    Glyph::Trash,
                    Field::Delete.into(),
                    "Delete branch old/experiment",
                    "Cursor · 1 h",
                    word("Denied", pal.danger),
                    false,
                ))
                .child(
                    kit::hairline_row(pal, true)
                        .child(kit::row_lines("Download on Wi-Fi only", Some("The model is 370 MB".into()), pal))
                        .child(kit::switch("g-row-switch", s.switch, pal).on_click(Self::on_view(cx, |this, _| {
                            this.gallery_state.switch = !this.gallery_state.switch;
                        }))),
                ),
        )
    }

    fn gallery_calm(pal: Palette) -> Div {
        let mut logos = row_of();
        for b in Brand::ALL {
            logos = logos.child(avatar(*b, 28.0, pal));
        }
        panel("Calm · ink drawings", pal)
            .child(kit::empty_state("All quiet on the water.", 300.0, pal))
            .child(div().flex().justify_center().child(illustration(Illustration::Kite, 220.0, pal)))
            .child(kit::section_label("Logos", pal))
            .child(logos)
    }

    fn gallery_type(pal: Palette) -> Div {
        let mut list = div().flex().flex_col();
        let count = SCALE.len();
        for (i, (name, style)) in SCALE.into_iter().enumerate() {
            let sample: AnyElement = match name {
                "ask-xl" => kit::ask_line("Delete ", "octo/old-api"),
                "ask" => kit::ask_line("Push 8 commits to ", "octo/app"),
                "title" => "Activity".into_any_element(),
                "stat" => "142 requests".into_any_element(),
                "sheet-title" => kit::ask_line("Remember ", "for"),
                "ask-small" => kit::ask_line("Read 2 emails from ", "Stripe"),
                "mood" => "All quiet. Three AIs are connected.".into_any_element(),
                "button" => "Approve".into_any_element(),
                "row" | "desk-row" => "Push to feature/* in octo/app".into_any_element(),
                "body" => "Approved without asking. Sent means sent.".into_any_element(),
                "meta" => "Claude Code · 3 of 10 uses".into_any_element(),
                "tag" => div().flex().child(kit::tag("Auto", pal.violet())).into_any_element(),
                "label" => tracked("GitHub · Push").text_color(pal.ink_meta()).into_any_element(),
                _ => "feature/passkeys · 9f86d08".into_any_element(),
            };
            let spec = format!(
                "{} {}/{} · {}{}",
                style.family.split(' ').next_back().unwrap_or(""),
                style.size,
                style.line,
                style.weight.0,
                if style.italic {
                    " italic"
                } else {
                    ""
                }
            );
            list = list.child(
                kit::hairline_row(pal, i + 1 == count)
                    .py(px(10.0))
                    .child(div().w(px(130.0)).flex_none().child(kit::section_label(name, pal)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .signal(style)
                            .text_color(if name == "mood" {
                                pal.ink_2()
                            } else {
                                pal.ink()
                            })
                            .child(sample),
                    )
                    .child(
                        div().flex_none().font_family(MONO).text_size(px(11.5)).text_color(pal.ink_meta()).child(spec),
                    ),
            );
        }
        panel("Type", pal).child(list)
    }

    fn gallery_colours(pal: Palette) -> Div {
        let mut grid = div().flex().flex_wrap().gap(px(10.0));
        for (name, t) in tokens::ALL {
            let hex = if pal.dark {
                t.dark
            } else {
                t.light
            };
            grid = grid.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .w(px(236.0))
                    .child(div().size(px(30.0)).rounded(px(8.0)).bg(pal.token(t)).border_1().border_color(pal.rule()))
                    .child(div().flex_1().text_size(px(13.5)).child(name))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .text_color(pal.ink_meta())
                            .child(format!("#{hex:06X}")),
                    ),
            );
        }
        panel("Colours", pal).child(grid)
    }

    fn gallery_glyphs(pal: Palette) -> Div {
        let mut grid = div().flex().flex_wrap().gap(px(6.0));
        for g in Glyph::ALL {
            grid = grid.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.0))
                    .w(px(88.0))
                    .py(px(8.0))
                    .child(glyph(*g, 22.0, pal.ink()))
                    .child(div().text_size(px(10.5)).text_color(pal.ink_meta()).truncate().child(g.name())),
            );
        }
        panel(&format!("Glyphs · {}", Glyph::ALL.len()), pal).child(grid)
    }
}
