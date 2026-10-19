use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::analyze::Analysis;
use crate::format::{Format, Kind};

/// One thing the ring can offer: a format change, an action (rotate, mute) or an intent (for e-mail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub title: String,
    pub sub: String,
    /// Format id of the result, or "folder" for actions that produce a directory.
    pub output: String,
    pub engine: String,
    /// Kinds ("video") or format ids ("gif") this target can take as input.
    pub accepts: Vec<String>,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    #[serde(default)]
    pub intent: bool,
    /// Result comes from a local language model and is not exactly reproducible.
    #[serde(default)]
    pub ai: bool,
    /// Offered even when the input already has the output format (resize, rotate, compress ...).
    #[serde(default)]
    pub same_format: bool,
    /// Overrides the output format's usual extension, e.g. "m4r" for ringtones.
    #[serde(default)]
    pub ext: Option<String>,
    /// Appended to the file stem, e.g. " (Mail)".
    #[serde(default)]
    pub suffix: String,
    /// Grouping in the "Mehr ..." palette.
    pub group: String,
    /// Engine-specific settings.
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub presets: Vec<Preset>,
    /// Controls the options dialog shows for this target (UI hint).
    #[serde(default)]
    pub controls: Vec<Control>,
    /// Open the options dialog right after the drop instead of using defaults.
    #[serde(default)]
    pub dialog: bool,
    /// A user-defined chain ("Ablauf"); `steps` lists what it runs.
    #[serde(default)]
    pub chain: bool,
    /// Chains only: offered in free ring slots.
    #[serde(default)]
    pub in_ring: bool,
    #[serde(default)]
    pub steps: Vec<crate::chains::ChainStep>,
}

/// Option controls a target supports. The UI builds its dialog from these; engines read the matching `Options` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Quality,
    Resize,
    Crop,
    Aspect,
    Rotate,
    Trim,
    TargetSize,
    AudioBitrate,
    SampleRate,
    Channels,
    Fps,
    /// Screenshot beautifier: background, padding, corner radius, shadow.
    Background,
    Padding,
    Corners,
    Shadow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    VideoStream,
    AudioStream,
    Animated,
    Ffmpeg,
    Macos,
    /// LibreDWG's `dwg2dxf` is installed (optional, never bundled).
    Dwg2dxf,
    Soffice,
    /// Local AI is configured and the target's `action` is switched on (no network check here).
    Ollama,
}

/// A named variant shown when the user dwells on a ring sector, e.g. "Web" / "Maximal".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub label: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Target {
    pub fn output_format(&self) -> Option<Format> {
        Format::from_id(&self.output)
    }

    pub fn output_ext(&self) -> &str {
        match &self.ext {
            Some(e) => e,
            None => self.output_format().map(|f| f.extension()).unwrap_or(""),
        }
    }

    /// Tools like rotate or compress keep the input's format instead of the nominal `output`.
    pub fn keeps_format(&self) -> bool {
        self.params.get("keep_format").and_then(|v| v.as_bool()).unwrap_or(false)
            || self.param_str("container") == Some("keep")
            || matches!(self.param_str("op"), Some("compress") | Some("strip_metadata"))
    }

    /// Extension of the result for this particular input.
    pub fn ext_for(&self, a: &Analysis) -> String {
        if self.chain {
            return crate::chains::final_ext(self, a);
        }
        if self.keeps_format() && !self.produces_folder() {
            return a.format.extension().to_string();
        }
        self.output_ext().to_string()
    }

    pub fn produces_folder(&self) -> bool {
        self.output == "folder"
    }

    pub fn param_str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    pub fn param_f64(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(|v| v.as_f64())
    }

    pub fn accepts(&self, a: &Analysis) -> bool {
        if self.chain {
            return crate::chains::accepts(self, a);
        }
        let fmt = a.format.id();
        let kind = kind_id(a.kind);
        let accepted = self.accepts.iter().any(|x| x == &fmt || x == kind);
        if !accepted {
            return false;
        }
        for r in &self.requires {
            // Without readable content we cannot know the streams; offer the targets and let the job report problems.
            if !a.readable && matches!(r, Requirement::VideoStream | Requirement::AudioStream | Requirement::Animated) {
                continue;
            }
            let ok = match r {
                Requirement::VideoStream => a.has_video(),
                Requirement::AudioStream => a.has_audio(),
                Requirement::Animated => a.image.as_ref().map(|i| i.animated).unwrap_or(false),
                Requirement::Ffmpeg => crate::tools::ffmpeg().is_some(),
                Requirement::Macos => cfg!(target_os = "macos"),
                Requirement::Soffice => crate::engines::office::soffice().is_some(),
                Requirement::Dwg2dxf => crate::engines::cad::dwg2dxf().is_some(),
                Requirement::Ollama => crate::ai::action_enabled(self.param_str("action")),
            };
            if !ok {
                return false;
            }
        }
        if !self.same_format && !self.intent && self.output_format() == Some(a.format) {
            return false;
        }
        self.encoders_available()
    }

    /// Like `accepts`, but for an intermediate file known only by its format (no stream details).
    pub(crate) fn accepts_format(&self, fmt: Format) -> bool {
        let kind = kind_id(fmt.kind());
        if !self.accepts.iter().any(|x| *x == fmt.id() || x == kind) {
            return false;
        }
        for r in &self.requires {
            let ok = match r {
                Requirement::VideoStream => kind == "video",
                Requirement::AudioStream => kind == "video" || kind == "audio",
                Requirement::Animated => matches!(fmt, Format::Gif | Format::Webp | Format::Avif | Format::Png),
                Requirement::Ffmpeg => crate::tools::ffmpeg().is_some(),
                Requirement::Macos => cfg!(target_os = "macos"),
                Requirement::Soffice => crate::engines::office::soffice().is_some(),
                Requirement::Dwg2dxf => crate::engines::cad::dwg2dxf().is_some(),
                Requirement::Ollama => crate::ai::action_enabled(self.param_str("action")),
            };
            if !ok {
                return false;
            }
        }
        if !self.same_format && !self.intent && self.output_format() == Some(fmt) {
            return false;
        }
        self.encoders_available()
    }

    /// Media targets need their codec in the bundled ffmpeg build (e.g. VP9 is missing from the LGPL build).
    fn encoders_available(&self) -> bool {
        if self.engine != "media" || crate::tools::ffmpeg().is_none() {
            return true;
        }
        let video: &[&str] = match self.param_str("video") {
            Some("h264") => &["h264_videotoolbox", "h264_mf", "h264_nvenc", "h264_qsv", "h264_amf", "libopenh264", "libx264"],
            Some("hevc") => &["hevc_videotoolbox", "hevc_mf", "hevc_nvenc", "hevc_qsv", "hevc_amf", "libx265"],
            Some("vp9") => &["libvpx-vp9"],
            Some("av1") => &["libsvtav1", "libaom-av1"],
            Some("prores") => &["prores_videotoolbox", "prores_ks"],
            Some("mpeg4") => &["mpeg4"],
            _ => &[],
        };
        let audio: &[&str] = match self.param_str("audio") {
            Some("aac") => &["aac_at", "aac"],
            Some("mp3") => &["libmp3lame"],
            Some("opus") => &["libopus", "opus"],
            Some("vorbis") => &["libvorbis", "vorbis"],
            Some("flac") => &["flac"],
            Some("alac") => &["alac", "alac_at"],
            Some("wma") => &["wmav2"],
            _ => &[],
        };
        (video.is_empty() || crate::tools::pick_encoder(video).is_some()) && (audio.is_empty() || crate::tools::pick_encoder(audio).is_some())
    }
}

