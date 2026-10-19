//! Picks the few targets the ring shows, based on the file's properties.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::analyze::Analysis;
use crate::catalog::{kind_id, Catalog, Target};

pub const RING_SIZE: usize = 6;

#[derive(Debug, Default, Deserialize)]
struct RingOrder {
    #[serde(default)]
    format: HashMap<String, Vec<String>>,
    #[serde(default)]
    kind: HashMap<String, Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(default)]
    format: Vec<String>,
    #[serde(default)]
    kind: Vec<String>,
    has_alpha: Option<bool>,
    animated: Option<bool>,
    has_gps: Option<bool>,
    has_audio: Option<bool>,
    has_video: Option<bool>,
    #[serde(default)]
    video_codec: Vec<String>,
    #[serde(default)]
    audio_codec: Vec<String>,
    size_gt_mb: Option<f64>,
    size_lt_mb: Option<f64>,
    duration_gt_s: Option<f64>,
    duration_lt_s: Option<f64>,
    longest_edge_gt: Option<u32>,
    ext_mismatch: Option<bool>,
}

impl Condition {
    fn matches(&self, a: &Analysis) -> bool {
        let img = a.image.as_ref();
        let dur = a.media.as_ref().map(|m| m.duration_s).unwrap_or(0.0);
        let mb = a.size as f64 / 1_048_576.0;
        let in_list = |list: &Vec<String>, v: &str| list.is_empty() || list.iter().any(|x| x == v);
        in_list(&self.format, &a.format.id())
            && in_list(&self.kind, kind_id(a.kind))
            && self.has_alpha.map_or(true, |v| img.map(|i| i.has_alpha) == Some(v))
            && self.animated.map_or(true, |v| img.map(|i| i.animated).unwrap_or(false) == v)
            && self.has_gps.map_or(true, |v| img.map(|i| i.has_gps).unwrap_or(false) == v)
            && self.has_audio.map_or(true, |v| a.has_audio() == v)
            && self.has_video.map_or(true, |v| a.has_video() == v)
            && (self.video_codec.is_empty() || a.video_codec().map_or(false, |c| self.video_codec.iter().any(|x| x == c)))
            && (self.audio_codec.is_empty() || a.audio_codec().map_or(false, |c| self.audio_codec.iter().any(|x| x == c)))
            && self.size_gt_mb.map_or(true, |v| mb > v)
            && self.size_lt_mb.map_or(true, |v| mb < v)
            && self.duration_gt_s.map_or(true, |v| dur > v)
            && self.duration_lt_s.map_or(true, |v| dur > 0.0 && dur < v)
            && self.longest_edge_gt.map_or(true, |v| img.map(|i| i.width.max(i.height) > v).unwrap_or(false))
            && self.ext_mismatch.map_or(true, |v| a.ext_mismatch == v)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Shown in `orbo suggest --explain`.
    #[serde(default)]
    why: String,
    when: Condition,
    #[serde(default)]
    promote: Vec<String>,
    #[serde(default)]
    demote: Vec<String>,
    #[serde(default)]
    add: Vec<String>,
    /// Like `add`, but only into free ring slots, so it never pushes anything out.
    #[serde(default)]
    fill: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    note: HashMap<String, String>,
    default: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RulesFile {
    #[serde(default)]
    ring: RingOrder,
    #[serde(default)]
    rules: Vec<Rule>,
}

const BUILTIN_EXTRA_RULES: [&str; 6] = [
    include_str!("../catalog/rules-model.yaml"),
    include_str!("../catalog/rules-font.yaml"),
    include_str!("../catalog/rules-ebook.yaml"),
    include_str!("../catalog/rules-office.yaml"),
    include_str!("../catalog/rules-cad.yaml"),
    include_str!("../catalog/rules-ai.yaml"),
];

pub struct Rules {
    ring: RingOrder,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub ext: String,
    pub intent: bool,
    pub ai: bool,
    pub group: String,
    pub chain: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestions {
    /// Center of the ring.
    pub default: Option<String>,
    /// At most RING_SIZE entries, clockwise from the top.
    pub ring: Vec<Suggestion>,
    /// Everything else that works for this file, for the "Mehr ..." palette.
    pub more: Vec<Suggestion>,
    /// Which rules fired, for debugging.
    pub fired: Vec<String>,
}

impl Rules {
    pub fn from_yaml(text: &str) -> Result<Rules> {
        let f: RulesFile = serde_yaml::from_str(text).context("rules.yaml")?;
        Ok(Rules { ring: f.ring, rules: f.rules })
    }

    pub fn builtin() -> &'static Rules {
        static R: OnceLock<Rules> = OnceLock::new();
        R.get_or_init(|| {
            let mut r = Rules::from_yaml(include_str!("../catalog/rules.yaml")).expect("eingebaute Regeln sind gültig");
            for extra in BUILTIN_EXTRA_RULES {
                let e = Rules::from_yaml(extra).expect("eingebaute Zusatzregeln sind gültig");
                r.ring.format.extend(e.ring.format);
                r.ring.kind.extend(e.ring.kind);
                r.rules.extend(e.rules);
            }
            r
        })
    }

    /// Every id the rules mention must exist in the catalog.
    pub fn validate(&self, catalog: &Catalog) -> Result<()> {
        let mut ids: Vec<&String> = self.ring.format.values().chain(self.ring.kind.values()).flatten().collect();
        for r in &self.rules {
            ids.extend(r.promote.iter().chain(&r.demote).chain(&r.add).chain(&r.fill).chain(&r.remove).chain(r.note.keys()).chain(r.default.iter()));
        }
        for id in ids {
            if catalog.get(id).is_none() {
                anyhow::bail!("Regeln nennen unbekanntes Ziel: {id}");
            }
        }
        Ok(())
    }

    pub fn suggest(&self, catalog: &Catalog, a: &Analysis) -> Suggestions {
        let compatible: Vec<&Target> = catalog.compatible(a);
        let ok = |id: &str| compatible.iter().any(|t| t.id == id);

        let base = self
            .ring
            .format
            .get(&a.format.id())
            .or_else(|| self.ring.kind.get(kind_id(a.kind)))
            .cloned()
            .unwrap_or_default();
        let mut order: Vec<String> = base.into_iter().filter(|id| ok(id)).collect();
        let mut notes: HashMap<String, String> = HashMap::new();
        let mut default = None;
        let mut fired = Vec::new();

        for rule in &self.rules {
            if !rule.when.matches(a) {
                continue;
            }
            fired.push(rule.why.clone());
            for id in &rule.remove {
                order.retain(|x| x != id);
            }
            for id in rule.add.iter().filter(|id| ok(id)) {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
            for id in rule.fill.iter().filter(|id| ok(id)) {
                if order.len() < RING_SIZE && !order.contains(id) {
                    order.push(id.clone());
                }
            }
            // Promoted ids keep their relative order at the front.
            for (pos, id) in rule.promote.iter().filter(|id| ok(id)).enumerate() {
                order.retain(|x| x != id);
                order.insert(pos.min(order.len()), id.clone());
            }
            for id in rule.demote.iter().filter(|id| ok(id)) {
                if order.contains(id) {
                    order.retain(|x| x != id);
                    order.push(id.clone());
                }
            }
            for (id, text) in &rule.note {
                notes.insert(id.clone(), text.clone());
            }
            if let Some(d) = &rule.default {
                if ok(d) {
                    default = Some(d.clone());
                }
            }
        }

        // Chains marked "im Ring" take only slots that are still free.
        for t in compatible.iter().filter(|t| t.chain && t.in_ring) {
            if order.len() < RING_SIZE && !order.contains(&t.id) {
                order.push(t.id.clone());
            }
        }

        // Fill a short ring with further compatible targets so the ring is never half empty.
        for t in &compatible {
            if order.len() >= RING_SIZE {
                break;
            }
            if !order.contains(&t.id) {
                order.push(t.id.clone());
            }
        }
        order.truncate(RING_SIZE);

        let to_s = |t: &Target| Suggestion {
            id: t.id.clone(),
            title: t.title.clone(),
            sub: notes.get(&t.id).cloned().unwrap_or_else(|| t.sub.clone()),
            ext: t.ext_for(a),
            intent: t.intent,
            ai: t.ai,
            group: t.group.clone(),
            chain: t.chain,
        };
        let ring: Vec<Suggestion> = order.iter().filter_map(|id| catalog.get(id)).map(to_s).collect();
        let more: Vec<Suggestion> = compatible.iter().filter(|t| !order.contains(&t.id)).map(|t| to_s(t)).collect();
        let default = default.filter(|d| order.contains(d)).or_else(|| ring.first().map(|s| s.id.clone()));
        Suggestions { default, ring, more, fired }
    }
}
