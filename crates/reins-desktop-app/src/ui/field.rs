//! One-line text fields: the server under "Other server", the inputs of the Rules lists, the work session form's
//! reason and branches, the phone key's digits in the secrets review, and "Add a host" in the Auto setup. Typed by hand (key presses, backspace, Enter, paste with cmd/ctrl+V): the window has no
//! text input widget.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    ClickEvent, Context, Div, InteractiveElement as _, KeyDownEvent, Keystroke, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use reins_desktop::settings::GuardList;

use super::Root;
use crate::theme::{MONO, Palette};

/// Which field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Server,
    Guard(GuardList),
    /// What the work session is for.
    SessionReason,
    /// The branch to push to, for the work session form's repository at this place (a handful at most).
    SessionBranch(u8),
    /// The twelve digits of the phone's key, in the secrets review (the first time only).
    PhoneKey,
    /// "Add a host" in this computer's Auto setup.
    AutoHost,
}

/// What a key press did to a field's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Changed,
    Submit,
    Ignored,
}

/// The place of a guard list's input.
pub fn slot(list: GuardList) -> usize {
    match list {
        GuardList::Commands => 0,
        GuardList::Files => 1,
        GuardList::AllowCommands => 2,
        GuardList::AllowFiles => 3,
    }
}

/// Applies one key press to `text`; `pasted` is the clipboard's text when the key was cmd/ctrl+V.
pub fn apply(text: &mut String, ks: &Keystroke, pasted: Option<&str>) -> Edit {
    match ks.key.as_str() {
        "backspace" => {
            if text.pop().is_none() {
                return Edit::Ignored;
            }
        }
        "enter" => return Edit::Submit,
        _ if pasted.is_some() => {
            // One line: a pasted newline would end up in the setting.
            let one_line = pasted.unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
            text.push_str(&one_line);
        }
        _ if !ks.modifiers.platform && !ks.modifiers.control => {
            let Some(ch) = ks.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) else {
                return Edit::Ignored;
            };
            text.push_str(ch);
        }
        _ => return Edit::Ignored,
    }
    Edit::Changed
}

impl Root {
    fn focus_of(&self, field: Field) -> &gpui::FocusHandle {
        match field {
            Field::Server => &self.server_field,
            Field::Guard(list) => &self.guard_fields[slot(list)],
            Field::SessionReason => &self.session_reason,
            Field::SessionBranch(i) => self.branch_fields.get(usize::from(i)).unwrap_or(&self.focus),
            Field::PhoneKey => &self.phone_key_field,
            Field::AutoHost => &self.auto_host_field,
        }
    }

    fn text_of(&self, field: Field, cx: &Context<'_, Self>) -> String {
        match field {
            Field::Server => self.model.read(cx).server_input.clone(),
            Field::Guard(list) => self.guard_inputs[slot(list)].clone(),
            Field::SessionReason => {
                self.model.read(cx).work.form.as_ref().map(|f| f.reason.clone()).unwrap_or_default()
            }
            Field::SessionBranch(i) => self
                .model
                .read(cx)
                .work
                .form
                .as_ref()
                .and_then(|f| f.branches.get(usize::from(i)).cloned())
                .unwrap_or_default(),
            Field::PhoneKey => match &self.model.read(cx).secrets {
                crate::secrets::Flow::Review(r) => r.key.as_ref().map(|k| k.typed.clone()).unwrap_or_default(),
                _ => String::new(),
            },
            Field::AutoHost => self.auto_setup.as_ref().map(|s| s.host.clone()).unwrap_or_default(),
        }
    }

    /// Enter in a field (or its button).
    pub(super) fn submit(&mut self, field: Field, cx: &mut Context<'_, Self>) {
        match field {
            Field::Server => self.model.update(cx, crate::model::Model::use_server),
            Field::Guard(list) => {
                // In the list picked, exactly as the guard will read it; refused with why, and kept to fix, when
                // the guard would not read it as typed.
                let side = usize::from(matches!(list, GuardList::AllowCommands | GuardList::AllowFiles));
                if self.guard_inputs[slot(list)].trim().is_empty() {
                    return;
                }
                match super::rules::check_item(list, &self.guard_inputs[slot(list)]) {
                    Ok(item) => {
                        self.guard_inputs[slot(list)].clear();
                        self.guard_error[side] = None;
                        self.model.update(cx, |m, cx| m.add_guard_item(list, &item, cx));
                    }
                    Err(why) => self.guard_error[side] = Some(why),
                }
            }
            // Enter asks the phone, once the form asks for something.
            Field::SessionReason | Field::SessionBranch(_) => {
                self.model.update(cx, crate::model::Model::start_work_session);
            }
            Field::PhoneKey => self.model.update(cx, crate::model::Model::check_phone_key),
            Field::AutoHost => {
                if let Some(s) = self.auto_setup.as_mut() {
                    s.add_host();
                }
            }
        }
        cx.notify();
    }

