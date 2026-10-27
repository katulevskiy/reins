//! The tray (Windows, Linux) or menu bar (macOS) icon, as the Next menu bar board draws it (`DeskNext-MenuBar.png`):
//! the Reins R while Reins is on (in ink on macOS, with how many wait, or a pause's clock, beside it; on a running mode's
//! colour while one runs), and on the field's colour with a badge in its corner when something needs you: orange with a
//! clock while a request waits on the phone, red with pause bars while git is paused, amber with "!" when Reins needs
//! the user (not paired, or the background service is not running). On macOS the ink R is a template image (the menu bar colours it); the coloured ones are drawn as
//! they are.
//!
//! Native menus cannot be styled, so the menu follows the board in words and order: two status lines (the first opens
//! the window: "Reins is on" over "Dana's iPhone approves", or what waits), "Pause git" with the board's lengths (15
//! minutes, 1 hour, 4 hours, 1 day, then "Until I resume…", which opens the window's pause sheet to be held), Resume,
//! End work session, Open Reins, Quit Reins.
//!
//! macOS and Windows use the `tray-icon` crate (an `NSStatusItem`, a `Shell_NotifyIcon` icon). Linux uses `ksni`, a
//! StatusNotifierItem on D-Bus (KDE, and GNOME with the AppIndicator extension), which needs no GTK.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    On,
    Paused,
    /// A request waits for an answer on the phone.
    Waiting,
    Attention,
}

impl Look {
    #[cfg(test)]
    pub const ALL: [Self; 4] = [Self::On, Self::Paused, Self::Waiting, Self::Attention];
}

use crate::pause::PauseFor;

/// A mode running on the phone: the icon takes its colour (and its clock goes beside it), unless something waits or
/// Reins needs the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeLook {
    Bypass,
    Overnight,
    Lockdown,
}

/// A change of the phone's mode from the menu: a request to the phone, like the window's picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeAct {
    Manual,
    Auto,
    Lockdown,
    /// Ends the running bypass or Overnight.
    Stop,
}

impl ModeAct {
    pub const ALL: [Self; 4] = [Self::Manual, Self::Auto, Self::Lockdown, Self::Stop];

    fn id(self) -> &'static str {
        match self {
            Self::Manual => "mode-manual",
            Self::Auto => "mode-auto",
            Self::Lockdown => "mode-lockdown",
            Self::Stop => "mode-stop",
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Auto => "Auto",
            Self::Lockdown => "Lockdown",
            Self::Stop => "Stop bypass or Overnight",
        }
    }
}

/// The modes submenu's title.
const MODE_MENU: &str = "Phone mode";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Status,
    Mode(ModeAct),
    Pause(PauseFor),
    Resume,
    /// Ends the running work session.
    EndSession,
    Open,
    Quit,
}

impl Action {
    /// Every action, the pause lengths in menu order.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn all() -> impl Iterator<Item = Self> {
        [Self::Status, Self::Resume, Self::EndSession, Self::Open, Self::Quit]
            .into_iter()
            .chain(PauseFor::ALL.into_iter().map(Self::Pause))
            .chain(ModeAct::ALL.into_iter().map(Self::Mode))
    }

    fn id(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Mode(m) => m.id(),
            Self::Pause(length) => length.id(),
            Self::Resume => "resume",
            Self::EndSession => "end-session",
            Self::Open => "open",
            Self::Quit => "quit",
        }
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|a| a.id() == id)
    }
}

/// The Pause submenu's title.
const PAUSE_MENU: &str = "Pause git";
/// The item that ends a work session.
const END_SESSION: &str = "End work session";

