use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

fn make_folder(base: &Path) -> PathBuf {
    let f = base.join("projekt");
    fs::create_dir_all(f.join("sub/tief")).unwrap();
    fs::create_dir_all(f.join("leer")).unwrap();
    fs::create_dir_all(f.join("__MACOSX")).unwrap();
    fs::write(f.join("a.txt"), "Hallo Müll").unwrap();
    fs::write(f.join("sub/b.txt"), "bbb").unwrap();
    fs::write(f.join("sub/tief/c.bin"), vec![7u8; 5000]).unwrap();
    fs::write(f.join(".DS_Store"), "junk").unwrap();
    fs::write(f.join("sub/.DS_Store"), "junk").unwrap();
    fs::write(f.join("sub/Thumbs.db"), "junk").unwrap();
    fs::write(f.join("__MACOSX/x"), "junk").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(f.join("run.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(f.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    f
}

fn zip_names(p: &Path) -> Vec<String> {
    let mut ar = zip::ZipArchive::new(File::open(p).unwrap()).unwrap();
    let mut v: Vec<String> = (0..ar.len()).map(|i| ar.by_index(i).unwrap().name().to_string()).collect();
    v.sort();
    v
}

fn tar_names(p: &Path) -> Vec<String> {
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(p).unwrap()));
    let mut v: Vec<String> = ar.entries().unwrap().map(|e| e.unwrap().path().unwrap().to_string_lossy().trim_end_matches('/').to_string()).collect();
    v.sort();
    v
}

const EXPECTED: [&str; 7] = ["projekt", "projekt/a.txt", "projekt/leer", "projekt/sub", "projekt/sub/b.txt", "projekt/sub/tief", "projekt/sub/tief/c.bin"];

#[test]
fn zip_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("notiz.txt");
    fs::write(&f, "inhalt").unwrap();
    let out = run(&f, "zip").unwrap();
    assert_eq!(out.extension().unwrap(), "zip");
    assert_eq!(zip_names(&out), vec!["notiz.txt"]);
    let mut ar = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
    let mut s = String::new();
    ar.by_name("notiz.txt").unwrap().read_to_string(&mut s).unwrap();
    assert_eq!(s, "inhalt");
}

#[test]
fn zip_folder_skips_junk_and_keeps_mode() {
    let dir = tempfile::tempdir().unwrap();
    let f = make_folder(dir.path());
    let out = run(&f, "zip").unwrap();
    let mut names = zip_names(&out);
    #[cfg(unix)]
    {
        assert!(names.contains(&"projekt/run.sh".to_string()));
        let mut ar = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        assert_eq!(ar.by_name("projekt/run.sh").unwrap().unix_mode().unwrap() & 0o777, 0o755);
        names.retain(|n| n != "projekt/run.sh");
    }
    let names: Vec<String> = names.into_iter().map(|n| n.trim_end_matches('/').to_string()).collect();
    assert_eq!(names, EXPECTED);
}

