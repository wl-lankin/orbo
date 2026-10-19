use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Audio,
    Document,
    Data,
    Archive,
    /// 3D meshes and scenes.
    Model,
    Cad,
    Font,
    Folder,
    Other,
}

/// Detected file format. The string ids (serde names) are used in the YAML catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    // images
    Png,
    Jpeg,
    Webp,
    Avif,
    Heic,
    Gif,
    Bmp,
    Tiff,
    Ico,
    Icns,
    Tga,
    Qoi,
    Pnm,
    Svg,
    Psd,
    Raw,
    // video
    Mp4,
    Mov,
    M4v,
    Mkv,
    Webm,
    Avi,
    Wmv,
    Flv,
    Mpeg,
    Mts,
    Threegp,
    Ogv,
    // audio
    Mp3,
    M4a,
    Aac,
    Wav,
    Flac,
    Aiff,
    Ogg,
    Opus,
    Wma,
    Amr,
    Caf,
    Ac3,
    // documents
    Pdf,
    Docx,
    Doc,
    Odt,
    Rtf,
    Txt,
    Md,
    Html,
    Epub,
    Pptx,
    // data
    Csv,
    Tsv,
    Xlsx,
    Xls,
    Ods,
    Json,
    Yaml,
    Toml,
    Xml,
    Srt,
    Vtt,
    // archives
    Zip,
    SevenZ,
    Rar,
    Tar,
    Gz,
    Tgz,
    Bz2,
    Xz,
    Dmg,
    Iso,
    // cad
    Dxf,
    Dwg,
    Step,
    Iges,
    // 3d
    Stl,
    Obj,
    Ply,
    Glb,
    Gltf,
    #[serde(rename = "3mf")]
    ThreeMf,
    Fbx,
    Dae,
    // fonts
    Ttf,
    Otf,
    Woff,
    Woff2,
    // more documents / ebooks / subtitles / office
    Mobi,
    Azw3,
    Fb2,
    Cbz,
    Ass,
    Sbv,
    Ppt,
    Odp,
    // other
    Folder,
    Unknown,
}

