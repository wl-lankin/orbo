use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use super::{Ctx, Engine, Request};
use crate::format::{sniff_delimiter, Format};

pub struct DataEngine;

struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

struct Sheet {
    name: String,
    table: Table,
}

impl Table {
    fn empty() -> Table {
        Table { header: Vec::new(), rows: Vec::new() }
    }

    /// Makes all rows as wide as the widest one and names blank headers.
    fn normalized(mut self) -> Table {
        let width = self.rows.iter().map(|r| r.len()).chain([self.header.len()]).max().unwrap_or(0);
        self.header.resize(width, String::new());
        for (i, h) in self.header.iter_mut().enumerate() {
            if h.trim().is_empty() {
                *h = format!("Spalte {}", i + 1);
            }
        }
        for r in &mut self.rows {
            r.resize(width, String::new());
        }
        self
    }

    fn from_matrix(mut matrix: Vec<Vec<String>>) -> Table {
        while matrix.last().is_some_and(|r| r.iter().all(|c| c.is_empty())) {
            matrix.pop();
        }
        if matrix.is_empty() {
            return Table::empty();
        }
        let header = matrix.remove(0);
        Table { header, rows: matrix }.normalized()
    }

    fn numeric_columns(&self) -> Vec<bool> {
        (0..self.header.len())
            .map(|c| {
                let mut any = false;
                for r in &self.rows {
                    let cell = &r[c];
                    if cell.is_empty() {
                        continue;
                    }
                    if !is_number(cell) {
                        return false;
                    }
                    any = true;
                }
                any
            })
            .collect()
    }
}

impl Engine for DataEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
        let from = req.analysis.format;
        (ctx.progress)(0.05);
        match (from, to) {
            (Format::Srt | Format::Vtt, "srt" | "vtt") => return subtitles(req, to),
            (Format::Json | Format::Yaml | Format::Toml, "json" | "yaml" | "toml") => return structured(req, to),
            (Format::Srt | Format::Vtt, _) => bail!("Untertitel lassen sich nur in SRT oder VTT umwandeln"),
            _ => {}
        }

        let sheets = read_sheets(req.input, from)?;
        (ctx.progress)(0.4);
        if (ctx.cancelled)() {
            bail!("abgebrochen");
        }
        if to == "xlsx" {
            write_xlsx(&sheets, req.output, ctx)?;
        } else {
            let table = sheets.into_iter().next().map(|s| s.table).unwrap_or_else(Table::empty);
            let out: Vec<u8> = match to {
                "csv" if req.bool("excel_de") => excel_de_csv(&table)?,
                "csv" => delimited(&table, b',', csv::Terminator::Any(b'\n'), false)?,
                "tsv" => delimited(&table, b'\t', csv::Terminator::Any(b'\n'), false)?,
                "json" => table_to_json(&table)?.into_bytes(),
                "md" => markdown(&table).into_bytes(),
                "html" => html(&table, &crate::output::stem_of(req.input)).into_bytes(),
                other => bail!("Ausgabeformat {other} wird von der Datenumwandlung nicht unterstützt"),
            };
            fs::write(req.output, out).context("Ausgabe konnte nicht geschrieben werden")?;
        }
        (ctx.progress)(0.95);
        Ok(())
    }
}

// ---------- reading ----------

fn read_sheets(path: &Path, from: Format) -> Result<Vec<Sheet>> {
    match from {
        Format::Csv | Format::Tsv => {
            let text = read_text(path)?;
            let delim = sniff_delimiter(&text).unwrap_or_else(|| header_delimiter(&text, from));
            Ok(vec![Sheet { name: "Tabelle1".into(), table: parse_delimited(&text, delim)? }])
        }
        Format::Xlsx | Format::Xls | Format::Ods => read_workbook(path),
        Format::Json | Format::Yaml | Format::Toml => {
            let value = read_value(path, from)?;
            Ok(vec![Sheet { name: "Tabelle1".into(), table: json_to_table(&value)? }])
        }
        other => bail!("{} kann nicht als Tabelle gelesen werden", other.id()),
    }
}

