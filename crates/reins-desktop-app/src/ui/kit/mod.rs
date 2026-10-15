//! Reins Signal's components for the desktop (HANDOFF §8, `screens/png/Components.png`), built here and shown in the
//! gallery (`--demo`, `REINS_DEMO_SECTION=gallery`); the screens move onto them in phase 2. Values come from the
//! boards' markup (`screens/source/Components.dc.html`, `Desk-*.dc.html`); the desktop sizes (38 px buttons, 50 px
//! rows) are a step down from the phone's.

#![allow(dead_code, reason = "Signal's foundation: the whole set is here; the screens adopt it in phase 2")]

pub mod button;
pub mod field;
pub mod hold;
pub mod menu;
pub mod parts;

pub use button::{Look, Size, button, disabled, focused, icon_button};
pub use field::field_surface;
pub use hold::{HoldLook, hold_button};
pub use menu::{menu, menu_item, menu_rule, open_menu};
pub use parts::{
    Mode, PillLook, ask_line, empty_state, fact_row, field_label, hairline_row, kbd, key_cap, kind_tile, logo_tile,
    mode_chip, pill, plate, row_lines, section_label, segment, segmented, status_pill, switch, tag, timer,
};
