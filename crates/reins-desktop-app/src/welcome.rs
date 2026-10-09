//! The welcome flow: 1 Pair · 2 AI tools · 3 Turn on · Done. Which step a start resumes at, and the AI tools step's
//! rows (installed or not, connected or not, connected just now and so undoable).

use reins_desktop::harness::Harness;
use serde::{Deserialize, Serialize};

/// A step of the welcome flow. `app.json` keeps the one the setup reached, so quitting half-way resumes there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Pair this computer with the phone (a QR code, or the browser sign-in).
    #[default]
    Pair,
    /// Connect the AI tools found here.
    Tools,
    /// Start the background service, send git through it, open at login.
    TurnOn,
    /// Reins is on: send a test, open the status window.
    Done,
}

impl Stage {
    pub const ALL: [Self; 4] = [Self::Pair, Self::Tools, Self::TurnOn, Self::Done];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Pair => "Pair",
            Self::Tools => "AI tools",
            Self::TurnOn => "Turn on",
            Self::Done => "Done",
        }
    }

    /// The number the step indicator shows (`Done` shows a check instead).
    #[must_use]
    pub fn number(self) -> Option<usize> {
        match self {
            Self::Pair => Some(1),
            Self::Tools => Some(2),
            Self::TurnOn => Some(3),
            Self::Done => None,
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Pair => Self::Tools,
            Self::Tools => Self::TurnOn,
            Self::TurnOn | Self::Done => Self::Done,
        }
    }

    /// Where "Back" goes (pairing again is not a step back: it is "Disconnect" in Settings).
    #[must_use]
    pub fn back(self) -> Option<Self> {
        match self {
            Self::TurnOn => Some(Self::Tools),
            Self::Pair | Self::Tools | Self::Done => None,
        }
    }

    /// Where a start opens: a step of the welcome flow, or `None` for the status window. `saved` is the step the setup
    /// reached last time.
    #[must_use]
    pub fn resume(paired: bool, setup_done: bool, saved: Self) -> Option<Self> {
        if !paired {
            Some(Self::Pair)
        } else if setup_done {
            None
        } else {
            Some(match saved {
                Self::Pair | Self::Tools => Self::Tools,
                // Done is reached only with the setup done: turning on did not finish.
                Self::TurnOn | Self::Done => Self::TurnOn,
            })
        }
    }
}

/// One AI tool on the "Connect your AI tools" step.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent flags the row shows")]
pub struct ToolRow {
    pub harness: Harness,
    pub installed: bool,
    /// Reins is fully in its settings (its MCP server and its hook).
    pub connected: bool,
    /// Connected on this step just now: Undo takes it out again.
    pub undoable: bool,
    /// Why connecting (or undoing) did not work.
    pub error: Option<String>,
    /// Being connected or undone (the demo takes a moment).
    pub busy: bool,
}

impl ToolRow {
    #[must_use]
    pub fn new(harness: Harness, installed: bool, connected: bool) -> Self {
        Self {
            harness,
            installed,
            connected,
            undoable: false,
            error: None,
            busy: false,
        }
    }

    /// Installed and not connected yet.
    #[must_use]
    pub fn can_connect(&self) -> bool {
        self.installed && !self.connected && !self.busy
    }
}

/// The rows in the order shown: installed ones first, otherwise as Reins lists the harnesses.
pub fn sort(rows: &mut [ToolRow]) {
    rows.sort_by_key(|r| (!(r.installed || r.connected), Harness::ALL.iter().position(|h| *h == r.harness)));
}

/// What "Connect all" connects.
#[must_use]
pub fn connectable(rows: &[ToolRow]) -> Vec<Harness> {
    rows.iter().filter(|r| r.can_connect()).map(|r| r.harness).collect()
}

/// A line under the list: how many are connected, or that none is installed.
#[must_use]
pub fn tools_line(rows: &[ToolRow]) -> String {
    let installed = rows.iter().filter(|r| r.installed || r.connected).count();
    let connected = rows.iter().filter(|r| r.connected).count();
    match (installed, connected) {
        (0, _) => "No AI tool found on this computer. Install one later and connect it under Connections.".to_owned(),
        (n, c) if c == n => {
            if n == 1 {
                "Connected. Reins watches over it from now on.".to_owned()
            } else {
                format!("All {n} connected. Reins watches over them from now on.")
            }
        }
        (n, 0) => format!("{n} found. Connect the ones Reins should watch over."),
        (n, c) => format!("{c} of {n} connected."),
    }
}

