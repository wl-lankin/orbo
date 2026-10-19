use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use clap::{Parser, Subcommand};
use orbo_core::catalog::Catalog;
use orbo_core::jobs::{convert_blocking, CropRect, JobSpec, Options};
use orbo_core::rules::Rules;

#[derive(Parser)]
#[command(name = "orbo", about = "Orbo: Dateien lokal umwandeln")]
struct Cli {
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
    /// Zeigt, was der Ring für diese Datei anbieten würde
    Suggest {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Welche Regeln gegriffen haben
        #[arg(long)]
        explain: bool,
    },
    /// Wandelt eine oder mehrere Dateien um
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
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
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
        Cmd::Suggest { file, json, explain } => {
            let (_, s) = orbo_core::suggest(&file)?;
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
            for t in &Catalog::builtin().targets {
                if t.group != group {
                    group = t.group.clone();
                    println!("\n{group}");
                }
                println!("  {:<16} {:<14} {:<30} ← {}", t.id, t.title, t.sub, t.accepts.join(", "));
            }
            Rules::builtin().validate(Catalog::builtin())?;
        }
    }
    Ok(())
}