/// Fallback for ragged files: the delimiter that occurs most often in the header line.
fn header_delimiter(text: &str, from: Format) -> char {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let default = if from == Format::Tsv { '\t' } else { ',' };
    [';', '\t', '|', ','].into_iter().max_by_key(|&d| (first.matches(d).count(), d == default)).filter(|&d| first.contains(d)).unwrap_or(default)
}

/// UTF-8 with or without BOM; anything else is decoded as Latin-1 (typical for old Excel exports).
fn read_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).context("Datei konnte nicht gelesen werden")?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    Ok(match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    })
}

fn parse_delimited(text: &str, delim: char) -> Result<Table> {
    let mut rdr = csv::ReaderBuilder::new().delimiter(delim as u8).has_headers(false).flexible(true).from_reader(text.as_bytes());
    let mut matrix = Vec::new();
    for rec in rdr.records() {
        let rec = rec.context("Die Tabelle ist beschädigt")?;
        matrix.push(rec.iter().map(|s| s.to_string()).collect());
    }
    Ok(Table::from_matrix(matrix))
}

fn read_workbook(path: &Path) -> Result<Vec<Sheet>> {
    use calamine::{open_workbook_auto_from_rs, Reader};
    let bytes = fs::read(path).context("Datei konnte nicht gelesen werden")?;
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|e| anyhow!("Die Arbeitsmappe konnte nicht geöffnet werden: {e}"))?;
    let mut sheets = Vec::new();
    for name in wb.sheet_names() {
        let range = wb.worksheet_range(&name).map_err(|e| anyhow!("Tabellenblatt {name} konnte nicht gelesen werden: {e}"))?;
        let matrix = range.rows().map(|row| row.iter().map(data_text).collect()).collect();
        sheets.push(Sheet { name, table: Table::from_matrix(matrix) });
    }
    Ok(sheets)
}

fn data_text(d: &calamine::Data) -> String {
    use calamine::Data;
    match d {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => float_text(*f),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => {
            let v = dt.as_f64();
            if dt.is_duration() {
                float_text(v)
            } else {
                serial_date_text(v)
            }
        }
    }
}

fn float_text(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

/// Excel serial date (1900 system, valid from 1900-03-01) as ISO text.
fn serial_date_text(serial: f64) -> String {
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86400.0).round() as i64;
    let (y, m, d) = civil_from_days(days - 25569);
    if secs == 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
    }
}

/// Days since 1970-01-01 to (year, month, day).
pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn read_value(path: &Path, from: Format) -> Result<Value> {
    let text = read_text(path)?;
    match from {
        Format::Json => serde_json::from_str(&text).map_err(|e| anyhow!("Das JSON ist ungültig (Zeile {}, Spalte {}): {e}", e.line(), e.column())),
        Format::Yaml => serde_yaml::from_str(&text).map_err(|e| anyhow!("Das YAML ist ungültig: {e}")),
        Format::Toml => {
            let v: toml::Value = toml::from_str(&text).map_err(|e| anyhow!("Das TOML ist ungültig: {e}"))?;
            Ok(toml_to_json(v))
        }
        _ => bail!("Kein strukturiertes Format"),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(map) if !map.is_empty() => {
            for (k, child) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&key, child, out);
            }
        }
        other => out.push((if prefix.is_empty() { "value".into() } else { prefix.to_string() }, value_text(other))),
    }
}

