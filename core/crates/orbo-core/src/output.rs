use std::path::{Path, PathBuf};

/// Next free path in `dir` for `stem` + `ext`, appending " 2", " 3", ... when taken. Never returns an existing path.
pub fn next_free(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let make = |s: &str| if ext.is_empty() { dir.join(s) } else { dir.join(format!("{s}.{ext}")) };
    let mut candidate = make(stem);
    let mut n = 2;
    while candidate.exists() {
        candidate = make(&format!("{stem} {n}"));
        n += 1;
    }
    candidate
}

/// Temp path in the same directory so the final rename stays on one filesystem and is atomic.
pub fn temp_for(final_path: &Path, job_id: u64) -> PathBuf {
    let dir = final_path.parent().unwrap_or(Path::new("."));
    let ext = final_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    dir.join(format!(".orbo-{job_id}-{}{ext}", std::process::id()))
}

/// Moves the finished temp file into place without ever overwriting an existing file.
pub fn commit(temp: &Path, wanted: &Path) -> std::io::Result<PathBuf> {
    let dir = wanted.parent().unwrap_or(Path::new("."));
    let stem = wanted.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = wanted.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let target = if wanted.exists() { next_free(dir, &stem, &ext) } else { wanted.to_path_buf() };
    std::fs::rename(temp, &target)?;
    Ok(target)
}

pub fn stem_of(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    if lower.ends_with(".tar.gz") {
        return name[..name.len() - 7].to_string();
    }
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name)
}
