//! Background job queue: runs conversions on worker threads, reports progress, supports cancel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};

use crate::analyze::{analyze, Analysis};
use crate::catalog::{Catalog, Target};
use crate::engines::{self, Ctx, Request};
use crate::output;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// User choices from the options dialog. Everything is optional; engines fall back to target params.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Options {
    /// Quality 1-100 for lossy encoders.
    pub quality: Option<u8>,
    /// Scale so the longest edge is at most this many pixels.
    pub max_edge: Option<u32>,
    /// Exact output size; if only one is set the other keeps the aspect ratio.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Crop in source pixels, applied before scaling.
    pub crop: Option<CropRect>,
    /// Center-crop to this aspect ratio, e.g. "1:1", "16:9", "9:16".
    pub aspect: Option<String>,
    /// Clockwise rotation in degrees (90, 180, 270).
    pub rotate: Option<u32>,
    pub flip_horizontal: Option<bool>,
    /// Trim for audio and video, in seconds.
    pub start_s: Option<f64>,
    pub end_s: Option<f64>,
    /// Aim for this output size in megabytes (video, images).
    pub target_mb: Option<f64>,
    pub audio_bitrate_k: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub fps: Option<f64>,
    /// Screenshot beautifier: background as "#rrggbb" or a gradient name from the image engine ("sunset", "ocean" ...).
    pub background: Option<String>,
    pub padding_px: Option<u32>,
    pub corner_px: Option<u32>,
    pub shadow_px: Option<u32>,
    /// Name of a target preset ("Web", "Maximal" ...).
    pub preset: Option<String>,
    /// Extra text appended to the output file name, e.g. " (JPG)".
    pub name_suffix: Option<String>,
    /// Write the result here instead of next to the original.
    pub out_dir: Option<PathBuf>,
}

pub type JobId = u64;

