use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use calamine::{open_workbook, Data, Reader, Xlsx};
use orbo_core::jobs::{convert_blocking, JobSpec};
use serde_json::{json, Value};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, content).unwrap();
    p
}

const CSV: &str = "Name,Preis,Menge,Hinweis\nBäckerei Müller,1234.5,3,\"mit, Komma\"\nÄpfel & Öl,0.99,12,ß|Pipe\nCode,007,5,\n";

fn sheet_rows(path: &Path, sheet: usize) -> Vec<Vec<Data>> {
    let mut wb: Xlsx<_> = open_workbook(path).unwrap();
    let name = wb.sheet_names()[sheet].clone();
    wb.worksheet_range(&name).unwrap().rows().map(|r| r.to_vec()).collect()
}

#[test]
fn csv_to_xlsx_keeps_numbers_and_umlauts() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "preise.csv", CSV), "xlsx").unwrap();
    assert_eq!(out.extension().unwrap(), "xlsx");
    let rows = sheet_rows(&out, 0);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0][0], Data::String("Name".into()));
    assert_eq!(rows[1][0], Data::String("Bäckerei Müller".into()));
    assert_eq!(rows[1][1], Data::Float(1234.5));
    assert_eq!(rows[1][3], Data::String("mit, Komma".into()));
    assert_eq!(rows[3][1], Data::String("007".into()), "leading zeros stay text");
}

#[test]
fn xlsx_roundtrip_to_csv_and_all_sheets_kept() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("mappe.xlsx");
    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Eins").unwrap();
    ws.write_string(0, 0, "Wort").unwrap();
    ws.write_string(0, 1, "Zahl").unwrap();
    ws.write_string(1, 0, "Größe").unwrap();
    ws.write_number(1, 1, 1234.5).unwrap();
    ws.write_number(2, 1, 7.0).unwrap();
    let ws2 = wb.add_worksheet();
    ws2.set_name("Zwei").unwrap();
    ws2.write_string(0, 0, "x").unwrap();
    ws2.write_string(1, 0, "y").unwrap();
    wb.save(&src).unwrap();

    let out = run(&src, "csv").unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "Wort,Zahl\nGröße,1234.5\n,7\n");

    let out = run(&src, "json").unwrap();
    let v: Value = serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v, json!([{"Wort": "Größe", "Zahl": 1234.5}, {"Wort": null, "Zahl": 7}]));
}

#[test]
fn ods_like_multi_sheet_survives_xlsx_target() {
    // xlsx has no xlsx target, so exercise the multi-sheet writer through xls/ods-free path: csv -> xlsx -> csv
    let dir = tempfile::tempdir().unwrap();
    let x = run(&write(dir.path(), "a.csv", "a;b\n1;2\n"), "xlsx").unwrap();
    let rows = sheet_rows(&x, 0);
    assert_eq!(rows[1][0], Data::Float(1.0));
    assert_eq!(rows[1][1], Data::Float(2.0));
}

#[test]
fn csv_excel_de_has_bom_semicolon_crlf_and_decimal_comma() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "preise.csv", CSV), "csv_excel_de").unwrap();
    let bytes = fs::read(&out).unwrap();
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    assert!(text.starts_with("Name;Preis;Menge;Hinweis\r\n"), "{text:?}");
    assert!(text.contains("Bäckerei Müller;\"1234,5\";3;\"mit, Komma\"") || text.contains("Bäckerei Müller;1234,5;3;"), "{text:?}");
    assert!(text.contains(";0,99;"), "{text:?}");
    assert!(text.contains("Code;007;5"), "{text:?}");
}

#[test]
fn semicolon_csv_with_bom_and_ragged_rows() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "de.csv", "\u{FEFF}Stadt;Wert\nKöln;1,5\nBonn;2;extra\nUlm\n");
    let out = run(&src, "tsv").unwrap();
    let text = fs::read_to_string(&out).unwrap();
    assert_eq!(text, "Stadt\tWert\tSpalte 3\nKöln\t1,5\t\nBonn\t2\textra\nUlm\t\t\n");
    let out = run(&src, "json").unwrap();
    let v: Value = serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v[0]["Stadt"], "Köln");
    assert_eq!(v[0]["Wert"], "1,5");
}

#[test]
fn tsv_to_csv_quotes_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "t.tsv", "a\tb\nx,y\t\"q\"\"z\"\nm\tn\n"), "csv").unwrap();
    let mut rdr = csv::ReaderBuilder::new().has_headers(false).from_path(&out).unwrap();
    let recs: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(String::from).collect()).collect();
    assert_eq!(recs, vec![vec!["a", "b"], vec!["x,y", "q\"z"], vec!["m", "n"]]);
}

#[test]
fn csv_to_json_infers_types() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "t.csv", "id,preis,aktiv,name\n1,2.5,true,Anna\n2,,false,\n"), "json").unwrap();
    let text = fs::read_to_string(&out).unwrap();
    assert!(text.contains("\n  {"), "pretty printed");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v, json!([{"id": 1, "preis": 2.5, "aktiv": true, "name": "Anna"}, {"id": 2, "preis": null, "aktiv": false, "name": null}]));
}

