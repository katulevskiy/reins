//! Rules, "What asks *your phone*" (`Desk-Rules.png`): the built-in rule groups as a grid of plates with their kind
//! tiles, switches and examples; your own commands and files to also ask about or never ask about (removable tags,
//! Command | File and a field); what happens when nobody answers, and how long each kind of request waits (two rows:
//! commands and files, then git, SSH and secrets). Checked on this computer; approving always happens on the phone.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::guard::{GROUPS, GuardConfig, OnNoAnswer};
use reins_desktop::settings::GuardList;

use super::field::{Field, slot};
use super::icons::{Glyph, glyph};
use super::parts::{caption, help, italic, mood};
use super::story::{bold, group_icon, group_tone, plain};
use super::{Data, Root, kit};
use crate::backend::Setting;
use crate::format;
use crate::theme::{Field as Kind, MONO, Palette, NextText as _, Tone};

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

/// A few examples of what a built-in group covers, for its plate.
fn examples(id: &str) -> Option<&'static str> {
    Some(match id {
        "git-history" => "push --force, reset --hard",
        "deletes" => "rm -r, rd /s",
        "infrastructure" => "terraform apply",
        "databases" => "DROP TABLE, TRUNCATE",
        "publishing" => "npm, cargo, docker push",
        "payments" => "reins payments-trust",
        "disks" => "dd, mkfs",
        "secrets" => "reins secrets scan, get",
        "env-files" => ".env, .env.local",
        "keys" => "id_ed25519, *.pem",
        "cloud" => "~/.aws, kubeconfig",
        "tokens" => ".netrc, .npmrc",
        _ => return None,
    })
}

/// A group's tile on its plate, as the board colours them: publishing and trusting a payment key in amber (flags),
/// the rest by how risky the group is ([`group_tone`]).
fn plate_tone(id: &str) -> Tone {
    match id {
        "publishing" | "payments" => Tone::Field(Kind::Warn),
        other => group_tone(other),
    }
}

