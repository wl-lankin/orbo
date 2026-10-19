use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde::Serialize;

use crate::format::{self, Format, Kind};
use crate::tools;

#[derive(Debug, Clone, Serialize)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub animated: bool,
    pub has_gps: bool,
    pub camera: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoStream {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioStream {
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub duration_s: f64,
    pub bitrate: u64,
    pub video: Option<VideoStream>,
    pub audio: Option<AudioStream>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    pub delimiter: Option<char>,
    pub rows_sampled: usize,
    pub columns: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub path: PathBuf,
    pub format: Format,
    pub kind: Kind,
    pub size: u64,
    /// The extension names a different format than the content.
    pub ext_mismatch: bool,
    /// False when the content could not be read (e.g. macOS folder permission missing).
    pub readable: bool,
    pub image: Option<ImageInfo>,
    pub media: Option<MediaInfo>,
    pub table: Option<TableInfo>,
    /// Number of entries for folders.
    pub entries: Option<usize>,
}

impl Analysis {
    pub fn video_codec(&self) -> Option<&str> {
        self.media.as_ref()?.video.as_ref().map(|v| v.codec.as_str())
    }

    pub fn audio_codec(&self) -> Option<&str> {
        self.media.as_ref()?.audio.as_ref().map(|a| a.codec.as_str())
    }

    pub fn has_audio(&self) -> bool {
        self.media.as_ref().map(|m| m.audio.is_some()).unwrap_or(false)
    }

    pub fn has_video(&self) -> bool {
        self.media.as_ref().map(|m| m.video.is_some()).unwrap_or(false)
    }
}

pub fn analyze(path: &Path) -> Result<Analysis> {
    let (mut format, ext_mismatch) = format::detect(path)?;
    let meta = std::fs::metadata(path)?;
    let mut a = Analysis {
        path: path.to_path_buf(),
        format,
        kind: format.kind(),
        size: if meta.is_dir() { dir_size(path) } else { meta.len() },
        ext_mismatch,
        readable: is_readable(path, meta.is_dir(), meta.len()),
        image: None,
        media: None,
        table: None,
        entries: None,
    };
    match a.kind {
        Kind::Image => a.image = image_info(path, format),
        Kind::Video | Kind::Audio => {
            a.media = probe(path);
            // An MP4/MOV/WebM without a video stream is really an audio file (e.g. voice memos).
            if let Some(m) = &a.media {
                if a.kind == Kind::Video && m.video.is_none() && m.audio.is_some() {
                    a.kind = Kind::Audio;
                    if matches!(format, Format::Mp4 | Format::Mov | Format::M4v) {
                        format = Format::M4a;
                        a.format = format;
                    }
                }
            }
        }
        Kind::Data => a.table = table_info(path, format),
        Kind::Folder => a.entries = std::fs::read_dir(path).ok().map(|r| r.count()),
        _ => {}
    }
    // Animated GIFs behave like short videos for suggestions.
    if format == Format::Gif {
        if let Some(i) = &a.image {
            if i.animated {
                a.media = probe(path);
            }
        }
    }
    Ok(a)
}

fn is_readable(path: &Path, is_dir: bool, len: u64) -> bool {
    if is_dir {
        return std::fs::read_dir(path).is_ok();
    }
    let mut b = [0u8; 1];
    match std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut b)) {
        Ok(n) => n == 1 || len == 0,
        Err(_) => false,
    }
}

fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()
}

fn image_info(path: &Path, format: Format) -> Option<ImageInfo> {
    use image::ImageDecoder;
    let (mut width, mut height, mut has_alpha) = (0, 0, false);
    if let Ok(reader) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
        if let Ok(dec) = reader.into_decoder() {
            (width, height) = dec.dimensions();
            has_alpha = dec.color_type().has_alpha();
        }
    }
    let animated = match format {
        Format::Gif => gif_frames(path) > 1,
        Format::Png => png_is_animated(path),
        Format::Webp => webp_is_animated(path),
        _ => false,
    };
    let (has_gps, camera) = exif_info(path);
    if width == 0 && cfg!(target_os = "macos") {
        // Formats the image crate cannot read (HEIC, AVIF, RAW): ask the system.
        if let Some((w, h, alpha)) = sips_info(path) {
            width = w;
            height = h;
            has_alpha = alpha;
        }
    }
    Some(ImageInfo { width, height, has_alpha, animated, has_gps, camera })
}

