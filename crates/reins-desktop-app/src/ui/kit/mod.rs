//! Reins Next's components for the desktop (`design/next/desktop`, the System board): buttons, panes of rows and
//! their section lines, fields, holds, menus, the wallpaper, and the smaller parts; shown together in the gallery
//! (`--demo`, `REINS_DEMO_SECTION=gallery`). Values come from the boards' markup (`DeskNext-*.dc.html`).

pub mod button;
pub mod field;
pub mod hold;
pub mod menu;
pub mod pane;
pub mod parts;
pub mod wallpaper;

pub use button::{Look, Size, button, disabled, focused, icon_button};
pub use field::field_surface;
pub use hold::{HoldLook, hold_button};
pub use menu::{menu, menu_item, menu_rule, open_menu};
pub use pane::{Glass, group, pane, pane_row, section, section_count, title_sub};
pub use parts::{
    Mode, PillLook, ask_line, empty_state, fact_row, field_label, hairline_row, kbd, kind_tile, mode_chip, pill, plate,
    request_icon, row_lines, section_label, segment, segmented, status_pill, switch, tag, timer,
};
pub use wallpaper::Wallpaper;