fn json_to_table(v: &Value) -> Result<Table> {
    let items: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![v],
        _ => bail!("Aus diesem JSON lässt sich keine Tabelle machen: erwartet wird eine Liste oder ein Objekt"),
    };
    if !items.is_empty() && items.iter().all(|i| i.is_array()) {
        let matrix = items.iter().map(|i| i.as_array().unwrap().iter().map(value_text).collect()).collect();
        return Ok(Table::from_matrix(matrix));
    }
    let mut columns: Vec<String> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut parsed: Vec<Vec<(usize, String)>> = Vec::with_capacity(items.len());
    for item in items {
        let mut pairs = Vec::new();
        flatten("", item, &mut pairs);
        let mut row = Vec::with_capacity(pairs.len());
        for (k, val) in pairs {
            let next = columns.len();
            let i = *index.entry(k.clone()).or_insert_with(|| {
                columns.push(k);
                next
            });
            row.push((i, val));
        }
        parsed.push(row);
    }
    let rows = parsed
        .into_iter()
        .map(|cells| {
            let mut row = vec![String::new(); columns.len()];
            for (i, val) in cells {
                row[i] = val;
            }
            row
        })
        .collect();
    Ok(Table { header: columns, rows }.normalized())
}

// ---------- number handling ----------

/// True for cells that are safe to treat as numbers: no leading zeros, no plus sign, at most 15 digits.
fn is_number(s: &str) -> bool {
    if s.is_empty() || s.len() > 24 || !s.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
        return false;
    }
    let t = s.strip_prefix('-').unwrap_or(s);
    let b = t.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() || (b[0] == b'0' && b.get(1).is_some_and(|c| c.is_ascii_digit())) {
        return false;
    }
    let mantissa = t.split(['e', 'E']).next().unwrap_or("");
    if mantissa.chars().filter(|c| c.is_ascii_digit()).count() > 15 {
        return false;
    }
    s.parse::<f64>().is_ok_and(|f| f.is_finite())
}

