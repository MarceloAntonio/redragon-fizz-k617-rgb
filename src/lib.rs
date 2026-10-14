//! Controle de RGB do Redragon K617 (USB 258a:0049, chip Sinowealth) no Linux via hidraw.
//!
//! Protocolo obtido por engenharia reversa do KeyboardDrv.exe v1.6.6 (software oficial):
//! - report 5 (6 bytes, feature): comando curto, ex. `05 83 B6 ..` = "me envie a config"
//! - report 6 (1032 bytes, feature): bloco de dados. Cabeçalho de 8 bytes + payload.
//!   06 03 B6 ..  grava config (136 bytes)      06 08 B8 00 40 ..  paleta (7 cores/modo)
//!   06 09 BC/C0 00 40 ..  cores por tecla (modo 21, 5 grupos × planos B,G,R × 126 LEDs)

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::{thread, time::Duration};

pub const REPORT_LEN: usize = 1032;
pub const HDR: usize = 8; // report id + 7 bytes de comando
const CMD_LEN: usize = 6; // report 5: id + 5 bytes (Report Count 5 no descritor)
pub const CFG_LEN: usize = 136;
pub const CUSTOM_MODE: u8 = 21;
pub const GROUPS: usize = 5;
const PALETTE_SLOT: usize = 21; // 7 cores RGB por modo
const PLANE: usize = 126; // 6 linhas × 21 colunas

pub type Rgb = [u8; 3];
pub type Cfg = [u8; CFG_LEN];

/// (nome, índice do LED, retângulo x1,y1,x2,y2 em px) — seção [KEY] do INI do instalador.
/// Nomes seguem as legendas ABNT2 (o INI usa os nomes do layout US para as mesmas posições).
pub const KEYS: &[(&str, usize, [u16; 4])] = &[
    ("esc", 21, [15,14,39,42]), ("1", 22, [51,14,75,42]), ("2", 23, [87,14,111,42]), ("3", 24, [124,14,148,42]),
    ("4", 25, [160,14,184,42]), ("5", 26, [197,14,221,42]), ("6", 27, [233,15,257,43]), ("7", 28, [269,14,293,42]),
    ("8", 29, [306,14,330,42]), ("9", 30, [341,14,365,42]), ("0", 31, [378,14,402,42]), ("-", 32, [414,14,438,42]),
    ("=", 33, [450,14,474,42]), ("backspace", 34, [487,14,546,42]), ("tab", 42, [15,51,56,79]), ("q", 43, [70,51,94,79]),
    ("w", 44, [107,51,131,79]), ("e", 45, [143,51,167,79]), ("r", 46, [179,50,203,78]), ("t", 47, [215,50,239,78]),
    ("y", 48, [250,50,274,78]), ("u", 49, [286,50,310,78]), ("i", 50, [322,50,346,78]), ("o", 51, [359,51,383,79]),
    ("p", 52, [395,51,419,79]), ("´", 53, [432,51,456,79]), ("[", 54, [468,51,492,79]), ("]", 75, [477,86,500,114]),
    ("capslock", 63, [14,86,65,114]), ("a", 64, [80,87,104,115]), ("s", 65, [116,87,140,115]), ("d", 66, [152,87,176,115]),
    ("f", 67, [188,87,212,115]), ("g", 68, [225,87,249,115]), ("h", 69, [260,87,284,115]), ("j", 70, [296,87,320,115]),
    ("k", 71, [332,87,356,115]), ("l", 72, [368,87,392,115]), ("ç", 73, [405,87,429,115]), ("~", 74, [441,87,465,115]),
    ("enter", 76, [514,87,546,115]), ("lshift", 84, [16,123,47,151]), ("z", 86, [98,123,122,151]), ("x", 87, [134,123,158,151]),
    ("c", 88, [170,123,194,151]), ("v", 89, [206,123,230,151]), ("b", 90, [242,123,266,151]), ("n", 91, [278,123,302,151]),
    ("m", 92, [314,123,338,151]), (",", 93, [351,123,375,151]), (".", 94, [386,123,410,151]), (";", 95, [422,123,446,151]),
    ("rshift", 97, [497,123,547,151]), ("lctrl", 105, [15,160,47,188]), ("lwin", 106, [62,160,94,188]), ("lalt", 107, [107,160,139,188]),
    ("space", 110, [150,160,364,188]), ("ralt", 113, [377,160,409,188]), ("fn", 114, [423,160,456,188]), ("app", 117, [468,160,501,188]),
    ("rctrl", 118, [514,160,547,188]), ("\\", 85, [61,122,85,150]), ("/", 96, [460,122,484,150]),
];

