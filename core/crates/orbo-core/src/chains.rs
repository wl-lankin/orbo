//! User-defined chained conversions ("Abläufe"). The app pushes the configured chains in through
//! `configure`; the catalog turns each valid chain into a target and the "chain" engine runs the steps.
//! Validation works on formats only (no file access), so it is cheap enough to run on every drag.

use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::analyze::Analysis;
use crate::catalog::{Catalog, Requirement, Target};
use crate::format::Format;

pub const GROUP: &str = "Abläufe";
pub const ID_PREFIX: &str = "chain_";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainStep {
    pub target: String,
    #[serde(default)]
    pub preset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<ChainStep>,
    /// Take a free ring slot for compatible files (never pushes core targets out).
    #[serde(default)]
    pub in_ring: bool,
}

static CHAINS: RwLock<Vec<Chain>> = RwLock::new(Vec::new());

/// Replaces the configured chains and rebuilds the merged catalog on next use.
pub fn configure(chains: Vec<Chain>) {
    *CHAINS.write().unwrap() = chains;
    crate::catalog::invalidate_current();
}

pub fn configured() -> Vec<Chain> {
    CHAINS.read().unwrap().clone()
}

const ALL_FORMATS: &[Format] = {
    use Format::*;
    &[
        Png, Jpeg, Webp, Avif, Heic, Gif, Bmp, Tiff, Ico, Icns, Tga, Qoi, Pnm, Svg, Psd, Raw, Mp4, Mov, M4v, Mkv, Webm, Avi, Wmv, Flv, Mpeg, Mts, Threegp, Ogv,
        Mp3, M4a, Aac, Wav, Flac, Aiff, Ogg, Opus, Wma, Amr, Caf, Ac3, Pdf, Docx, Doc, Odt, Rtf, Txt, Md, Html, Epub, Pptx, Csv, Tsv, Xlsx, Xls, Ods, Json, Yaml,
        Toml, Xml, Srt, Vtt, Zip, SevenZ, Rar, Tar, Gz, Tgz, Bz2, Xz, Dmg, Iso, Dxf, Dwg, Step, Iges, Stl, Obj, Ply, Glb, Gltf, ThreeMf, Fbx, Dae, Ttf, Otf, Woff,
        Woff2, Mobi, Azw3, Fb2, Cbz, Ass, Sbv, Ppt, Odp, Folder, Unknown,
    ]
};

fn builtin_step(step: &ChainStep) -> Result<&'static Target, String> {
    Catalog::builtin().get(&step.target).ok_or_else(|| format!("Das Ziel „{}“ gibt es nicht (mehr).", step.target))
}

fn picks_folder(t: &Target, preset: Option<&str>) -> bool {
    let options = crate::jobs::Options { preset: preset.map(String::from), ..Default::default() };
    crate::jobs::effective_params(t, &options).get("pick_folder").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Checks everything that does not depend on the input file type.
fn check_structure(steps: &[ChainStep]) -> Result<Vec<&'static Target>, (usize, String)> {
    if steps.is_empty() {
        return Err((0, "Ein Ablauf braucht mindestens einen Schritt.".into()));
    }
    let mut out = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let t = builtin_step(s).map_err(|e| (i, e))?;
        if let Some(p) = &s.preset {
            if !t.presets.iter().any(|x| &x.label == p) {
                return Err((i, format!("Schritt {} ({}): Die Voreinstellung „{p}“ gibt es nicht.", i + 1, t.title)));
            }
        }
        if t.merge {
            return Err((i, format!("Schritt {} ({}): Zusammenführen ist in Abläufen nicht möglich.", i + 1, t.title)));
        }
        if picks_folder(t, s.preset.as_deref()) {
            return Err((i, format!("Schritt {} ({}): „Ordner wählen“ ist in Abläufen nicht möglich.", i + 1, t.title)));
        }
        if t.produces_folder() && i + 1 < steps.len() {
            return Err((i, format!("Schritt {} ({}) erzeugt einen Ordner und darf nur der letzte Schritt sein.", i + 1, t.title)));
        }
        out.push(t);
    }
    Ok(out)
}

/// Format of the result when `t` runs on `input`.
fn output_after(t: &Target, input: Format) -> Format {
    if t.produces_folder() {
        Format::Folder
    } else if t.keeps_format() {
        input
    } else {
        t.output_format().unwrap_or(input)
    }
}

/// Runs the chain on formats only. Err = (failing step index, explanation).
fn simulate(targets: &[&Target], start: Format) -> Result<Format, (usize, String)> {
    let mut cur = start;
    for (i, t) in targets.iter().enumerate() {
        if !t.accepts_format(cur) {
            let msg = if i == 0 {
                format!("Schritt 1 ({}) passt nicht zu diesem Dateityp.", t.title)
            } else {
                format!("Schritt {} ({}) kann die Ausgabe von Schritt {} ({}) nicht verarbeiten.", i + 1, t.title, i, targets[i - 1].title)
            };
            return Err((i, msg));
        }
        cur = output_after(t, cur);
    }
    Ok(cur)
}

#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    pub ok: bool,
    pub error: Option<String>,
    pub error_step: Option<usize>,
    /// Input format ids the whole chain works for.
    pub inputs: Vec<String>,
}