impl Format {
    pub fn kind(self) -> Kind {
        use Format::*;
        match self {
            Png | Jpeg | Webp | Avif | Heic | Gif | Bmp | Tiff | Ico | Icns | Tga | Qoi | Pnm | Svg | Psd | Raw => Kind::Image,
            Mp4 | Mov | M4v | Mkv | Webm | Avi | Wmv | Flv | Mpeg | Mts | Threegp | Ogv => Kind::Video,
            Mp3 | M4a | Aac | Wav | Flac | Aiff | Ogg | Opus | Wma | Amr | Caf | Ac3 => Kind::Audio,
            Pdf | Docx | Doc | Odt | Rtf | Txt | Md | Html | Epub | Pptx => Kind::Document,
            Csv | Tsv | Xlsx | Xls | Ods | Json | Yaml | Toml | Xml | Srt | Vtt => Kind::Data,
            Zip | SevenZ | Rar | Tar | Gz | Tgz | Bz2 | Xz | Dmg | Iso => Kind::Archive,
            Dxf | Dwg | Step | Iges => Kind::Cad,
            Stl | Obj | Ply | Glb | Gltf | ThreeMf | Fbx | Dae => Kind::Model,
            Ttf | Otf | Woff | Woff2 => Kind::Font,
            Mobi | Azw3 | Fb2 | Cbz | Ppt | Odp => Kind::Document,
            Ass | Sbv => Kind::Data,
            Folder => Kind::Folder,
            Unknown => Kind::Other,
        }
    }

    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
    }

    pub fn from_id(id: &str) -> Option<Format> {
        serde_json::from_value(serde_json::Value::String(id.to_lowercase())).ok()
    }

    pub fn from_extension(ext: &str) -> Format {
        use Format::*;
        match ext.to_lowercase().as_str() {
            "png" | "apng" => Png,
            "jpg" | "jpeg" | "jpe" | "jfif" => Jpeg,
            "webp" => Webp,
            "avif" => Avif,
            "heic" | "heif" | "hif" => Heic,
            "gif" => Gif,
            "bmp" | "dib" => Bmp,
            "tif" | "tiff" => Tiff,
            "ico" => Ico,
            "icns" => Icns,
            "tga" => Tga,
            "qoi" => Qoi,
            "pbm" | "pgm" | "ppm" | "pnm" => Pnm,
            "svg" => Svg,
            "psd" => Psd,
            "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" => Raw,
            "mp4" => Mp4,
            "mov" | "qt" => Mov,
            "m4v" => M4v,
            "mkv" => Mkv,
            "webm" => Webm,
            "avi" => Avi,
            "wmv" | "asf" => Wmv,
            "flv" => Flv,
            "mpg" | "mpeg" | "vob" => Mpeg,
            "mts" | "m2ts" | "ts" => Mts,
            "3gp" | "3g2" => Threegp,
            "ogv" => Ogv,
            "mp3" => Mp3,
            "m4a" | "m4b" | "m4r" => M4a,
            "aac" => Aac,
            "wav" | "wave" => Wav,
            "flac" => Flac,
            "aif" | "aiff" | "aifc" => Aiff,
            "ogg" | "oga" => Ogg,
            "opus" => Opus,
            "wma" => Wma,
            "amr" => Amr,
            "caf" => Caf,
            "ac3" => Ac3,
            "pdf" => Pdf,
            "docx" => Docx,
            "doc" => Doc,
            "odt" => Odt,
            "rtf" => Rtf,
            "txt" | "text" | "log" => Txt,
            "md" | "markdown" => Md,
            "html" | "htm" => Html,
            "epub" => Epub,
            "pptx" => Pptx,
            "csv" => Csv,
            "tsv" | "tab" => Tsv,
            "xlsx" | "xlsm" => Xlsx,
            "xls" => Xls,
            "ods" => Ods,
            "json" | "geojson" => Json,
            "yaml" | "yml" => Yaml,
            "toml" => Toml,
            "xml" => Xml,
            "srt" => Srt,
            "vtt" => Vtt,
            "zip" => Zip,
            "7z" => SevenZ,
            "rar" => Rar,
            "tar" => Tar,
            "gz" => Gz,
            "tgz" => Tgz,
            "bz2" => Bz2,
            "xz" => Xz,
            "dmg" => Dmg,
            "iso" => Iso,
            "dxf" => Dxf,
            "dwg" => Dwg,
            "step" | "stp" => Step,
            "iges" | "igs" => Iges,
            "stl" => Stl,
            "obj" => Obj,
            "ply" => Ply,
            "glb" => Glb,
            "gltf" => Gltf,
            "3mf" => ThreeMf,
            "fbx" => Fbx,
            "dae" => Dae,
            "ttf" => Ttf,
            "otf" => Otf,
            "woff" => Woff,
            "woff2" => Woff2,
            "mobi" | "prc" => Mobi,
            "azw3" | "azw" => Azw3,
            "fb2" => Fb2,
            "cbz" => Cbz,
            "ass" | "ssa" => Ass,
            "sbv" => Sbv,
            "ppt" => Ppt,
            "odp" => Odp,
            _ => Unknown,
        }
    }

    /// Preferred file extension for outputs of this format.
    pub fn extension(self) -> &'static str {
        use Format::*;
        match self {
            Png => "png",
            Jpeg => "jpg",
            Webp => "webp",
            Avif => "avif",
            Heic => "heic",
            Gif => "gif",
            Bmp => "bmp",
            Tiff => "tiff",
            Ico => "ico",
            Icns => "icns",
            Tga => "tga",
            Qoi => "qoi",
            Pnm => "ppm",
            Svg => "svg",
            Psd => "psd",
            Raw => "dng",
            Mp4 => "mp4",
            Mov => "mov",
            M4v => "m4v",
            Mkv => "mkv",
            Webm => "webm",
            Avi => "avi",
            Wmv => "wmv",
            Flv => "flv",
            Mpeg => "mpg",
            Mts => "ts",
            Threegp => "3gp",
            Ogv => "ogv",
            Mp3 => "mp3",
            M4a => "m4a",
            Aac => "aac",
            Wav => "wav",
            Flac => "flac",
            Aiff => "aiff",
            Ogg => "ogg",
            Opus => "opus",
            Wma => "wma",
            Amr => "amr",
            Caf => "caf",
            Ac3 => "ac3",
            Pdf => "pdf",
            Docx => "docx",
            Doc => "doc",
            Odt => "odt",
            Rtf => "rtf",
            Txt => "txt",
            Md => "md",
            Html => "html",
            Epub => "epub",
            Pptx => "pptx",
            Csv => "csv",
            Tsv => "tsv",
            Xlsx => "xlsx",
            Xls => "xls",
            Ods => "ods",
            Json => "json",
            Yaml => "yaml",
            Toml => "toml",
            Xml => "xml",
            Srt => "srt",
            Vtt => "vtt",
            Zip => "zip",
            SevenZ => "7z",
            Rar => "rar",
            Tar => "tar",
            Gz => "gz",
            Tgz => "tar.gz",
            Bz2 => "bz2",
            Xz => "xz",
            Dmg => "dmg",
            Iso => "iso",
            Dxf => "dxf",
            Dwg => "dwg",
            Step => "step",
            Iges => "igs",
            Stl => "stl",
            Obj => "obj",
            Ply => "ply",
            Glb => "glb",
            Gltf => "gltf",
            ThreeMf => "3mf",
            Fbx => "fbx",
            Dae => "dae",
            Ttf => "ttf",
            Otf => "otf",
            Woff => "woff",
            Woff2 => "woff2",
            Mobi => "mobi",
            Azw3 => "azw3",
            Fb2 => "fb2",
            Cbz => "cbz",
            Ass => "ass",
            Sbv => "sbv",
            Ppt => "ppt",
            Odp => "odp",
            Folder => "",
            Unknown => "",
        }
    }
}

