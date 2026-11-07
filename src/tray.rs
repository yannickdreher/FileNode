use anyhow::Result;
use eframe::egui;
use noritec_drive::i18n::{text, Language};
use std::sync::mpsc;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Show(Option<egui::Pos2>),
    Settings,
    ToggleSync,
    Check,
    OpenFolder,
    Quit,
}

pub fn popup_position(context: &egui::Context, anchor: egui::Pos2) -> egui::Pos2 {
    let size = context
        .input(|input| input.viewport().monitor_size)
        .unwrap_or(egui::vec2(1920.0, 1080.0));
    #[allow(unused_mut)]
    let mut bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{
                GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
            },
        };
        let scale = context.pixels_per_point();
        let point = POINT {
            x: (anchor.x * scale) as i32,
            y: (anchor.y * scale) as i32,
        };
        // The native work area includes negative monitor coordinates and excludes the taskbar.
        unsafe {
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                bounds = egui::Rect::from_min_max(
                    egui::pos2(
                        info.rcWork.left as f32 / scale,
                        info.rcWork.top as f32 / scale,
                    ),
                    egui::pos2(
                        info.rcWork.right as f32 / scale,
                        info.rcWork.bottom as f32 / scale,
                    ),
                );
            }
        }
    }
    // Also remains reachable when the tray sits on the left or top edge.
    egui::pos2(
        (anchor.x - 400.0).clamp(bounds.left(), (bounds.right() - 410.0).max(bounds.left())),
        (anchor.y - 610.0).clamp(bounds.top(), (bounds.bottom() - 600.0).max(bounds.top())),
    )
}

pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    settings: MenuItem,
    pause: MenuItem,
    check: MenuItem,
    open: MenuItem,
    quit: MenuItem,
    last_state: Option<(Language, bool, String)>,
}

/// A small original cloud mark, shared by window and notification area icons.
pub fn icon_pixels(color: [u8; 3]) -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0i32..32 {
        for x in 0i32..32 {
            let inside = (x - 11).pow(2) + (y - 18).pow(2) <= 7i32.pow(2)
                || (x - 18).pow(2) + (y - 13).pow(2) <= 9i32.pow(2)
                || (x - 25).pow(2) + (y - 19).pow(2) <= 6i32.pow(2)
                || (7..=26).contains(&x) && (18..=24).contains(&y);
            if inside {
                let index = ((y * 32 + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            }
        }
    }
    pixels
}

impl Tray {
    pub fn new(
        ctx: &egui::Context,
        sender: mpsc::Sender<Command>,
        language: Language,
    ) -> Result<Self> {
        // Do not hide the only window if the desktop cannot present the tray icon.
        #[cfg(target_os = "linux")]
        {
            let connection = zbus::blocking::Connection::session()?;
            let host: bool = connection
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "NameHasOwner",
                    &("org.kde.StatusNotifierWatcher",),
                )?
                .body()
                .deserialize()?;
            anyhow::ensure!(
                host,
                "No StatusNotifier host is available; keep the desktop window visible"
            );
            let registered: zbus::zvariant::OwnedValue = connection
                .call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &(
                        "org.kde.StatusNotifierWatcher",
                        "IsStatusNotifierHostRegistered",
                    ),
                )?
                .body()
                .deserialize()?;
            anyhow::ensure!(
                bool::try_from(registered)?,
                "StatusNotifier has no registered host; keep the desktop window visible"
            );
        }
        let show = MenuItem::with_id("show", text(language, "tray.show"), true, None);
        let settings = MenuItem::with_id("settings", text(language, "tray.settings"), true, None);
        let pause = MenuItem::with_id("pause", text(language, "action.resume"), true, None);
        let check = MenuItem::with_id("check", text(language, "action.check"), true, None);
        let open = MenuItem::with_id("open", text(language, "action.open"), true, None);
        let quit = MenuItem::with_id("quit", text(language, "tray.quit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &settings,
            &PredefinedMenuItem::separator(),
            &pause,
            &check,
            &open,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_id("noritec-drive")
            .with_icon(Icon::from_rgba(icon_pixels([78, 211, 161]), 32, 32)?)
            .with_tooltip("Noritec Drive")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        let context = ctx.clone();
        let events = sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let scale = context.pixels_per_point();
                let position = egui::pos2(position.x as f32 / scale, position.y as f32 / scale);
                let _ = events.send(Command::Show(Some(position)));
                context.request_repaint();
            }
        }));
        let context = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                "show" => Command::Show(None),
                "settings" => Command::Settings,
                "pause" => Command::ToggleSync,
                "check" => Command::Check,
                "open" => Command::OpenFolder,
                "quit" => Command::Quit,
                _ => return,
            };
            let _ = sender.send(command);
            context.request_repaint();
        }));
        Ok(Self {
            icon,
            show,
            settings,
            pause,
            check,
            open,
            quit,
            last_state: None,
        })
    }

    pub fn update(&mut self, language: Language, running: bool, status: &str, error: bool) {
        let key = (language, running, format!("{error}:{status}"));
        if self.last_state.as_ref() == Some(&key) {
            return;
        }
        self.last_state = Some(key);
        self.show.set_text(text(language, "tray.show"));
        self.settings.set_text(text(language, "tray.settings"));
        self.pause.set_text(text(
            language,
            if running {
                "action.pause"
            } else {
                "action.resume"
            },
        ));
        self.check.set_text(text(language, "action.check"));
        self.check.set_enabled(running);
        self.open.set_text(text(language, "action.open"));
        self.quit.set_text(text(language, "tray.quit"));
        let color = if error {
            [244, 126, 126]
        } else if running {
            [78, 211, 161]
        } else {
            [160, 167, 180]
        };
        if let Ok(icon) = Icon::from_rgba(icon_pixels(color), 32, 32) {
            let _ = self.icon.set_icon(Some(icon));
        }
        let tooltip: String = format!("Noritec Drive — {status}")
            .chars()
            .take(120)
            .collect();
        let _ = self.icon.set_tooltip(Some(tooltip));
    }
}
