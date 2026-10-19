//! Video and audio conversion by driving ffmpeg as a child process.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use super::{Ctx, Engine, Request};
use crate::analyze::Analysis;
use crate::format::Format;
use crate::tools;

pub struct MediaEngine;

impl Engine for MediaEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let ff = tools::ffmpeg().ok_or_else(|| anyhow!("ffmpeg wurde nicht gefunden"))?;
        let plan = build(req)?;
        run(ff, &plan, ctx)
    }
}

struct Plan {
    args: Vec<String>,
    /// Expected output duration in seconds, 0 when unknown.
    out_dur: f64,
}

#[derive(Clone, PartialEq)]
enum Sel {
    None,
    Copy,
    Enc(String),
}

fn s(x: impl Into<String>) -> String {
    x.into()
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|i| i.to_string()));
}

fn num(v: f64) -> String {
    let t = format!("{v:.3}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------------------------------------------------------------- containers

/// Logical container name from the input format, for `container: keep`.
fn keep_container(f: Format) -> Result<&'static str> {
    use Format::*;
    Ok(match f {
        Mp4 | M4v => "mp4",
        Mov => "mov",
        Mkv => "mkv",
        Webm => "webm",
        Avi => "avi",
        Mp3 => "mp3",
        M4a => "ipod",
        Aac => "adts",
        Wav => "wav",
        Flac => "flac",
        Aiff => "aiff",
        Ogg => "ogg",
        Opus => "opus",
        Wma => "asf",
        Flv => "flv",
        Mts => "mpegts",
        Threegp => "3gp",
        _ => bail!("Das Containerformat {} wird für diese Aktion nicht unterstützt", f.id()),
    })
}

fn normalize_container(c: &str) -> String {
    match c {
        "m4a" => s("ipod"),
        other => s(other),
    }
}

fn muxer(c: &str) -> &str {
    if c == "mkv" {
        "matroska"
    } else {
        c
    }
}

fn natural_audio(container: &str) -> &'static str {
    match container {
        "mp3" | "avi" => "mp3",
        "wav" => "pcm",
        "flac" => "flac",
        "ogg" => "vorbis",
        "opus" | "webm" => "opus",
        "aiff" => "pcm_be",
        "asf" => "wma",
        _ => "aac",
    }
}

fn natural_video(container: &str) -> &'static str {
    match container {
        "webm" => "vp9",
        "avi" => "mpeg4",
        _ => "h264",
    }
}

fn is_pcm(c: &str) -> bool {
    c.starts_with("pcm_")
}

fn audio_copy_ok(container: &str, codec: &str) -> bool {
    match container {
        "mkv" => true,
        "mp4" => matches!(codec, "aac" | "mp3" | "alac" | "opus" | "ac3" | "eac3"),
        "mov" => matches!(codec, "aac" | "mp3" | "alac" | "ac3") || is_pcm(codec),
        "ipod" => matches!(codec, "aac" | "alac"),
        "3gp" => codec == "aac",
        "flv" => matches!(codec, "aac" | "mp3"),
        "mpegts" => matches!(codec, "aac" | "mp3" | "ac3"),
        "webm" => matches!(codec, "opus" | "vorbis"),
        "avi" => matches!(codec, "mp3" | "ac3") || is_pcm(codec),
        "mp3" => codec == "mp3",
        "wav" => is_pcm(codec) && !codec.ends_with("be"),
        "aiff" => matches!(codec, "pcm_s16be" | "pcm_s24be" | "pcm_s32be" | "pcm_u8"),
        "flac" => codec == "flac",
        "ogg" => matches!(codec, "vorbis" | "opus" | "flac"),
        "opus" => codec == "opus",
        "adts" => codec == "aac",
        "asf" => matches!(codec, "wmav1" | "wmav2" | "wmapro" | "mp3"),
        _ => false,
    }
}

fn video_copy_ok(container: &str, codec: &str) -> bool {
    match container {
        "mkv" => true,
        "mp4" | "ipod" => matches!(codec, "h264" | "hevc" | "mpeg4"),
        "mov" => matches!(codec, "h264" | "hevc" | "mpeg4" | "prores" | "mjpeg"),
        "webm" => matches!(codec, "vp8" | "vp9" | "av1"),
        "avi" => matches!(codec, "mpeg4" | "h264" | "mjpeg" | "msmpeg4v3"),
        "flv" => matches!(codec, "h264" | "flv1"),
        "mpegts" => matches!(codec, "h264" | "hevc" | "mpeg2video"),
        "3gp" => matches!(codec, "h264" | "mpeg4" | "h263"),
        _ => false,
    }
}