fn gif_frames(path: &Path) -> usize {
    let Ok(bytes) = std::fs::read(path) else { return 0 };
    // Count image descriptors (0x2C) that follow a graphic control extension; cheap and good enough.
    bytes.windows(3).filter(|w| w[0] == 0x00 && w[1] == 0x2C).count().max(1)
}

fn png_is_animated(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.windows(4).take(4096).any(|w| w == b"acTL")).unwrap_or(false)
}

fn webp_is_animated(path: &Path) -> bool {
    std::fs::read(path).map(|b| b.windows(4).take(256).any(|w| w == b"ANIM")).unwrap_or(false)
}

fn exif_info(path: &Path) -> (bool, Option<String>) {
    let Ok(file) = std::fs::File::open(path) else { return (false, None) };
    let mut reader = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else { return (false, None) };
    let has_gps = exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY).is_some();
    let make = exif.get_field(exif::Tag::Make, exif::In::PRIMARY).map(|f| f.display_value().to_string().trim_matches('"').to_string());
    let model = exif.get_field(exif::Tag::Model, exif::In::PRIMARY).map(|f| f.display_value().to_string().trim_matches('"').to_string());
    let camera = match (make, model) {
        (Some(a), Some(b)) if b.starts_with(&a) => Some(b),
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    };
    (has_gps, camera)
}

fn sips_info(path: &Path) -> Option<(u32, u32, bool)> {
    let out = Command::new("/usr/bin/sips").args(["-g", "pixelWidth", "-g", "pixelHeight", "-g", "hasAlpha"]).arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |key: &str| text.lines().find(|l| l.trim_start().starts_with(key)).and_then(|l| l.split(':').nth(1)).map(|v| v.trim().to_string());
    Some((get("pixelWidth")?.parse().ok()?, get("pixelHeight")?.parse().ok()?, get("hasAlpha").map(|v| v == "yes").unwrap_or(false)))
}

pub fn probe(path: &Path) -> Option<MediaInfo> {
    let ffprobe = tools::ffprobe()?;
    let out = tools::command(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let num = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64());
    let video = streams
        .iter()
        // Cover art in audio files shows up as a video stream with attached_pic.
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0)
        .map(|s| {
            let fps = s["avg_frame_rate"].as_str().and_then(|r| {
                let (a, b) = r.split_once('/')?;
                let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
                (b > 0.0).then(|| a / b)
            });
            VideoStream {
                codec: s["codec_name"].as_str().unwrap_or("").to_string(),
                width: s["width"].as_u64().unwrap_or(0) as u32,
                height: s["height"].as_u64().unwrap_or(0) as u32,
                fps: fps.unwrap_or(0.0),
            }
        });
    let audio = streams.iter().find(|s| s["codec_type"] == "audio").map(|s| AudioStream {
        codec: s["codec_name"].as_str().unwrap_or("").to_string(),
        channels: s["channels"].as_u64().unwrap_or(0) as u32,
        sample_rate: num(&s["sample_rate"]).unwrap_or(0.0) as u32,
    });
    Some(MediaInfo {
        duration_s: num(&v["format"]["duration"]).unwrap_or(0.0),
        bitrate: num(&v["format"]["bit_rate"]).unwrap_or(0.0) as u64,
        video,
        audio,
    })
}

fn table_info(path: &Path, format: Format) -> Option<TableInfo> {
    match format {
        Format::Csv | Format::Tsv => {
            let mut buf = vec![0u8; 64 * 1024];
            let n = std::io::Read::read(&mut std::fs::File::open(path).ok()?, &mut buf).ok()?;
            let text = String::from_utf8_lossy(&buf[..n]);
            let delimiter = format::sniff_delimiter(&text).or(Some(if format == Format::Tsv { '\t' } else { ',' }));
            let rows: Vec<&str> = text.lines().collect();
            let columns = rows.first().map(|l| l.matches(delimiter.unwrap()).count() + 1).unwrap_or(0);
            Some(TableInfo { delimiter, rows_sampled: rows.len(), columns })
        }
        _ => None,
    }
}