/// Validates a chain for the builder: structure first, then whether any input format makes it through.
pub fn validate(steps: &[ChainStep]) -> Validation {
    let targets = match check_structure(steps) {
        Ok(t) => t,
        Err((i, e)) => return Validation { ok: false, error: Some(e), error_step: Some(i), inputs: vec![] },
    };
    let mut inputs = Vec::new();
    let mut worst: Option<(usize, String)> = None;
    for f in ALL_FORMATS.iter().filter(|f| !matches!(f, Format::Folder | Format::Unknown)) {
        match simulate(&targets, *f) {
            Ok(_) => inputs.push(f.id()),
            Err((i, m)) => {
                // Failures at step 1 only say "wrong file type"; later ones explain the combination.
                if worst.as_ref().map_or(true, |(wi, _)| i > *wi) {
                    worst = Some((i, m));
                }
            }
        }
    }
    if inputs.is_empty() {
        let (i, m) = worst.unwrap_or((0, "Dieser Ablauf passt zu keinem Dateityp.".into()));
        return Validation { ok: false, error: Some(m), error_step: Some(i), inputs };
    }
    Validation { ok: true, error: None, error_step: None, inputs }
}

/// Targets that may follow `steps` in the builder. Empty `steps` = every target that can start a chain.
pub fn candidates(steps: &[ChainStep]) -> Vec<&'static Target> {
    let all = &Catalog::builtin().targets;
    let usable = |t: &&Target| !t.merge && !t.accepts.iter().all(|a| a == "folder") && !picks_folder(t, None);
    if steps.is_empty() {
        return all.iter().filter(usable).collect();
    }
    let Ok(targets) = check_structure(steps) else { return vec![] };
    let mut reachable: Vec<Format> = Vec::new();
    for f in ALL_FORMATS.iter().filter(|f| !matches!(f, Format::Folder | Format::Unknown)) {
        if let Ok(out) = simulate(&targets, *f) {
            if out != Format::Folder && !reachable.contains(&out) {
                reachable.push(out);
            }
        }
    }
    all.iter().filter(usable).filter(|t| reachable.iter().any(|f| t.accepts_format(*f))).collect()
}

/// Used by `Target::accepts` for chain targets: step 1 sees the real analysis, later steps only formats.
pub(crate) fn accepts(chain: &Target, a: &Analysis) -> bool {
    let Ok(targets) = chain.steps.iter().map(builtin_step).collect::<Result<Vec<_>, _>>() else { return false };
    let Some(first) = targets.first() else { return false };
    if !first.accepts(a) {
        return false;
    }
    let mut cur = output_after(first, a.format);
    for t in targets.iter().skip(1) {
        if cur == Format::Folder || !t.accepts_format(cur) {
            return false;
        }
        cur = output_after(t, cur);
    }
    true
}

/// Extension of the chain's result for this input.
pub(crate) fn final_ext(chain: &Target, a: &Analysis) -> String {
    let Ok(targets) = chain.steps.iter().map(builtin_step).collect::<Result<Vec<_>, _>>() else { return chain.output_ext().to_string() };
    let mut cur = a.format;
    for t in &targets {
        cur = output_after(t, cur);
    }
    match targets.last() {
        Some(_) if cur == Format::Folder => String::new(),
        Some(last) if !last.keeps_format() && last.ext.is_some() => last.ext.clone().unwrap_or_default(),
        _ => cur.extension().to_string(),
    }
}

fn summary(targets: &[&Target], steps: &[ChainStep]) -> String {
    let labels: Vec<String> = targets
        .iter()
        .zip(steps)
        .map(|(t, s)| match &s.preset {
            Some(p) => format!("{} {}", t.title, p),
            None => t.title.clone(),
        })
        .collect();
    let full = labels.join(" · ");
    if full.chars().count() <= 40 || labels.len() <= 2 {
        full
    } else {
        format!("{} · {} · …", labels[0], labels[1])
    }
}

/// The catalog target for a chain, or None when it is invalid (unknown step, bad combination).
pub fn build_target(c: &Chain) -> Option<Target> {
    let targets = check_structure(&c.steps).ok()?;
    if !validate(&c.steps).ok {
        return None;
    }
    let first = targets.first()?;
    let last = targets.last()?;
    // Nominal output: the last step that really changes the format, else the last step.
    let out_target = targets.iter().rev().find(|t| !t.keeps_format()).unwrap_or(last);
    let output = if last.produces_folder() { "folder".to_string() } else { out_target.output.clone() };
    let mut requires: Vec<Requirement> = Vec::new();
    for r in targets.iter().flat_map(|t| t.requires.iter()) {
        if !requires.contains(r) {
            requires.push(*r);
        }
    }
    Some(Target {
        id: format!("{ID_PREFIX}{}", c.id),
        title: c.name.clone(),
        sub: summary(&targets, &c.steps),
        output,
        engine: "chain".into(),
        accepts: first.accepts.clone(),
        requires,
        intent: false,
        ai: targets.iter().any(|t| t.ai),
        same_format: true,
        ext: None,
        suffix: String::new(),
        group: GROUP.into(),
        params: serde_json::json!({}),
        presets: vec![],
        controls: vec![],
        dialog: false,
        chain: true,
        in_ring: c.in_ring,
        steps: c.steps.clone(),
        merge: false,
    })
}