#[test]
fn tgz_folder_and_unpack_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let f = make_folder(dir.path());
    let out = run(&f, "tar_gz").unwrap();
    let mut names = tar_names(&out);
    names.retain(|n| n != "projekt/run.sh");
    assert_eq!(names, EXPECTED);

    let moved = dir.path().join("work");
    fs::create_dir(&moved).unwrap();
    let tgz = moved.join("paket.tar.gz");
    fs::rename(&out, &tgz).unwrap();
    let dest = run(&tgz, "unpack").unwrap();
    assert!(dest.is_dir());
    assert_eq!(fs::read_to_string(dest.join("projekt/a.txt")).unwrap(), "Hallo Müll");
    assert_eq!(fs::read(dest.join("projekt/sub/tief/c.bin")).unwrap().len(), 5000);
    assert!(dest.join("projekt/leer").is_dir());
    assert!(!dest.join("projekt/.DS_Store").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dest.join("projekt/run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    }
}

#[test]
fn unpack_zip_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let f = make_folder(dir.path());
    let z = run(&f, "zip").unwrap();
    let work = dir.path().join("work");
    fs::create_dir(&work).unwrap();
    let z2 = work.join("bundle.zip");
    fs::rename(&z, &z2).unwrap();
    let dest = run(&z2, "unpack").unwrap();
    assert_eq!(dest.file_name().unwrap(), "bundle");
    assert_eq!(fs::read_to_string(dest.join("projekt/sub/b.txt")).unwrap(), "bbb");
    assert!(dest.join("projekt/leer").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dest.join("projekt/run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    }
}

#[test]
fn unpack_plain_gz() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("log.txt.gz");
    let mut enc = flate2::write::GzEncoder::new(File::create(&p).unwrap(), flate2::Compression::default());
    enc.write_all("zeile eins\nzeile zwei\n".as_bytes()).unwrap();
    enc.finish().unwrap();
    let dest = run(&p, "unpack").unwrap();
    assert_eq!(fs::read_to_string(dest.join("log.txt")).unwrap(), "zeile eins\nzeile zwei\n");
}

#[test]
fn repack_tgz_to_zip() {
    let dir = tempfile::tempdir().unwrap();
    let f = make_folder(dir.path());
    let tgz = run(&f, "tar_gz").unwrap();
    let z = run(&tgz, "repack_zip").unwrap();
    assert_eq!(z.extension().unwrap(), "zip");
    let names: Vec<String> = zip_names(&z).into_iter().map(|n| n.trim_end_matches('/').to_string()).filter(|n| n != "projekt/run.sh").collect();
    assert_eq!(names, EXPECTED);
    let mut ar = zip::ZipArchive::new(File::open(&z).unwrap()).unwrap();
    let mut s = String::new();
    ar.by_name("projekt/a.txt").unwrap().read_to_string(&mut s).unwrap();
    assert_eq!(s, "Hallo Müll");
}

#[test]
fn zip_slip_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    fs::create_dir(&work).unwrap();
    for evil in ["../evil.txt", "/abs-evil.txt", "ok/../../evil.txt"] {
        let p = work.join("boese.zip");
        let mut zw = zip::ZipWriter::new(File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        zw.start_file("gut.txt", o).unwrap();
        zw.write_all(b"gut").unwrap();
        zw.start_file(evil, o).unwrap();
        zw.write_all(b"boese").unwrap();
        zw.finish().unwrap();
        let err = run(&p, "unpack").unwrap_err();
        assert!(format!("{err:#}").contains("unsicher"), "{err:#}");
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!Path::new("/abs-evil.txt").exists());
        assert!(!work.join("boese").exists(), "partial output removed");
        fs::remove_file(&p).unwrap();
    }
}

#[test]
fn tar_slip_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    fs::create_dir(&work).unwrap();
    let p = work.join("boese.tar.gz");
    let gz = flate2::write::GzEncoder::new(File::create(&p).unwrap(), flate2::Compression::default());
    let mut tb = tar::Builder::new(gz);
    let mut h = tar::Header::new_gnu();
    h.as_old_mut().name[..13].copy_from_slice(b"../evil_t.txt");
    h.set_size(3);
    h.set_mode(0o644);
    h.set_cksum();
    tb.append(&h, &b"abc"[..]).unwrap();
    tb.into_inner().unwrap().finish().unwrap();
    let err = run(&p, "unpack").unwrap_err();
    assert!(format!("{err:#}").contains("unsicher"), "{err:#}");
    assert!(!dir.path().join("evil_t.txt").exists());
    assert!(run(&p, "repack_zip").is_err());
}

#[test]
fn unpack_here_places_entries_next_to_archive_without_overwriting() {
    use std::io::Write;
    let d = tempfile::tempdir().unwrap();
    let zip_path = d.path().join("paket.zip");
    {
        let f = std::fs::File::create(&zip_path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts: zip::write::SimpleFileOptions = Default::default();
        z.start_file("notiz.txt", opts).unwrap();
        z.write_all(b"neu").unwrap();
        z.finish().unwrap();
    }
    std::fs::write(d.path().join("notiz.txt"), b"alt").unwrap();
    let spec = orbo_core::jobs::JobSpec {
        input: zip_path,
        target: "unpack".into(),
        options: orbo_core::jobs::Options { preset: Some("Hier entpacken".into()), ..Default::default() },
    };
    let out = orbo_core::jobs::convert_blocking(&spec, 9, &std::sync::atomic::AtomicBool::new(false), &|_| {}).unwrap();
    assert_eq!(out.file_name().unwrap(), "notiz 2.txt");
    assert_eq!(std::fs::read(d.path().join("notiz.txt")).unwrap(), b"alt");
    assert_eq!(std::fs::read(&out).unwrap(), b"neu");
}
