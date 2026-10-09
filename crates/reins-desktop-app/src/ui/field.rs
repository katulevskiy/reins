//! One-line text fields: the server under "Use another server", and the inputs of the Rules lists. Typed by hand
//! (key presses, backspace, Enter, paste with cmd/ctrl+V): the window has no text input widget.

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
        }
    }

    fn text_of(&self, field: Field, cx: &Context<'_, Self>) -> String {
        match field {
            Field::Server => self.model.read(cx).server_input.clone(),
            Field::Guard(list) => self.guard_inputs[slot(list)].clone(),
        }
    }

    /// Enter in a field (or its button).
    pub(super) fn submit(&mut self, field: Field, cx: &mut Context<'_, Self>) {
        match field {
            Field::Server => self.model.update(cx, crate::model::Model::use_server),
            Field::Guard(list) => {
                let typed = std::mem::take(&mut self.guard_inputs[slot(list)]);
                self.model.update(cx, |m, cx| m.add_guard_item(list, &typed, cx));
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
                    Field::Guard(list) => self.guard_inputs[slot(list)] = text,
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
        } else if focused {
            format!("{input}▏").into()
        } else {
            input.clone().into()
        };
        let id = match field {
            Field::Server => "server-field".into(),
            Field::Guard(list) => SharedString::from(format!("guard-field-{}", slot(list))),
        };
        let click_focus = focus.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .flex_1()
            .min_w(px(0.0))
            .h(px(32.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(8.0))
            .bg(pal.elevated)
            .border_1()
            .border_color(if focused {
                pal.accent
            } else {
                pal.hairline
            })
            .font_family(MONO)
            .text_size(px(12.0))
            .text_color(if input.is_empty() && !focused {
                pal.tertiary
            } else {
                pal.text
            })
            .cursor_text()
            .child(div().whitespace_nowrap().text_ellipsis_start().min_w(px(0.0)).child(shown))
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