/// The menu's two status lines (`Desk-Extras.png`): what Reins is doing, then who approves or what waits.
/// `status` is the window's line (with a work session's time left while one runs); `phone` names the phone that
/// approves; `pause_left` is a pause's time left ("47 min left").
#[must_use]
pub fn status_lines(look: Look, status: String, phone: &str, pause_left: Option<String>) -> (String, String) {
    match look {
        Look::Waiting => match status.split_once(": ") {
            Some((title, what)) => (title.to_owned(), what.to_owned()),
            None => (status, format!("Answer on {phone}")),
        },
        Look::Paused => match pause_left {
            Some(left) => ("git paused".to_owned(), left),
            None => (status, "Pushes go straight to the hosts".to_owned()),
        },
        Look::Attention => (status, "Open Reins to fix it".to_owned()),
        Look::On => (status, format!("{phone} approves")),
    }
}

/// What the tray shows.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent menu items enabled or not")]
pub struct Shown {
    pub look: Look,
    /// Beside the icon, where the platform shows one (macOS): how many wait, a pause's clock.
    pub title: Option<String>,
    /// The first menu line ("Reins is on").
    pub status: String,
    /// The second, under it ("Dana's iPhone approves").
    pub detail: String,
    pub can_pause: bool,
    pub can_resume: bool,
    /// A work session runs: "End work session" ends it.
    pub can_end: bool,
    /// A mode running on the phone (its colour), and whether the modes menu works (paired, nothing asked yet) and
    /// Stop does (a bypass or Overnight runs).
    pub mode: Option<ModeLook>,
    pub can_mode: bool,
    pub can_stop: bool,
}

/// The icon's pixels: RGBA, `width` by `height`.
pub(crate) struct Pixels {
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Decodes one of the bundled PNGs (8-bit gray, gray and alpha, RGB or RGBA) to RGBA.
pub(crate) fn decode(png_bytes: &[u8]) -> Result<Pixels, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("the icon is too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let data = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed icons are not expected after expansion".to_owned()),
    };
    Ok(Pixels {
        rgba,
        width: info.width,
        height: info.height,
    })
}

/// Whether the icon for `look` is a macOS template image: only the ink R; the coloured states keep their colour.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn templated(look: Look) -> bool {
    look == Look::On
}

/// The icon `shown` wears: a running mode's colour over the calm and paused ones; what waits and what needs the user
/// keep theirs.
fn icon_of(shown: &Shown) -> Result<Pixels, String> {
    match (shown.look, shown.mode) {
        (Look::On | Look::Paused, Some(mode)) => mode_pixels(mode),
        (look, _) => pixels(look),
    }
}

/// Whether `shown`'s icon is the template image.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn templated_shown(shown: &Shown) -> bool {
    templated(shown.look) && shown.mode.is_none()
}

/// A running mode's icon: the R on its field's colour.
fn mode_pixels(mode: ModeLook) -> Result<Pixels, String> {
    let bytes: &[u8] = match (cfg!(target_os = "macos"), mode) {
        (true, ModeLook::Bypass) => include_bytes!("../assets/icons/tray-bypass.png"),
        (true, ModeLook::Overnight) => include_bytes!("../assets/icons/tray-overnight.png"),
        (true, ModeLook::Lockdown) => include_bytes!("../assets/icons/tray-lockdown.png"),
        (false, ModeLook::Bypass) => include_bytes!("../assets/icons/tray-color-bypass.png"),
        (false, ModeLook::Overnight) => include_bytes!("../assets/icons/tray-color-overnight.png"),
        (false, ModeLook::Lockdown) => include_bytes!("../assets/icons/tray-color-lockdown.png"),
    };
    decode(bytes)
}

/// The icon for `look`: on macOS the menu bar's size (the ink R a template image, the rest coloured plates), the
/// tray's elsewhere.
fn pixels(look: Look) -> Result<Pixels, String> {
    let bytes: &[u8] = match (cfg!(target_os = "macos"), look) {
        (true, Look::On) => include_bytes!("../assets/icons/tray-on.png"),
        (true, Look::Paused) => include_bytes!("../assets/icons/tray-paused.png"),
        (true, Look::Waiting) => include_bytes!("../assets/icons/tray-waiting.png"),
        (true, Look::Attention) => include_bytes!("../assets/icons/tray-pair.png"),
        (false, Look::On) => include_bytes!("../assets/icons/tray-color-on.png"),
        (false, Look::Paused) => include_bytes!("../assets/icons/tray-color-paused.png"),
        (false, Look::Waiting) => include_bytes!("../assets/icons/tray-color-waiting.png"),
        (false, Look::Attention) => include_bytes!("../assets/icons/tray-color-pair.png"),
    };
    decode(bytes)
}

