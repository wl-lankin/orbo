//! Runs a user-defined chain: every step goes through its normal engine, intermediates live in a private
//! temp dir that is removed in every case, and the last step writes straight to the requested output.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};

use super::{get, Ctx, Engine, Request};
use crate::analyze::analyze;
use crate::catalog::Catalog;
use crate::jobs::{effective_params, Options};

pub struct ChainEngine;

struct WorkDir(PathBuf);

impl WorkDir {
    fn create(beside: &Path) -> Result<WorkDir> {
        let name = format!(".orbo-chain-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let preferred = beside.parent().map(|p| p.join(&name));
        if let Some(p) = preferred {
            if std::fs::create_dir(&p).is_ok() {
                return Ok(WorkDir(p));
            }
        }
        let p = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&p)?;
        Ok(WorkDir(p))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Engine for ChainEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let steps = &req.target.steps;
        let n = steps.len();
        if n == 0 {
            bail!("Der Ablauf hat keine Schritte");
        }
        let catalog = Catalog::builtin();
        let work = WorkDir::create(req.output)?;
        let mut current = req.input.to_path_buf();
        for (i, step) in steps.iter().enumerate() {
            if (ctx.cancelled)() {
                bail!("abgebrochen");
            }
            let t = catalog.get(&step.target).ok_or_else(|| anyhow!("Schritt {}: Unbekanntes Ziel {}", i + 1, step.target))?;
            let label = format!("Schritt {} ({})", i + 1, t.title);
            let fail = |e: anyhow::Error| anyhow!("{label}: {e:#}");
            let owned;
            let analysis = if i == 0 {
                req.analysis
            } else {
                owned = analyze(&current).map_err(fail)?;
                &owned
            };
            if !t.accepts(analysis) {
                return Err(fail(anyhow!("{} wird hier nicht unterstützt", analysis.format.id())));
            }
            let engine = get(&t.engine).ok_or_else(|| fail(anyhow!("Engine fehlt: {}", t.engine)))?;
            let options = Options { preset: step.preset.clone(), ..Default::default() };
            let params = effective_params(t, &options);
            if params.get("pick_folder").and_then(|v| v.as_bool()).unwrap_or(false) {
                return Err(fail(anyhow!("„Ordner wählen“ ist in Abläufen nicht möglich")));
            }
            let last = i + 1 == n;
            let out = if last { req.output.to_path_buf() } else { work.0.join(format!("step{}.{}", i + 1, t.ext_for(analysis))) };
            let stem = Mutex::new(None);
            let progress = |f: f32| (ctx.progress)((i as f32 + f.clamp(0.0, 1.0)) / n as f32);
            let sub = Ctx { progress: &progress, cancelled: ctx.cancelled, final_stem: &stem };
            let sub_req = Request { input: &current, output: &out, analysis, target: t, params: &params, options: &options };
            engine.convert(&sub_req, &sub).map_err(fail)?;
            if (ctx.cancelled)() {
                bail!("abgebrochen");
            }
            if !out.exists() {
                return Err(fail(anyhow!("keine Ausgabe geschrieben")));
            }
            if last {
                if let Some(s) = stem.into_inner().ok().flatten() {
                    *ctx.final_stem.lock().unwrap() = Some(s);
                }
            } else {
                if current != req.input {
                    let _ = std::fs::remove_file(&current);
                }
                current = out;
            }
        }
        (ctx.progress)(1.0);
        Ok(())
    }
}
