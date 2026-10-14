//! Rules ("What asks your phone"): the built-in rule groups as a grid of cards coloured by how risky they are, your
//! own commands and files to ask about or never ask about (removable tags and a field), what happens when nobody
//! answers, and how long things wait. Checked on this computer; approving always happens on the phone.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::guard::{GROUPS, GuardConfig, OnNoAnswer};
use reins_desktop::settings::GuardList;

use super::field::{Field, slot};
use super::icons::{Icon, icon};
use super::parts::{
    Btn, btn, caption, card, empty, fine, group, group_title, help, page_header, segment, segment_frame, segmented,
    tile, toggle,
};
use super::story::{group_icon, group_tone};
use super::{Data, Root};
use crate::backend::Setting;
use crate::format;
use crate::theme::{MONO, Palette};

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

/// A few examples of what a built-in group covers, for its card (the (?) has them all).
fn examples(id: &str) -> Option<&'static str> {
    Some(match id {
        "git-history" => "Force push, reset --hard",
        "deletes" => "rm -r, rd /s",
        "infrastructure" => "terraform apply, kubectl delete",
        "databases" => "DROP TABLE, TRUNCATE",
        "publishing" => "npm, cargo, docker push",
        "disks" => "dd, mkfs, Format-Volume",
        "cloud" => "~/.aws, kubeconfig",
        "tokens" => ".netrc, .npmrc",
        _ => return None,
    })
}

/// The list a card's field adds to: the "ask" or the "never ask" side, as a command or as a file. The user picks
/// which; nothing is guessed from what was typed (the guard matches the two lists differently: a command filed as a
/// file would never be asked about).
#[must_use]
pub fn list_of(never: bool, file: bool) -> GuardList {
    match (never, file) {
        (false, false) => GuardList::Commands,
        (false, true) => GuardList::Files,
        (true, false) => GuardList::AllowCommands,
        (true, true) => GuardList::AllowFiles,
    }
}

/// Whether `list` holds files (else commands).
#[must_use]
pub fn is_files(list: GuardList) -> bool {
    matches!(list, GuardList::Files | GuardList::AllowFiles)
}

/// What `typed` is stored as in `list`, as the guard reads it: trimmed (as `settings::set_guard_list` writes it), and
/// only what `GuardConfig::validate` accepts when it loads `config.toml`. A command pattern is one command: the guard
/// matches a pattern's first command only, so `make deploy; rm x` would not mean what it says.
pub fn check_item(list: GuardList, typed: &str) -> Result<String, String> {
    let item = typed.trim().to_owned();
    if item.is_empty() {
        return Err("Type a command or a path first".to_owned());
    }
    if !is_files(list)
        && !item.starts_with("text:")
        && (item.contains([';', '|', '&', '`', '\n']) || item.contains("$("))
    {
        return Err("One command per entry".to_owned());
    }
    // The guard matches a file pattern against the end of a path whose `.` parts are dropped: a `.` part in the
    // pattern would never match.
    if is_files(list) && item.split(['/', '\\']).any(|part| part == ".") {
        return Err("Leave out ./ (a pattern matches the end of a path)".to_owned());
    }
    let mut guard = GuardConfig::default();
    match list {
        GuardList::Commands => guard.commands.push(item.clone()),
        GuardList::Files => guard.files.push(item.clone()),
        GuardList::AllowCommands => guard.allow_commands.push(item.clone()),
        GuardList::AllowFiles => guard.allow_files.push(item.clone()),
    }
    guard.validate()?;
    Ok(item)
}

/// The field's example for `list`.
fn placeholder(list: GuardList) -> &'static str {
    match list {
        GuardList::Commands => "make deploy*",
        GuardList::Files => "~/secrets/**",
        GuardList::AllowCommands => "git push origin feature/*",
        GuardList::AllowFiles => ".env.local",
    }
}