/// The board's names for the groups whose label in the guard is longer.
fn short_label(label: &str) -> &str {
    match label {
        "Infrastructure changes" => "Infrastructure",
        "Cloud and cluster credentials" => "Cloud credentials",
        other => other,
    }
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
        GuardList::Commands => "kubectl delete*",
        GuardList::Files => "~/secrets/**",
        GuardList::AllowCommands => "npm test*",
        GuardList::AllowFiles => ".env.example",
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
        let title = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div().title().text_color(pal.ink()).child(italic(&[plain("What asks "), bold("your phone")])),
                    )
                    .child(help(
                        "rules",
                        "When a rule matches, your phone is asked, and approving always happens there. Anything else \
                         goes through.",
                        d,
                        pal,
                        cx,
                    )),
            )
            .child(mood("Checked here, before an AI tool runs a command or opens a file.", pal));
        let mut page = div().flex().flex_col().gap(px(22.0)).child(title);
        page = page.when_some(d.notice.clone(), |p, n| p.child(caption(n, pal).text_color(pal.danger)));
        page = page.when_some(d.note.clone(), |p, n| p.child(caption(n, pal)));
        let Some(config) = d.config() else {
            return page.child(caption("Reading the settings…", pal)).into_any_element();
        };
        let guard = config.guard.clone();
        let approval = config.approval_timeout_secs;

        // Built-in groups: three columns where they fit, two in a narrow window.
        let content = window.viewport_size().width.as_f32() - 236.0 - 88.0;
        let mut grid = div()
            .grid()
            .grid_cols(if content >= 790.0 {
                3
            } else {
                2
            })
            .gap(px(10.0));
        let mut on_count = 0;
        for g in GROUPS {
            let on = guard.defaults && !guard.group_off(g.id);
            on_count += usize::from(on);
            let id = g.id;
            let mut plate = div()
                .id(SharedString::from(format!("group-{id}")))
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(12.0))
                .min_w(px(0.0))
                .rounded(px(16.0))
                .bg(pal.paper_raised())
                .border_1()
                .border_color(pal.rule())
                .when(!on, |p| p.opacity(0.7))
                .child(kit::kind_tile(group_icon(id), plate_tone(id), 32.0, pal))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(14.0))
                                .line_height(px(18.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(pal.ink())
                                .child(short_label(g.label).to_owned()),
                        )
                        .when_some(examples(id), |t, ex| {
                            t.child(
                                div()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .line_height(px(15.0))
                                    .text_color(pal.ink_meta())
                                    .child(ex),
                            )
                        }),
                )
                .child(kit::switch(SharedString::from(format!("group-switch-{id}")), on, pal));
            if guard.defaults {
                plate = plate
                    .cursor_pointer()
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::GuardGroup(id.to_owned(), !on), cx)));
            } else {
                plate = plate.opacity(0.5);
            }
            grid = grid.child(plate);
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
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(kit::section_label(format!("Built in · {on_count} of {} on", GROUPS.len()), pal))
                        .child(help("rules-builtin", about, d, pal, cx)),
                )
                .child(grid),
        );

        // Own lists, side by side.
        let also = self.list_card(
            false,
            [(GuardList::Commands, &guard.commands), (GuardList::Files, &guard.files)],
            d,
            pal,
            window,
            cx,
        );
        let never = self.list_card(
            true,
            [(GuardList::AllowCommands, &guard.allow_commands), (GuardList::AllowFiles, &guard.allow_files)],
            d,
            pal,
            window,
            cx,
        );
        page = page.child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(px(14.0))
                .child(div().flex_1().min_w(px(360.0)).child(also))
                .child(div().flex_1().min_w(px(360.0)).child(never)),
        );

        // No answer, and the two waits.
        let mut answer = kit::segmented(pal);
        for (i, (choice, label)) in
            [(OnNoAnswer::Deny, "Deny"), (OnNoAnswer::Ask, "AI tool decides")].into_iter().enumerate()
        {
            answer = answer.child(
                kit::segment(("no-answer", i), label, guard.on_no_answer == choice, pal)
                    .on_click(Self::on_model(cx, move |m, cx| m.change(Setting::OnNoAnswer(choice), cx))),
            );
        }
        let waits = |id: &'static str, current: u64, make: fn(u64) -> Setting, cx: &mut Context<'_, Self>| {
            let mut bar = kit::segmented(pal);
            for secs in wait_choices(current) {
                bar = bar.child(
                    kit::segment(SharedString::from(format!("{id}-{secs}")), wait_label(secs), secs == current, pal)
                        .on_click(Self::on_model(cx, move |m, cx| m.change(make(secs), cx))),
                );
            }
            bar
        };
        let setting = |label: &'static str, control: Div| {
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(div().text_size(px(14.0)).text_color(pal.ink_2()).child(label))
                .child(control)
        };
        let hook_wait = waits("hook-wait", guard.timeout_secs, Setting::GuardTimeout, cx);
        let approval_wait = waits("approval-wait", approval, Setting::ApprovalTimeout, cx);
        page = page.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(40.0))
                .gap_y(px(14.0))
                .pt(px(4.0))
                .child(setting("If nobody answers", answer))
                .child(setting("Wait for your phone", hook_wait))
                .child(setting("git, SSH and secrets wait", approval_wait).child(help(
                    "rules-waits",
                    "The first wait is for commands and files; this one for git, SSH and secrets (changing it \
                         restarts the background service).",
                    d,
                    pal,
                    cx,
                ))),
        );
        page.into_any_element()
    }

    /// One side's lists as one plate: their items as removable tags, then Command | File, a field and Add.
    fn list_card(
        &self,
        never: bool,
        lists: [(GuardList, &Vec<String>); 2],
        d: &Data,
        pal: Palette,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let side = usize::from(never);
        let mut tags = div().flex().flex_wrap().gap(px(6.0));
        let mut any = false;
        for (list, items) in lists {
            let n = slot(list);
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
                        .pl(px(10.0))
                        .pr(px(6.0))
                        .rounded(px(8.0))
                        .bg(pal.paper_sunken())
                        .font_family(MONO)
                        .text_size(px(12.0))
                        .text_color(pal.ink())
                        .child(div().min_w(px(0.0)).truncate().child(item.clone()))
                        .child(
                            div()
                                .id(SharedString::from(format!("remove-{n}-{i}")))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(18.0))
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(pal.danger.opacity(0.14)))
                                .child(glyph(Glyph::Close, 12.0, pal.ink_meta()))
                                .on_click(Self::on_model(cx, move |m, cx| m.remove_guard_item(list, &remove, cx))),
                        ),
                );
            }
        }
        // The list the field adds to, as picked: Command or File.
        let file = self.guard_file[side];
        let list = list_of(never, file);
        let mut kinds = kit::segmented(pal);
        for (i, (label, is_file)) in [("Command", false), ("File", true)].into_iter().enumerate() {
            kinds = kinds.child(
                kit::segment(SharedString::from(format!("guard-kind-{side}-{i}")), label, file == is_file, pal)
                    .on_click(Self::on_view(cx, move |this, _| {
                        this.guard_file[side] = is_file;
                        this.guard_error[side] = None;
                    })),
            );
        }
        let field = Field::Guard(list);
        let typed = !self.guard_inputs[slot(list)].trim().is_empty();
        let add = kit::button(
            ("add", slot(list)),
            if typed {
                kit::Look::Inverse
            } else {
                kit::Look::Soft
            },
            kit::Size::Regular,
            None,
            "Add",
            pal,
        )
        .h(px(36.0))
        .px(px(14.0));
        let (label, about) = if never {
            ("Never ask about", "Exceptions to every rule above: these go through without asking.")
        } else {
            (
                "Also ask about",
                "Commands match from the start of the command, and * matches anything (make deploy*). Files are \
                 paths or globs (~/secrets/**).",
            )
        };
        kit::plate(pal)
            .p(px(18.0))
            .gap(px(12.0))
            .child(div().flex().items_center().gap(px(6.0)).child(kit::section_label(label, pal)).child(help(
                if never {
                    "rules-never"
                } else {
                    "rules-also"
                },
                about,
                d,
                pal,
                cx,
            )))
            .when(any, |c| c.child(tags))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(kinds)
                    .child(self.text_field(field, placeholder(list), pal, window, cx))
                    .child(if typed {
                        add.on_click(cx.listener(move |this, _, _, cx| this.submit(field, cx)))
                    } else {
                        kit::disabled(add)
                    }),
            )
            .when_some(self.guard_error[side].clone(), |c, e| {
                c.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(12.5))
                        .text_color(pal.danger)
                        .child(glyph(Glyph::CircleAlert, 14.0, pal.danger))
                        .child(e),
                )
            })
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