/// Maps a MIME type reported by `infer` to a format.
fn from_mime(mime: &str, ext_guess: Format) -> Option<Format> {
    use Format::*;
    Some(match mime {
        "image/png" => Png,
        "image/jpeg" => Jpeg,
        "image/webp" => Webp,
        "image/avif" => Avif,
        "image/heif" | "image/heic" | "image/heif-sequence" | "image/heic-sequence" => Heic,
        "image/gif" => Gif,
        "image/bmp" => Bmp,
        "image/tiff" => {
            // DNG and several camera RAW formats are TIFF containers.
            if ext_guess == Raw {
                Raw
            } else {
                Tiff
            }
        }
        "image/vnd.microsoft.icon" | "image/x-icon" => Ico,
        "image/x-icns" => Icns,
        "image/vnd.adobe.photoshop" => Psd,
        "image/x-canon-cr2" | "image/x-canon-cr3" | "image/x-nikon-nef" | "image/x-sony-arw" | "image/x-fujifilm-raf"
        | "image/x-olympus-orf" | "image/x-panasonic-rw2" => Raw,
        "video/mp4" => {
            if ext_guess == M4v {
                M4v
            } else {
                Mp4
            }
        }
        "video/quicktime" => Mov,
        "video/x-m4v" => M4v,
        "video/x-matroska" => Mkv,
        "video/webm" => Webm,
        "video/x-msvideo" => Avi,
        "video/x-ms-wmv" | "video/x-ms-asf" => {
            if ext_guess == Wma {
                Wma
            } else {
                Wmv
            }
        }
        "video/x-flv" => Flv,
        "video/mpeg" => Mpeg,
        "video/mp2t" => Mts,
        "video/3gpp" | "video/3gpp2" => Threegp,
        "audio/mpeg" => Mp3,
        "audio/m4a" | "audio/x-m4a" | "audio/mp4" => M4a,
        "audio/aac" => Aac,
        "audio/x-wav" | "audio/wav" => Wav,
        "audio/x-flac" | "audio/flac" => Flac,
        "audio/x-aiff" | "audio/aiff" => Aiff,
        "audio/ogg" => match ext_guess {
            Opus => Opus,
            Ogv => Ogv,
            _ => Ogg,
        },
        "audio/opus" => Opus,
        "audio/amr" => Amr,
        "audio/x-caf" => Caf,
        "application/pdf" => Pdf,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Docx,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Xlsx,
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Pptx,
        "application/vnd.oasis.opendocument.text" => Odt,
        "application/vnd.oasis.opendocument.spreadsheet" => Ods,
        "application/epub+zip" => Epub,
        "application/msword" => Doc,
        "application/vnd.ms-excel" => Xls,
        "application/rtf" | "text/rtf" => Rtf,
        "application/zip" => match ext_guess {
            // infer reports some zip-based containers only as zip.
            Docx | Xlsx | Pptx | Odt | Ods | Odp | Epub | ThreeMf | Cbz => ext_guess,
            _ => Zip,
        },
        "application/x-7z-compressed" => SevenZ,
        "application/vnd.rar" | "application/x-rar-compressed" => Rar,
        "application/x-tar" => Tar,
        "application/gzip" => {
            if ext_guess == Tgz {
                Tgz
            } else {
                Gz
            }
        }
        "application/x-bzip2" => Bz2,
        "application/x-xz" => Xz,
        "application/x-apple-diskimage" => Dmg,
        "application/x-iso9660-image" => Iso,
        "application/font-woff" | "font/woff" => Woff,
        "font/woff2" | "application/font-woff2" => Woff2,
        "application/font-sfnt" | "font/ttf" | "application/x-font-ttf" => {
            if ext_guess == Otf {
                Otf
            } else {
                Ttf
            }
        }
        "font/otf" | "application/x-font-otf" => Otf,
        "application/x-mobipocket-ebook" => Mobi,
        "application/vnd.ms-powerpoint" => Ppt,
        "application/vnd.oasis.opendocument.presentation" => Odp,
        "model/gltf-binary" => Glb,
        "image/vnd.dwg" | "application/acad" => Dwg,
        "text/html" => Html,
        "text/xml" | "application/xml" => Xml,
        _ => return None,
    })
}

