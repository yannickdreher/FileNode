use anyhow::Result;
use eframe::egui::{self, Color32, RichText};
use noritec_drive::{
    config::Config,
    engine,
    i18n::{text, Language, Message},
    jmap::Jmap,
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

enum Event {
    Log(Message),
    Done(engine::Report),
    Error(String),
    Stopped,
}

struct App {
    config: Config,
    path: PathBuf,
    local_path: String,
    secret: String,
    logs: VecDeque<(String, Message)>,
    status: Message,
    error: Option<String>,
    running: bool,
    cancel: Arc<AtomicBool>,
    immediate: Arc<AtomicBool>,
    receiver: Option<mpsc::Receiver<Event>>,
    worker: Option<thread::JoinHandle<()>>,
    last_success: Option<String>,
}

pub fn run(path: PathBuf, language: Option<Language>) -> Result<()> {
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
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Noritec Drive",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(12.0, 12.0);
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
            style.visuals.selection.bg_fill = Color32::from_rgb(22, 100, 82);
            cc.egui_ctx.set_style(style);
            Ok(Box::new(App {
                local_path: config.local_folder.display().to_string(),
                config,
                path,
                secret: String::new(),
                logs: VecDeque::new(),
                status: Message::new("status.ready"),
                error: None,
                running: false,
                cancel: Arc::new(AtomicBool::new(false)),
                immediate: Arc::new(AtomicBool::new(false)),
                receiver: None,
                worker: None,
                last_success: None,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Could not start desktop application: {e}"))
}

impl App {
    fn start(&mut self) -> Result<()> {
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
        let (tx, rx) = mpsc::channel();
        self.running = true;
        self.receiver = Some(rx);
        self.worker = Some(thread::spawn(move || {
            let secret = zeroize::Zeroizing::new(secret);
            let mut backoff = 5;
            let store = Store::open(&config);
            match store {
                Err(error) => {
                    let _ = tx.send(Event::Error(format!("{error:#}")));
                }
                Ok(mut store) => {
                    while !cancel.load(Ordering::Relaxed) {
                        let result = Jmap::connect(&config, secret.to_string(), cancel.clone())
                            .and_then(|mut remote| {
                                let mut log = |line| {
                                    let _ = tx.send(Event::Log(line));
                                };
                                engine::sync(&config, &mut store, &mut remote, &cancel, &mut log)
                            });
                        let seconds = match result {
                            Ok(report) => {
                                backoff = 5;
                                let rescan = report.needs_rescan;
                                let _ = tx.send(Event::Done(report));
                                if rescan {
                                    1
                                } else {
                                    config.interval_seconds
                                }
                            }
                            Err(error) => {
                                if cancel.load(Ordering::Relaxed) {
                                    break;
                                }
                                let _ = tx.send(Event::Error(format!("{error:#}")));
                                let delay = backoff;
                                backoff = (backoff * 2).min(300);
                                delay
                            }
                        };
                        let until = Instant::now() + Duration::from_secs(seconds);
                        while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                            if immediate.swap(false, Ordering::Relaxed) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            }
            let _ = tx.send(Event::Stopped);
        }));
        Ok(())
    }

    fn poll(&mut self) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|r| r.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Log(line) => {
                    self.logs
                        .push_front((chrono::Local::now().format("%H:%M:%S").to_string(), line));
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
                    self.logs.push_front((
                        chrono::Local::now().format("%H:%M:%S").to_string(),
                        Message::new("log.error"),
                    ));
                    self.logs.truncate(150);
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_millis(250));
        let language = self.config.language.resolve();
        let green = Color32::from_rgb(78, 211, 161);
        egui::SidePanel::left("navigation")
            .exact_width(205.0)
            .show(ctx, |ui| {
                ui.add_space(24.0);
                ui.label(RichText::new("NORITEC").size(16.0).strong().color(green));
                ui.label(RichText::new("Drive").size(34.0).strong());
                ui.add_space(24.0);
                ui.label(
                    RichText::new(text(language, "nav.storage"))
                        .size(10.0)
                        .color(Color32::GRAY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.0, green);
                    ui.label(text(language, "nav.sync"));
                });
                ui.label("    Stalwart / JMAP");
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new(text(language, "nav.preview"))
                            .small()
                            .color(Color32::GRAY),
                    );
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("main_scroll")
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    let previous_language = self.config.language;
                    ui.horizontal(|ui| {
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
                    });
                    if previous_language != self.config.language {
                        if let Err(error) = self.config.save(&self.path) {
                            self.error = Some(format!("{error:#}"));
                        }
                    }
                    ui.heading(text(language, "home.title"));
                    ui.label(text(language, "home.subtitle"));
                    ui.add_space(12.0);
                    egui::Frame::group(ui.style())
                        .inner_margin(16.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(22.0, 22.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    rect.center(),
                                    5.0,
                                    if self.error.is_some() {
                                        Color32::LIGHT_RED
                                    } else {
                                        green
                                    },
                                );
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(self.status.render(language)).strong());
                                    if let Some(time) = &self.last_success {
                                        ui.small(
                                            Message::new("status.last_success")
                                                .with("time", time)
                                                .render(language),
                                        );
                                    }
                                });
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if !self.running {
                                    if ui.button(text(language, "action.connect")).clicked() {
                                        if let Err(error) = self.start() {
                                            self.error = Some(format!("{error:#}"));
                                        }
                                    }
                                } else {
                                    if ui.button(text(language, "action.check")).clicked() {
                                        self.immediate.store(true, Ordering::Relaxed);
                                    }
                                    if ui.button(text(language, "action.pause")).clicked() {
                                        self.cancel.store(true, Ordering::Relaxed);
                                        self.status = Message::new("status.stopping");
                                    }
                                }
                                if ui.button(text(language, "action.open")).clicked() {
                                    #[cfg(windows)]
                                    let result = std::process::Command::new("explorer.exe")
                                        .arg(&self.local_path)
                                        .spawn();
                                    #[cfg(target_os = "linux")]
                                    let result = std::process::Command::new("xdg-open")
                                        .arg(&self.local_path)
                                        .spawn();
                                    if let Err(error) = result {
                                        self.error = Some(error.to_string());
                                    }
                                }
                            });
                        });
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::LIGHT_RED, text(language, "error.summary"));
                        egui::CollapsingHeader::new(text(language, "error.details")).show(
                            ui,
                            |ui| {
                                ui.label(error);
                            },
                        );
                    }
                    ui.add_space(12.0);
                    egui::CollapsingHeader::new(text(language, "settings.title"))
                        .default_open(!self.running)
                        .show(ui, |ui| {
                            ui.add_enabled_ui(!self.running, |ui| {
                                egui::Grid::new("settings")
                                    .num_columns(2)
                                    .spacing([16.0, 10.0])
                                    .show(ui, |ui| {
                                        ui.label(text(language, "settings.server"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.config.server)
                                                .desired_width(420.0),
                                        );
                                        ui.end_row();
                                        ui.label(text(language, "settings.username"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.config.username)
                                                .desired_width(420.0),
                                        );
                                        ui.end_row();
                                        ui.label(text(language, "settings.password"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.secret)
                                                .password(true)
                                                .hint_text(text(language, "settings.password_hint"))
                                                .desired_width(420.0),
                                        );
                                        ui.end_row();
                                        ui.label(text(language, "settings.local"));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.local_path)
                                                .desired_width(420.0),
                                        );
                                        ui.end_row();
                                        ui.label(text(language, "settings.remote"));
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.config.remote_folder,
                                            )
                                            .desired_width(420.0),
                                        );
                                        ui.end_row();
                                        ui.label(text(language, "settings.interval"));
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::DragValue::new(
                                                    &mut self.config.interval_seconds,
                                                )
                                                .range(5..=86400),
                                            );
                                            ui.label(text(language, "settings.seconds"));
                                        });
                                        ui.end_row();
                                    });
                                ui.add_space(8.0);
                                ui.checkbox(
                                    &mut self.config.propagate_deletes,
                                    text(language, "settings.deletes"),
                                );
                                ui.small(text(language, "settings.credentials"));
                            });
                        });
                    ui.add_space(14.0);
                    ui.separator();
                    ui.label(RichText::new(text(language, "activity.title")).strong());
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            if self.logs.is_empty() {
                                ui.label(
                                    RichText::new(text(language, "activity.empty"))
                                        .color(Color32::GRAY),
                                );
                            }
                            for (time, message) in &self.logs {
                                ui.label(format!("{time}  {}", message.render(language)));
                            }
                        });
                });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.secret.zeroize();
        // Finish the in-flight operation before releasing the state lock. HTTP has a
        // bounded timeout; leaving half an operation is reconciled from the journal.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