// ---------------------------------------------------------------- filters

struct Vf {
    chain: Vec<String>,
    scaled: bool,
    /// Estimated output size, (0, 0) when the source size is unknown.
    est: (f64, f64),
    fps: f64,
}

fn parse_aspect(a: &str) -> Option<f64> {
    let (x, y) = a.split_once(':')?;
    let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
    (x > 0.0 && y > 0.0).then(|| x / y)
}

fn rotation(req: &Request) -> u32 {
    let r = req.options.rotate.map(|r| r as i64).or_else(|| req.f64("rotate").map(|r| r as i64)).unwrap_or(0);
    r.rem_euclid(360) as u32
}

fn speed_of(req: &Request) -> f64 {
    req.f64("speed").filter(|v| *v > 0.0).unwrap_or(1.0)
}

/// Fit into lw x lh without upscaling; dimensions are kept even.
fn scale_limit_expr(lw: Option<u32>, lh: Option<u32>, flags: &str, even: bool) -> String {
    let w = lw.map(|v| v.to_string()).unwrap_or_else(|| s("100000"));
    let h = lh.map(|v| v.to_string()).unwrap_or_else(|| s("100000"));
    let mut e = format!("scale='min(iw,{w})':'min(ih,{h})':force_original_aspect_ratio=decrease");
    if even {
        e.push_str(":force_divisible_by=2");
    }
    if !flags.is_empty() {
        e.push_str(&format!(":flags={flags}"));
    }
    e
}

/// Rotate, crop, scale, speed and fps filters in processing order. `height_cap` lowers the resolution further.
fn video_filters(req: &Request, height_cap: Option<u32>, even: bool) -> Vf {
    let mut chain = Vec::new();
    let v = req.analysis.media.as_ref().and_then(|m| m.video.as_ref());
    let (mut w, mut h) = v.map(|v| (v.width as f64, v.height as f64)).unwrap_or((0.0, 0.0));

    match rotation(req) {
        90 => chain.push(s("transpose=1")),
        180 => chain.push(s("hflip,vflip")),
        270 => chain.push(s("transpose=2")),
        _ => {}
    }
    if matches!(rotation(req), 90 | 270) {
        std::mem::swap(&mut w, &mut h);
    }
    if req.options.flip_horizontal == Some(true) {
        chain.push(s("hflip"));
    }
    if let Some(c) = req.options.crop {
        chain.push(format!("crop={}:{}:{}:{}", c.width, c.height, c.x, c.y));
        w = c.width as f64;
        h = c.height as f64;
    }
    let aspect = req.options.aspect.clone().or_else(|| req.str("aspect").map(String::from));
    if let Some(r) = aspect.as_deref().and_then(parse_aspect) {
        chain.push(format!("crop='if(gt(iw/ih,{r}),ih*{r},iw)':'if(gt(iw/ih,{r}),ih,iw/{r})'"));
        if w > 0.0 {
            if w / h > r {
                w = h * r;
            } else {
                h = w / r;
            }
        }
    }

    let mut scaled = false;
    let (ow, oh) = (req.options.width, req.options.height);
    if ow.is_some() || oh.is_some() {
        let even_dim = |x: u32| (x & !1).max(2);
        let e = match (ow, oh) {
            (Some(a), Some(b)) => format!("scale={}:{}", even_dim(a), even_dim(b)),
            (Some(a), None) => format!("scale={}:-2", even_dim(a)),
            (None, Some(b)) => format!("scale=-2:{}", even_dim(b)),
            _ => unreachable!(),
        };
        chain.push(e);
        scaled = true;
        match (ow, oh) {
            (Some(a), Some(b)) => (w, h) = (a as f64, b as f64),
            (Some(a), None) if w > 0.0 => (w, h) = (a as f64, h * a as f64 / w),
            (None, Some(b)) if h > 0.0 => (w, h) = (w * b as f64 / h, b as f64),
            _ => {}
        }
    } else {
        let mut lw = req.f64("max_width").map(|x| x as u32);
        let mut lh = req.f64("max_height").map(|x| x as u32);
        if let Some(e) = req.options.max_edge {
            lw = Some(e);
            lh = Some(e);
        }
        if let Some(c) = height_cap {
            lh = Some(lh.map_or(c, |x| x.min(c)));
        }
        if lw.is_some() || lh.is_some() {
            let fits = w > 0.0 && lw.map_or(true, |x| w <= x as f64) && lh.map_or(true, |x| h <= x as f64);
            if !fits {
                chain.push(scale_limit_expr(lw, lh, "lanczos", true));
                scaled = true;
                if w > 0.0 {
                    let f = (lw.map_or(1.0, |x| x as f64 / w)).min(lh.map_or(1.0, |x| x as f64 / h)).min(1.0);
                    w *= f;
                    h *= f;
                }
            }
        }
    }
    if !scaled && even {
        chain.push(s("scale=trunc(iw/2)*2:trunc(ih/2)*2"));
    }

    let speed = speed_of(req);
    if (speed - 1.0).abs() > 1e-6 {
        chain.push(format!("setpts=PTS/{}", num(speed)));
    }
    let src_fps = v.map(|v| v.fps).filter(|f| *f > 0.0).unwrap_or(30.0);
    let fps_opt = req.options.fps.or_else(|| req.f64("fps")).filter(|f| *f > 0.0);
    if let Some(f) = fps_opt {
        chain.push(format!("fps={}", num(f)));
    }
    Vf { chain, scaled, est: (w, h), fps: fps_opt.unwrap_or(src_fps) }
}

