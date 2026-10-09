//! Keyboard shortcuts: Cmd (macOS) or Ctrl (elsewhere) with 1…6 for the sections, R to check and read everything
//! again, W to close the window (Reins stays in the tray), Q to quit; Esc closes what is open.

use gpui::Keystroke;

use crate::model::Section;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Section(Section),
    /// Read the status again and run the checks.
    Refresh,
    /// Close the window; Reins stays in the tray or menu bar.
    Close,
    Quit,
    /// Close what is open (an activity row, a menu, the phone's QR code).
    Escape,
}

/// The shortcut `ks` is, if any.
#[must_use]
pub fn of(ks: &Keystroke) -> Option<Shortcut> {
    let m = &ks.modifiers;
    if ks.key == "escape" {
        return (!m.modified()).then_some(Shortcut::Escape);
    }
    // Cmd on macOS, Ctrl elsewhere, and nothing else.
    if !m.secondary() || m.alt || m.shift || m.function || (m.control && m.platform) {
        return None;
    }
    match ks.key.as_str() {
        "r" => Some(Shortcut::Refresh),
        "w" => Some(Shortcut::Close),
        "q" => Some(Shortcut::Quit),
        key => {
            let n: usize = key.parse().ok()?;
            Section::ALL.get(n.checked_sub(1)?).copied().map(Shortcut::Section)
        }
    }
}

/// How a shortcut is written on this computer: "⌘1" on macOS, "Ctrl+1" elsewhere.
#[must_use]
pub fn hint(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{}", key.to_ascii_uppercase())
    } else {
        format!("Ctrl+{}", key.to_ascii_uppercase())
    }
}

/// The list Settings shows: the keys and what they do.
#[must_use]
pub fn list() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Section::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| (hint(&(i + 1).to_string()), format!("Go to {}", s.label())))
        .collect();
    out.push((hint("r"), "Check again and refresh".to_owned()));
    out.push((hint("w"), "Close the window (Reins keeps running)".to_owned()));
    out.push((hint("q"), "Quit Reins".to_owned()));
    out.push(("Esc".to_owned(), "Close an open activity row".to_owned()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> Keystroke {
        Keystroke::parse(k).unwrap()
    }

    /// Cmd on macOS, Ctrl elsewhere.
    fn primary(k: &str) -> Keystroke {
        key(&format!(
            "{}-{k}",
            if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            }
        ))
    }

    #[test]
    fn numbers_go_to_the_sections() {
        for (i, s) in Section::ALL.into_iter().enumerate() {
            assert_eq!(of(&primary(&(i + 1).to_string())), Some(Shortcut::Section(s)));
        }
        assert_eq!(of(&primary("7")), None);
        assert_eq!(of(&primary("0")), None);
        assert_eq!(of(&key("1")), None, "a plain digit is typing");
    }

    #[test]
    fn refresh_close_quit_and_escape() {
        assert_eq!(of(&primary("r")), Some(Shortcut::Refresh));
        assert_eq!(of(&primary("w")), Some(Shortcut::Close));
        assert_eq!(of(&primary("q")), Some(Shortcut::Quit));
        assert_eq!(of(&key("escape")), Some(Shortcut::Escape));
        assert_eq!(of(&key("shift-escape")), None);
        assert_eq!(of(&key("r")), None);
        assert_eq!(of(&primary("v")), None, "paste stays with the text fields");
        let shifted = if cfg!(target_os = "macos") {
            "cmd-shift-r"
        } else {
            "ctrl-shift-r"
        };
        assert_eq!(of(&key(shifted)), None);
        assert_eq!(of(&key("alt-1")), None);
    }

    #[test]
    fn the_list_names_every_shortcut() {
        let list = list();
        assert_eq!(list.len(), Section::ALL.len() + 4);
        assert!(list[0].0.ends_with('1') && list[0].1 == "Go to Overview");
        assert!(list.iter().all(|(k, what)| !k.is_empty() && !what.is_empty()));
    }
}
