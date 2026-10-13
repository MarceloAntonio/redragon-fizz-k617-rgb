//! Interface gráfica: escolhe efeito/cor/velocidade/brilho ou pinta tecla por tecla (modo 21).

use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use k617rgb::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x1a);
const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x24);
const KEY_OFF: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x32);

enum Job {
    Mode { mode: u8, color: Color, speed: u8, bright: u8 },
    Custom { group: usize, colors: Vec<(usize, Rgb)> },
}

/// Thread de HID: abre o teclado, lê a config, envia os pacotes. A UI nunca bloqueia.
fn worker(ctx: egui::Context) -> (Sender<Job>, Receiver<Result<String, String>>) {
    let (job_tx, job_rx) = channel::<Job>();
    let (res_tx, res_rx) = channel();
    thread::spawn(move || {
        for job in job_rx {
            let r = (|| {
                let f = open_device().map_err(|e| e.to_string())?;
                let mut cfg = read_cfg(&f).map_err(|e| format!("lendo config: {e}"))?;
                let (packets, msg) = match job {
                    Job::Mode { mode, color, speed, bright } => (
                        mode_packets(&mut cfg, mode, color, Some(speed), Some(bright)),
                        format!("Efeito \"{}\" aplicado", MODES[mode as usize - 1]),
                    ),
                    Job::Custom { group, colors } => {
                        (custom_packets(&mut cfg, group, &colors), format!("Grupo {} aplicado", group + 1))
                    }
                };
                for p in packets {
                    set_feature(&f, &p).map_err(|e| format!("enviando: {e}"))?;
                }
                Ok(msg)
            })();
            let _ = res_tx.send(r);
            ctx.request_repaint();
        }
    });
    (job_tx, res_rx)
}

struct App {
    custom: bool,
    mode: u8,
    color: Rgb,
    random: bool,
    speed: u8,
    bright: u8,
    group: usize,
    groups: Vec<Vec<Rgb>>, // [grupo][índice em KEYS]
    tx: Sender<Job>,
    rx: Receiver<Result<String, String>>,
    status: Result<String, String>,
    busy: bool,
}

/// Estado salvo como texto: "custom mode r g b random speed bright group" + uma linha hex por grupo.
fn load(s: &str, app: &mut App) -> Option<()> {
    let mut lines = s.lines();
    let v: Vec<u8> = lines.next()?.split(' ').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [custom, mode, r, g, b, random, speed, bright, group] = v[..] else { return None };
    (app.custom, app.mode, app.color, app.random) = (custom == 1, mode.clamp(1, 20), [r, g, b], random == 1);
    (app.speed, app.bright, app.group) = (speed.clamp(1, 4), bright.min(4), (group as usize).min(GROUPS - 1));
    for (dst, line) in app.groups.iter_mut().zip(lines) {
        for (c, hex) in dst.iter_mut().zip(line.split(' ')) {
            let n = u32::from_str_radix(hex, 16).ok()?;
            *c = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        }
    }
    Some(())
}

fn save(app: &App) -> String {
    let [r, g, b] = app.color;
    let mut s = format!(
        "{} {} {r} {g} {b} {} {} {} {}\n",
        app.custom as u8, app.mode, app.random as u8, app.speed, app.bright, app.group
    );
    for grp in &app.groups {
        let row: Vec<String> = grp.iter().map(|[r, g, b]| format!("{r:02x}{g:02x}{b:02x}")).collect();
        s += &(row.join(" ") + "\n");
    }
    s
}