fn audio_filters(req: &Request) -> Vec<String> {
    let mut f = Vec::new();
    let mut speed = speed_of(req);
    if (speed - 1.0).abs() > 1e-6 {
        while speed > 2.0 {
            f.push(s("atempo=2.0"));
            speed /= 2.0;
        }
        while speed < 0.5 {
            f.push(s("atempo=0.5"));
            speed /= 0.5;
        }
        f.push(format!("atempo={}", num(speed)));
    }
    if let Some(db) = req.f64("volume_db").filter(|d| d.abs() > 1e-6) {
        f.push(format!("volume={}dB", num(db)));
    }
    if req.str("filter") == Some("loudnorm") {
        f.push(s("loudnorm=I=-16:TP=-1.5:LRA=11"));
    }
    f
}

// ---------------------------------------------------------------- encoders

fn require_enc(prefs: &[&str]) -> Result<String> {
    tools::pick_encoder(prefs).ok_or_else(|| anyhow!("Dieses ffmpeg enthält keinen passenden Encoder ({})", prefs.join(", ")))
}

fn video_prefs(codec: &str, software_first: bool) -> Result<Vec<&'static str>> {
    let mut v: Vec<&'static str> = match codec {
        "h264" => vec!["h264_videotoolbox", "h264_mf", "h264_nvenc", "h264_qsv", "h264_amf", "libopenh264", "libx264"],
        "hevc" => vec!["hevc_videotoolbox", "hevc_mf", "hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"],
        "vp9" => vec!["libvpx-vp9"],
        "av1" => vec!["libsvtav1", "libaom-av1"],
        "prores" => vec!["prores_videotoolbox", "prores_ks"],
        "mpeg4" => vec!["mpeg4"],
        other => bail!("Unbekannter Videocodec: {other}"),
    };
    if software_first {
        // Hardware encoders overshoot a size budget; the software ones follow maxrate closely.
        v.sort_by_key(|e| !matches!(*e, "libx264" | "libx265"));
    }
    Ok(v)
}