impl Root {
    pub(super) fn rules(
        &mut self,
        d: &Data,
        pal: Palette,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let mut page = div().flex().flex_col().gap(px(18.0)).child(page_header(
            "What asks your phone",
            Some(help(
                "rules",
                "Checked on this computer, before an AI tool runs a command or opens a file. When a rule matches, \
                 your phone is asked, and approving always happens there. Anything else goes through.",
                d,
                pal,
                cx,
            )),
        ));
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).px(px(4.0)).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal).px(px(4.0))));
        let Some(config) = d.config() else {
            return page.child(card(pal).child(empty("Reading the settings…", pal))).into_any_element();
        };
        let guard = config.guard.clone();
        let approval = config.approval_timeout_secs;

        // Built-in groups, a grid of cards.
        // Three columns where they fit; two in a narrow window.
        let content = window.viewport_size().width.as_f32() - 232.0 - 68.0;
        let mut grid = div()
            .grid()
            .grid_cols(if content >= 660.0 {
                3
            } else {
                2
            })
            .gap(px(8.0));
        let mut on_count = 0;
        for g in GROUPS {
            let on = guard.defaults && !guard.group_off(g.id);
            on_count += usize::from(on);
            let id = g.id;
            let color = pal.tone(group_tone(id));
            let mut c = div()
                .id(SharedString::from(format!("group-{id}")))
                .flex()
                .items_center()
                .gap(px(11.0))
                .p(px(12.0))
                .min_h(px(62.0))
                .min_w(px(0.0))
                .rounded(px(14.0))
                .bg(pal.elevated)
                .border_1()
                .border_color(pal.hairline)
                .child(tile(group_icon(id), color, 30.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .line_height(px(17.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(g.label),
                        )
                        .when_some(examples(id), |t, ex| {
                            t.child(div().text_size(px(11.5)).line_height(px(15.0)).text_color(pal.tertiary).child(ex))
                        }),
                )
                .child(toggle(on, pal));
            if guard.defaults {
                c = c
                    .cursor_pointer()
                    .hover(|s| s.border_color(color.opacity(0.35)))
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::GuardGroup(id.to_owned(), !on), cx)));
            } else {
                c = c.opacity(0.5);
            }
            grid = grid.child(c);
        }
        let about = if guard.defaults {
            "On: the AI tool waits for your phone. Off: it goes ahead without asking."
        } else {
            "The built-in rules are off in config.toml (`guard.defaults = false`); only your own lists below count."
        };
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    group_title("Built-in", pal)
                        .child(div().text_size(px(13.0)).text_color(pal.tertiary).child(format!("{on_count} on")))
                        .child(help("rules-builtin", about, d, pal, cx)),
                )
                .child(grid),
        );

        // Own lists, side by side.
        let also = self.list_card(
            false,
            [(GuardList::Commands, &guard.commands), (GuardList::Files, &guard.files)],
            pal,
            window,
            cx,
        );
        let never = self.list_card(
            true,
            [(GuardList::AllowCommands, &guard.allow_commands), (GuardList::AllowFiles, &guard.allow_files)],
            pal,
            window,
            cx,
        );
        page = page.child(two(
            group(
                "Also ask about",
                Some(help(
                    "rules-also",
                    "Commands match from the start of the command, and * matches anything (make deploy*). Files \
                         are paths or globs (~/secrets/**).",
                    d,
                    pal,
                    cx,
                )),
                also,
                pal,
            ),
            group(
                "Never ask about",
                Some(help(
                    "rules-never",
                    "Exceptions to every rule above: these go through without asking.",
                    d,
                    pal,
                    cx,
                )),
                never,
                pal,
            ),
        ));

        // No answer, and the waits.
        let mut answer = segmented(pal).w_full();
        for (i, (choice, label)) in
            [(OnNoAnswer::Deny, "Deny"), (OnNoAnswer::Ask, "AI tool decides")].into_iter().enumerate()
        {
            answer = answer.child(
                segment(("no-answer", i), label, guard.on_no_answer == choice, pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::OnNoAnswer(choice), cx))),
            );
        }
        let waits = |id: &'static str, current: u64, make: fn(u64) -> Setting, cx: &mut Context<'_, Self>| {
            let mut bar = segmented(pal).w_full();
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
        page = page.child(two(
            group(
                "If nobody answers",
                Some(help(
                    "rules-no-answer",
                    "If your phone does not answer in time: refuse the command (the AI tool is told why), or let the \
                     AI tool's own permission prompt decide.",
                    d,
                    pal,
                    cx,
                )),
                answer,
                pal,
            ),
            group(
                "Wait for your phone",
                Some(help(
                    "rules-waits",
                    "The first is for commands and files; the second for git, SSH and secrets (changing it restarts \
                     the background service).",
                    d,
                    pal,
                    cx,
                )),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(hook_wait)
                    .child(fine("git, SSH and secrets", pal).px(px(4.0)))
                    .child(approval_wait),
                pal,
            ),
        ));
        page.into_any_element()
    }

    /// One side's lists as one card: their items as removable tags (a terminal for a command, a file for a file), and
    /// a field with Command | File, which says the list it adds to.
    fn list_card(
        &self,
        never: bool,
        lists: [(GuardList, &Vec<String>); 2],
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let side = usize::from(never);
        let mut tags = div().flex().flex_wrap().gap(px(6.0));
        let mut any = false;
        for (list, items) in lists {
            let n = slot(list);
            let kind = if is_files(list) {
                Icon::File
            } else {
                Icon::Terminal
            };
            for (i, item) in items.iter().enumerate() {
                any = true;
                let remove = item.clone();
                tags = tags.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .max_w_full()
                        .h(px(28.0))
                        .pl(px(8.0))
                        .pr(px(4.0))
                        .rounded(px(8.0))
                        .bg(pal.tile())
                        .font_family(MONO)
                        .text_size(px(12.0))
                        .child(icon(kind, 12.0, pal.secondary))
                        .child(div().min_w(px(0.0)).truncate().child(item.clone()))
                        .child(
                            div()
                                .id(SharedString::from(format!("remove-{n}-{i}")))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(20.0))
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(pal.danger.opacity(0.14)))
                                .child(icon(Icon::X, 12.0, pal.tertiary))
                                .on_click(Self::on_model(cx, move |m, cx| m.remove_guard_item(list, &remove, cx))),
                        ),
                );
            }
        }
        // The list the field adds to, as picked: Command or File.
        let file = self.guard_file[side];
        let list = list_of(never, file);
        let mut kinds = segmented(pal);
        for (i, (label, ic, is_file)) in
            [("Command", Icon::Terminal, false), ("File", Icon::File, true)].into_iter().enumerate()
        {
            let on = file == is_file;
            kinds = kinds.child(
                segment_frame(SharedString::from(format!("guard-kind-{side}-{i}")), on, pal)
                    .h(px(28.0))
                    .px(px(9.0))
                    .child(icon(
                        ic,
                        12.0,
                        if on {
                            pal.text
                        } else {
                            pal.secondary
                        },
                    ))
                    .child(label)
                    .on_click(Self::on_view(cx, move |this, _| {
                        this.guard_file[side] = is_file;
                        this.guard_error[side] = None;
                    })),
            );
        }
        let field = Field::Guard(list);
        let typed = !self.guard_inputs[slot(list)].trim().is_empty();
        let add = btn(("add", slot(list)), "Add", Btn::Plain, pal).h(px(34.0));
        card(pal)
            .p(px(12.0))
            .gap(px(10.0))
            .when(any, |c| c.child(tags))
            .child(kinds)
            .child(div().flex().gap(px(8.0)).child(self.text_field(field, placeholder(list), pal, window, cx)).child(
                if typed {
                    add.on_click(cx.listener(move |this, _, _, cx| this.submit(field, cx)))
                } else {
                    add.opacity(0.5).cursor_default()
                },
            ))
            .when_some(self.guard_error[side].clone(), |c, e| {
                c.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(12.0))
                        .text_color(pal.danger)
                        .child(icon(Icon::CircleAlert, 13.0, pal.danger))
                        .child(e),
                )
            })
            .into_any_element()
    }
}