pub const MODES: &[&str] = &[
    "fixo", "respirar", "arco-íris", "piscante", "pingos de chuva", "roda arco-íris",
    "ondas", "estrelas", "sombra", "cobra", "fluxo neon", "reação", "onda senoidal",
    "varredura", "moinho", "cachoeira", "florido", "furacão", "colisão", "perfeito",
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Color {
    Keep,
    Random,
    Rgb(Rgb),
}

// ---------- pacotes (funções puras) ----------

fn header(cmd: [u8; 7]) -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[0] = 6;
    p[1..HDR].copy_from_slice(&cmd);
    p
}

pub fn write_cfg_packet(cfg: &Cfg) -> Vec<u8> {
    let mut p = header([0x03, 0xB6, 0, 0, 0, 0, 0]);
    p[HDR..HDR + CFG_LEN].copy_from_slice(cfg);
    p
}

/// Cor escolhida no slot 0 de todos os modos; demais slots com cores básicas.
fn palette_packet(rgb: Rgb) -> Vec<u8> {
    let mut p = header([0x08, 0xB8, 0, 0x40, 0, 0, 0]);
    let others = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 255, 255], [255, 0, 255]];
    for mode in 1..=MODES.len() + 2 {
        let at = HDR + PALETTE_SLOT * mode;
        for (k, c) in std::iter::once(rgb).chain(others).enumerate() {
            p[at + 3 * k..at + 3 * k + 3].copy_from_slice(&c);
        }
    }
    p
}

/// Planos na ordem B,G,R (GameRGBIndex=2,1,0). Só o `group` recebe as cores.
fn keys_packets(group: usize, colors: &[(usize, Rgb)]) -> [Vec<u8>; 2] {
    let mut data = vec![0u8; 2048];
    for &(led, [r, g, b]) in colors {
        for (plane, v) in [(2, r), (1, g), (0, b)] {
            data[(3 * group + plane) * PLANE + led] = v;
        }
    }
    let mut a = header([0x09, 0xBC, 0, 0x40, 0, 0, 0]);
    let mut b = header([0x09, 0xC0, 0, 0x40, 0, 0, 0]);
    a[HDR..HDR + 1024].copy_from_slice(&data[..1024]);
    b[HDR..HDR + 1024].copy_from_slice(&data[1024..]);
    [a, b]
}

/// Bytes por modo: cfg[0x1c+2m] = cor (nibble baixo; 7 = aleatória), cfg[0x1d+2m] = vel<<4 | brilho.
pub fn apply_mode(cfg: &mut Cfg, mode: u8, color_idx: Option<u8>, speed: Option<u8>, bright: Option<u8>) {
    let (c, sb) = (0x1c + 2 * mode as usize, 0x1d + 2 * mode as usize);
    cfg[12] = (mode == CUSTOM_MODE) as u8;
    cfg[13] = mode;
    if mode != CUSTOM_MODE {
        if let Some(i) = color_idx {
            cfg[c] = (cfg[c] & 0xF0) | i;
        }
        if let Some(s) = speed {
            cfg[sb] = (cfg[sb] & 0x0F) | (s << 4);
        }
        if let Some(b) = bright {
            cfg[sb] = (cfg[sb] & 0xF0) | b;
        }
    }
    cfg[0x46] = 0x5A;
    cfg[0x47] = 0xA5;
}

/// Pacotes para ativar um efeito (1-20). Atualiza `cfg`.
pub fn mode_packets(cfg: &mut Cfg, mode: u8, color: Color, speed: Option<u8>, bright: Option<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let color_idx = match color {
        Color::Keep => None,
        Color::Random => Some(7),
        Color::Rgb(c) => {
            out.push(palette_packet(c));
            Some(0)
        }
    };
    apply_mode(cfg, mode, color_idx, speed, bright);
    out.push(write_cfg_packet(cfg));
    out
}