pub fn kind_id(k: Kind) -> &'static str {
    match k {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Audio => "audio",
        Kind::Document => "document",
        Kind::Data => "data",
        Kind::Archive => "archive",
        Kind::Model => "model",
        Kind::Cad => "cad",
        Kind::Font => "typeface",
        Kind::Folder => "folder",
        Kind::Other => "other",
    }
}

const BUILTIN_TARGETS: [&str; 7] = [
    include_str!("../catalog/targets.yaml"),
    include_str!("../catalog/targets-model.yaml"),
    include_str!("../catalog/targets-font.yaml"),
    include_str!("../catalog/targets-ebook.yaml"),
    include_str!("../catalog/targets-office.yaml"),
    include_str!("../catalog/targets-cad.yaml"),
    include_str!("../catalog/targets-ai.yaml"),
];

static CURRENT: RwLock<Option<Arc<Catalog>>> = RwLock::new(None);

pub(crate) fn invalidate_current() {
    *CURRENT.write().unwrap() = None;
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    targets: Vec<Target>,
}

pub struct Catalog {
    pub targets: Vec<Target>,
    index: HashMap<String, usize>,
}

impl Catalog {
    pub fn from_yaml(text: &str) -> Result<Catalog> {
        let file: CatalogFile = serde_yaml::from_str(text).context("targets.yaml")?;
        let mut index = HashMap::new();
        for (i, t) in file.targets.iter().enumerate() {
            if index.insert(t.id.clone(), i).is_some() {
                anyhow::bail!("Ziel-ID doppelt: {}", t.id);
            }
            if t.output != "folder" && Format::from_id(&t.output).is_none() {
                anyhow::bail!("Ziel {}: unbekanntes Ausgabeformat {}", t.id, t.output);
            }
        }
        Ok(Catalog { targets: file.targets, index })
    }

    /// The catalog shipped with the app; one YAML file per area, merged.
    pub fn builtin() -> &'static Catalog {
        static C: OnceLock<Catalog> = OnceLock::new();
        C.get_or_init(|| Catalog::from_yaml_parts(&BUILTIN_TARGETS).expect("eingebauter Katalog ist gültig"))
    }

    /// Built-in targets plus the configured chains. Rebuilt after `chains::configure`.
    pub fn current() -> Arc<Catalog> {
        if let Some(c) = CURRENT.read().unwrap().as_ref() {
            return c.clone();
        }
        let mut targets = Catalog::builtin().targets.clone();
        targets.extend(crate::chains::configured().iter().filter_map(crate::chains::build_target));
        let mut index = HashMap::new();
        for (i, t) in targets.iter().enumerate() {
            index.entry(t.id.clone()).or_insert(i);
        }
        let c = Arc::new(Catalog { targets, index });
        *CURRENT.write().unwrap() = Some(c.clone());
        c
    }

    pub fn from_yaml_parts(parts: &[&str]) -> Result<Catalog> {
        let mut all = Vec::new();
        for (i, text) in parts.iter().enumerate() {
            let file: CatalogFile = serde_yaml::from_str(text).with_context(|| format!("Katalog-Teil {i}"))?;
            all.extend(file.targets);
        }
        let yaml = serde_yaml::to_string(&serde_json::json!({ "targets": all }))?;
        Catalog::from_yaml(&yaml)
    }

    pub fn get(&self, id: &str) -> Option<&Target> {
        self.index.get(id).map(|&i| &self.targets[i])
    }

    pub fn compatible<'a>(&'a self, a: &Analysis) -> Vec<&'a Target> {
        self.targets.iter().filter(|t| t.accepts(a)).collect()
    }
}