fn c32([r, g, b]: Rgb) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn label(name: &str) -> String {
    match name {
        "backspace" => "Bksp".into(),
        "enter" => "Enter".into(),
        "capslock" => "Caps".into(),
        "lshift" | "rshift" => "Shift".into(),
        "lctrl" | "rctrl" => "Ctrl".into(),
        "lalt" => "Alt".into(),
        "ralt" => "AltGr".into(),
        "lwin" => "Win".into(),
        "space" => String::new(),
        "app" => "Menu".into(),
        n => n.to_uppercase(),
    }
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.visuals.panel_fill = BG;
            style.visuals.window_fill = PANEL;
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(12.0, 6.0);
        });

        let (tx, rx) = worker(cc.egui_ctx.clone());
        let mut app = App {
            custom: false,
            mode: 1,
            color: [0, 170, 255],
            random: false,
            speed: 3,
            bright: 4,
            group: 0,
            groups: vec![vec![[0, 170, 255]; KEYS.len()]; GROUPS],
            tx,
            rx,
            status: Ok("Pronto. Nada é enviado até clicar em Aplicar.".into()),
            busy: false,
        };
        if let Some(s) = cc.storage.and_then(|s| s.get_string("state")) {
            load(&s, &mut app);
        }
        app
    }

    fn apply(&mut self) {
        let job = if self.custom {
            let colors = KEYS.iter().zip(&self.groups[self.group]).map(|(&(_, led, _), &c)| (led, c)).collect();
            Job::Custom { group: self.group, colors }
        } else {
            let color = if self.random { Color::Random } else { Color::Rgb(self.color) };
            Job::Mode { mode: self.mode, color, speed: self.speed, bright: self.bright }
        };
        self.busy = self.tx.send(job).is_ok();
        self.status = Ok("Enviando…".into());
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("apply")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 12)))
            .show(ui, |ui| self.apply_bar(ui));
        egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui));
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.heading("K617 RGB");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.custom, false, "Efeitos");
            ui.selectable_value(&mut self.custom, true, "Por tecla");
        });
        ui.separator();

        if self.custom {
            ui.label("Grupo salvo no teclado");
            ui.horizontal(|ui| {
                for g in 0..GROUPS {
                    ui.selectable_value(&mut self.group, g, format!("{}", g + 1));
                }
            });
            ui.label("Pincel");
            ui.color_edit_button_srgb(&mut self.color);
            ui.horizontal(|ui| {
                if ui.button("Preencher tudo").clicked() {
                    self.groups[self.group].fill(self.color);
                }
                if ui.button("Limpar").clicked() {
                    self.groups[self.group].fill([0, 0, 0]);
                }
            });
            ui.weak("Clique ou arraste para pintar.\nShift+clique copia a cor da tecla.");
        } else {
            ui.label("Efeito");
            egui::Grid::new("modes").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
                for (i, name) in MODES.iter().enumerate() {
                    ui.selectable_value(&mut self.mode, i as u8 + 1, *name);
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.separator();
            ui.label("Cor");
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!self.random, |ui| ui.color_edit_button_srgb(&mut self.color));
                ui.checkbox(&mut self.random, "Aleatória");
            });
            ui.add(egui::Slider::new(&mut self.speed, 1..=4).text("Velocidade"));
            ui.add(egui::Slider::new(&mut self.bright, 0..=4).text("Brilho"));
        }
    }

    fn apply_bar(&mut self, ui: &mut egui::Ui) {
        let apply = egui::Button::new(egui::RichText::new("Aplicar").strong().size(16.0))
            .fill(c32(self.color).gamma_multiply(0.55))
            .min_size(Vec2::new(ui.available_width(), 36.0));
        if ui.add_enabled(!self.busy, apply).clicked() {
            self.apply();
        }
        match &self.status {
            Ok(m) => ui.label(m),
            Err(e) => ui.colored_label(Color32::from_rgb(0xff, 0x6b, 0x6b), e),
        };
    }

    fn keyboard(&mut self, ui: &mut egui::Ui) {
        let (w, h) = (547.0 - 10.0, 188.0 - 10.0); // limites do layout do INI
        let avail = ui.available_size();
        let scale = (avail.x / w).min(avail.y / h).min(2.2);
        let size = Vec2::new(w, h) * scale;
        let at = ui.cursor().min + (avail - size) / 2.0; // centraliza
        let resp = ui.allocate_rect(Rect::from_min_size(at, size), Sense::click_and_drag());
        let painter = ui.painter_at(resp.rect);
        let origin = resp.rect.min - Vec2::splat(10.0 * scale);
        let t = ui.input(|i| i.time) as f32;
        let pointer = resp.hover_pos();
        let pressed = resp.is_pointer_button_down_on() || resp.clicked();
        let shift = ui.input(|i| i.modifiers.shift);

        for (i, &(name, _, [x1, y1, x2, y2])) in KEYS.iter().enumerate() {
            let rect = Rect::from_min_max(
                origin + Vec2::new(x1 as f32, y1 as f32) * scale,
                origin + Vec2::new(x2 as f32, y2 as f32) * scale,
            );
            let hovered = pointer.is_some_and(|p| rect.contains(p));
            if self.custom && hovered && pressed {
                if shift {
                    self.color = self.groups[self.group][i];
                } else {
                    self.groups[self.group][i] = self.color;
                }
            }
            let fill = if self.custom {
                c32(self.groups[self.group][i])
            } else if self.bright == 0 {
                KEY_OFF
            } else if self.random {
                let h = (x1 as f32 / w + t * 0.15).fract(); // prévia: arco-íris passando
                egui::ecolor::Hsva::new(h, 0.8, 1.0, 1.0).into()
            } else {
                c32(self.color).gamma_multiply(0.4 + 0.15 * self.bright as f32)
            };
            painter.rect_filled(rect.shrink(1.5 * scale), 6.0, fill);
            if hovered {
                painter.rect_stroke(rect.shrink(1.5 * scale), 6.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
            }
            let [r, g, b, _] = fill.to_array().map(|v| v as f32);
            let text = if 0.299 * r + 0.587 * g + 0.114 * b > 140.0 { Color32::BLACK } else { Color32::WHITE };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, label(name), egui::FontId::proportional(9.0 * scale), text);
        }
        if self.random && !self.custom {
            ui.ctx().request_repaint();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok(r) = self.rx.try_recv() {
            self.status = r;
            self.busy = false;
        }
        egui::Panel::left("side")
            .exact_size(260.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(16.0))
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(24.0))
            .show(ui, |ui| self.keyboard(ui));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("state", save(self));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("K617 RGB")
            .with_app_id("k617rgb")
            .with_inner_size([1180.0, 560.0])
            .with_min_inner_size([820.0, 440.0]),
        ..Default::default()
    };
    eframe::run_native("k617rgb", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
