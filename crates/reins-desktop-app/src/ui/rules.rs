//! Rules ("What asks your phone"): which commands and files the AI tools' hooks ask the phone about, what goes
//! through without asking, what happens when nobody answers, and how long things wait. Checked on this computer;
//! approving always happens on the phone.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::guard::{GROUPS, OnNoAnswer};
use reins_desktop::settings::GuardList;

use super::field::{Field, slot};
use super::parts::{
    button, caption, card, empty, fine, group, labelled, mono, page_header, row, segment, segmented, toggle,
};
use super::{Data, Root};
use crate::backend::Setting;
use crate::format;
use crate::theme::Palette;

/// The wait times offered, in seconds.
const WAITS: [u64; 4] = [60, 120, 300, 600];

/// The wait choices to show for `current`: the usual ones, and `current` too when it is none of them.
#[must_use]
pub fn wait_choices(current: u64) -> Vec<u64> {
    let mut choices = WAITS.to_vec();
    if !choices.contains(&current) {
        choices.push(current);
        choices.sort_unstable();
    }
    choices
}

/// "1 min", "2 min 30 s".
fn wait_label(secs: u64) -> String {
    format::duration(i64::try_from(secs).unwrap_or(i64::MAX))
}

impl Root {
    pub(super) fn rules(
        &mut self,
        d: &Data,
        pal: Palette,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(22.0)).child(page_header(
            "What asks your phone",
            "Checked on this computer, before an AI tool runs a command or opens a file. When a rule matches, your \
             phone is asked, and approving always happens there. Anything else goes through.",
            pal,
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        let Some(config) = d.config() else {
            return page.child(card(pal).child(empty("Reading the settings…", pal))).into_any_element();
        };
        let guard = config.guard.clone();
        let approval = config.approval_timeout_secs;

        // Built-in groups.
        let mut groups = card(pal);
        for (i, g) in GROUPS.iter().enumerate() {
            let on = guard.defaults && !guard.group_off(g.id);
            let id = g.id;
            let mut r = row(pal, i == 0)
                .id(SharedString::from(format!("group-{id}")))
                .child(labelled(g.label, Some(g.detail.to_owned()), pal))
                .child(
                    fine(
                        if on {
                            "Asks"
                        } else {
                            "Goes through"
                        },
                        pal,
                    )
                    .flex_none(),
                )
                .child(toggle(on, pal));
            if guard.defaults {
                r = r
                    .cursor_pointer()
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::GuardGroup(id.to_owned(), !on), cx)));
            } else {
                r = r.opacity(0.5);
            }
            groups = groups.child(r);
        }
        let about = if guard.defaults {
            "On: the AI tool waits for your phone. Off: it goes ahead without asking."
        } else {
            "The built-in rules are off in config.toml (`guard.defaults = false`); only your own lists below count."
        };
        page = page.child(group("Built-in rules", Some(about), groups, pal));

        // Own lists.
        page = page.child(group(
            "Also ask about",
            Some("Commands match from the start of the command, and * matches anything (make deploy*). Files are paths or globs (~/secrets/**)."),
            card(pal)
                .child(self.list_editor(GuardList::Commands, "Commands", "make deploy*", &guard.commands, true, pal, window, cx))
                .child(self.list_editor(GuardList::Files, "Files", "~/secrets/**", &guard.files, false, pal, window, cx)),
            pal,
        ));
        page = page.child(group(
            "Never ask about",
            Some("Exceptions to every rule above: these go through without asking."),
            card(pal)
                .child(self.list_editor(
                    GuardList::AllowCommands,
                    "Commands",
                    "git push origin feature/*",
                    &guard.allow_commands,
                    true,
                    pal,
                    window,
                    cx,
                ))
                .child(self.list_editor(
                    GuardList::AllowFiles,
                    "Files",
                    "./.env.local",
                    &guard.allow_files,
                    false,
                    pal,
                    window,
                    cx,
                )),
            pal,
        ));

        // No answer.
        let mut answer = segmented(pal);
        for (i, (choice, label)) in
            [(OnNoAnswer::Deny, "Deny it"), (OnNoAnswer::Ask, "Leave it to the AI tool")].into_iter().enumerate()
        {
            answer = answer.child(
                segment(("no-answer", i), label, guard.on_no_answer == choice, pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::OnNoAnswer(choice), cx))),
            );
        }
        page = page.child(group(
            "When nobody answers",
            Some("If your phone does not answer in time: refuse the command (the AI tool is told why), or let the AI tool's own permission prompt decide."),
            card(pal).child(row(pal, true).child(div().flex_1().child(fine("Then", pal).text_size(px(12.5)))).child(answer)),
            pal,
        ));

        // Waits.
        let waits = |id: &'static str, current: u64, make: fn(u64) -> Setting, cx: &mut Context<'_, Self>| {
            let mut bar = segmented(pal);
            for secs in wait_choices(current) {
                bar = bar.child(
                    segment(SharedString::from(format!("{id}-{secs}")), wait_label(secs), secs == current, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.change(make(secs), cx))),
                );
            }
            bar
        };
        let hook_wait = waits("hook-wait", guard.timeout_secs, Setting::GuardTimeout, cx);
        let approval_wait = waits("approval-wait", approval, Setting::ApprovalTimeout, cx);
        page = page.child(group(
            "How long to wait for your phone",
            None,
            card(pal)
                .child(
                    row(pal, true)
                        .child(labelled("Commands and files", Some("Before an AI tool goes ahead".to_owned()), pal))
                        .child(hook_wait),
                )
                .child(
                    row(pal, false)
                        .child(labelled(
                            "git, SSH, API keys and secrets",
                            Some("Changing it restarts the background service".to_owned()),
                            pal,
                        ))
                        .child(approval_wait),
                ),
            pal,
        ));
        page.into_any_element()
    }

    /// One of the guard's own lists: its items (each removable) and a field to add one.
    #[allow(clippy::too_many_arguments, reason = "one call per list, all of it needed")]
    fn list_editor(
        &self,
        list: GuardList,
        title: &'static str,
        placeholder: &str,
        items: &[String],
        first: bool,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let n = slot(list);
        let mut block = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(16.0))
            .py(px(12.0))
            .when(!first, |b| b.border_t_1().border_color(pal.hairline))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(title))
                    .child(fine(format::count(items.len() as u64, "rule", "rules"), pal)),
            );
        for (i, item) in items.iter().enumerate() {
            let remove = item.clone();
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .pl(px(10.0))
                    .pr(px(4.0))
                    .h(px(30.0))
                    .rounded(px(7.0))
                    .bg(pal.control_fill.opacity(0.6))
                    .child(mono(item.clone(), pal).flex_1().min_w(px(0.0)).truncate())
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-{n}-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(22.0))
                            .rounded(px(6.0))
                            .text_color(pal.tertiary)
                            .cursor_pointer()
                            .hover(|s| s.bg(pal.danger.opacity(0.12)).text_color(pal.danger))
                            .child("×")
                            .on_click(Self::on_model(cx, move |m, cx| m.remove_guard_item(list, &remove, cx))),
                    ),
            );
        }
        let field = Field::Guard(list);
        block
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .pt(px(2.0))
                    .child(self.text_field(field, placeholder, pal, window, cx))
                    .child(
                        button(("add", n), "Add", pal, false, !self.guard_inputs[n].trim().is_empty())
                            .on_click(cx.listener(move |this, _, _, cx| this.submit(field, cx))),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_offer_the_current_value() {
        assert_eq!(wait_choices(120), vec![60, 120, 300, 600]);
        assert_eq!(wait_choices(90), vec![60, 90, 120, 300, 600]);
        assert_eq!(wait_label(90), "1 min 30 s");
        assert_eq!(wait_label(300), "5 min");
    }
}
