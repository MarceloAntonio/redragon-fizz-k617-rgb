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
use std::{env, process, thread, time::Duration};

const REPORT_LEN: usize = 1032;
const HDR: usize = 8; // report id + 7 bytes de comando
const CMD_LEN: usize = 6; // report 5: id + 5 bytes (Report Count 5 no descritor)
const CFG_LEN: usize = 136;
const CUSTOM_MODE: u8 = 21;
const PALETTE_SLOT: usize = 21; // 7 cores RGB por modo
const PLANE: usize = 126; // 6 linhas × 21 colunas

// (nome, índice do LED) — seção [KEY] do INI do instalador, layout ABNT2
const KEYS: &[(&str, usize)] = &[
    ("esc", 21), ("1", 22), ("2", 23), ("3", 24), ("4", 25), ("5", 26), ("6", 27), ("7", 28),
    ("8", 29), ("9", 30), ("0", 31), ("-", 32), ("=", 33), ("backspace", 34), ("tab", 42),
    ("q", 43), ("w", 44), ("e", 45), ("r", 46), ("t", 47), ("y", 48), ("u", 49), ("i", 50),
    ("o", 51), ("p", 52), ("[", 53), ("]", 54), ("\\", 75), ("capslock", 63), ("a", 64),
    ("s", 65), ("d", 66), ("f", 67), ("g", 68), ("h", 69), ("j", 70), ("k", 71), ("l", 72),
    (";", 73), ("'", 74), ("enter", 76), ("lshift", 84), ("z", 86), ("x", 87), ("c", 88),
    ("v", 89), ("b", 90), ("n", 91), ("m", 92), (",", 93), (".", 94), ("/", 95),
    ("rshift", 97), ("lctrl", 105), ("lwin", 106), ("lalt", 107), ("space", 110),
    ("ralt", 113), ("fn", 114), ("app", 117), ("rctrl", 118), ("<", 85), ("?", 96),
];

const MODES: &[&str] = &[
    "fixo", "respirar", "arco-íris", "piscante", "pingos de chuva", "roda arco-íris",
    "ondas", "estrelas", "sombra", "cobra", "fluxo neon", "reação", "onda senoidal",
    "varredura", "moinho", "cachoeira", "florido", "furacão", "colisão", "perfeito",
];

// ---------- pacotes (funções puras) ----------

fn header(cmd: [u8; 7]) -> Vec<u8> {
    let mut p = vec![0u8; REPORT_LEN];
    p[0] = 6;
    p[1..HDR].copy_from_slice(&cmd);
    p
}

fn write_cfg_packet(cfg: &[u8; CFG_LEN]) -> Vec<u8> {
    let mut p = header([0x03, 0xB6, 0, 0, 0, 0, 0]);
    p[HDR..HDR + CFG_LEN].copy_from_slice(cfg);
    p
}