    fn on_field_key(&mut self, field: Field, ev: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        let ks = &ev.keystroke;
        let pasted = (ks.modifiers.secondary() && ks.key == "v")
            .then(|| cx.read_from_clipboard().and_then(|c| c.text()))
            .flatten();
        let mut text = self.text_of(field, cx);
        match apply(&mut text, ks, pasted.as_deref()) {
            Edit::Changed => {
                match field {
                    Field::Server => self.model.update(cx, |m, _| m.server_input = text),
                    Field::Guard(list) => {
                        self.guard_inputs[slot(list)] = text;
                        self.guard_error
                            [usize::from(matches!(list, GuardList::AllowCommands | GuardList::AllowFiles))] = None;
                    }
                    Field::SessionReason => self.model.update(cx, |m, _| {
                        if let Some(form) = m.work.form.as_mut() {
                            form.reason = text;
                        }
                    }),
                    Field::SessionBranch(i) => self.model.update(cx, |m, _| {
                        if let Some(branch) = m.work.form.as_mut().and_then(|f| f.branches.get_mut(usize::from(i))) {
                            *branch = text;
                        }
                    }),
                    // Digits, and the dashes or spaces they are shown with; never longer than the key's.
                    Field::PhoneKey => self.model.update(cx, |m, cx| {
                        m.review(cx, |r| {
                            if let Some(k) = r.key.as_mut().filter(|k| !k.checking) {
                                k.typed = text
                                    .chars()
                                    .filter(|c| c.is_ascii_digit() || matches!(c, '-' | ' '))
                                    .take(16)
                                    .collect();
                                r.error = None;
                            }
                        });
                    }),
                    Field::AutoHost => {
                        if let Some(s) = self.auto_setup.as_mut() {
                            s.host = text;
                            s.host_refused = false;
                        }
                    }
                }
                cx.notify();
            }
            Edit::Submit => self.submit(field, cx),
            Edit::Ignored => return,
        }
        cx.stop_propagation();
    }

    /// The field itself: mono text, a caret while focused, `placeholder` while empty.
    pub(super) fn text_field(
        &self,
        field: Field,
        placeholder: &str,
        pal: Palette,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let focus = self.focus_of(field).clone();
        let focused = focus.is_focused(window);
        let input = self.text_of(field, cx);
        let shown: SharedString = if input.is_empty() && !focused {
            placeholder.to_owned().into()
        } else {
            input.clone().into()
        };
        let id = match field {
            Field::Server => "server-field".into(),
            Field::Guard(list) => SharedString::from(format!("guard-field-{}", slot(list))),
            Field::SessionReason => "session-reason".into(),
            Field::SessionBranch(i) => SharedString::from(format!("session-branch-{i}")),
            Field::PhoneKey => "secrets-phone-key".into(),
            Field::AutoHost => "auto-host".into(),
        };
        let click_focus = focus.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .flex_1()
            .min_w(px(0.0))
            // The boards' input: 38 px, 12 px corners, paper with a hairline (violet while focused).
            .h(px(38.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(12.0))
            .bg(pal.paper())
            .border_1()
            .border_color(if focused {
                pal.violet()
            } else {
                pal.rule()
            })
            .font_family(MONO)
            .text_size(px(12.5))
            .text_color(if input.is_empty() && !focused {
                pal.ink_faint()
            } else {
                pal.ink()
            })
            .cursor_text()
            .child(div().whitespace_nowrap().text_ellipsis_start().min_w(px(0.0)).child(shown))
            // The caret, drawn.
            .when(focused, |f| f.child(div().flex_none().ml(px(1.0)).w(px(1.5)).h(px(15.0)).bg(pal.violet())))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| window.focus(&click_focus, cx)))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| this.on_field_key(field, ev, cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> Keystroke {
        Keystroke::parse(k).unwrap()
    }

    fn typed(k: &str, ch: &str) -> Keystroke {
        let mut ks = key(k);
        ks.key_char = Some(ch.to_owned());
        ks
    }

    #[test]
    fn typing_editing_and_enter() {
        let mut text = String::new();
        assert_eq!(apply(&mut text, &typed("m", "m"), None), Edit::Changed);
        assert_eq!(apply(&mut text, &typed("space", " "), None), Edit::Changed);
        assert_eq!(apply(&mut text, &typed("shift-a", "A"), None), Edit::Changed);
        assert_eq!(text, "m A");
        assert_eq!(apply(&mut text, &key("backspace"), None), Edit::Changed);
        assert_eq!(text, "m ");
        assert_eq!(apply(&mut text, &key("enter"), None), Edit::Submit);
        assert_eq!(apply(&mut text, &key("ctrl-a"), None), Edit::Ignored);
        assert_eq!(apply(&mut String::new(), &key("backspace"), None), Edit::Ignored);
    }

    #[test]
    fn pastes_are_one_line() {
        let mut text = "make ".to_owned();
        assert_eq!(apply(&mut text, &key("ctrl-v"), Some("deploy\n  --prod\n")), Edit::Changed);
        assert_eq!(text, "make deploy --prod");
    }

    #[test]
    fn every_list_has_its_own_slot() {
        let lists = [GuardList::Commands, GuardList::Files, GuardList::AllowCommands, GuardList::AllowFiles];
        let slots: std::collections::HashSet<_> = lists.into_iter().map(slot).collect();
        assert_eq!(slots.len(), 4);
        assert!(slots.iter().all(|&s| s < 4));
    }
}