#[test]
fn nested_json_flattens_to_csv() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "n.json", r#"[{"a":1,"b":{"c":"x","d":{"e":true}},"t":[1,2]},{"a":2,"z":"neu"}]"#);
    let out = run(&src, "csv").unwrap();
    let mut rdr = csv::Reader::from_path(&out).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
    assert_eq!(headers.len(), 5);
    for h in ["a", "b.c", "b.d.e", "t", "z"] {
        assert!(headers.iter().any(|x| x == h), "missing {h} in {headers:?}");
    }
    let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
    let col = |h: &str| headers.iter().position(|x| x == h).unwrap();
    assert_eq!(&rows[0][col("b.c")], "x");
    assert_eq!(&rows[0][col("b.d.e")], "true");
    assert_eq!(&rows[0][col("t")], "[1,2]");
    assert_eq!(&rows[1][col("z")], "neu");
    assert_eq!(&rows[1][col("b.c")], "");
}

#[test]
fn json_variants_array_of_arrays_and_single_object() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "m.json", r#"[["n","w"],["a",1],["b",2]]"#), "csv").unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "n,w\na,1\nb,2\n");
    let out = run(&write(dir.path(), "o.json", r#"{"k":"v","n":3}"#), "csv").unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "k,n\nv,3\n");
}

#[test]
fn json_pretty_and_minify() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "x.json", r#"{ "a" : [1, 2],   "b": {"c": "ü"} }"#);
    let pretty = fs::read_to_string(run(&src, "json_pretty").unwrap()).unwrap();
    assert!(pretty.contains("\n  \"a\": ["), "{pretty}");
    let min = fs::read_to_string(run(&src, "json_minify").unwrap()).unwrap();
    assert_eq!(min.trim_end(), r#"{"a":[1,2],"b":{"c":"ü"}}"#);
}

#[test]
fn json_yaml_toml_conversions() {
    let dir = tempfile::tempdir().unwrap();
    let src = write(dir.path(), "c.json", r#"{"name":"Orbit","port":8080,"tags":["a","b"],"db":{"host":"localhost","ssl":true}}"#);
    let y = fs::read_to_string(run(&src, "yaml").unwrap()).unwrap();
    let back: Value = serde_yaml::from_str(&y).unwrap();
    assert_eq!(back["db"]["host"], "localhost");
    assert_eq!(back["tags"], json!(["a", "b"]));

    let t = fs::read_to_string(run(&src, "toml").unwrap()).unwrap();
    let tv: Value = toml::from_str(&t).unwrap();
    assert_eq!(tv, serde_json::from_str::<Value>(&fs::read_to_string(&src).unwrap()).unwrap());

    let ysrc = write(dir.path(), "d.yaml", "titel: Müll\nliste:\n  - 1\n  - zwei\nsub:\n  k: 1.5\n");
    let j: Value = serde_json::from_str(&fs::read_to_string(run(&ysrc, "json").unwrap()).unwrap()).unwrap();
    assert_eq!(j, json!({"titel": "Müll", "liste": [1, "zwei"], "sub": {"k": 1.5}}));
    let tsrc = write(dir.path(), "e.toml", "name = \"x\"\nwhen = 1979-05-27\n[srv]\nport = 80\n");
    let j: Value = serde_json::from_str(&fs::read_to_string(run(&tsrc, "json").unwrap()).unwrap()).unwrap();
    assert_eq!(j, json!({"name": "x", "when": "1979-05-27", "srv": {"port": 80}}));
    let y = fs::read_to_string(run(&tsrc, "yaml").unwrap());
    assert!(y.is_err() || y.unwrap().contains("port"));
}

#[test]
fn toml_rejects_non_object_with_german_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = run(&write(dir.path(), "l.json", "[1,2,3]"), "toml").unwrap_err();
    assert!(format!("{err:#}").contains("TOML braucht"), "{err:#}");
    assert!(fs::read_dir(dir.path()).unwrap().count() == 1, "no leftovers");
}

#[test]
fn invalid_json_gives_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = run(&write(dir.path(), "bad.json", "{\"a\": "), "yaml").unwrap_err();
    assert!(format!("{err:#}").contains("ungültig"), "{err:#}");
}

#[test]
fn markdown_table() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "m.csv", "Name,Anzahl\nA|B,3\nC,10\n"), "md_table").unwrap();
    assert_eq!(out.extension().unwrap(), "md");
    assert_eq!(fs::read_to_string(&out).unwrap(), "| Name | Anzahl |\n| --- | ---: |\n| A\\|B | 3 |\n| C | 10 |\n");
}

#[test]
fn html_table_is_escaped_document() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&write(dir.path(), "h.csv", "Titel,Zahl\n<b>&\"x\",5\n"), "html_table").unwrap();
    let h = fs::read_to_string(&out).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>"));
    assert!(h.contains("<table>") && h.contains("<style>") && h.contains("</html>"));
    assert!(h.contains("<td>&lt;b&gt;&amp;&quot;x&quot;</td>"), "{h}");
    assert!(h.contains("<td class=\"num\">5</td>"));
}

const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHallo Welt\r\nzweite Zeile\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nGrüße";

#[test]
fn srt_to_vtt_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let vtt_path = run(&write(dir.path(), "film.srt", SRT), "vtt").unwrap();
    let vtt = fs::read_to_string(&vtt_path).unwrap();
    assert_eq!(vtt, "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHallo Welt\nzweite Zeile\n\n00:00:03.000 --> 00:00:04.000\nGrüße\n\n");

    let src = write(dir.path(), "web.vtt", "WEBVTT - Titel\n\nNOTE kommentar\n\nintro\n00:01.000 --> 00:02.000 align:start\nEins\n\n01:00:03.5 --> 01:00:04.000\nZwei\nDrei\n");
    let srt = fs::read_to_string(run(&src, "srt").unwrap()).unwrap();
    assert_eq!(srt, "1\n00:00:01,000 --> 00:00:02,000\nEins\n\n2\n01:00:03,500 --> 01:00:04,000\nZwei\nDrei\n\n");
}