/// Cor escolhida no slot 0 de todos os modos; demais slots com cores básicas.
fn palette_packet(rgb: [u8; 3]) -> Vec<u8> {
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
fn keys_packets(group: usize, colors: &[(usize, [u8; 3])]) -> [Vec<u8>; 2] {
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
fn apply_mode(cfg: &mut [u8; CFG_LEN], mode: u8, color_idx: Option<u8>, speed: Option<u8>, bright: Option<u8>) {
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

fn set_feature(f: &File, buf: &[u8]) -> io::Result<()> {
    feature(f, 0x06, &mut buf.to_vec())?;
    thread::sleep(Duration::from_millis(50));
    Ok(())
}

/// Acha o hidraw do K617 cuja descrição HID declara o report 6 (interface de controle).
fn open_device() -> io::Result<File> {
    for e in fs::read_dir("/sys/class/hidraw")? {
        let dir = e?.path();
        let uevent = fs::read_to_string(dir.join("device/uevent")).unwrap_or_default();
        if !uevent.to_uppercase().contains("0000258A:00000049") {
            continue;
        }
        let desc = fs::read(dir.join("device/report_descriptor")).unwrap_or_default();
        if desc.windows(2).any(|w| w == [0x85, 0x06]) {
            let dev = format!("/dev/{}", dir.file_name().unwrap().to_string_lossy());
            return OpenOptions::new().read(true).write(true).open(&dev).map_err(|e| {
                io::Error::new(e.kind(), format!("{dev}: {e} (veja a regra udev no README)"))
            });
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "K617 (258a:0049) não encontrado"))
}

fn read_cfg(f: &File) -> io::Result<[u8; CFG_LEN]> {
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

// ---------- CLI ----------

fn parse_rgb(s: &str) -> Result<[u8; 3], String> {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).map_err(|_| format!("cor inválida: {s}"))?;
    if s.trim_start_matches('#').len() != 6 {
        return Err(format!("cor inválida: {s} (use RRGGBB)"));
    }
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn num(args: &[String], name: &str, lo: u8, hi: u8) -> Result<Option<u8>, String> {
    flag(args, name)
        .map(|v| v.parse().ok().filter(|n| (lo..=hi).contains(n)).ok_or(format!("{name} deve ser {lo}-{hi}")))
        .transpose()
}

fn usage() -> ! {
    let modes: String = MODES.iter().enumerate().map(|(i, m)| format!("  {:>2} {m}\n", i + 1)).collect();
    eprintln!(
        "uso: k617rgb [--dry-run] <comando>\n\n\
         \x20 dump                                   mostra os 136 bytes de config\n\
         \x20 mode <1-20> [--color RRGGBB|random] [--speed 1-4] [--brightness 0-4]\n\
         \x20 off                                    brilho 0 no modo atual\n\
         \x20 keys <grupo 1-5> <RRGGBB> [tecla=RRGGBB ...]   cor por tecla (modo 21)\n\n\
         modos:\n{modes}"
    );
    process::exit(2)
}

fn run(args: &[String]) -> Result<(), String> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let args: Vec<String> = args.iter().filter(|a| *a != "--dry-run").cloned().collect();
    let cmd = args.first().map(String::as_str).unwrap_or_else(|| usage());
    if !["dump", "mode", "off", "keys"].contains(&cmd) {
        usage();
    }

    let dev = if dry { None } else { Some(open_device().map_err(|e| e.to_string())?) };
    let mut cfg = match &dev {
        Some(f) => read_cfg(f).map_err(|e| format!("lendo config: {e}"))?,
        None => [0u8; CFG_LEN],
    };
    let send = |p: &[u8]| -> Result<(), String> {
        match &dev {
            Some(f) => set_feature(f, p).map_err(|e| format!("enviando {:02x?}: {e}", &p[..HDR])),
            None => Ok(println!("{:02x?} …", &p[..HDR + 24])),
        }
    };

    match cmd {
        "dump" => {
            for (i, row) in cfg.chunks(16).enumerate() {
                println!("{:02x}: {:02x?}", i * 16, row);
            }
        }
        "mode" => {
            let mode: u8 = args.get(1).and_then(|m| m.parse().ok()).filter(|m| (1..=20).contains(m)).ok_or("modo deve ser 1-20")?;
            let color = flag(&args, "--color");
            let color_idx = match color {
                Some("random") => Some(7),
                Some(c) => {
                    send(&palette_packet(parse_rgb(c)?))?;
                    Some(0)
                }
                None => None,
            };
            apply_mode(&mut cfg, mode, color_idx, num(&args, "--speed", 1, 4)?, num(&args, "--brightness", 0, 4)?);
            send(&write_cfg_packet(&cfg))?;
        }
        "off" => {
            let mode = if (1..=20).contains(&cfg[13]) { cfg[13] } else { 1 };
            apply_mode(&mut cfg, mode, None, None, Some(0));
            send(&write_cfg_packet(&cfg))?;
        }
        "keys" => {
            let group: usize = args.get(1).and_then(|g| g.parse().ok()).filter(|g| (1..=5).contains(g)).ok_or("grupo deve ser 1-5")?;
            let base = parse_rgb(args.get(2).ok_or("falta a cor base")?)?;
            let mut colors: Vec<(usize, [u8; 3])> = KEYS.iter().map(|&(_, led)| (led, base)).collect();
            for kv in &args[3..] {
                let (k, c) = kv.split_once('=').ok_or(format!("use tecla=RRGGBB: {kv}"))?;
                let led = KEYS.iter().find(|(n, _)| *n == k.to_lowercase()).ok_or(format!("tecla desconhecida: {k}"))?.1;
                colors.push((led, parse_rgb(c)?));
            }
            for p in keys_packets(group - 1, &colors) {
                send(&p)?;
            }
            apply_mode(&mut cfg, CUSTOM_MODE, None, None, None);
            cfg[14] = 0x20 + (group - 1) as u8; // StartG + grupo
            send(&write_cfg_packet(&cfg))?;
        }
        _ => usage(),
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("erro: {e}");
        process::exit(1);
    }
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
    }
}
