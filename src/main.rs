use k617rgb::*;
use std::{env, process};

fn parse_rgb(s: &str) -> Result<Rgb, String> {
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
    let send = |packets: Vec<Vec<u8>>| -> Result<(), String> {
        for p in packets {
            match &dev {
                Some(f) => set_feature(f, &p).map_err(|e| format!("enviando {:02x?}: {e}", &p[..HDR]))?,
                None => println!("{:02x?} …", &p[..HDR + 24]),
            }
        }
        Ok(())
    };

    match cmd {
        "dump" => {
            for (i, row) in cfg.chunks(16).enumerate() {
                println!("{:02x}: {:02x?}", i * 16, row);
            }
        }
        "mode" => {
            let mode: u8 = args.get(1).and_then(|m| m.parse().ok()).filter(|m| (1..=20).contains(m)).ok_or("modo deve ser 1-20")?;
            let color = match flag(&args, "--color") {
                Some("random") => Color::Random,
                Some(c) => Color::Rgb(parse_rgb(c)?),
                None => Color::Keep,
            };
            let (speed, bright) = (num(&args, "--speed", 1, 4)?, num(&args, "--brightness", 0, 4)?);
            send(mode_packets(&mut cfg, mode, color, speed, bright))?;
        }
        "off" => {
            let mode = if (1..=20).contains(&cfg[13]) { cfg[13] } else { 1 };
            send(mode_packets(&mut cfg, mode, Color::Keep, None, Some(0)))?;
        }
        "keys" => {
            let group: usize = args.get(1).and_then(|g| g.parse().ok()).filter(|g| (1..=GROUPS).contains(g)).ok_or("grupo deve ser 1-5")?;
            let base = parse_rgb(args.get(2).ok_or("falta a cor base")?)?;
            let mut colors: Vec<(usize, Rgb)> = KEYS.iter().map(|&(_, led, _)| (led, base)).collect();
            for kv in &args[3..] {
                let (k, c) = kv.split_once('=').ok_or(format!("use tecla=RRGGBB: {kv}"))?;
                let led = KEYS.iter().find(|(n, ..)| *n == k.to_lowercase()).ok_or(format!("tecla desconhecida: {k}"))?.1;
                colors.push((led, parse_rgb(c)?));
            }
            send(custom_packets(&mut cfg, group - 1, &colors))?;
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
