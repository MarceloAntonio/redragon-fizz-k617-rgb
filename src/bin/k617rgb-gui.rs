//! Interface gráfica: escolhe efeito/cor/velocidade/brilho ou pinta tecla por tecla (modo 21).

use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};
use k617rgb::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x1a);
const PANEL: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x24);
const PANEL2: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1f);
const HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x33);
const CHASSIS: Color32 = Color32::from_rgb(0x07, 0x07, 0x09);
const ACCENT: Color32 = Color32::from_rgb(0x0e, 0x5f, 0xa8);
const SWATCHES: [Rgb; 8] =
    [[255, 0, 0], [255, 128, 0], [255, 255, 0], [0, 255, 0], [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 255]];
const OK: Color32 = Color32::from_rgb(0x4c, 0xd1, 0x7a);
const ERR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);

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
                let mut cfg = read_cfg(&f).map_err(|e| format!("o teclado não respondeu: {e}"))?;
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
                    set_feature(&f, &p).map_err(|e| format!("falha ao enviar (teclado desconectado?): {e}"))?;
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
    hex: String, // texto do campo hex enquanto é editado
    speed: u8,
    bright: u8,
    group: usize,
    groups: Vec<Vec<Rgb>>, // [grupo][índice em KEYS]
    tx: Sender<Job>,
    rx: Receiver<Result<String, String>>,
    status: Result<String, String>,
    busy: bool,
    conn: Result<(), String>,
    checked_at: f64,
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

/// Prévia aproximada de cada efeito: o firmware não informa o desenho exato, só o modo.
/// `u`,`v` = centro da tecla normalizado (0..1), `i` = índice em KEYS, `t` = tempo já escalado pela velocidade.
/// Devolve (cor, nível 0..1).
fn preview(mode: u8, base: Option<Rgb>, u: f32, v: f32, i: usize, t: f32) -> (Color32, f32) {
    use std::f32::consts::TAU;
    let hash = |n: u32| (n.wrapping_mul(2_654_435_761) >> 16) as f32 / 65_536.0;
    let hue = |h: f32| Color32::from(egui::ecolor::Hsva::new(h.rem_euclid(1.0), 0.85, 1.0, 1.0));
    let col = |h: f32| base.map_or_else(|| hue(h), c32);
    let (dx, dy) = (u - 0.5, (v - 0.5) * 0.35); // o teclado é ~3x mais largo que alto
    let (ang, r) = (dy.atan2(dx) / TAU, (dx * dx + dy * dy).sqrt());
    let tri = |x: f32| 1.0 - (x.fract() * 2.0 - 1.0).abs(); // 0→1→0
    let peak = |d: f32, width: f32| (1.0 - d.abs() / width).max(0.0);
    // teclas sorteadas que acendem e apagam devagar; sorteio novo a cada ciclo
    let blink = |rate: f32, chance: f32| {
        let k = t * rate + hash(i as u32 * 7 + 1);
        let pick = hash((i as u32).wrapping_mul(131) ^ (k.floor() as u32).wrapping_mul(977));
        if pick < chance { 1.0 - k.fract() } else { 0.0 }
    };
    let n = KEYS.len() as f32;
    match mode {
        1 => (col(u), 1.0),                                                // fixo
        2 => (col(u), 0.5 - 0.5 * (t * 2.0).cos()),                        // respirar
        3 => (hue(u - t * 0.3), 1.0),                                      // arco-íris
        4 => (col(u), if (t * 1.5).fract() < 0.5 { 1.0 } else { 0.0 }),    // piscante
        5 => (col(hash(i as u32)), blink(0.8, 0.25)),                      // pingos de chuva
        6 => (hue(ang - t * 0.3), 1.0),                                    // roda arco-íris
        7 => (col(u), 0.5 + 0.5 * (u * TAU - t * 3.0).sin()),              // ondas
        8 => (col(hash(i as u32 + 3)), blink(1.5, 0.12)),                  // estrelas
        9 => (col(u), 1.0 - blink(0.8, 0.2)),                              // sombra
        10 => (col(i as f32 / n), peak(((t * 12.0) - i as f32).rem_euclid(n), 6.0)), // cobra
        11 => (hue(t * 0.15), 1.0),                                        // fluxo neon
        12 => (col(hash(i as u32 + 5)), blink(1.0, 0.1)),                  // reação
        13 => (col(u), peak(v - 0.5 - 0.4 * (u * TAU - t * 2.5).sin(), 0.25)), // onda senoidal
        14 => (col(u), peak(u - tri(t * 0.5), 0.15)),                      // varredura
        15 => (col(ang), if ((ang - t * 0.3) * 4.0).rem_euclid(1.0) < 0.5 { 1.0 } else { 0.1 }), // moinho
        16 => (col(u), 1.0 - (v - t * 0.8).rem_euclid(1.0)),               // cachoeira
        17 => (col(r * 2.0), 0.5 + 0.5 * (r * 25.0 - t * 4.0).cos()),      // florido
        18 => (col(ang + r * 3.0), 0.5 + 0.5 * ((ang + r * 3.0 - t * 0.5) * TAU * 2.0).cos()), // furacão
        19 => {                                                            // colisão
            let p = 0.5 * tri(t * 0.6);
            (col(u), peak((u - p).abs().min((u - 1.0 + p).abs()), 0.12))
        }
        _ => (hue(u + v * 0.3 - t * 0.3), 1.0),                            // perfeito
    }
}

