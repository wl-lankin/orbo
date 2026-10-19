mod mcp;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use clap::{Parser, Subcommand};
use orbo_core::catalog::Catalog;
use orbo_core::jobs::{convert_blocking, convert_merge_blocking, CropRect, JobSpec, MergeSpec, Options};
use orbo_core::rules::Rules;

#[derive(Parser)]
#[command(name = "orbo", about = "Orbo: Dateien lokal umwandeln")]
struct Cli {
    /// Einstellungen der Orbo-App (Abläufe, KI-Ziele) nicht laden
    #[arg(long, global = true)]
    no_settings: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Erkennt Format und Eigenschaften einer Datei
    Analyze {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Zeigt, was der Ring für diese Datei anbieten würde (bei mehreren Dateien auch Zusammenführen)
    Suggest {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
        /// Welche Regeln gegriffen haben
        #[arg(long)]
        explain: bool,
    },
    /// Wandelt eine oder mehrere Dateien um; Ziele mit „mehrere Dateien“ (z. B. pdf_merge) machen aus allen eine
    Convert {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Ziel-ID, z. B. jpg, mp3, mail_video (siehe `orbo targets`)
        #[arg(long)]
        to: String,
        #[arg(long)]
        quality: Option<u8>,
        #[arg(long)]
        max_edge: Option<u32>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        /// x,y,breite,höhe in Pixeln
        #[arg(long)]
        crop: Option<String>,
        /// z. B. 1:1, 16:9, 9:16
        #[arg(long)]
        aspect: Option<String>,
        #[arg(long)]
        rotate: Option<u32>,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        target_mb: Option<f64>,
        #[arg(long)]
        bitrate: Option<u32>,
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        background: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Listet alle Ziele
    Targets,
    /// Startet den MCP-Server (stdio) für Claude, Codex und andere KI-Programme
    Mcp,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if !cli.no_settings {
        orbo_core::appconfig::apply_app_settings();
    }
    match cli.cmd {
        Cmd::Analyze { file, json } => {
            let a = orbo_core::analyze(&file)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&a)?);
            } else {
                println!("{:?} ({:?}), {} Bytes{}", a.format, a.kind, a.size, if a.ext_mismatch { ", Endung passt nicht zum Inhalt" } else { "" });
                if let Some(i) = &a.image {
                    println!("Bild {}×{}, Transparenz {}, animiert {}, GPS {}", i.width, i.height, i.has_alpha, i.animated, i.has_gps);
                }
                if let Some(m) = &a.media {
                    println!("Dauer {:.1} s, Video {:?}, Audio {:?}", m.duration_s, m.video.as_ref().map(|v| &v.codec), m.audio.as_ref().map(|a| &a.codec));
                }
            }
        }
        Cmd::Suggest { files, json, explain } => {
            let (_, s) = orbo_core::suggest_many(&files)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                println!("Mitte: {}", s.default.as_deref().unwrap_or("-"));
                for (i, x) in s.ring.iter().enumerate() {
                    println!("  {}. {:<12} {:<28} .{}  [{}]", i + 1, x.title, x.sub, x.ext, x.id);
                }
                println!("Mehr ({}): {}", s.more.len(), s.more.iter().map(|x| x.id.as_str()).collect::<Vec<_>>().join(", "));
                if explain {
                    for f in &s.fired {
                        println!("  Regel: {f}");
                    }
                }
            }
        }
        Cmd::Convert { files, to, quality, max_edge, width, height, crop, aspect, rotate, start, end, target_mb, bitrate, preset, background, out } => {
            let crop = crop.map(|c| {
                let v: Vec<u32> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                anyhow::ensure!(v.len() == 4, "--crop braucht x,y,breite,höhe");
                Ok(CropRect { x: v[0], y: v[1], width: v[2], height: v[3] })
            });
            let options = Options {
                quality,
                max_edge,
                width,
                height,
                crop: crop.transpose()?,
                aspect,
                rotate,
                start_s: start,
                end_s: end,
                target_mb,
                audio_bitrate_k: bitrate,
                preset,
                background,
                out_dir: out,
                ..Default::default()
            };
            let merge = Catalog::current().get(&to).map_or(false, |t| t.merge);
            if merge {
                if files.len() < 2 {
                    eprintln!("✗ „{to}“ führt mehrere Dateien zu einer zusammen und braucht mindestens zwei Eingabedateien (angegeben: {}).", files.len());
                    std::process::exit(1);
                }
                let started = std::time::Instant::now();
                let spec = MergeSpec { inputs: files.clone(), target: to.clone(), options };
                match convert_merge_blocking(&spec, 1, &AtomicBool::new(false), &|_| {}) {
                    Ok(p) => {
                        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        println!("✓ {} Dateien → {} ({} KB, {} ms)", files.len(), p.display(), bytes / 1024, started.elapsed().as_millis());
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("✗ {e:#}");
                        std::process::exit(1);
                    }
                }
            }
            let mut failed = 0;
            for (i, input) in files.into_iter().enumerate() {
                let spec = JobSpec { input: input.clone(), target: to.clone(), options: options.clone() };
                let started = std::time::Instant::now();
                match convert_blocking(&spec, i as u64 + 1, &AtomicBool::new(false), &|_| {}) {
                    Ok(p) => {
                        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        println!("✓ {} → {} ({} KB, {} ms)", input.display(), p.display(), bytes / 1024, started.elapsed().as_millis());
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("✗ {}: {e:#}", input.display());
                    }
                }
            }
            if failed > 0 {
                std::process::exit(1);
            }
        }
        Cmd::Targets => {
            let mut group = String::new();
            for t in &Catalog::current().targets {
                if t.group != group {
                    group = t.group.clone();
                    println!("\n{group}");
                }
                let many = if t.merge { "  (mehrere Dateien)" } else { "" };
                println!("  {:<16} {:<14} {:<30} ← {}{many}", t.id, t.title, t.sub, t.accepts.join(", "));
            }
            Rules::builtin().validate(Catalog::builtin())?;
        }
        Cmd::Mcp => mcp::serve()?,
    }
    Ok(())
}
