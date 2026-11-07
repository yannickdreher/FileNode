use crate::{
    autostart,
    tray::{self, Command, Tray},
};
use anyhow::Result;
use eframe::egui::{self, Color32, RichText};
use noritec_drive::{
    config::Config,
    engine,
    i18n::{text, Language, Message},
    jmap::Jmap,
    scanner::LocalScanner,
    store::Store,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

const ACCENT: Color32 = Color32::from_rgb(78, 211, 161);
fn settings_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("settings")
}

enum Event {
    Log(Message),
    Done(engine::Report),
    Error(String),
    Stopped,
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Sync,
    Account,
    General,
    About,
}

struct App {
    config: Config,
    path: PathBuf,
    local_path: String,
    secret: String,
    logs: VecDeque<(chrono::DateTime<chrono::Local>, Message)>,
    status: Message,
    error: Option<String>,
    tray_warning: Option<String>,
    running: bool,
    cancel: Arc<AtomicBool>,
    immediate: Arc<AtomicBool>,
    receiver: Option<mpsc::Receiver<Event>>,
    worker: Option<thread::JoinHandle<()>>,
    commands: mpsc::Receiver<Command>,
    tray: Option<Tray>,
    settings_open: bool,
    settings_needs_focus: bool,
    page: Page,
    popup_visible: bool,
    popup_had_focus: bool,
    quitting: bool,
    first_frame: bool,
    background: bool,
    last_success: Option<String>,
}