fn label(name: &str) -> String {
    match name {
        "esc" => "Esc".into(),
        "tab" => "Tab".into(),
        "backspace" => "Bksp".into(),
        "enter" => "Enter".into(),
        "capslock" => "CapsLk".into(),
        "lshift" | "rshift" => "Shift".into(),
        "lctrl" | "rctrl" => "Ctrl".into(),
        "lalt" => "Alt".into(),
        "ralt" => "AltGr".into(),
        "lwin" => "Win".into(),
        "fn" => "Fn".into(),
        "space" => String::new(),
        "app" => "Menu".into(),
        n => n.to_uppercase(),
    }
}

/// Legenda secundária impressa nas teclas do Fizz (função com Fn).
fn sublabel(name: &str) -> Option<&'static str> {
    Some(match name {
        "1" => "F1", "2" => "F2", "3" => "F3", "4" => "F4", "5" => "F5", "6" => "F6",
        "7" => "F7", "8" => "F8", "9" => "F9", "0" => "F10", "-" => "F11", "=" => "F12",
        "backspace" => "MR", "p" => "PRTSC", "´" => "HOME", "[" => "END", "ç" => "PGUP", "~" => "PGDN",
        "." => "INSERT", ";" => "DELETE",
        "fn" => "FN1", "app" => "FN2",
        _ => return None,
    })
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.visuals.panel_fill = BG;
            style.visuals.window_fill = PANEL;
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(12.0, 6.0);
            style.visuals.selection.bg_fill = ACCENT;
        });

        let (tx, rx) = worker(cc.egui_ctx.clone());
        let mut app = App {
            custom: false,
            mode: 1,
            color: [0, 170, 255],
            random: false,
            hex: String::new(),
            speed: 3,
            bright: 4,
            group: 0,
            groups: vec![vec![[0, 170, 255]; KEYS.len()]; GROUPS],
            tx,
            rx,
            status: Ok("Nada é enviado até clicar em Aplicar.".into()),
            busy: false,
            conn: Ok(()),
            checked_at: f64::NEG_INFINITY,
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

    /// Item da lista de efeitos: linha inteira clicável, destaque azul quando selecionado.
    fn item(ui: &mut egui::Ui, selected: bool, text: &str) -> bool {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
        let fill = if selected { ACCENT } else if resp.hovered() { HOVER } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 3.0, fill);
        let color = if selected { Color32::WHITE } else { Color32::from_gray(205) };
        ui.painter().text(rect.left_center() + Vec2::new(12.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(15.0), color);
        resp.clicked()
    }

    fn effects(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Efeito de luz").size(20.0).color(Color32::from_gray(225)));
        ui.add_space(8.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, name) in MODES.iter().enumerate() {
                let mut name = name.to_string();
                name[..1].make_ascii_uppercase();
                if Self::item(ui, !self.custom && self.mode == i as u8 + 1, &name) {
                    (self.custom, self.mode) = (false, i as u8 + 1);
                }
            }
            if Self::item(ui, self.custom, "Personalizado (por tecla)") {
                self.custom = true;
            }
        });
    }

    fn header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("REDRAGON K617").size(18.0).strong().color(Color32::from_gray(225)));
            ui.add_space(16.0);
            let (color, text) = if self.conn.is_ok() { (OK, "Conectado") } else { (ERR, "Não encontrado") };
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, color);
            ui.colored_label(color, text);
            if !self.custom {
                ui.add_space(16.0);
                ui.weak("Prévia aproximada do efeito");
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(260.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.spacing_mut().slider_width = 200.0;
                if self.custom {
                    ui.label("Grupo salvo no teclado");
                    ui.horizontal(|ui| {
                        for g in 0..GROUPS {
                            ui.selectable_value(&mut self.group, g, format!(" {} ", g + 1));
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Preencher tudo").clicked() {
                            self.groups[self.group].fill(self.color);
                        }
                        if ui.button("Limpar").clicked() {
                            self.groups[self.group].fill([0, 0, 0]);
                        }
                    });
                    ui.add_space(6.0);
                    ui.weak("Clique ou arraste no teclado para pintar.\nShift+clique copia a cor da tecla.");
                } else {
                    ui.label("Brilho");
                    ui.add(egui::Slider::new(&mut self.bright, 0..=4).trailing_fill(true));
                    ui.add_space(8.0);
                    ui.label("Velocidade");
                    ui.add(egui::Slider::new(&mut self.speed, 1..=4).trailing_fill(true));
                    ui.add_space(8.0);
                    ui.checkbox(&mut self.random, "Cores aleatórias");
                }
            });
            ui.add_space(40.0);

            ui.vertical(|ui| {
                ui.add_enabled_ui(self.custom || !self.random, |ui| {
                    ui.spacing_mut().slider_width = 120.0;
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    let hex = egui::TextEdit::singleline(&mut self.hex)
                        .font(egui::TextStyle::Monospace)
                        .char_limit(7)
                        .desired_width(80.0);
                    let resp = ui.add(hex).on_hover_text("Digite a cor em hexadecimal, ex.: FF8800");
                    if resp.changed() {
                        if let Ok(c) = parse_rgb(self.hex.trim()) {
                            self.color = c;
                        }
                    }
                    if !resp.has_focus() {
                        let [r, g, b] = self.color;
                        self.hex = format!("#{r:02X}{g:02X}{b:02X}");
                    }
                    let mut c = c32(self.color);
                    use egui::widgets::color_picker::{color_picker_color32, Alpha};
                    if color_picker_color32(ui, &mut c, Alpha::Opaque) {
                        let [r, g, b, _] = c.to_array();
                        self.color = [r, g, b];
                    }
                });
            });
            ui.add_space(40.0);

            ui.vertical(|ui| {
                ui.set_width(270.0);
                ui.label("Cores rápidas");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for c in SWATCHES {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                        ui.painter().rect_filled(r, 3.0, c32(c));
                        if self.color == c {
                            ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
                        }
                        if resp.clicked() {
                            (self.color, self.random) = (c, false);
                        }
                    }
                });
                ui.add_space(40.0);
                self.apply_bar(ui);
            });
        });
    }

    fn apply_bar(&mut self, ui: &mut egui::Ui) {
        let apply = egui::Button::new(egui::RichText::new("Aplicar").strong().size(16.0))
            .fill(ACCENT)
            .min_size(Vec2::new(ui.available_width(), 38.0));
        if ui.add_enabled(!self.busy && self.conn.is_ok(), apply).clicked() {
            self.apply();
        }
        match (&self.conn, &self.status) {
            (Err(e), _) => ui.colored_label(ERR, e),
            (_, Err(e)) => ui.colored_label(ERR, e),
            (_, Ok(m)) if m.ends_with("aplicado") => ui.colored_label(OK, format!("✔ {m}")),
            (_, Ok(m)) => ui.weak(m),
        };
    }

    fn keyboard(&mut self, ui: &mut egui::Ui) {
        let (w, h) = (547.0 - 10.0, 188.0 - 10.0); // limites do layout do INI
        let pad = 9.0; // borda do gabinete, em px do layout
        let avail = ui.available_size();
        let scale = (avail.x / (w + 2.0 * pad)).min(avail.y / (h + 2.0 * pad)).min(2.2);
        let size = Vec2::new(w, h) * scale;
        let at = ui.cursor().min + (avail - size) / 2.0; // centraliza
        let resp = ui.allocate_rect(Rect::from_min_size(at, size), Sense::click_and_drag());
        let painter = ui.painter().clone();
        let origin = resp.rect.min - Vec2::splat(10.0 * scale);
        let px = |[x1, y1, x2, y2]: [u16; 4]| {
            Rect::from_min_max(origin + Vec2::new(x1 as f32, y1 as f32) * scale, origin + Vec2::new(x2 as f32, y2 as f32) * scale)
        };
        let t = ui.input(|i| i.time) as f32;
        let pointer = resp.hover_pos();
        let pressed = resp.is_pointer_button_down_on() || resp.clicked();
        let shift = ui.input(|i| i.modifiers.shift);

        // gabinete fino e preto, como o do Fizz
        let body = resp.rect.expand(pad * scale);
        painter.rect_filled(body.translate(Vec2::new(0.0, 8.0)), 6.0, Color32::from_black_alpha(110));
        painter.rect(body, 5.0, CHASSIS, Stroke::new(1.5, Color32::from_gray(40)), egui::StrokeKind::Inside);

        // cor acesa de cada tecla + retângulos (o Enter ABNT2 é em L: parte de cima na linha do Tab)
        let keys: Vec<(&str, Vec<Rect>, Color32)> = KEYS
            .iter()
            .enumerate()
            .map(|(i, &(name, _, r @ [x1, y1, x2, y2]))| {
                let mut rects = vec![px(r)];
                if name == "enter" {
                    rects.push(px([505, 51, 546, 100]));
                }
                let hovered = pointer.is_some_and(|p| rects.iter().any(|r| r.contains(p)));
                if self.custom && hovered && pressed {
                    if shift {
                        self.color = self.groups[self.group][i];
                    } else {
                        self.groups[self.group][i] = self.color;
                    }
                }
                let lit = if self.custom {
                    c32(self.groups[self.group][i])
                } else {
                    let (u, v) = (((x1 + x2) as f32 / 2.0 - 10.0) / w, ((y1 + y2) as f32 / 2.0 - 10.0) / h);
                    let base = (!self.random).then_some(self.color);
                    let (c, level) = preview(self.mode, base, u, v, i, t * (0.4 + 0.3 * self.speed as f32));
                    let bright = if self.bright == 0 { 0.0 } else { 0.4 + 0.15 * self.bright as f32 };
                    Color32::BLACK.lerp_to_gamma(c, level.clamp(0.0, 1.0) * bright)
                };
                (name, rects, lit)
            })
            .collect();

        // 1º a luz que vaza por baixo das teclas, depois as teclas por cima
        for (_, rects, lit) in &keys {
            for r in rects {
                let glow = egui::epaint::Shadow { offset: [0, 2], blur: (7.0 * scale) as u8, spread: 1, color: lit.gamma_multiply(0.9) };
                painter.add(glow.as_shape(*r, 3.0 * scale));
            }
        }
        for (name, rects, lit) in &keys {
            for r in rects {
                let cap = r.shrink(0.8 * scale);
                painter.rect_filled(cap, 2.5 * scale, Color32::from_rgb(0x0c, 0x0c, 0x0e));
            }
            for r in rects {
                let top = Rect::from_min_max(r.min + Vec2::new(2.0, 1.2) * scale, r.max - Vec2::new(2.0, 3.6) * scale);
                painter.rect_filled(top, 2.0 * scale, Color32::from_rgb(0x19, 0x19, 0x1d));
            }
            let top = rects.iter().fold(rects[0], |a, b| a.union(*b));
            let top = Rect::from_min_max(top.min + Vec2::new(2.0, 1.2) * scale, top.max - Vec2::new(2.0, 3.6) * scale);
            if self.custom && pointer.is_some_and(|p| rects.iter().any(|r| r.contains(p))) {
                for r in rects {
                    painter.rect_stroke(r.shrink(0.8 * scale), 2.5 * scale, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
                }
            }
            // legenda acesa na cor do LED (tecla apagada: cinza escuro, ainda legível)
            let [r, g, b, _] = lit.to_array();
            let legend = Color32::from_rgb(r.max(62), g.max(62), b.max(68));
            let corner = if *name == "enter" { rects[1].min } else { top.min };
            painter.text(
                corner + Vec2::new(3.0, 2.0) * scale,
                egui::Align2::LEFT_TOP,
                label(name),
                egui::FontId::proportional(7.0 * scale),
                legend,
            );
            if let Some(s) = sublabel(name) {
                painter.text(
                    egui::pos2(top.center().x, top.max.y - 2.0 * scale),
                    egui::Align2::CENTER_BOTTOM,
                    s,
                    egui::FontId::proportional(4.6 * scale),
                    Color32::from_gray(150),
                );
            }
        }
        if !self.custom && self.mode != 1 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33)); // ~30 fps de prévia
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok(r) = self.rx.try_recv() {
            self.status = r;
            self.busy = false;
            self.checked_at = f64::NEG_INFINITY; // reconfere a conexão depois de cada envio
        }
        // Detecta conectar/desconectar: abrir o hidraw é só uma leitura do sysfs.
        let now = ui.input(|i| i.time);
        if !self.busy && now - self.checked_at > 2.0 {
            self.conn = open_device().map(drop).map_err(|e| e.to_string());
            self.checked_at = now;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));

        egui::Panel::left("effects")
            .exact_size(240.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12.0))
            .show(ui, |ui| self.effects(ui));
        egui::Panel::bottom("controls")
            .frame(egui::Frame::new().fill(PANEL2).inner_margin(egui::Margin::symmetric(32, 22)))
            .show(ui, |ui| self.controls(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(20.0))
            .show(ui, |ui| {
                self.header(ui);
                self.keyboard(ui);
            });
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
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1100.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("k617rgb", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