#[derive(Debug, Clone)]
pub struct JobSpec {
    pub input: PathBuf,
    pub target: String,
    pub options: Options,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JobEvent {
    Started { id: JobId, input: PathBuf, target: String },
    Progress { id: JobId, fraction: f32 },
    Finished { id: JobId, output: PathBuf, bytes: u64, millis: u128 },
    Failed { id: JobId, error: String },
    Cancelled { id: JobId },
}

struct Queued {
    id: JobId,
    spec: JobSpec,
    cancel: Arc<AtomicBool>,
}

pub struct JobQueue {
    tx: Option<Sender<Queued>>,
    events: Receiver<JobEvent>,
    cancels: Arc<Mutex<HashMap<JobId, Arc<AtomicBool>>>>,
    next: AtomicU64,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl JobQueue {
    /// `workers` = parallel jobs. Engines like ffmpeg are multi-threaded themselves, so 2 is a good default.
    pub fn new(workers: usize) -> JobQueue {
        let (tx, rx) = unbounded::<Queued>();
        let (etx, events) = unbounded::<JobEvent>();
        let cancels: Arc<Mutex<HashMap<JobId, Arc<AtomicBool>>>> = Arc::default();
        let handles = (0..workers.max(1))
            .map(|_| {
                let rx = rx.clone();
                let etx = etx.clone();
                let cancels = cancels.clone();
                std::thread::spawn(move || {
                    for job in rx {
                        let id = job.id;
                        run_one(job, &etx);
                        cancels.lock().unwrap().remove(&id);
                    }
                })
            })
            .collect();
        JobQueue { tx: Some(tx), events, cancels, next: AtomicU64::new(1), workers: handles }
    }

    pub fn submit(&self, spec: JobSpec) -> JobId {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(id, cancel.clone());
        if let Some(tx) = &self.tx {
            let _ = tx.send(Queued { id, spec, cancel });
        }
        id
    }

    pub fn cancel(&self, id: JobId) {
        if let Some(c) = self.cancels.lock().unwrap().get(&id) {
            c.store(true, Ordering::Relaxed);
        }
    }

    pub fn events(&self) -> &Receiver<JobEvent> {
        &self.events
    }

    /// Stops accepting jobs and waits until running ones are done.
    pub fn shutdown(mut self) {
        self.tx.take();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn run_one(job: Queued, events: &Sender<JobEvent>) {
    let Queued { id, spec, cancel } = job;
    let _ = events.send(JobEvent::Started { id, input: spec.input.clone(), target: spec.target.clone() });
    let start = Instant::now();
    let result = convert_blocking(&spec, id, &cancel, &|f| {
        let _ = events.send(JobEvent::Progress { id, fraction: f.clamp(0.0, 1.0) });
    });
    let ev = match result {
        Ok(output) => {
            let bytes = std::fs::metadata(&output).map(|m| if m.is_dir() { dir_bytes(&output) } else { m.len() }).unwrap_or(0);
            JobEvent::Finished { id, output, bytes, millis: start.elapsed().as_millis() }
        }
        Err(_) if cancel.load(Ordering::Relaxed) => JobEvent::Cancelled { id },
        Err(e) => JobEvent::Failed { id, error: format!("{e:#}") },
    };
    let _ = events.send(ev);
}

fn dir_bytes(p: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(p).into_iter().filter_map(|e| e.ok()).filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()
}

/// Merges target params with the chosen preset's params.
pub fn effective_params(target: &Target, options: &Options) -> serde_json::Value {
    let mut params = target.params.clone();
    if !params.is_object() {
        params = serde_json::json!({});
    }
    if let Some(name) = &options.preset {
        if let Some(p) = target.presets.iter().find(|p| &p.label == name) {
            if let (Some(dst), Some(src)) = (params.as_object_mut(), p.params.as_object()) {
                for (k, v) in src {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
    }
    params
}

/// Runs one conversion on the current thread. Used by the queue and directly by the CLI and tests.
pub fn convert_blocking(spec: &JobSpec, job_id: JobId, cancel: &AtomicBool, progress: &(dyn Fn(f32) + Sync)) -> Result<PathBuf> {
    let catalog = Catalog::current();
    let target = catalog.get(&spec.target).ok_or_else(|| anyhow!("Unbekanntes Ziel: {}", spec.target))?;
    let analysis: Analysis = analyze(&spec.input)?;
    if !target.accepts(&analysis) {
        anyhow::bail!("{} kann nicht nach {} umgewandelt werden", analysis.format.id(), target.id);
    }
    let engine = engines::get(&target.engine).ok_or_else(|| anyhow!("Engine fehlt: {}", target.engine))?;

    let dir = spec.options.out_dir.clone().unwrap_or_else(|| spec.input.parent().map(|p| p.to_path_buf()).unwrap_or_default());
    let stem = format!("{}{}{}", output::stem_of(&spec.input), target.suffix, spec.options.name_suffix.as_deref().unwrap_or(""));
    let wanted = output::next_free(&dir, &stem, &target.ext_for(&analysis));
    let temp = output::temp_for(&wanted, job_id);
    let params = effective_params(target, &spec.options);

    let req = Request { input: &spec.input, output: &temp, analysis: &analysis, target, params: &params, options: &spec.options };
    let cancelled = || cancel.load(Ordering::Relaxed);
    let final_stem = std::sync::Mutex::new(None);
    let ctx = Ctx { progress, cancelled: &cancelled, final_stem: &final_stem };
    progress(0.0);
    let res = engine.convert(&req, &ctx);
    if res.is_err() || cancelled() {
        if temp.is_dir() {
            let _ = std::fs::remove_dir_all(&temp);
        } else {
            let _ = std::fs::remove_file(&temp);
        }
        res?;
        anyhow::bail!("abgebrochen");
    }
    if !temp.exists() {
        anyhow::bail!("Engine {} hat keine Ausgabe geschrieben", target.engine);
    }
    // "Hier entpacken": move the extracted entries next to the archive instead of into a new folder.
    if temp.is_dir() && params.get("into").and_then(|v| v.as_str()) == Some("here") {
        let mut first = None;
        for entry in std::fs::read_dir(&temp)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = std::path::Path::new(&name);
            let (stem, ext) = match (path.file_stem(), path.extension()) {
                (Some(s), Some(e)) if entry.path().is_file() => (s.to_string_lossy().into_owned(), e.to_string_lossy().into_owned()),
                _ => (name.clone(), String::new()),
            };
            let dest = output::next_free(&dir, &stem, &ext);
            std::fs::rename(entry.path(), &dest)?;
            first.get_or_insert(dest);
        }
        let _ = std::fs::remove_dir(&temp);
        progress(1.0);
        return Ok(first.unwrap_or(dir));
    }
    let wanted = match final_stem.into_inner().ok().flatten() {
        Some(stem) => output::next_free(&dir, &stem, &target.ext_for(&analysis)),
        None => wanted,
    };
    let final_path = output::commit(&temp, &wanted)?;
    progress(1.0);
    Ok(final_path)
}