/// Two blocks side by side while there is room.
fn two(left: impl IntoElement, right: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap(px(16.0))
        .child(div().flex_1().min_w(px(260.0)).child(left))
        .child(div().flex_1().min_w(px(260.0)).child(right))
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

    /// The guard as the settings would be after adding `typed` to `list` from the field.
    fn after_adding(list: GuardList, typed: &str) -> GuardConfig {
        let item = check_item(list, typed).unwrap();
        let mut guard = GuardConfig::default();
        match list {
            GuardList::Commands => guard.commands.push(item),
            GuardList::Files => guard.files.push(item),
            GuardList::AllowCommands => guard.allow_commands.push(item),
            GuardList::AllowFiles => guard.allow_files.push(item),
        }
        guard
    }

    #[test]
    fn the_field_adds_to_the_list_picked_and_never_guesses() {
        assert_eq!(list_of(false, false), GuardList::Commands);
        assert_eq!(list_of(false, true), GuardList::Files);
        assert_eq!(list_of(true, false), GuardList::AllowCommands);
        assert_eq!(list_of(true, true), GuardList::AllowFiles);

        // A command that looks like a path is a command when Command is picked, and the guard asks about it.
        let guard = after_adding(list_of(false, false), "  ./deploy.sh ");
        assert_eq!(guard.commands, vec!["./deploy.sh"], "stored as shown: trimmed only");
        assert!(guard.files.is_empty());
        assert!(guard.check_command("./deploy.sh --prod").is_some());
        let rm = after_adding(GuardList::Commands, "rm -rf ~/build");
        assert!(rm.check_command("rm -rf ~/build").is_some());

        // A glob is a file when File is picked.
        let guard = after_adding(list_of(false, true), "~/secrets/**");
        assert_eq!(guard.files, vec!["~/secrets/**"]);
        assert!(guard.commands.is_empty());
        assert!(guard.check_file("/home/dana/secrets/token").is_some());

        // The exceptions likewise.
        let mut never = after_adding(list_of(true, false), "git push origin feature/*");
        assert_eq!(never.allow_commands, vec!["git push origin feature/*"]);
        never.commands.push("git push".to_owned());
        assert!(never.check_command("git push origin feature/login").is_none());
        assert!(never.check_command("git push origin main").is_some());
        let mut never_file = after_adding(list_of(true, true), ".env.local");
        assert_eq!(never_file.allow_files, vec![".env.local"]);
        never_file.files.push(".env.*".to_owned());
        assert!(never_file.check_file("app/.env.local").is_none());
        assert!(never_file.check_file("app/.env.prod").is_some());
    }

    #[test]
    fn entries_the_guard_would_not_read_as_typed_are_refused() {
        assert!(check_item(GuardList::Commands, "   ").is_err());
        assert!(check_item(GuardList::Files, "/").is_err(), "an empty file pattern");
        assert!(check_item(GuardList::AllowFiles, "//").is_err());
        assert!(check_item(GuardList::AllowFiles, "./.env.local").is_err(), "a ./ part never matches");
        assert!(check_item(GuardList::Commands, "make deploy; rm x").is_err(), "two commands");
        assert!(check_item(GuardList::AllowCommands, "git push | tee log").is_err());
        assert!(check_item(GuardList::Commands, "echo $(whoami)").is_err());
        assert_eq!(check_item(GuardList::Commands, "text:drop table").unwrap(), "text:drop table");
        // A file pattern may hold what a command may not.
        assert_eq!(check_item(GuardList::Files, "~/a;b/*.pem").unwrap(), "~/a;b/*.pem");
    }

    #[test]
    fn some_groups_have_examples() {
        assert!(GROUPS.iter().filter_map(|g| examples(g.id)).count() >= 6);
    }
}
