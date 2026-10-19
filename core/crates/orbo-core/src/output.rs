use std::cmp::Ordering;
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

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut n = String::new();
    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
        n.push(c);
        it.next();
    }
    n
}

/// Compares names the way people count: case-insensitive, and digit runs by value ("Seite 2" before "Seite 10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (na, nb) = (take_digits(&mut ai), take_digits(&mut bi));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Merge order: natural sort by file name, ties by full path.
pub fn natural_sorted(paths: &[PathBuf]) -> Vec<PathBuf> {
    let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut v = paths.to_vec();
    v.sort_by(|a, b| natural_cmp(&name(a), &name(b)).then_with(|| a.cmp(b)));
    v
}