fn guess_from_name(path: &Path) -> Format {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".tar.gz") {
        return Format::Tgz;
    }
    path.extension().map(|e| Format::from_extension(&e.to_string_lossy())).unwrap_or(Format::Unknown)
}

/// Detects the format from file content first and falls back to the extension for text formats.
/// Returns the format and whether the extension disagrees with the content.
pub fn detect(path: &Path) -> std::io::Result<(Format, bool)> {
    if path.is_dir() {
        return Ok((Format::Folder, false));
    }
    let by_name = guess_from_name(path);
    let mut head = vec![0u8; 8192];
    // Protected folders (Desktop, Documents, Downloads) can deny reading or return nothing until the
    // user grants access. Then the extension is the best guess; the content must not be read as text.
    let n = match std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) {
        Ok(n) => n,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Ok((by_name, false)),
        Err(e) => return Err(e),
    };
    if n == 0 {
        return Ok((by_name, false));
    }
    head.truncate(n);

    if let Some(t) = infer::get(&head) {
        if let Some(mut f) = from_mime(t.mime_type(), by_name) {
            // infer reports SVG files with an XML declaration as plain XML.
            if f == Format::Xml && String::from_utf8_lossy(&head).contains("<svg") {
                f = Format::Svg;
            }
            if f == Format::Xml && (by_name == Format::Fb2 || String::from_utf8_lossy(&head).contains("<FictionBook")) {
                f = Format::Fb2;
            } else if f == Format::Xml && (by_name == Format::Dae || String::from_utf8_lossy(&head).contains("<COLLADA")) {
                f = Format::Dae;
            }
            let mismatch = by_name != Format::Unknown && by_name != f && !same_family(by_name, f);
            return Ok((f, mismatch));
        }
    }
    if let Some(f) = sniff_text(&head, by_name) {
        return Ok((f, by_name != Format::Unknown && by_name != f && !same_family(by_name, f)));
    }
    Ok((by_name, false))
}