fn infer_json(cell: &str) -> Value {
    if cell.is_empty() {
        return Value::Null;
    }
    match cell.to_ascii_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if is_number(cell) {
        if let Ok(i) = cell.parse::<i64>() {
            return Value::from(i);
        }
        if let Some(n) = cell.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(cell.to_string())
}

// ---------- writing ----------

fn delimited(t: &Table, delim: u8, term: csv::Terminator, decimal_comma: bool) -> Result<Vec<u8>> {
    let mut w = csv::WriterBuilder::new().delimiter(delim).terminator(term).flexible(true).from_writer(Vec::new());
    if !t.header.is_empty() {
        w.write_record(&t.header)?;
    }
    for r in &t.rows {
        if decimal_comma {
            w.write_record(r.iter().map(|c| if is_number(c) { c.replace('.', ",") } else { c.clone() }))?;
        } else {
            w.write_record(r)?;
        }
    }
    w.into_inner().map_err(|e| anyhow!("{e}"))
}

fn excel_de_csv(t: &Table) -> Result<Vec<u8>> {
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend(delimited(t, b';', csv::Terminator::CRLF, true)?);
    Ok(out)
}

fn table_to_json(t: &Table) -> Result<String> {
    let mut keys: Vec<String> = Vec::with_capacity(t.header.len());
    for h in &t.header {
        let mut k = h.clone();
        let mut n = 2;
        while keys.contains(&k) {
            k = format!("{h}_{n}");
            n += 1;
        }
        keys.push(k);
    }
    let arr: Vec<Value> = t
        .rows
        .iter()
        .map(|r| Value::Object(keys.iter().zip(r).map(|(k, c)| (k.clone(), infer_json(c))).collect()))
        .collect();
    Ok(serde_json::to_string_pretty(&arr)? + "\n")
}

fn markdown(t: &Table) -> String {
    let esc = |s: &str| s.replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>");
    let num = t.numeric_columns();
    let mut out = String::new();
    out.push_str(&format!("| {} |\n", t.header.iter().map(|h| esc(h)).collect::<Vec<_>>().join(" | ")));
    out.push_str(&format!("| {} |\n", num.iter().map(|&n| if n { "---:" } else { "---" }).collect::<Vec<_>>().join(" | ")));
    for r in &t.rows {
        out.push_str(&format!("| {} |\n", r.iter().map(|c| esc(c)).collect::<Vec<_>>().join(" | ")));
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html(t: &Table, title: &str) -> String {
    let num = t.numeric_columns();
    let mut out = String::from(
        "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    );
    out.push_str(&format!("<title>{}</title>\n<style>\n", html_escape(title)));
    out.push_str(
        "body{font-family:system-ui,sans-serif;margin:2rem;color:#222}\n\
         table{border-collapse:collapse;min-width:50%}\n\
         th,td{border:1px solid #d0d4da;padding:.4rem .75rem;text-align:left}\n\
         th{background:#f1f3f5}\n\
         tbody tr:nth-child(even){background:#fafbfc}\n\
         .num{text-align:right;font-variant-numeric:tabular-nums}\n</style>\n</head>\n<body>\n<table>\n<thead>\n<tr>",
    );
    for (i, h) in t.header.iter().enumerate() {
        out.push_str(&format!("<th{}>{}</th>", if num[i] { " class=\"num\"" } else { "" }, html_escape(h)));
    }
    out.push_str("</tr>\n</thead>\n<tbody>\n");
    for r in &t.rows {
        out.push_str("<tr>");
        for (i, c) in r.iter().enumerate() {
            out.push_str(&format!("<td{}>{}</td>", if num[i] { " class=\"num\"" } else { "" }, html_escape(c)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</body>\n</html>\n");
    out
}

fn sheet_name(raw: &str, used: &[String]) -> String {
    let base: String = raw.chars().filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\')).take(31).collect();
    let base = if base.trim().is_empty() { "Tabelle".to_string() } else { base };
    let mut name = base.clone();
    let mut n = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
        let suffix = format!(" {n}");
        name = format!("{}{suffix}", base.chars().take(31 - suffix.len()).collect::<String>());
        n += 1;
    }
    name
}

fn write_xlsx(sheets: &[Sheet], out: &Path, ctx: &Ctx) -> Result<()> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut wb = Workbook::new();
    let bold = Format::new().set_bold();
    let mut names: Vec<String> = Vec::new();
    let empty = [Sheet { name: "Tabelle1".into(), table: Table::empty() }];
    let sheets = if sheets.is_empty() { &empty[..] } else { sheets };
    for sheet in sheets {
        let t = &sheet.table;
        if t.header.len() > 16384 || t.rows.len() >= 1_048_576 {
            bail!("Die Tabelle ist für Excel zu groß (höchstens 16.384 Spalten und 1.048.575 Zeilen)");
        }
        let name = sheet_name(&sheet.name, &names);
        names.push(name.clone());
        let ws = wb.add_worksheet();
        ws.set_name(&name)?;
        let clip = |s: &str| s.chars().take(32000).collect::<String>();
        for (c, h) in t.header.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, clip(h), &bold)?;
        }
        for (r, row) in t.rows.iter().enumerate() {
            if r % 5000 == 0 && (ctx.cancelled)() {
                bail!("abgebrochen");
            }
            for (c, cell) in row.iter().enumerate() {
                if cell.is_empty() {
                    continue;
                }
                let (r, c) = (r as u32 + 1, c as u16);
                if is_number(cell) {
                    ws.write_number(r, c, cell.parse::<f64>()?)?;
                } else {
                    ws.write_string(r, c, clip(cell))?;
                }
            }
        }
        for c in 0..t.header.len() {
            let longest = std::iter::once(&t.header[c]).chain(t.rows.iter().take(1000).map(|r| &r[c])).map(|s| s.chars().count()).max().unwrap_or(0);
            ws.set_column_width(c as u16, (longest as f64 + 2.0).clamp(8.0, 60.0))?;
        }
        if !t.header.is_empty() {
            ws.autofilter(0, 0, t.rows.len() as u32, t.header.len() as u16 - 1)?;
            ws.set_freeze_panes(1, 0)?;
        }
    }
    wb.save(out).map_err(|e| anyhow!("Excel-Datei konnte nicht geschrieben werden: {e}"))
}

// ---------- structured data ----------

fn toml_to_json(v: toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::from(i),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Value::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

/// TOML has no null, so null entries are dropped.
fn json_to_toml(v: &Value) -> Option<toml::Value> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64()?),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(a.iter().filter_map(json_to_toml).collect()),
        Value::Object(o) => toml::Value::Table(o.iter().filter_map(|(k, v)| json_to_toml(v).map(|t| (k.clone(), t))).collect()),
    })
}

fn structured(req: &Request, to: &str) -> Result<()> {
    let value = read_value(req.input, req.analysis.format)?;
    let out = match to {
        "json" if req.bool("minify") => serde_json::to_string(&value)? + "\n",
        "json" => serde_json::to_string_pretty(&value)? + "\n",
        "yaml" => serde_yaml::to_string(&value).map_err(|e| anyhow!("YAML konnte nicht erzeugt werden: {e}"))?,
        "toml" => {
            if !value.is_object() {
                bail!("TOML braucht auf oberster Ebene ein Objekt mit Schlüsseln; hier steht eine Liste oder ein einzelner Wert");
            }
            let t = json_to_toml(&value).ok_or_else(|| anyhow!("Inhalt kann nicht nach TOML umgewandelt werden"))?;
            toml::to_string_pretty(&t).map_err(|e| anyhow!("TOML konnte nicht erzeugt werden: {e}"))?
        }
        other => bail!("Ausgabeformat {other} wird nicht unterstützt"),
    };
    fs::write(req.output, out).context("Ausgabe konnte nicht geschrieben werden")
}

// ---------- subtitles ----------

struct Cue {
    start: String,
    end: String,
    text: Vec<String>,
}

fn normalize_time(raw: &str) -> Result<String> {
    let raw = raw.trim().replace(',', ".");
    let (clock, frac) = raw.split_once('.').unwrap_or((&raw, "0"));
    let mut parts: Vec<&str> = clock.split(':').collect();
    if parts.len() == 2 {
        parts.insert(0, "0");
    }
    if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) || !frac.chars().all(|c| c.is_ascii_digit()) {
        bail!("Ungültige Zeitangabe im Untertitel: {raw}");
    }
    let ms: String = format!("{frac:0<3}").chars().take(3).collect();
    Ok(format!("{:02}:{}:{}.{ms}", parts[0].parse::<u32>()?, parts[1], parts[2]))
}