pub use imp::Tray;

#[cfg(any(target_os = "macos", windows))]
mod imp {
    use futures::channel::mpsc::UnboundedSender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::{Action, END_SESSION, Look, MODE_MENU, ModeAct, PAUSE_MENU, PauseFor, Shown, icon_of, pixels};

    pub struct Tray {
        icon: TrayIcon,
        modes: Submenu,
        stop: MenuItem,
        status: MenuItem,
        detail: MenuItem,
        pause: Submenu,
        resume: MenuItem,
        end: MenuItem,
        shown: Option<Shown>,
    }

    fn icon(look: Look) -> Result<Icon, String> {
        let p = pixels(look)?;
        Icon::from_rgba(p.rgba, p.width, p.height).map_err(|e| e.to_string())
    }

    fn icon_for(shown: &Shown) -> Result<Icon, String> {
        let p = icon_of(shown)?;
        Icon::from_rgba(p.rgba, p.width, p.height).map_err(|e| e.to_string())
    }

    impl Tray {
        /// Must run on the main thread, after the app finished launching.
        pub fn new(actions: UnboundedSender<Action>, _runtime: &tokio::runtime::Handle) -> Result<Self, String> {
            let status = MenuItem::with_id(Action::Status.id(), "Reins", true, None);
            // The second status line says more; it does nothing.
            let detail = MenuItem::new("", false, None);
            let [quarter, hour, four, day, manual] =
                PauseFor::ALL.map(|p| MenuItem::with_id(Action::Pause(p).id(), p.label(), true, None));
            let pause = Submenu::with_items(
                PAUSE_MENU,
                true,
                &[&quarter, &hour, &four, &day, &PredefinedMenuItem::separator(), &manual],
            )
            .map_err(|e| e.to_string())?;
            let [manual, auto, lockdown] = [ModeAct::Manual, ModeAct::Auto, ModeAct::Lockdown]
                .map(|m| MenuItem::with_id(Action::Mode(m).id(), m.label(), true, None));
            let stop = MenuItem::with_id(Action::Mode(ModeAct::Stop).id(), ModeAct::Stop.label(), false, None);
            let modes =
                Submenu::with_items(MODE_MENU, false, &[&manual, &auto, &PredefinedMenuItem::separator(), &lockdown])
                    .map_err(|e| e.to_string())?;
            let resume = MenuItem::with_id(Action::Resume.id(), "Resume", false, None);
            let end = MenuItem::with_id(Action::EndSession.id(), END_SESSION, false, None);
            let open = MenuItem::with_id(Action::Open.id(), "Open Reins", true, None);
            let quit = MenuItem::with_id(Action::Quit.id(), "Quit Reins", true, None);
            let menu = Menu::new();
            menu.append_items(&[
                &status,
                &detail,
                &PredefinedMenuItem::separator(),
                &modes,
                &stop,
                &PredefinedMenuItem::separator(),
                &pause,
                &resume,
                &end,
                &PredefinedMenuItem::separator(),
                &open,
                &quit,
            ])
            .map_err(|e| e.to_string())?;
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(action) = Action::from_id(event.id.0.as_str()) {
                    let _closed = actions.unbounded_send(action);
                }
            }));
            let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Reins");
            // Amber until the first status: Reins does not know yet that all is well.
            let builder = builder.with_icon(icon(Look::Attention)?);
            let icon = builder.build().map_err(|e| e.to_string())?;
            Ok(Self {
                icon,
                modes,
                stop,
                status,
                detail,
                pause,
                resume,
                end,
                shown: None,
            })
        }

        pub fn show(&mut self, shown: &Shown) {
            if self.shown.as_ref() == Some(shown) {
                return;
            }
            if self.shown.as_ref().is_none_or(|s| (s.look, s.mode) != (shown.look, shown.mode)) {
                match icon_for(shown) {
                    Ok(i) => {
                        // The ink R is a template image on macOS (the menu bar colours it); the coloured
                        // states are drawn as they are.
                        #[cfg(target_os = "macos")]
                        let set = if super::templated_shown(shown) {
                            self.icon.set_icon_templated(Some(i))
                        } else {
                            self.icon.set_icon(Some(i))
                        };
                        #[cfg(not(target_os = "macos"))]
                        let set = self.icon.set_icon(Some(i));
                        if let Err(e) = set {
                            log::warn!("tray icon: {e}");
                        }
                    }
                    Err(e) => log::warn!("tray icon: {e}"),
                }
            }
            #[cfg(target_os = "macos")]
            self.icon.set_title(shown.title.as_deref());
            self.status.set_text(&shown.status);
            self.detail.set_text(&shown.detail);
            self.pause.set_enabled(shown.can_pause);
            self.resume.set_enabled(shown.can_resume);
            self.end.set_enabled(shown.can_end);
            self.modes.set_enabled(shown.can_mode);
            self.stop.set_enabled(shown.can_stop);
            if let Err(e) = self.icon.set_tooltip(Some(format!("Reins: {}", shown.status))) {
                log::warn!("tray tooltip: {e}");
            }
            self.shown = Some(shown.clone());
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use futures::channel::mpsc::UnboundedSender;
    use ksni::TrayMethods as _;

    use super::{Action, END_SESSION, Look, MODE_MENU, ModeAct, PAUSE_MENU, PauseFor, Shown, icon_of};

    struct Item {
        shown: Shown,
        actions: UnboundedSender<Action>,
    }

    impl Item {
        fn send(&self, action: Action) {
            let _closed = self.actions.unbounded_send(action);
        }
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            "reins".to_owned()
        }

        fn title(&self) -> String {
            "Reins".to_owned()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            match icon_of(&self.shown) {
                // StatusNotifierItem wants ARGB32 in network byte order.
                Ok(p) => vec![ksni::Icon {
                    width: i32::try_from(p.width).unwrap_or(0),
                    height: i32::try_from(p.height).unwrap_or(0),
                    data: p.rgba.chunks(4).flat_map(|c| [c[3], c[0], c[1], c[2]]).collect(),
                }],
                Err(e) => {
                    log::warn!("tray icon: {e}");
                    Vec::new()
                }
            }
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: format!("Reins: {}", self.shown.status),
                ..ksni::ToolTip::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Action::Open);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{StandardItem, SubMenu};
            let item = |label: &str, action: Action, enabled: bool| -> ksni::MenuItem<Self> {
                StandardItem {
                    label: label.to_owned(),
                    enabled,
                    activate: Box::new(move |this: &mut Self| this.send(action)),
                    ..StandardItem::default()
                }
                .into()
            };
            let mut lengths: Vec<ksni::MenuItem<Self>> = PauseFor::ALL
                .into_iter()
                .filter(|p| *p != PauseFor::Manual)
                .map(|p| item(p.label(), Action::Pause(p), true))
                .collect();
            lengths.push(ksni::MenuItem::Separator);
            lengths.push(item(PauseFor::Manual.label(), Action::Pause(PauseFor::Manual), true));
            vec![
                item(&self.shown.status, Action::Status, true),
                // The second status line says more; it does nothing.
                StandardItem {
                    label: self.shown.detail.clone(),
                    enabled: false,
                    ..StandardItem::default()
                }
                .into(),
                ksni::MenuItem::Separator,
                SubMenu {
                    label: MODE_MENU.to_owned(),
                    enabled: self.shown.can_mode,
                    submenu: vec![
                        item(ModeAct::Manual.label(), Action::Mode(ModeAct::Manual), true),
                        item(ModeAct::Auto.label(), Action::Mode(ModeAct::Auto), true),
                        ksni::MenuItem::Separator,
                        item(ModeAct::Lockdown.label(), Action::Mode(ModeAct::Lockdown), true),
                    ],
                    ..SubMenu::default()
                }
                .into(),
                item(ModeAct::Stop.label(), Action::Mode(ModeAct::Stop), self.shown.can_stop),
                ksni::MenuItem::Separator,
                SubMenu {
                    label: PAUSE_MENU.to_owned(),
                    enabled: self.shown.can_pause,
                    submenu: lengths,
                    ..SubMenu::default()
                }
                .into(),
                item("Resume", Action::Resume, self.shown.can_resume),
                item(END_SESSION, Action::EndSession, self.shown.can_end),
                ksni::MenuItem::Separator,
                item("Open Reins", Action::Open, true),
                item("Quit Reins", Action::Quit, true),
            ]
        }
    }

    pub struct Tray {
        handle: Option<ksni::Handle<Item>>,
        runtime: tokio::runtime::Handle,
        shown: Option<Shown>,
    }

    impl Tray {
        pub fn new(actions: UnboundedSender<Action>, runtime: &tokio::runtime::Handle) -> Result<Self, String> {
            let item = Item {
                shown: Shown {
                    title: None,
                    look: Look::Attention,
                    status: "Reins".to_owned(),
                    detail: String::new(),
                    can_pause: false,
                    can_resume: false,
                    can_end: false,
                    mode: None,
                    can_mode: false,
                    can_stop: false,
                },
                actions,
            };
            // No StatusNotifierWatcher (a desktop without a tray) is not an error: Reins then lives in its window.
            let handle = runtime.block_on(item.spawn()).map_err(|e| log::warn!("no tray on this desktop: {e}")).ok();
            Ok(Self {
                handle,
                runtime: runtime.clone(),
                shown: None,
            })
        }

        pub fn show(&mut self, shown: &Shown) {
            if self.shown.as_ref() == Some(shown) {
                return;
            }
            if let Some(handle) = self.handle.clone() {
                let shown = shown.clone();
                self.runtime.spawn(async move {
                    handle.update(move |item| item.shown = shown).await;
                });
            }
            self.shown = Some(shown.clone());
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod imp {
    pub struct Tray;

    impl Tray {
        pub fn new(
            _actions: futures::channel::mpsc::UnboundedSender<super::Action>,
            _runtime: &tokio::runtime::Handle,
        ) -> Result<Self, String> {
            Err("no tray icon on this system".to_owned())
        }

        pub fn show(&mut self, _shown: &super::Shown) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both sets, whatever this computer shows: the macOS template images and the coloured ones.
    const PNGS: [(&str, &[u8]); 8] = [
        ("tray-on", include_bytes!("../assets/icons/tray-on.png")),
        ("tray-paused", include_bytes!("../assets/icons/tray-paused.png")),
        ("tray-waiting", include_bytes!("../assets/icons/tray-waiting.png")),
        ("tray-pair", include_bytes!("../assets/icons/tray-pair.png")),
        ("tray-color-on", include_bytes!("../assets/icons/tray-color-on.png")),
        ("tray-color-paused", include_bytes!("../assets/icons/tray-color-paused.png")),
        ("tray-color-waiting", include_bytes!("../assets/icons/tray-color-waiting.png")),
        ("tray-color-pair", include_bytes!("../assets/icons/tray-color-pair.png")),
    ];

    #[test]
    fn every_icon_decodes() {
        for look in Look::ALL {
            let p = pixels(look).unwrap();
            assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
            assert!(p.width >= 32 && p.width == p.height);
            // Something is drawn, and the corners are see-through (or the plate's rounded corner).
            assert!(p.rgba.chunks(4).any(|c| c[3] > 200));
            assert!(p.rgba[3] < 128);
        }
        // Every set's icons have the same size, and the waiting ones differ from the others.
        let decoded: Vec<(&str, Pixels)> = PNGS.iter().map(|(name, bytes)| (*name, decode(bytes).unwrap())).collect();
        for (name, p) in &decoded {
            let set = if name.contains("color") {
                "tray-color-on"
            } else {
                "tray-on"
            };
            let reference = &decoded.iter().find(|(n, _)| *n == set).unwrap().1;
            assert_eq!((p.width, p.height), (reference.width, reference.height), "{name}");
            if name.ends_with("waiting") {
                assert_ne!(p.rgba, reference.rgba, "{name}");
            }
        }
        // The template image is one colour (the menu bar colours it): only its alpha differs. The other states keep
        // their field's colour.
        let template = &decoded.iter().find(|(n, _)| *n == "tray-on").unwrap().1;
        assert!(template.rgba.chunks(4).filter(|c| c[3] > 0).all(|c| c[..3] == [0, 0, 0]));
        for (name, p) in decoded.iter().filter(|(n, _)| *n != "tray-on") {
            assert!(p.rgba.chunks(4).any(|c| c[3] == 255 && c[..3] != [0, 0, 0]), "{name} has its colour");
        }
        assert!(templated(Look::On) && !templated(Look::Waiting) && !templated(Look::Paused));
    }

    #[test]
    fn a_running_mode_colours_the_icon_unless_something_needs_the_user() {
        let shown = |look, mode| Shown {
            look,
            title: None,
            status: String::new(),
            detail: String::new(),
            can_pause: false,
            can_resume: false,
            can_end: false,
            mode,
            can_mode: true,
            can_stop: false,
        };
        let bypass = icon_of(&shown(Look::On, Some(ModeLook::Bypass))).unwrap();
        assert_ne!(bypass.rgba, pixels(Look::On).unwrap().rgba);
        assert!(!templated_shown(&shown(Look::On, Some(ModeLook::Bypass))));
        assert!(templated_shown(&shown(Look::On, None)));
        let waiting = icon_of(&shown(Look::Waiting, Some(ModeLook::Lockdown))).unwrap();
        assert_eq!(waiting.rgba, pixels(Look::Waiting).unwrap().rgba, "what waits keeps its colour");
        for mode in [ModeLook::Bypass, ModeLook::Overnight, ModeLook::Lockdown] {
            let p = mode_pixels(mode).unwrap();
            assert_eq!(p.width, pixels(Look::On).unwrap().width);
            assert!(p.rgba.chunks(4).any(|c| c[3] == 255 && c[..3] != [0, 0, 0]));
        }
    }

    #[test]
    fn the_status_says_two_lines() {
        let two = |look, status: &str, left: Option<&str>| {
            status_lines(look, status.to_owned(), "Dana's iPhone", left.map(str::to_owned))
        };
        assert_eq!(two(Look::On, "Reins is on", None), ("Reins is on".to_owned(), "Dana's iPhone approves".to_owned()));
        assert_eq!(
            two(Look::Waiting, "Waiting on your phone: push to acme/web (main)", None),
            ("Waiting on your phone".to_owned(), "push to acme/web (main)".to_owned())
        );
        assert_eq!(two(Look::Paused, "Paused for 47 min more", Some("47 min left")).0, "git paused");
        assert_eq!(two(Look::Paused, "Paused: git goes direct", None).0, "Paused: git goes direct");
        assert_eq!(two(Look::Attention, "Not paired", None).1, "Open Reins to fix it");
    }

    /// The board's lengths: a day is "1 day", and "Until I resume…" (held in the window) comes last.
    #[test]
    fn the_pause_menu_follows_the_board() {
        let labels: Vec<&str> = PauseFor::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels, ["15 minutes", "1 hour", "4 hours", "1 day", "Until I resume…"]);
    }

    #[test]
    fn menu_ids_round_trip() {
        let all: Vec<Action> = Action::all().collect();
        assert_eq!(all.len(), 5 + PauseFor::ALL.len() + ModeAct::ALL.len());
        for a in all {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
        assert_eq!(Action::from_id("pause-manual"), Some(Action::Pause(PauseFor::Manual)));
        assert_eq!(Action::from_id("end-session"), Some(Action::EndSession));
        assert_eq!(Action::from_id("pause"), None);
    }
}