pub fn run(
    path: PathBuf,
    language: Option<Language>,
    background: bool,
    settings: bool,
    no_tray: bool,
) -> Result<()> {
    let mut config = if path.exists() {
        Config::load(&path)?
    } else {
        Config::default()
    };
    if let Some(language) = language {
        config.language = language;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([410.0, 600.0])
            .with_min_inner_size([380.0, 500.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_icon(egui::IconData {
                rgba: tray::icon_pixels([78, 211, 161]),
                width: 32,
                height: 32,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Noritec Drive",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            style.visuals.panel_fill = Color32::from_rgb(35, 36, 38);
            style.visuals.window_fill = style.visuals.panel_fill;
            style.visuals.selection.bg_fill = Color32::from_rgb(22, 100, 82);
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
            cc.egui_ctx.set_style(style);
            let (sender, commands) = mpsc::channel();
            let (tray, tray_warning) = if no_tray {
                (None, None)
            } else {
                match Tray::new(&cc.egui_ctx, sender, config.language.resolve()) {
                    Ok(tray) => (Some(tray), None),
                    Err(error) => (None, Some(format!("{error:#}"))),
                }
            };
            Ok(Box::new(App {
                local_path: config.local_folder.display().to_string(),
                config,
                path,
                secret: String::new(),
                logs: VecDeque::new(),
                status: Message::new("status.ready"),
                error: None,
                tray_warning,
                running: false,
                cancel: Arc::new(AtomicBool::new(false)),
                immediate: Arc::new(AtomicBool::new(false)),
                receiver: None,
                worker: None,
                commands,
                tray,
                settings_open: settings,
                settings_needs_focus: settings,
                page: Page::Sync,
                popup_visible: true,
                popup_had_focus: false,
                quitting: false,
                first_frame: true,
                background,
                last_success: None,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Could not start desktop application: {error}"))
}

impl App {
    fn start(&mut self, context: &egui::Context) -> Result<()> {
        if self.running {
            return Ok(());
        }
        self.config.local_folder = PathBuf::from(&self.local_path);
        if !self.config.local_folder.exists() {
            std::fs::create_dir_all(&self.config.local_folder)?;
        }
        self.config.local_folder = self.config.local_folder.canonicalize()?;
        self.local_path = self.config.local_folder.display().to_string();
        self.config.validate()?;
        let entry = keyring::Entry::new("de.noritec.drive", &self.config.credential_key())?;
        if !self.secret.is_empty() {
            entry.set_password(&self.secret)?;
            self.secret.zeroize();
        }
        let secret = entry.get_password()?;
        self.config.save(&self.path)?;
        self.error = None;
        self.status = Message::new("status.connecting");
        self.cancel = Arc::new(AtomicBool::new(false));
        self.immediate = Arc::new(AtomicBool::new(false));
        let (cancel, immediate) = (self.cancel.clone(), self.immediate.clone());
        let config = self.config.clone();
        let context = context.clone();
        let (tx, rx) = mpsc::channel();
        self.running = true;
        self.receiver = Some(rx);
        self.worker = Some(thread::spawn(move || {
            let send = |event| {
                let _ = tx.send(event);
                context.request_repaint_of(egui::ViewportId::ROOT);
            };
            let secret = zeroize::Zeroizing::new(secret);
            let mut backoff = 5;
            match Store::open(&config) {
                Err(error) => send(Event::Error(format!("{error:#}"))),
                Ok(mut store) => {
                    let mut scanner =
                        LocalScanner::new(&config.local_folder, config.full_scan_seconds);
                    if !scanner.watching() {
                        send(Event::Log(Message::new("log.watcher_fallback")));
                    }
                    let mut connection = None;
                    while !cancel.load(Ordering::Relaxed) {
                        let result = (|| {
                            if connection.is_none() {
                                scanner.invalidate();
                                connection = Some(Jmap::connect(
                                    &config,
                                    secret.to_string(),
                                    cancel.clone(),
                                )?);
                            }
                            let mut log = |message| send(Event::Log(message));
                            engine::sync_with_scanner(
                                &config,
                                &mut store,
                                connection.as_mut().unwrap(),
                                &cancel,
                                &mut log,
                                &mut scanner,
                            )
                        })();
                        let (seconds, failed) = match result {
                            Ok(report) => {
                                backoff = 5;
                                let rescan = report.needs_rescan;
                                send(Event::Done(report));
                                (if rescan { 1 } else { config.interval_seconds }, false)
                            }
                            Err(error) => {
                                if cancel.load(Ordering::Relaxed) {
                                    break;
                                }
                                send(Event::Error(format!("{error:#}")));
                                connection = None;
                                let delay = backoff;
                                backoff = (backoff * 2).min(300);
                                (delay, true)
                            }
                        };
                        let until = Instant::now() + Duration::from_secs(seconds);
                        while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                            if immediate.swap(false, Ordering::Relaxed) {
                                scanner.invalidate();
                                break;
                            }
                            if !failed && scanner.ready() {
                                break;
                            }
                            // Cap the wait to make stop/check responsive even without filesystem events.
                            let delay = until
                                .saturating_duration_since(Instant::now())
                                .min(Duration::from_secs(1));
                            if failed {
                                thread::sleep(delay);
                            } else {
                                scanner.wait(delay);
                            }
                        }
                    }
                }
            }
            send(Event::Stopped);
        }));
        Ok(())
    }

    fn pause(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = Message::new("status.stopping");
    }

    fn show(&mut self, context: &egui::Context, anchor: Option<egui::Pos2>) {
        if let Some(anchor) = anchor {
            // Coordinates supplied by the notification area are physical; Tray converts to points.
            // The compositor may ignore explicit placement on Wayland.
            context.send_viewport_cmd(egui::ViewportCommand::OuterPosition(tray::popup_position(
                context, anchor,
            )));
        }
        self.popup_visible = true;
        self.popup_had_focus = false;
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide(&mut self, context: &egui::Context) {
        if self.tray.is_some() {
            self.popup_visible = false;
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn command(&mut self, command: Command, context: &egui::Context) {
        match command {
            Command::Show(anchor) => self.show(context, anchor),
            Command::Settings => {
                self.settings_open = true;
                self.settings_needs_focus = true;
            }
            Command::ToggleSync => {
                if self.running {
                    self.pause();
                } else if let Err(error) = self.start(context) {
                    self.error = Some(format!("{error:#}"));
                    self.settings_open = true;
                    self.show(context, None);
                }
            }
            Command::Check => self.immediate.store(true, Ordering::Relaxed),
            Command::OpenFolder => {
                if let Err(error) = open::that_detached(&self.local_path) {
                    self.error = Some(error.to_string());
                }
            }
            Command::Quit => {
                self.quitting = true;
                self.cancel.store(true, Ordering::Relaxed);
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn poll(&mut self) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Log(message) => {
                    if message.argument("path").is_some()
                        && self.logs.front().is_some_and(|(_, old)| {
                            old.argument("path") == message.argument("path")
                        })
                    {
                        self.logs.pop_front();
                    }
                    self.logs.push_front((chrono::Local::now(), message));
                    self.logs.truncate(150);
                }
                Event::Done(report) => {
                    self.error = None;
                    self.last_success = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                    self.status = if report.needs_rescan {
                        Message::new("status.rescan")
                    } else if report.blocked > 0 {
                        Message::new("status.blocked").with("count", report.blocked)
                    } else {
                        Message::new("status.current")
                    };
                }
                Event::Error(error) => {
                    self.status = Message::new("status.retry");
                    self.error = Some(error);
                }
                Event::Stopped => {
                    self.running = false;
                    self.status = Message::new("status.paused");
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                }
            }
        }
    }

    fn status_panel(&mut self, context: &egui::Context) {
        let language = self.config.language.resolve();
        egui::TopBottomPanel::top("status_header")
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(35, 36, 38))
                    .inner_margin(18.0),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    let title = ui.add(
                        egui::Label::new(
                            RichText::new("Noritec Drive")
                                .size(17.0)
                                .strong()
                                .color(ACCENT),
                        )
                        .sense(egui::Sense::drag()),
                    );
                    if title.drag_started() {
                        context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("×")
                            .on_hover_text(text(language, "tray.hide"))
                            .clicked()
                        {
                            if self.tray.is_some() {
                                self.hide(context);
                            } else {
                                self.command(Command::Quit, context);
                            }
                        }
                        ui.menu_button(text(language, "tray.menu"), |ui| {
                            for (key, command) in [
                                ("tray.settings", Command::Settings),
                                (
                                    if self.running {
                                        "action.pause"
                                    } else {
                                        "action.resume"
                                    },
                                    Command::ToggleSync,
                                ),
                                ("action.check", Command::Check),
                                ("tray.quit", Command::Quit),
                            ] {
                                if ui.button(text(language, key)).clicked() {
                                    ui.close_menu();
                                    self.command(command, context);
                                }
                            }
                        });
                    });
                });
                ui.label(
                    RichText::new(if self.config.username.is_empty() {
                        text(language, "account.not_connected")
                    } else {
                        &self.config.username
                    })
                    .small()
                    .color(Color32::GRAY),
                );
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        4.0,
                        if self.error.is_some() {
                            Color32::LIGHT_RED
                        } else if self.running {
                            ACCENT
                        } else {
                            Color32::GRAY
                        },
                    );
                    ui.label(self.status.render(language));
                });
                if let Some(time) = &self.last_success {
                    ui.small(
                        Message::new("status.last_success")
                            .with("time", time)
                            .render(language),
                    );
                }
            });
        egui::TopBottomPanel::bottom("status_footer")
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(35, 36, 38))
                    .inner_margin(14.0),
            )
            .show(context, |ui| {
                ui.separator();
                ui.columns(3, |columns| {
                    if columns[0].button(text(language, "action.open")).clicked() {
                        self.command(Command::OpenFolder, context);
                    }
                    if columns[1].button(text(language, "action.online")).clicked() {
                        if let Err(error) = open::that_detached(&self.config.web_url) {
                            self.error = Some(error.to_string());
                        }
                    }
                    if columns[2]
                        .button(text(language, "action.recovery"))
                        .clicked()
                    {
                        let history =
                            PathBuf::from(&self.local_path).join(".noritec-drive/history");
                        if let Err(error) = open::that_detached(history) {
                            self.error = Some(error.to_string());
                        }
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(35, 36, 38))
                    .inner_margin(18.0),
            )
            .show(context, |ui| {
                if self.error.is_some() {
                    ui.colored_label(Color32::LIGHT_RED, text(language, "error.summary"));
                    egui::CollapsingHeader::new(text(language, "error.details")).show(ui, |ui| {
                        ui.label(self.error.as_deref().unwrap_or_default());
                    });
                }
                if self.tray_warning.is_some() {
                    ui.small(text(language, "tray.unavailable"));
                }
                if !self.running && self.logs.is_empty() {
                    ui.add_space(24.0);
                    ui.heading(text(language, "status.welcome"));
                    ui.label(text(language, "status.setup_hint"));
                    if ui.button(text(language, "tray.settings")).clicked() {
                        self.settings_open = true;
                        self.settings_needs_focus = true;
                        self.page = Page::Account;
                    }
                    if !self.config.username.is_empty()
                        && ui.button(text(language, "action.resume")).clicked()
                    {
                        self.command(Command::ToggleSync, context);
                    }
                }
                ui.add_space(12.0);
                ui.label(RichText::new(text(language, "activity.recent")).strong());
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.logs.is_empty() {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(text(language, "activity.empty")).color(Color32::GRAY),
                        );
                    }
                    for (time, message) in &self.logs {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(28.0, 34.0), egui::Sense::hover());
                            ui.painter().rect_stroke(
                                rect.shrink(4.0),
                                2.0,
                                egui::Stroke::new(1.5_f32, ACCENT),
                                egui::StrokeKind::Inside,
                            );
                            ui.vertical(|ui| {
                                if let Some(path) =
                                    message.argument("path").or_else(|| message.argument("to"))
                                {
                                    ui.label(
                                        RichText::new(path.rsplit('/').next().unwrap_or(path))
                                            .strong(),
                                    );
                                }
                                ui.add(egui::Label::new(message.render(language)).wrap());
                                ui.label(
                                    RichText::new(time.format("%d.%m.%Y · %H:%M:%S").to_string())
                                        .small()
                                        .color(Color32::GRAY),
                                );
                            });
                        });
                        ui.add_space(6.0);
                        ui.separator();
                    }
                });
            });
    }

    fn settings(&mut self, context: &egui::Context) {
        let language = self.config.language.resolve();
        if context.input(|input| input.viewport().close_requested()) {
            self.settings_open = false;
        }
        egui::SidePanel::left("settings_nav")
            .exact_width(190.0)
            .show(context, |ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("NORITEC").strong().color(ACCENT));
                ui.label(RichText::new("Drive").size(28.0).strong());
                ui.add_space(20.0);
                for (page, key) in [
                    (Page::Sync, "settings.sync"),
                    (Page::Account, "settings.account"),
                    (Page::General, "settings.general"),
                    (Page::About, "settings.about"),
                ] {
                    ui.selectable_value(&mut self.page, page, text(language, key));
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(12.0);
                    ui.small("v0.1 · Preview");
                });
            });
        egui::CentralPanel::default().show(context, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(20.0);
                ui.heading(text(
                    language,
                    match self.page {
                        Page::Sync => "settings.sync",
                        Page::Account => "settings.account",
                        Page::General => "settings.general",
                        Page::About => "settings.about",
                    },
                ));
                ui.add_space(18.0);
                match self.page {
                    Page::Sync => {
                        ui.label(text(language, "settings.sync_hint"));
                        ui.add_space(12.0);
                        ui.add_enabled_ui(!self.running, |ui| {
                            field(
                                ui,
                                text(language, "settings.local"),
                                &mut self.local_path,
                                false,
                            );
                            field(
                                ui,
                                text(language, "settings.remote"),
                                &mut self.config.remote_folder,
                                false,
                            );
                            ui.add_space(12.0);
                            ui.checkbox(
                                &mut self.config.propagate_deletes,
                                text(language, "settings.deletes"),
                            );
                            ui.add_space(12.0);
                            egui::CollapsingHeader::new(text(language, "settings.advanced")).show(
                                ui,
                                |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(text(language, "settings.interval"));
                                        ui.add(
                                            egui::DragValue::new(&mut self.config.interval_seconds)
                                                .range(5..=86400),
                                        );
                                        ui.label(text(language, "settings.seconds"));
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(text(language, "settings.full_scan"));
                                        ui.add(
                                            egui::DragValue::new(
                                                &mut self.config.full_scan_seconds,
                                            )
                                            .range(60..=86400),
                                        );
                                        ui.label(text(language, "settings.seconds"));
                                    });
                                },
                            );
                        });
                        if self.running {
                            ui.small(text(language, "settings.pause_hint"));
                        }
                        ui.add_space(20.0);
                        if ui
                            .button(text(
                                language,
                                if self.running {
                                    "action.pause"
                                } else {
                                    "action.connect"
                                },
                            ))
                            .clicked()
                        {
                            self.command(Command::ToggleSync, context);
                        }
                    }
                    Page::Account => {
                        ui.add_enabled_ui(!self.running, |ui| {
                            field(
                                ui,
                                text(language, "settings.server"),
                                &mut self.config.server,
                                false,
                            );
                            field(
                                ui,
                                text(language, "settings.web"),
                                &mut self.config.web_url,
                                false,
                            );
                            field(
                                ui,
                                text(language, "settings.username"),
                                &mut self.config.username,
                                false,
                            );
                            ui.label(text(language, "settings.password"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.secret)
                                    .password(true)
                                    .desired_width(430.0)
                                    .hint_text(text(language, "settings.password_hint")),
                            );
                            ui.add_space(8.0);
                            ui.small(text(language, "settings.credentials"));
                        });
                        ui.add_space(20.0);
                        if ui
                            .button(text(
                                language,
                                if self.running {
                                    "action.pause"
                                } else {
                                    "action.connect"
                                },
                            ))
                            .clicked()
                        {
                            self.command(Command::ToggleSync, context);
                        }
                    }
                    Page::General => {
                        let previous = self.config.language;
                        ui.label(text(language, "language.label"));
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(match self.config.language {
                                Language::Auto => text(language, "language.auto"),
                                Language::English => "English",
                                Language::German => "Deutsch",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.config.language,
                                    Language::Auto,
                                    text(language, "language.auto"),
                                );
                                ui.selectable_value(
                                    &mut self.config.language,
                                    Language::English,
                                    "English",
                                );
                                ui.selectable_value(
                                    &mut self.config.language,
                                    Language::German,
                                    "Deutsch",
                                );
                            });
                        if previous != self.config.language {
                            self.save();
                        }
                        ui.add_space(20.0);
                        let previous = self.config.autostart;
                        if ui
                            .checkbox(
                                &mut self.config.autostart,
                                text(language, "settings.autostart"),
                            )
                            .changed()
                        {
                            if let Err(error) = autostart::set(self.config.autostart, &self.path) {
                                self.config.autostart = previous;
                                self.error = Some(format!("{error:#}"));
                            } else {
                                self.save();
                            }
                        }
                        ui.checkbox(
                            &mut self.config.auto_connect,
                            text(language, "settings.auto_connect"),
                        );
                        ui.add_space(14.0);
                        ui.label(text(language, "settings.background_hint"));
                        if self.tray_warning.is_some() {
                            ui.colored_label(Color32::YELLOW, text(language, "tray.unavailable"));
                        }
                    }
                    Page::About => {
                        ui.label(RichText::new("Noritec Drive").size(22.0).strong());
                        ui.label("0.1.0 · Preview");
                        ui.add_space(12.0);
                        ui.label(text(language, "about.description"));
                        ui.add_space(12.0);
                        ui.label(text(language, "about.limits"));
                        ui.hyperlink_to(
                            text(language, "about.project"),
                            "https://gitlab.example.com/noritec/drive",
                        );
                    }
                }
                ui.add_space(24.0);
                if self.page != Page::About && ui.button(text(language, "settings.save")).clicked()
                {
                    self.save();
                }
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, text(language, "error.summary"));
                    egui::CollapsingHeader::new(text(language, "error.details")).show(ui, |ui| {
                        ui.label(error);
                    });
                }
            });
        });
    }

    fn save(&mut self) {
        self.config.local_folder = PathBuf::from(&self.local_path);
        if let Err(error) = self.config.save(&self.path) {
            self.error = Some(format!("{error:#}"));
        }
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, password: bool) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .password(password)
            .desired_width(430.0),
    );
    ui.add_space(6.0);
}

