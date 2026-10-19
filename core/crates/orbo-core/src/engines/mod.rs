//! Conversion engines. Each engine handles the targets whose `engine` field names it.

pub mod ai;
pub mod archive;
pub mod cad;
pub mod chain;
pub mod data;
pub mod document;
pub mod ebook;
pub mod font;
pub mod image;
pub mod media;
pub mod model3d;
pub mod office;

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;

use crate::analyze::Analysis;
use crate::catalog::Target;
use crate::jobs::Options;

pub struct Request<'a> {
    pub input: &'a Path,
    /// Temporary output path with the final extension (or a directory path for folder outputs).
    /// The engine writes exactly here; the job runner moves it into place afterwards.
    pub output: &'a Path,
    pub analysis: &'a Analysis,
    pub target: &'a Target,
    /// Target params merged with the chosen preset's params (preset wins).
    pub params: &'a serde_json::Value,
    pub options: &'a Options,
}

impl Request<'_> {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(|v| v.as_f64())
    }

    pub fn bool(&self, key: &str) -> bool {
        self.params.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// User quality 1-100: explicit option, else target param "quality", else the given default.
    pub fn quality(&self, default: u8) -> u8 {
        self.options.quality.or_else(|| self.f64("quality").map(|q| q as u8)).unwrap_or(default).clamp(1, 100)
    }
}

pub struct Ctx<'a> {
    /// Reports progress in 0.0..=1.0.
    pub progress: &'a (dyn Fn(f32) + Sync),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
    /// An engine may set the complete file stem of the result (no extension) when it decides the name itself.
    pub final_stem: &'a Mutex<Option<String>>,
}

pub trait Engine: Send + Sync {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()>;
}

pub fn get(name: &str) -> Option<&'static dyn Engine> {
    match name {
        "image" => Some(&image::ImageEngine),
        "media" => Some(&media::MediaEngine),
        "data" => Some(&data::DataEngine),
        "archive" => Some(&archive::ArchiveEngine),
        "document" => Some(&document::DocumentEngine),
        "model3d" => Some(&model3d::Engine_),
        "font" => Some(&font::Engine_),
        "ebook" => Some(&ebook::Engine_),
        "office" => Some(&office::Engine_),
        "cad" => Some(&cad::Engine_),
        "ai" => Some(&ai::AiEngine),
        "chain" => Some(&chain::ChainEngine),
        _ => None,
    }
}