/// The AI tools connected, for the Done step ("Claude Code and Codex").
#[must_use]
pub fn connected_names(rows: &[ToolRow]) -> Option<String> {
    let names: Vec<&str> = rows.iter().filter(|r| r.connected).map(|r| r.harness.label()).collect();
    match names.as_slice() {
        [] => None,
        [one] => Some((*one).to_owned()),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_in_order() {
        assert_eq!(Stage::Pair.next(), Stage::Tools);
        assert_eq!(Stage::Tools.next(), Stage::TurnOn);
        assert_eq!(Stage::TurnOn.next(), Stage::Done);
        assert_eq!(Stage::Done.next(), Stage::Done);
        assert_eq!(Stage::TurnOn.back(), Some(Stage::Tools));
        assert_eq!(Stage::Tools.back(), None);
        let numbers: Vec<_> = Stage::ALL.iter().map(|s| s.number()).collect();
        assert_eq!(numbers, vec![Some(1), Some(2), Some(3), None]);
        assert!(Stage::ALL.windows(2).all(|w| w[0] < w[1] && w[0].next() == w[1]));
    }

    #[test]
    fn a_start_resumes_where_the_setup_was() {
        // Not paired: always the first step, whatever was saved.
        assert_eq!(Stage::resume(false, true, Stage::TurnOn), Some(Stage::Pair));
        assert_eq!(Stage::resume(false, false, Stage::Tools), Some(Stage::Pair));
        // Paired and set up: the status window.
        assert_eq!(Stage::resume(true, true, Stage::Tools), None);
        // Paired, quit half-way.
        assert_eq!(Stage::resume(true, false, Stage::Pair), Some(Stage::Tools));
        assert_eq!(Stage::resume(true, false, Stage::Tools), Some(Stage::Tools));
        assert_eq!(Stage::resume(true, false, Stage::TurnOn), Some(Stage::TurnOn));
        assert_eq!(Stage::resume(true, false, Stage::Done), Some(Stage::TurnOn));
    }

    #[test]
    fn stages_are_saved_by_name() {
        assert_eq!(serde_json::to_string(&Stage::TurnOn).unwrap(), "\"turn_on\"");
        assert_eq!(serde_json::from_str::<Stage>("\"tools\"").unwrap(), Stage::Tools);
    }

    #[test]
    fn installed_tools_come_first_and_connect_all_takes_the_rest() {
        let mut rows = vec![
            ToolRow::new(Harness::ClaudeCode, false, false),
            ToolRow::new(Harness::Codex, true, true),
            ToolRow::new(Harness::Gemini, true, false),
            ToolRow::new(Harness::Cursor, false, false),
        ];
        sort(&mut rows);
        let order: Vec<_> = rows.iter().map(|r| r.harness).collect();
        assert_eq!(order, vec![Harness::Codex, Harness::Gemini, Harness::ClaudeCode, Harness::Cursor]);
        assert_eq!(connectable(&rows), vec![Harness::Gemini]);
        assert_eq!(tools_line(&rows), "1 of 2 connected.");
        assert_eq!(connected_names(&rows).as_deref(), Some("Codex"));
        rows[1].connected = true;
        assert!(connectable(&rows).is_empty());
        assert_eq!(tools_line(&rows), "All 2 connected. Reins watches over them from now on.");
        assert_eq!(connected_names(&rows).as_deref(), Some("Codex and Gemini CLI"));
        rows[1].connected = false;
        rows[1].busy = true;
        assert!(connectable(&rows).is_empty(), "busy rows are not connected twice");
    }

    #[test]
    fn the_line_says_when_nothing_is_installed() {
        let rows: Vec<ToolRow> = Harness::ALL.into_iter().map(|h| ToolRow::new(h, false, false)).collect();
        assert!(tools_line(&rows).starts_with("No AI tool found"));
        assert_eq!(connected_names(&rows), None);
        let mut rows = rows;
        rows[0].installed = true;
        rows[1].installed = true;
        assert_eq!(tools_line(&rows), "2 found. Connect the ones Reins should watch over.");
    }
}