impl eframe::App for App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.first_frame {
            self.first_frame = false;
            if self.config.auto_connect && !self.config.username.is_empty() {
                if let Err(error) = self.start(context) {
                    self.error = Some(format!("{error:#}"));
                    self.settings_open = true;
                }
            }
            if self.background
                && self.tray.is_some()
                && self.error.is_none()
                && !self.config.username.is_empty()
            {
                self.hide(context);
            }
        }
        let commands: Vec<_> = self.commands.try_iter().collect();
        for command in commands {
            self.command(command, context);
        }
        if context.input(|input| input.viewport().close_requested())
            && !self.quitting
            && self.tray.is_some()
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(context);
        }
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.hide(context);
        }
        let focused = context.input(|input| input.viewport().focused);
        if focused == Some(true) {
            self.popup_had_focus = true;
        }
        if self.popup_visible
            && self.popup_had_focus
            && focused == Some(false)
            && !self.settings_open
        {
            self.hide(context);
        }
        if let Some(tray) = &mut self.tray {
            tray.update(
                self.config.language.resolve(),
                self.running,
                &self.status.render(self.config.language),
                self.error.is_some(),
            );
        }
        self.status_panel(context);
        if self.settings_open {
            context.show_viewport_immediate(
                settings_id(),
                egui::ViewportBuilder::default()
                    .with_title(format!(
                        "Noritec Drive — {}",
                        text(self.config.language, "tray.settings")
                    ))
                    .with_inner_size([880.0, 670.0])
                    .with_min_inner_size([760.0, 540.0])
                    .with_icon(egui::IconData {
                        rgba: tray::icon_pixels([78, 211, 161]),
                        width: 32,
                        height: 32,
                    }),
                |context, _class| self.settings(context),
            );
            if self.settings_needs_focus {
                self.settings_needs_focus = false;
                context.send_viewport_cmd_to(settings_id(), egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd_to(settings_id(), egui::ViewportCommand::Focus);
                self.hide(context);
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.secret.zeroize();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