fn parse_cues(text: &str) -> Result<Vec<Cue>> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut flush = |block: &mut Vec<&str>| -> Result<()> {
        if let Some(pos) = block.iter().position(|l| l.contains("-->")) {
            let (a, b) = block[pos].split_once("-->").unwrap();
            let end = b.split_whitespace().next().unwrap_or("");
            cues.push(Cue { start: normalize_time(a)?, end: normalize_time(end)?, text: block[pos + 1..].iter().map(|s| s.to_string()).collect() });
        }
        block.clear();
        Ok(())
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut block)?;
        } else {
            block.push(line);
        }
    }
    flush(&mut block)?;
    Ok(cues)
}

fn subtitles(req: &Request, to: &str) -> Result<()> {
    let cues = parse_cues(&read_text(req.input)?)?;
    let mut out = String::new();
    if to == "vtt" {
        out.push_str("WEBVTT\n\n");
    }
    for (i, c) in cues.iter().enumerate() {
        if to == "srt" {
            out.push_str(&format!("{}\n{} --> {}\n", i + 1, c.start.replace('.', ","), c.end.replace('.', ",")));
        } else {
            out.push_str(&format!("{} --> {}\n", c.start, c.end));
        }
        for l in &c.text {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    fs::write(req.output, out).context("Ausgabe konnte nicht geschrieben werden")
}