/// Pacotes para o modo personalizado: cor fixa por LED no grupo 0-4. Atualiza `cfg`.
pub fn custom_packets(cfg: &mut Cfg, group: usize, colors: &[(usize, Rgb)]) -> Vec<Vec<u8>> {
    let mut out = keys_packets(group, colors).to_vec();
    apply_mode(cfg, CUSTOM_MODE, None, None, None);
    cfg[14] = 0x20 + group as u8; // StartG + grupo
    out.push(write_cfg_packet(cfg));
    out
}

// ---------- hidraw ----------

const fn hidioc(nr: u64, len: usize) -> u64 {
    (3 << 30) | ((len as u64) << 16) | ((b'H' as u64) << 8) | nr
}

extern "C" {
    fn ioctl(fd: i32, req: u64, ...) -> i32;
}

fn feature(f: &File, nr: u64, buf: &mut [u8]) -> io::Result<()> {
    // SAFETY: buf é válido por buf.len() bytes, que é o tamanho codificado no request.
    let r = unsafe { ioctl(f.as_raw_fd(), hidioc(nr, buf.len()), buf.as_mut_ptr()) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

pub fn set_feature(f: &File, buf: &[u8]) -> io::Result<()> {
    feature(f, 0x06, &mut buf.to_vec())?;
    thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// Acha o hidraw do K617 cuja descrição HID declara o report 6 (interface de controle).
pub fn open_device() -> io::Result<File> {
    for e in fs::read_dir("/sys/class/hidraw")? {
        let dir = e?.path();
        let uevent = fs::read_to_string(dir.join("device/uevent")).unwrap_or_default();
        if !uevent.to_uppercase().contains("0000258A:00000049") {
            continue;
        }
        let desc = fs::read(dir.join("device/report_descriptor")).unwrap_or_default();
        if desc.windows(2).any(|w| w == [0x85, 0x06]) {
            let dev = format!("/dev/{}", dir.file_name().unwrap().to_string_lossy());
            return OpenOptions::new().read(true).write(true).open(&dev).map_err(|e| match e.kind() {
                io::ErrorKind::PermissionDenied => {
                    io::Error::new(e.kind(), format!("Sem permissão para {dev}. Configure a regra udev (veja o README)."))
                }
                _ => io::Error::new(e.kind(), format!("{dev}: {e}")),
            });
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "Teclado K617 (258a:0049) não encontrado. Confira o cabo USB."))
}

pub fn read_cfg(f: &File) -> io::Result<Cfg> {
    let cmd: [u8; CMD_LEN] = [0x05, 0x83, 0xB6, 0, 0, 0];
    set_feature(f, &cmd)?;
    let mut buf = vec![0u8; REPORT_LEN];
    buf[0] = 6;
    feature(f, 0x07, &mut buf)?;
    if buf[1] != 0x83 {
        return Err(io::Error::other(format!("resposta inesperada: {:02x?}", &buf[..16])));
    }
    Ok(buf[HDR..HDR + CFG_LEN].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_layout() {
        assert_eq!(hidioc(0x06, 7), 0xC007_4806); // HIDIOCSFEATURE(7)

        let mut cfg = [0u8; CFG_LEN];
        apply_mode(&mut cfg, 1, Some(0), Some(3), Some(4));
        assert_eq!((cfg[12], cfg[13], cfg[0x1e], cfg[0x1f]), (0, 1, 0x00, 0x34));
        assert_eq!(&cfg[0x46..0x48], &[0x5A, 0xA5]);
        let p = write_cfg_packet(&cfg);
        assert_eq!((p.len(), &p[..3], p[HDR + 13]), (REPORT_LEN, &[6, 3, 0xB6][..], 1));

        let p = palette_packet([1, 2, 3]);
        assert_eq!(&p[HDR..HDR + 21], &[0; 21]); // slot do modo 0 fica vazio
        assert_eq!(&p[HDR + 21..HDR + 24], &[1, 2, 3]);

        // grupo 2, Esc (LED 21) vermelho -> plano R = 3*1+2
        let [a, _] = keys_packets(1, &[(21, [255, 0, 0])]);
        assert_eq!(a[HDR + 5 * PLANE + 21], 255);
        assert_eq!(a[HDR + 3 * PLANE + 21], 0);

        let ps = custom_packets(&mut cfg, 1, &[]);
        assert_eq!((ps.len(), cfg[12], cfg[13], cfg[14]), (3, 1, CUSTOM_MODE, 0x21));
        assert_eq!(mode_packets(&mut cfg, 2, Color::Keep, None, None).len(), 1);
    }
}
