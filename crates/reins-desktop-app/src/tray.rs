//! The tray (Windows, Linux) or menu bar (macOS) icon: the Reins shield, with a check while Reins is on, pause bars
//! while paused and an exclamation mark while it needs the user (not paired, or the background service is not
//! running). Its menu: the state (opens the status window), a Pause submenu (15 minutes, 1 hour, 4 hours, 24 hours,
//! Until I resume), Resume, Open Reins, Quit Reins.
//!
//! macOS and Windows use the `tray-icon` crate (an `NSStatusItem`, a `Shell_NotifyIcon` icon). Linux uses `ksni`, a
//! StatusNotifierItem on D-Bus (KDE, and GNOME with the AppIndicator extension), which needs no GTK.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    On,
    Paused,
    Attention,
}

use crate::pause::PauseFor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Status,
    Pause(PauseFor),
    Resume,
    Open,
    Quit,
}

impl Action {
    /// Every action, the pause lengths in menu order.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn all() -> impl Iterator<Item = Self> {
        [Self::Status, Self::Resume, Self::Open, Self::Quit]
            .into_iter()
            .chain(PauseFor::ALL.into_iter().map(Self::Pause))
    }

    fn id(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Pause(length) => length.id(),
            Self::Resume => "resume",
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

/// What the tray shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub look: Look,
    /// The first menu line ("Reins is on").
    pub status: String,
    pub can_pause: bool,
    pub can_resume: bool,
}

/// The icon's pixels: RGBA, `width` by `height`.
struct Pixels {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Decodes one of the bundled PNGs (8-bit gray, gray and alpha, RGB or RGBA) to RGBA.
fn decode(png_bytes: &[u8]) -> Result<Pixels, String> {
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

/// The icon for `look`: a template image on macOS (the system colours it for the menu bar), the coloured mark elsewhere.
fn pixels(look: Look) -> Result<Pixels, String> {
    let bytes: &[u8] = match (cfg!(target_os = "macos"), look) {
        (true, Look::On) => include_bytes!("../assets/icons/tray-on.png"),
        (true, Look::Paused) => include_bytes!("../assets/icons/tray-paused.png"),
        (true, Look::Attention) => include_bytes!("../assets/icons/tray-pair.png"),
        (false, Look::On) => include_bytes!("../assets/icons/tray-color-on.png"),
        (false, Look::Paused) => include_bytes!("../assets/icons/tray-color-paused.png"),
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

    use super::{Action, Look, PAUSE_MENU, PauseFor, Shown, pixels};

    pub struct Tray {
        icon: TrayIcon,
        status: MenuItem,
        pause: Submenu,
        resume: MenuItem,
        shown: Option<Shown>,
    }

    fn icon(look: Look) -> Result<Icon, String> {
        let p = pixels(look)?;
        Icon::from_rgba(p.rgba, p.width, p.height).map_err(|e| e.to_string())
    }

    impl Tray {
        /// Must run on the main thread, after the app finished launching.
        pub fn new(actions: UnboundedSender<Action>, _runtime: &tokio::runtime::Handle) -> Result<Self, String> {
            let status = MenuItem::with_id(Action::Status.id(), "Reins", true, None);
            let [quarter, hour, four, day, manual] =
                PauseFor::ALL.map(|p| MenuItem::with_id(Action::Pause(p).id(), p.label(), true, None));
            let pause = Submenu::with_items(PAUSE_MENU, true, &[&quarter, &hour, &four, &day, &manual])
                .map_err(|e| e.to_string())?;
            let resume = MenuItem::with_id(Action::Resume.id(), "Resume", false, None);
            let open = MenuItem::with_id(Action::Open.id(), "Open Reins", true, None);
            let quit = MenuItem::with_id(Action::Quit.id(), "Quit Reins", true, None);
            let menu = Menu::new();
            menu.append_items(&[
                &status,
                &PredefinedMenuItem::separator(),
                &pause,
                &resume,
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
            let first = icon(Look::Attention)?;
            // A template image on macOS: the menu bar colours it.
            #[cfg(target_os = "macos")]
            let builder = builder.with_icon_templated(first);
            #[cfg(not(target_os = "macos"))]
            let builder = builder.with_icon(first);
            let icon = builder.build().map_err(|e| e.to_string())?;
            Ok(Self {
                icon,
                status,
                pause,
                resume,
                shown: None,
            })
        }

        pub fn show(&mut self, shown: &Shown) {
            if self.shown.as_ref() == Some(shown) {
                return;
            }
            if self.shown.as_ref().is_none_or(|s| s.look != shown.look) {
                match icon(shown.look) {
                    Ok(i) => {
                        #[cfg(target_os = "macos")]
                        let set = self.icon.set_icon_templated(Some(i));
                        #[cfg(not(target_os = "macos"))]
                        let set = self.icon.set_icon(Some(i));
                        if let Err(e) = set {
                            log::warn!("tray icon: {e}");
                        }
                    }
                    Err(e) => log::warn!("tray icon: {e}"),
                }
            }
            self.status.set_text(&shown.status);
            self.pause.set_enabled(shown.can_pause);
            self.resume.set_enabled(shown.can_resume);
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

    use super::{Action, Look, PAUSE_MENU, PauseFor, Shown, pixels};

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
            match pixels(self.shown.look) {
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
            vec![
                item(&self.shown.status, Action::Status, true),
                ksni::MenuItem::Separator,
                SubMenu {
                    label: PAUSE_MENU.to_owned(),
                    enabled: self.shown.can_pause,
                    submenu: PauseFor::ALL.into_iter().map(|p| item(p.label(), Action::Pause(p), true)).collect(),
                    ..SubMenu::default()
                }
                .into(),
                item("Resume", Action::Resume, self.shown.can_resume),
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
                    look: Look::Attention,
                    status: "Reins".to_owned(),
                    can_pause: false,
                    can_resume: false,
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

    #[test]
    fn every_icon_decodes() {
        for look in [Look::On, Look::Paused, Look::Attention] {
            let p = pixels(look).unwrap();
            assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
            assert!(p.width >= 32 && p.width == p.height);
            // Something is drawn, and the corners are see-through (or the plate's rounded corner).
            assert!(p.rgba.chunks(4).any(|c| c[3] > 200));
            assert!(p.rgba[3] < 128);
        }
    }

    #[test]
    fn menu_ids_round_trip() {
        let all: Vec<Action> = Action::all().collect();
        assert_eq!(all.len(), 4 + PauseFor::ALL.len());
        for a in all {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
        assert_eq!(Action::from_id("pause-manual"), Some(Action::Pause(PauseFor::Manual)));
        assert_eq!(Action::from_id("pause"), None);
    }
}
