//! MCP server over stdio (JSON-RPC 2.0, one message per line). Hand-written: the surface is four tools,
//! so an SDK would add more build weight than it saves.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orbo_core::catalog::{kind_id, Catalog, Target};
use orbo_core::jobs::{convert_blocking, convert_merge_blocking, JobSpec, MergeSpec, Options};
use serde_json::{json, Value};

const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_FILES: usize = 100;
/// Upper bound for one `orbo_convert` call; the remaining files are reported as not started.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(20 * 60);

pub fn serve() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&line) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// Handles one incoming message; `None` for notifications, which get no answer.
pub fn handle_line(line: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error_reply(Value::Null, -32700, &format!("Ungültiges JSON: {e}"))),
    };
    let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
        // A response or garbage; a server that never sends requests ignores it.
        return msg.get("id").filter(|_| msg.get("result").is_none() && msg.get("error").is_none()).map(|id| error_reply(id.clone(), -32600, "Ungültige Anfrage"));
    };
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = id else { return None };
    Some(match dispatch(method, &params) {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error_reply(id, code, &message),
    })
}

fn error_reply(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn dispatch(method: &str, params: &Value) -> Result<Value, (i32, String)> {
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let version = SUPPORTED_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&SUPPORTED_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "orbo", "title": "Orbo", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Orbo wandelt Dateien lokal um. Mit orbo_suggest siehst du, was für eine Datei passt, mit orbo_convert wandelst du sie um. Ziele mit „merge“ (PDFs zusammenführen, Bilder als ein PDF, Audio aneinanderhängen) machen aus mehreren Dateien eine; orbo_suggest nennt sie unter „merge_targets“, wenn du mehrere Pfade übergibst. Ergebnisse landen neben der Originaldatei, vorhandene Dateien werden nie überschrieben."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).ok_or((-32602, "Parameter „name“ fehlt".to_string()))?;
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let outcome = match name {
                "orbo_suggest" => tool_suggest(&args),
                "orbo_list_targets" => tool_list_targets(&args),
                "orbo_convert" => tool_convert(&args),
                "orbo_analyze" => tool_analyze(&args),
                _ => return Err((-32602, format!("Unbekanntes Werkzeug: {name}"))),
            };
            Ok(match outcome {
                Ok(v) => tool_result(&v, false),
                Err(msg) => tool_result(&Value::String(msg), true),
            })
        }
        _ => Err((-32601, format!("Methode nicht unterstützt: {method}"))),
    }
}