/// Quality 1-100 maps to a CRF-like value: 100 -> 16, 1 -> 40 (x264 scale). Other encoders shift it:
/// x265 +4, vp9/av1 +8, nvenc +1. VideoToolbox takes the quality directly as `-q:v`, mpeg4 uses
/// qscale 2..14. Encoders without a quality mode get a bitrate derived from resolution and fps.
fn video_enc_args(enc: &str, codec: &str, q: u8, rate_k: Option<u32>, est: (f64, f64), fps: f64) -> Vec<String> {
    let crf = 40.0 - (q as f64 - 1.0) * 24.0 / 99.0;
    let mut a = vec![s("-c:v"), s(enc)];
    let bitrate_mode = |a: &mut Vec<String>, k: u32| {
        push(a, &["-b:v", &format!("{k}k")]);
    };
    let fallback_k = || {
        let (w, h) = if est.0 > 0.0 { est } else { (1280.0, 720.0) };
        let bpp = 0.04 + q as f64 / 100.0 * 0.10;
        let factor = if matches!(codec, "hevc" | "vp9" | "av1") { 0.7 } else { 1.0 };
        ((w * h * fps.clamp(1.0, 60.0) * bpp * factor / 1000.0) as u32).clamp(300, 60000)
    };
    if let Some(k) = rate_k {
        // A size budget is a ceiling, not a goal: small sources must not grow. x264/x265 get quality
        // mode capped by maxrate; the others get the lower of budget and quality-based rate.
        match enc {
            "libx264" | "libx265" => {
                let c = if enc == "libx265" { crf + 4.0 } else { crf };
                push(&mut a, &["-crf", &format!("{:.0}", c), "-maxrate", &format!("{k}k"), "-bufsize", &format!("{}k", k * 2)]);
            }
            _ => {
                let k = k.min(fallback_k());
                bitrate_mode(&mut a, k);
                push(&mut a, &["-maxrate", &format!("{}k", k * 3 / 2), "-bufsize", &format!("{}k", k * 2)]);
            }
        }
        match enc {
            "libx264" => push(&mut a, &["-preset", "veryfast"]),
            "libx265" => push(&mut a, &["-preset", "fast"]),
            "libvpx-vp9" => push(&mut a, &["-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]),
            "libsvtav1" => push(&mut a, &["-preset", "8"]),
            _ => {}
        }
    } else {
        match enc {
            "libx264" => push(&mut a, &["-crf", &format!("{:.0}", crf), "-preset", "veryfast"]),
            "libx265" => push(&mut a, &["-crf", &format!("{:.0}", crf + 4.0), "-preset", "fast"]),
            "h264_videotoolbox" | "hevc_videotoolbox" => push(&mut a, &["-q:v", &q.to_string()]),
            "h264_nvenc" | "hevc_nvenc" => push(&mut a, &["-rc", "vbr", "-cq", &format!("{:.0}", crf + 1.0), "-b:v", "0"]),
            "h264_qsv" | "hevc_qsv" => push(&mut a, &["-global_quality", &format!("{:.0}", crf)]),
            "libvpx-vp9" => push(&mut a, &["-crf", &format!("{:.0}", crf + 8.0), "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]),
            "libsvtav1" => push(&mut a, &["-crf", &format!("{:.0}", (crf + 8.0).min(63.0)), "-preset", "8"]),
            "libaom-av1" => push(&mut a, &["-crf", &format!("{:.0}", (crf + 8.0).min(63.0)), "-b:v", "0", "-cpu-used", "6", "-row-mt", "1"]),
            "mpeg4" => push(&mut a, &["-q:v", &format!("{:.0}", 2.0 + (100.0 - q as f64) * 0.12)]),
            "prores_ks" => push(&mut a, &["-profile:v", "3", "-vendor", "apl0"]),
            "prores_videotoolbox" => {}
            _ => bitrate_mode(&mut a, fallback_k()),
        }
    }
    match codec {
        "prores" => {
            if enc == "prores_ks" {
                push(&mut a, &["-pix_fmt", "yuv422p10le"]);
            }
        }
        _ => push(&mut a, &["-pix_fmt", "yuv420p"]),
    }
    a
}

fn audio_enc_args(codec: &str, bitrate_k: u32) -> Result<Vec<String>> {
    let br = format!("{bitrate_k}k");
    let mut a: Vec<String> = Vec::new();
    match codec {
        "aac" => {
            let e = require_enc(&["aac_at", "aac"])?;
            push(&mut a, &["-c:a", &e, "-b:a", &br]);
        }
        "mp3" => push(&mut a, &["-c:a", &require_enc(&["libmp3lame"])?, "-b:a", &br]),
        "opus" => {
            let e = require_enc(&["libopus", "opus"])?;
            push(&mut a, &["-c:a", &e, "-b:a", &br]);
            if e == "opus" {
                push(&mut a, &["-strict", "-2"]);
            }
        }
        "vorbis" => {
            let e = require_enc(&["libvorbis", "vorbis"])?;
            push(&mut a, &["-c:a", &e, "-b:a", &br]);
            if e == "vorbis" {
                // The native encoder is stereo only; an explicit channel option later on wins.
                push(&mut a, &["-strict", "-2", "-ac", "2"]);
            }
        }
        "flac" => push(&mut a, &["-c:a", "flac"]),
        "alac" => push(&mut a, &["-c:a", &require_enc(&["alac"])?]),
        "pcm" => push(&mut a, &["-c:a", "pcm_s16le"]),
        "pcm_be" => push(&mut a, &["-c:a", "pcm_s16be"]),
        "wma" => push(&mut a, &["-c:a", "wmav2", "-b:a", &br]),
        other => bail!("Unbekannter Audiocodec: {other}"),
    }
    Ok(a)
}

// ---------------------------------------------------------------- planning

fn base_args() -> Vec<String> {
    ["-hide_banner", "-loglevel", "error", "-y", "-nostdin", "-progress", "pipe:1", "-nostats"].iter().map(|x| x.to_string()).collect()
}

struct Timing {
    start: f64,
    /// Explicit input duration limit.
    clip: Option<f64>,
    /// Duration of the part that gets processed, 0 when unknown.
    span: f64,
}

fn timing(req: &Request) -> Result<Timing> {
    let in_dur = req.analysis.media.as_ref().map(|m| m.duration_s).unwrap_or(0.0);
    let start = req.options.start_s.unwrap_or(0.0).max(0.0);
    let mut end = req.options.end_s.filter(|e| *e > 0.0);
    if in_dur > 0.0 {
        if start >= in_dur {
            bail!("Der Startpunkt liegt hinter dem Ende der Datei");
        }
        end = end.map(|e| e.min(in_dur));
    }
    if let Some(e) = end {
        if e <= start {
            bail!("Der Endpunkt muss hinter dem Startpunkt liegen");
        }
    }
    let remaining = if in_dur > 0.0 { in_dur - start } else { 0.0 };
    let mut clip = end.map(|e| e - start);
    if clip.is_none() {
        if let Some(c) = req.f64("clip_s").filter(|c| *c > 0.0) {
            if remaining == 0.0 || c < remaining - 0.05 {
                clip = Some(c);
            }
        }
    }
    let span = clip.unwrap_or(remaining);
    Ok(Timing { start, clip, span })
}

fn input_args(args: &mut Vec<String>, req: &Request, t: &Timing) {
    if t.start > 0.0 {
        push(args, &["-ss", &num(t.start)]);
    }
    if let Some(c) = t.clip {
        push(args, &["-t", &num(c)]);
    }
    args.push(s("-i"));
    args.push(req.input.to_string_lossy().into_owned());
}

fn build(req: &Request) -> Result<Plan> {
    let t = timing(req)?;
    let speed = speed_of(req);
    let out_dur = if t.span > 0.0 { t.span / speed } else { 0.0 };
    let out = req.output.to_string_lossy().into_owned();
    let mut args = base_args();

    match req.str("op") {
        Some("poster") => {
            if !req.analysis.has_video() {
                bail!("Die Datei enthält kein Video");
            }
            let vf = video_filters(req, None, false);
            let seek = t.start + if t.span > 0.0 { t.span * 0.1 } else { 0.0 };
            if seek > 0.0 {
                push(&mut args, &["-ss", &num(seek)]);
            }
            args.push(s("-i"));
            args.push(req.input.to_string_lossy().into_owned());
            let mut chain = vf.chain;
            chain.push(s("thumbnail"));
            push(&mut args, &["-an", "-vf", &chain.join(","), "-frames:v", "1", "-q:v", "2", "-pix_fmt", "yuvj420p", "-f", "image2", "-update", "1", &out]);
            return Ok(Plan { args, out_dur: 0.0 });
        }
        Some("frames") => {
            if !req.analysis.has_video() {
                bail!("Die Datei enthält kein Video");
            }
            std::fs::create_dir_all(req.output)?;
            let every = req.f64("every_s").filter(|e| *e > 0.0).unwrap_or(1.0);
            let vf = video_filters(req, None, false);
            input_args(&mut args, req, &t);
            let mut chain = vf.chain;
            chain.push(format!("fps=1/{}", num(every)));
            let pattern = req.output.join("Bild_%04d.jpg").to_string_lossy().into_owned();
            push(&mut args, &["-an", "-vf", &chain.join(","), "-q:v", "2", "-pix_fmt", "yuvj420p", "-f", "image2", &pattern]);
            return Ok(Plan { args, out_dur });
        }
        Some("gif") => {
            if !req.analysis.has_video() {
                bail!("Die Datei enthält kein Video");
            }
            let vf = video_filters(req, None, false);
            let lw = req.options.width.or(req.options.max_edge).or_else(|| req.f64("max_width").map(|x| x as u32));
            let lh = req.options.height.or(req.options.max_edge);
            let gif_fps = req.options.fps.or_else(|| req.f64("fps")).unwrap_or(12.0);
            input_args(&mut args, req, &t);
            let mut chain = vf.chain;
            if !chain.iter().any(|c| c.starts_with("fps=")) {
                chain.push(format!("fps={}", num(gif_fps)));
            }
            if lw.is_some() || lh.is_some() {
                chain.push(scale_limit_expr(lw, lh, "lanczos", false));
            }
            let graph = format!("[0:v:0]{},split[a][b];[a]palettegen[p];[b][p]paletteuse[g]", chain.join(","));
            push(&mut args, &["-an", "-filter_complex", &graph, "-map", "[g]", "-loop", "0", "-f", "gif", &out]);
            return Ok(Plan { args, out_dur });
        }
        _ => {}
    }

    let a = req.analysis;
    let container = match req.str("container").unwrap_or("keep") {
        "keep" => s(keep_container(a.format)?),
        c => normalize_container(c),
    };
    let video_param = req.str("video").unwrap_or("keep");
    let audio_param = req.str("audio").unwrap_or("keep");
    let src_v = a.video_codec().unwrap_or("").to_string();
    let src_a = a.audio_codec().unwrap_or("").to_string();

    let mut vf = video_filters(req, None, false);
    let af = audio_filters(req);
    let q_set = req.options.quality.is_some() || req.f64("quality").is_some();
    let target_mb = req.options.target_mb.or_else(|| req.f64("target_mb")).filter(|m| *m > 0.0);
    let abr_opt = req.options.audio_bitrate_k.or_else(|| req.f64("audio_bitrate_k").map(|x| x as u32));
    let channels = req.options.channels.or_else(|| req.f64("channels").map(|x| x as u32));
    let sample_rate = req.options.sample_rate;

    let mut vsel = match video_param {
        _ if !a.has_video() => Sel::None,
        "none" => Sel::None,
        "copy" => Sel::Copy,
        "keep" => Sel::Enc(s("keep")),
        other => Sel::Enc(s(other)),
    };
    let mut asel = match audio_param {
        _ if !a.has_audio() => Sel::None,
        "none" => Sel::None,
        "copy" => Sel::Copy,
        "keep" => Sel::Enc(s("keep")),
        other => Sel::Enc(s(other)),
    };
    if vsel == Sel::None && asel == Sel::None {
        bail!("Die Datei enthält nichts, was in dieses Format umgewandelt werden kann");
    }

    // Remux: nothing changes except the container, so copy the streams.
    let untouched = vf.chain.is_empty()
        && af.is_empty()
        && t.start == 0.0
        && t.clip.is_none()
        && !q_set
        && target_mb.is_none()
        && req.options.audio_bitrate_k.is_none()
        && channels.is_none()
        && sample_rate.is_none();
    if req.bool("remux_if_possible") && untouched {
        let v_ok = vsel == Sel::None || video_copy_ok(&container, &src_v);
        let a_ok = asel == Sel::None || audio_copy_ok(&container, &src_a);
        if v_ok && a_ok {
            if vsel != Sel::None {
                vsel = Sel::Copy;
            }
            if asel != Sel::None {
                asel = Sel::Copy;
            }
        }
    }

    // keep: copy unless something forces a re-encode, then use the container's usual codec.
    if vsel == Sel::Enc(s("keep")) {
        vsel = if vf.chain.is_empty() { Sel::Copy } else { Sel::Enc(s(natural_video(&container))) };
    }
    // A copied stream can only start on a keyframe, so a trim start needs a re-encode to be exact.
    if vsel == Sel::Copy && (!vf.chain.is_empty() || t.start > 0.0 || !video_copy_ok(&container, &src_v)) {
        vsel = Sel::Enc(s(natural_video(&container)));
    }
    if asel == Sel::Enc(s("keep")) {
        asel = Sel::Enc(s(natural_audio(&container)));
    }
    let audio_changes = !af.is_empty() || channels.is_some() || sample_rate.is_some() || req.options.audio_bitrate_k.is_some();
    if asel == Sel::Copy && (audio_changes || !audio_copy_ok(&container, &src_a)) {
        asel = Sel::Enc(s(natural_audio(&container)));
    }

    let abitrate = abr_opt.unwrap_or(if container == "opus" { 96 } else { 192 }).clamp(32, 512);
    let mut vargs: Vec<String> = Vec::new();
    let mut final_vcodec = if vsel == Sel::Copy { src_v.clone() } else { s("") };

    if let Sel::Enc(codec) = &vsel {
        // Without filters the chain is empty, but yuv420p encoders still need even dimensions.
        if vf.chain.is_empty() || !vf.scaled {
            vf = video_filters(req, None, codec != "prores");
        }
        let mut rate_k = None;
        if let Some(mb) = target_mb {
            if t.span <= 0.0 {
                bail!("Die Dauer des Videos ist unbekannt, die Zielgröße lässt sich nicht berechnen");
            }
            let dur = t.span / speed;
            let total_k = mb * 1_000_000.0 * 8.0 * 0.95 / 1000.0 / dur;
            let audio_k = if asel == Sel::None { 0.0 } else { abr_opt.unwrap_or(128) as f64 };
            let mut vk = total_k - audio_k;
            if vk < 150.0 {
                vf = video_filters(req, Some(360), codec != "prores");
                vk = vk.max(80.0);
            }
            rate_k = Some(vk as u32);
        }
        let prefs = video_prefs(codec, target_mb.is_some())?;
        let enc = require_enc(&prefs)?;
        vargs = video_enc_args(&enc, codec, req.quality(70), rate_k, vf.est, vf.fps);
        final_vcodec = codec.clone();
    }
    if final_vcodec == "hevc" && matches!(container.as_str(), "mp4" | "mov" | "ipod") {
        push(&mut vargs, &["-tag:v", "hvc1"]);
    }

    let mut aargs: Vec<String> = Vec::new();
    match &asel {
        Sel::Enc(codec) => aargs = audio_enc_args(codec, abitrate)?,
        Sel::Copy => push(&mut aargs, &["-c:a", "copy"]),
        Sel::None => {}
    }
    if asel != Sel::Copy && asel != Sel::None {
        if let Some(c) = channels {
            push(&mut aargs, &["-ac", &c.to_string()]);
        }
        let sr = sample_rate.or_else(|| {
            // loudnorm resamples to 192 kHz internally
            (req.str("filter") == Some("loudnorm")).then(|| a.media.as_ref().and_then(|m| m.audio.as_ref()).map(|x| x.sample_rate).filter(|r| *r > 0).unwrap_or(44100))
        });
        if let Some(sr) = sr {
            push(&mut aargs, &["-ar", &sr.to_string()]);
        }
        if !af.is_empty() {
            push(&mut aargs, &["-af", &af.join(",")]);
        }
    }

    input_args(&mut args, req, &t);
    match &vsel {
        Sel::None => args.push(s("-vn")),
        _ => push(&mut args, &["-map", "0:v:0?"]),
    }
    match &asel {
        Sel::None => args.push(s("-an")),
        _ => push(&mut args, &["-map", "0:a:0?"]),
    }
    match &vsel {
        Sel::Copy => push(&mut args, &["-c:v", "copy"]),
        Sel::Enc(_) => {
            if !vf.chain.is_empty() {
                push(&mut args, &["-vf", &vf.chain.join(",")]);
            }
            args.extend(vargs);
        }
        Sel::None => {}
    }
    args.extend(aargs);
    if matches!(container.as_str(), "mp4" | "mov" | "ipod") {
        push(&mut args, &["-movflags", "+faststart"]);
    }
    push(&mut args, &["-max_muxing_queue_size", "1024", "-f", muxer(&container), &out]);
    Ok(Plan { args, out_dur })
}

// ---------------------------------------------------------------- running

fn run(ff: &Path, plan: &Plan, ctx: &Ctx) -> Result<()> {
    let mut cmd = Command::new(ff);
    cmd.args(&plan.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| anyhow!("ffmpeg konnte nicht gestartet werden: {e}"))?;

    let (tx, rx) = mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let stderr = child.stderr.take().expect("stderr is piped");
    let sink = stderr_lines.clone();
    let err_reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            let mut v = sink.lock().unwrap();
            v.push(line);
            if v.len() > 200 {
                v.remove(0);
            }
        }
    });

    let mut cancelled = false;
    loop {
        if (ctx.cancelled)() {
            cancelled = true;
            let _ = child.kill();
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if plan.out_dur > 0.0 {
                    if let Some(us) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
                        if let Ok(us) = us.trim().parse::<f64>() {
                            (ctx.progress)(((us / 1e6 / plan.out_dur) as f32).clamp(0.0, 0.99));
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait();
    let _ = reader.join();
    let _ = err_reader.join();
    if cancelled {
        bail!("abgebrochen");
    }
    let status = status?;
    if !status.success() {
        let lines = stderr_lines.lock().unwrap();
        let tail: Vec<&str> = lines.iter().rev().take(15).rev().map(|l| l.as_str()).collect();
        bail!("ffmpeg ist fehlgeschlagen:\n{}", tail.join("\n"));
    }
    Ok(())
}

// ---------------------------------------------------------------- concatenation

/// (ffmpeg muxer, codec key for `audio_enc_args`, file extension) of a concatenated audio file.
/// "keep" follows the first file and falls back to MP3 for formats Orbo does not encode.
pub(crate) fn concat_format(first: Format, wanted: &str) -> (&'static str, &'static str, &'static str) {
    let key = if wanted == "keep" {
        match first {
            Format::Mp3 => "mp3",
            Format::M4a => "m4a",
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Ogg => "ogg",
            Format::Opus => "opus",
            Format::Aiff => "aiff",
            Format::Aac => "aac",
            _ => "mp3",
        }
    } else {
        wanted
    };
    match key {
        "m4a" => ("ipod", "aac", "m4a"),
        "wav" => ("wav", "pcm", "wav"),
        "flac" => ("flac", "flac", "flac"),
        "ogg" => ("ogg", "vorbis", "ogg"),
        "opus" => ("opus", "opus", "opus"),
        "aiff" => ("aiff", "pcm_be", "aiff"),
        "aac" => ("adts", "aac", "aac"),
        _ => ("mp3", "mp3", "mp3"),
    }
}

/// Joins audio files end to end into one re-encoded file. All inputs are brought to the sample rate and
/// channel layout of the first one so that ffmpeg's concat filter accepts them.
pub(crate) fn concat(inputs: &[PathBuf], analyses: &[Analysis], wanted: &str, bitrate_k: u32, output: &Path, ctx: &Ctx) -> Result<()> {
    let ff = tools::ffmpeg().ok_or_else(|| anyhow!("ffmpeg wurde nicht gefunden"))?;
    let (container, codec, _) = concat_format(analyses[0].format, wanted);
    let first = analyses[0].media.as_ref().and_then(|m| m.audio.as_ref());
    let rate = first.map(|a| a.sample_rate).filter(|r| *r > 0).unwrap_or(44100);
    let layout = if first.map(|a| a.channels) == Some(1) { "mono" } else { "stereo" };

    let mut args = base_args();
    let mut graph = String::new();
    for (i, p) in inputs.iter().enumerate() {
        push(&mut args, &["-i"]);
        args.push(p.to_string_lossy().into_owned());
        graph.push_str(&format!("[{i}:a:0]aformat=sample_fmts=fltp:sample_rates={rate}:channel_layouts={layout}[a{i}];"));
    }
    for i in 0..inputs.len() {
        graph.push_str(&format!("[a{i}]"));
    }
    graph.push_str(&format!("concat=n={}:v=0:a=1[out]", inputs.len()));
    push(&mut args, &["-filter_complex", &graph, "-map", "[out]"]);
    args.extend(audio_enc_args(codec, bitrate_k)?);
    push(&mut args, &["-f", muxer(container), &output.to_string_lossy()]);
    let out_dur = analyses.iter().filter_map(|a| a.media.as_ref()).map(|m| m.duration_s).sum();
    run(ff, &Plan { args, out_dur }, ctx)
}