/// Containers that legitimately share a signature with another format.
fn same_family(a: Format, b: Format) -> bool {
    use Format::*;
    let fam = |f: Format| match f {
        Mp4 | M4v | Mov | M4a | Threegp => 1,
        Ogg | Opus | Ogv => 2,
        Tiff | Raw => 3,
        Txt | Md | Csv | Tsv | Json | Yaml | Toml | Srt | Vtt | Xml | Fb2 | Html | Svg | Dxf | Obj | Stl | Ply | Gltf | Ass | Sbv | Step | Iges | Dae => 4,
        Gz | Tgz => 5,
        _ => 0,
    };
    fam(a) != 0 && fam(a) == fam(b)
}

fn sniff_text(head: &[u8], by_name: Format) -> Option<Format> {
    let text = std::str::from_utf8(head).ok().or_else(|| {
        // A multi-byte UTF-8 character may be cut at the buffer end.
        let cut = head.len().saturating_sub(4);
        std::str::from_utf8(&head[..cut]).ok()
    })?;
    if text.contains('\0') {
        return None;
    }
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg")) {
        return Some(Format::Svg);
    }
    if t.starts_with("<!DOCTYPE html") || t.starts_with("<html") || t.starts_with("<!doctype html") {
        return Some(Format::Html);
    }
    if t.starts_with("<?xml") {
        if by_name == Format::Fb2 || text.contains("<FictionBook") {
            return Some(Format::Fb2);
        }
        if by_name == Format::Dae || text.contains("<COLLADA") {
            return Some(Format::Dae);
        }
        return Some(Format::Xml);
    }
    if t.starts_with("WEBVTT") {
        return Some(Format::Vtt);
    }
    if is_srt(t) {
        return Some(Format::Srt);
    }
    // Text formats without a signature: trust the extension when it names a text format.
    if matches!(by_name, Format::Dxf | Format::Obj | Format::Stl | Format::Ply | Format::Gltf | Format::Ass | Format::Sbv | Format::Step | Format::Iges | Format::Dae) {
        return Some(by_name);
    }
    if matches!(by_name, Format::Json | Format::Yaml | Format::Toml | Format::Csv | Format::Tsv | Format::Md | Format::Txt | Format::Xml) {
        return Some(by_name);
    }
    if (t.starts_with('{') || t.starts_with('[')) && serde_json::from_str::<serde_json::Value>(t).is_ok() {
        return Some(Format::Json);
    }
    if let Some(d) = sniff_delimiter(t) {
        return Some(if d == '\t' { Format::Tsv } else { Format::Csv });
    }
    Some(Format::Txt)
}

fn is_srt(t: &str) -> bool {
    let mut lines = t.lines();
    let first = lines.next().unwrap_or("").trim();
    let second = lines.next().unwrap_or("");
    first.parse::<u32>().is_ok() && second.contains("-->")
}

/// Finds a delimiter that appears the same number of times (≥1) on the first lines.
pub fn sniff_delimiter(text: &str) -> Option<char> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(6).collect();
    if lines.len() < 2 {
        return None;
    }
    for d in [';', ',', '\t', '|'] {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(d).count()).collect();
        if counts[0] >= 1 && counts.iter().all(|&c| c == counts[0]) {
            return Some(d);
        }
    }
    None
}