fn tool_result(v: &Value, is_error: bool) -> Value {
    let text = match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn tool_definitions() -> Value {
    let paths = json!({ "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": MAX_FILES, "description": "Absolute Dateipfade (~ wird aufgelöst)" });
    json!([
        {
            "name": "orbo_suggest",
            "title": "Umwandlungen vorschlagen",
            "description": "Analysiert Dateien (Format, Art, Größe, Eigenschaften) und nennt, was Orbo daraus machen kann: das empfohlene Ziel, bis zu sechs Ring-Vorschläge und alle kompatiblen Ziele mit Titel, Gruppe und Voreinstellungen. Bei zwei oder mehr Pfaden listet „merge_targets“ die Ziele, die alle Dateien zu einer zusammenführen (PDFs, Bilder als ein PDF, Audio).",
            "inputSchema": { "type": "object", "properties": { "paths": paths }, "required": ["paths"] },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "orbo_list_targets",
            "title": "Ziele auflisten",
            "description": "Listet die Umwandlungsziele von Orbo, einschließlich eigener Abläufe und aktivierter KI-Ziele. Optional gefiltert nach Dateiart (z. B. image, video, audio, document) oder Eingabeformat (z. B. png, mp4).",
            "inputSchema": { "type": "object", "properties": {
                "kind": { "type": "string", "description": "Dateiart: image, video, audio, document, data, archive, model, cad, typeface, folder" },
                "format": { "type": "string", "description": "Eingabeformat, z. B. png, mov, pdf" } } },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "orbo_convert",
            "title": "Dateien umwandeln",
            "description": "Wandelt Dateien nacheinander um und liefert pro Datei den Ausgabepfad und die Größen. Bestehende Dateien werden nie überschrieben; das Ergebnis liegt standardmäßig neben der Originaldatei. Die Ziel-IDs liefern orbo_suggest und orbo_list_targets. Ziele mit „merge: true“ (pdf_merge, images_pdf, audio_concat) brauchen mindestens zwei Pfade und erzeugen EINE Ausgabedatei, in natürlicher Reihenfolge nach Dateiname („Seite 2“ vor „Seite 10“), benannt nach der ersten Datei plus „ (zusammengeführt)“.",
            "inputSchema": { "type": "object", "properties": {
                "paths": paths,
                "target": { "type": "string", "description": "Ziel-ID, z. B. webp, mp3, mail_video" },
                "preset": { "type": "string", "description": "Name einer Voreinstellung des Ziels" },
                "quality": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Qualität 1 bis 100 für verlustbehaftete Formate" },
                "max_edge": { "type": "integer", "minimum": 1, "description": "Längste Kante in Pixeln, größere Bilder werden verkleinert" },
                "out_dir": { "type": "string", "description": "Zielordner (absolut, muss existieren) statt neben dem Original" } },
                "required": ["paths", "target"] },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "orbo_analyze",
            "title": "Datei analysieren",
            "description": "Erkennt Format und Eigenschaften einer Datei (Art, Größe, Bildmaße, Dauer, Codecs, Tabellenform).",
            "inputSchema": { "type": "object", "properties": { "path": { "type": "string", "description": "Absoluter Dateipfad" } }, "required": ["path"] },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

fn expand(raw: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if raw == "~" {
        return home().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn existing_path(raw: &str) -> Result<PathBuf, String> {
    let p = expand(raw.trim());
    if !p.is_absolute() {
        return Err(format!("Der Pfad muss absolut sein: {raw}"));
    }
    if !p.exists() {
        return Err(format!("Die Datei oder der Ordner existiert nicht: {}", p.display()));
    }
    Ok(p)
}

fn path_list(args: &Value) -> Result<Vec<String>, String> {
    let list = args.get("paths").and_then(|p| p.as_array()).ok_or("Parameter „paths“ fehlt (Liste von Dateipfaden).")?;
    if list.is_empty() {
        return Err("Parameter „paths“ ist leer.".into());
    }
    if list.len() > MAX_FILES {
        return Err(format!("Zu viele Dateien auf einmal (höchstens {MAX_FILES})."));
    }
    list.iter().map(|v| v.as_str().map(String::from).ok_or_else(|| "„paths“ darf nur Texte enthalten.".to_string())).collect()
}

fn target_json(t: &Target) -> Value {
    json!({
        "id": t.id,
        "title": t.title,
        "sub": t.sub,
        "group": t.group,
        "ext": t.output_ext(),
        "ai": t.ai,
        "chain": t.chain,
        "merge": t.merge,
        "presets": t.presets.iter().map(|p| p.label.clone()).collect::<Vec<_>>(),
    })
}

fn tool_analyze(args: &Value) -> Result<Value, String> {
    let raw = args.get("path").and_then(|p| p.as_str()).ok_or("Parameter „path“ fehlt.")?;
    let path = existing_path(raw)?;
    let a = orbo_core::analyze(&path).map_err(|e| format!("{e:#}"))?;
    let mut v = serde_json::to_value(&a).map_err(|e| e.to_string())?;
    v["kind_id"] = json!(kind_id(a.kind));
    v["format_id"] = json!(a.format.id());
    Ok(v)
}

fn suggest_one(raw: &str) -> Value {
    let result = existing_path(raw).and_then(|p| orbo_core::suggest(&p).map_err(|e| format!("{e:#}")));
    let (a, s) = match result {
        Ok(x) => x,
        Err(e) => return json!({ "path": raw, "error": e }),
    };
    let catalog = Catalog::current();
    let details = |id: &str| catalog.get(id).map(target_json);
    let compatible: Vec<Value> = s.ring.iter().chain(s.more.iter()).filter_map(|x| details(&x.id)).collect();
    json!({
        "path": a.path,
        "format": a.format.id(),
        "kind": kind_id(a.kind),
        "size": a.size,
        "extension_mismatch": a.ext_mismatch,
        "image": a.image,
        "media": a.media,
        "table": a.table,
        "default_target": s.default,
        "suggested": s.ring.iter().take(6).filter_map(|x| details(&x.id)).collect::<Vec<_>>(),
        "compatible_targets": compatible,
    })
}

fn tool_suggest(args: &Value) -> Result<Value, String> {
    let paths = path_list(args)?;
    Ok(json!({ "files": paths.iter().map(|p| suggest_one(p)).collect::<Vec<_>>(), "merge_targets": merge_targets(&paths) }))
}

/// Targets that join all given files into one; empty for a single file or when a file cannot be read.
fn merge_targets(paths: &[String]) -> Vec<Value> {
    if paths.len() < 2 {
        return Vec::new();
    }
    let analyses: Result<Vec<_>, String> = paths.iter().map(|p| existing_path(p).and_then(|p| orbo_core::analyze(&p).map_err(|e| format!("{e:#}")))).collect();
    let Ok(analyses) = analyses else { return Vec::new() };
    Catalog::current().compatible_all(&analyses).into_iter().filter(|t| t.merge).map(target_json).collect()
}

fn tool_list_targets(args: &Value) -> Result<Value, String> {
    let kind = args.get("kind").and_then(|v| v.as_str()).map(|s| s.to_lowercase());
    let format = args.get("format").and_then(|v| v.as_str()).map(|s| s.trim_start_matches('.').to_lowercase());
    let catalog = Catalog::current();
    let targets: Vec<Value> = catalog
        .targets
        .iter()
        .filter(|t| !t.requires.contains(&orbo_core::catalog::Requirement::Ollama) || orbo_core::ai::action_enabled(t.param_str("action")))
        .filter(|t| kind.as_ref().map_or(true, |k| t.accepts.iter().any(|a| a == k)))
        .filter(|t| format.as_ref().map_or(true, |f| t.accepts.iter().any(|a| a == f)))
        .map(target_json)
        .collect();
    Ok(json!({ "count": targets.len(), "targets": targets }))
}

fn int_arg(args: &Value, key: &str, max: u64) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=max).contains(&n) => Ok(Some(n)),
            _ => Err(format!("„{key}“ muss eine ganze Zahl von 1 bis {max} sein.")),
        },
    }
}

fn tool_convert(args: &Value) -> Result<Value, String> {
    let timeout = CONVERT_TIMEOUT;
    let paths = path_list(args)?;
    let target_id = args.get("target").and_then(|v| v.as_str()).ok_or("Parameter „target“ fehlt.")?;
    let catalog = Catalog::current();
    let target = catalog.get(target_id).ok_or_else(|| format!("Unbekanntes Ziel „{target_id}“. Gültige Ziele liefern orbo_suggest und orbo_list_targets."))?;
    let preset = args.get("preset").and_then(|v| v.as_str()).map(String::from);
    if let Some(p) = &preset {
        if !target.presets.iter().any(|x| &x.label == p) {
            let known: Vec<_> = target.presets.iter().map(|x| x.label.as_str()).collect();
            return Err(format!("Das Ziel „{}“ hat keine Voreinstellung „{p}“. Verfügbar: {}.", target.id, if known.is_empty() { "keine".into() } else { known.join(", ") }));
        }
    }
    let out_dir = match args.get("out_dir").and_then(|v| v.as_str()) {
        Some(d) => {
            let p = existing_path(d)?;
            if !p.is_dir() {
                return Err(format!("„out_dir“ ist kein Ordner: {}", p.display()));
            }
            Some(p)
        }
        None => None,
    };
    let options = Options {
        quality: int_arg(args, "quality", 100)?.map(|n| n as u8),
        max_edge: int_arg(args, "max_edge", 100_000)?.map(|n| n as u32),
        preset,
        out_dir,
        ..Default::default()
    };

    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (cancel, done) = (cancel.clone(), done.clone());
        std::thread::spawn(move || {
            let start = Instant::now();
            while !done.load(Ordering::Relaxed) {
                if start.elapsed() >= timeout {
                    cancel.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };

    if target.merge {
        let outcome = run_merge(&paths, target_id, &options, &cancel);
        done.store(true, Ordering::Relaxed);
        let _ = watchdog.join();
        return outcome;
    }

    let mut results = Vec::new();
    for (i, raw) in paths.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            results.push(json!({ "input": raw, "ok": false, "error": "Nicht gestartet: Das Zeitlimit für diesen Aufruf ist erreicht." }));
            continue;
        }
        let input = match existing_path(raw) {
            Ok(p) => p,
            Err(e) => {
                results.push(json!({ "input": raw, "ok": false, "error": e }));
                continue;
            }
        };
        let size_in = std::fs::metadata(&input).map(|m| if m.is_dir() { dir_size(&input) } else { m.len() }).unwrap_or(0);
        let spec = JobSpec { input: input.clone(), target: target_id.to_string(), options: options.clone() };
        results.push(match convert_blocking(&spec, i as u64 + 1, &cancel, &|_| {}) {
            Ok(out) => {
                let size_out = if out.is_dir() { dir_size(&out) } else { std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) };
                json!({ "input": input, "ok": true, "output": out, "size_in": size_in, "size_out": size_out })
            }
            Err(e) => json!({ "input": input, "ok": false, "error": format!("{e:#}") }),
        });
    }
    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    let failed = results.iter().filter(|r| r["ok"] == false).count();
    let summary = json!({ "target": target_id, "converted": results.len() - failed, "failed": failed, "results": results });
    if failed == results.len() {
        // Every file failed: report as a tool error so the model sees the failure, with the details attached.
        return Err(serde_json::to_string_pretty(&summary).unwrap_or_default());
    }
    Ok(summary)
}

/// All paths become one file; the result is reported like a single conversion with an `inputs` list.
fn run_merge(paths: &[String], target_id: &str, options: &Options, cancel: &AtomicBool) -> Result<Value, String> {
    if paths.len() < 2 {
        return Err(format!("Das Ziel „{target_id}“ führt mehrere Dateien zu einer zusammen und braucht in „paths“ mindestens zwei Dateien."));
    }
    let inputs = paths.iter().map(|p| existing_path(p)).collect::<Result<Vec<_>, _>>()?;
    let size_in: u64 = inputs.iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    let spec = MergeSpec { inputs, target: target_id.to_string(), options: options.clone() };
    let ordered = orbo_core::output::natural_sorted(&spec.inputs);
    let summary = |result: Value, failed: u32| json!({ "target": target_id, "converted": 1 - failed, "failed": failed, "results": [result] });
    match convert_merge_blocking(&spec, 1, cancel, &|_| {}) {
        Ok(out) => {
            let size_out = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            Ok(summary(json!({ "inputs": ordered, "ok": true, "output": out, "size_in": size_in, "size_out": size_out }), 0))
        }
        Err(e) => Err(serde_json::to_string_pretty(&summary(json!({ "inputs": ordered, "ok": false, "error": format!("{e:#}") }), 1)).unwrap_or_default()),
    }
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|it| it.filter_map(|e| e.ok()).map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        }).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(method: &str, params: Value) -> Value {
        handle_line(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()).expect("reply")
    }

    fn tool(name: &str, args: Value) -> Value {
        call("tools/call", json!({ "name": name, "arguments": args }))["result"].clone()
    }

    fn text_of(result: &Value) -> String {
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn png(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        image::RgbaImage::from_fn(32, 24, |x, y| image::Rgba([x as u8 * 7, y as u8 * 9, 120, 255])).save(&p).unwrap();
        p
    }

    #[test]
    fn initialize_negotiates_version() {
        let r = call("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } }));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "orbo");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        let r = call("initialize", json!({ "protocolVersion": "1999-01-01" }));
        assert_eq!(r["result"]["protocolVersion"], SUPPORTED_VERSIONS[0]);
    }

    #[test]
    fn notifications_get_no_reply_and_ping_works() {
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(call("ping", Value::Null)["result"], json!({}));
        assert_eq!(call("nope", Value::Null)["error"]["code"], -32601);
        assert_eq!(handle_line("{bad").unwrap()["error"]["code"], -32700);
    }

    #[test]
    fn lists_four_tools() {
        let r = call("tools/list", Value::Null);
        let names: Vec<_> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["orbo_suggest", "orbo_list_targets", "orbo_convert", "orbo_analyze"]);
        for t in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn converts_png_to_webp_in_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let input = png(dir.path(), "bild.png");
        let out_dir = dir.path().join("out");
        std::fs::create_dir(&out_dir).unwrap();
        let r = tool("orbo_convert", json!({ "paths": [input], "target": "webp", "quality": 80, "out_dir": out_dir }));
        assert_eq!(r["isError"], false, "{}", text_of(&r));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["converted"], 1);
        let output = PathBuf::from(v["results"][0]["output"].as_str().unwrap());
        assert_eq!(output, out_dir.join("bild.webp"));
        assert!(output.is_file() && v["results"][0]["size_out"].as_u64().unwrap() > 0);
        assert!(input.is_file());
        // A second run must not overwrite.
        let r = tool("orbo_convert", json!({ "paths": [input], "target": "webp", "out_dir": out_dir }));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["results"][0]["output"], out_dir.join("bild 2.webp").to_string_lossy().as_ref());
    }

    #[test]
    fn merge_targets_make_one_output() {
        let dir = tempfile::tempdir().unwrap();
        let a = png(dir.path(), "Seite 2.png");
        let b = png(dir.path(), "Seite 10.png");
        let r = tool("orbo_suggest", json!({ "paths": [b, a] }));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        let ids: Vec<&str> = v["merge_targets"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"images_pdf") && !ids.contains(&"pdf_merge"), "{ids:?}");
        let r = tool("orbo_suggest", json!({ "paths": [a] }));
        assert!(serde_json::from_str::<Value>(&text_of(&r)).unwrap()["merge_targets"].as_array().unwrap().is_empty());

        let r = tool("orbo_convert", json!({ "paths": [b, a], "target": "images_pdf", "preset": "A4 hoch" }));
        assert_eq!(r["isError"], false, "{}", text_of(&r));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["converted"], 1);
        assert_eq!(v["results"].as_array().unwrap().len(), 1);
        let output = PathBuf::from(v["results"][0]["output"].as_str().unwrap());
        assert_eq!(output, dir.path().join("Seite 2 (zusammengeführt).pdf"));
        assert!(output.is_file());
        assert_eq!(v["results"][0]["inputs"][0], a.to_string_lossy().as_ref());

        // The PDF can be merged again with another image.
        let c = png(dir.path(), "Seite 3.png");
        let r = tool("orbo_convert", json!({ "paths": [output, c], "target": "pdf_merge" }));
        assert_eq!(r["isError"], false, "{}", text_of(&r));

        let r = tool("orbo_convert", json!({ "paths": [a], "target": "images_pdf" }));
        assert_eq!(r["isError"], true);
        assert!(text_of(&r).contains("mindestens zwei"));
    }

    #[test]
    fn missing_and_relative_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("x.png");
        let r = tool("orbo_convert", json!({ "paths": [missing], "target": "webp" }));
        assert_eq!(r["isError"], true);
        assert!(text_of(&r).contains("existiert nicht"));
        let r = tool("orbo_analyze", json!({ "path": "relativ/x.png" }));
        assert_eq!(r["isError"], true);
        assert!(text_of(&r).contains("absolut"));
        let r = tool("orbo_convert", json!({ "paths": ["/tmp"], "target": "gibtsnicht" }));
        assert_eq!(r["isError"], true);
        assert_eq!(call("tools/call", json!({ "name": "rm_rf" }))["error"]["code"], -32602);
    }

    #[test]
    fn suggest_and_targets() {
        let dir = tempfile::tempdir().unwrap();
        let input = png(dir.path(), "a.png");
        let r = tool("orbo_suggest", json!({ "paths": [input, "/nonexistent/b.png"] }));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(v["files"][0]["kind"], "image");
        assert!(v["files"][0]["suggested"].as_array().unwrap().len() <= 6);
        assert!(v["files"][0]["compatible_targets"].as_array().unwrap().iter().any(|t| t["id"] == "webp"));
        assert!(v["files"][1]["error"].is_string());
        let r = tool("orbo_list_targets", json!({ "kind": "image" }));
        let v: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert!(v["count"].as_u64().unwrap() > 5);
    }
}
