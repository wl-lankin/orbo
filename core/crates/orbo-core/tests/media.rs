use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use orbo_core::analyze::{self, MediaInfo};
use orbo_core::jobs::{convert_blocking, JobSpec, Options};
use orbo_core::tools;

/// Returns early from the test when ffmpeg or an H.264 encoder is missing.
macro_rules! need_ffmpeg {
    () => {
        if tools::ffmpeg().is_none() || tools::ffprobe().is_none() || tools::pick_encoder(&["libx264", "h264_videotoolbox", "libopenh264"]).is_none() {
            eprintln!("ffmpeg fehlt, Test übersprungen");
            return;
        }
    };
}

fn h264() -> String {
    tools::pick_encoder(&["libx264", "h264_videotoolbox", "libopenh264"]).unwrap()
}

fn make(dir: &Path, name: &str, args: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let status = Command::new(tools::ffmpeg().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin"])
        .args(args)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "Fixture {name} konnte nicht erzeugt werden");
    out
}

fn mov(dir: &Path) -> PathBuf {
    make(
        dir,
        "clip.mov",
        &[
            "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25:duration=4", "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:v", &h264(), "-b:v", "6M",
            "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
        ],
    )
}

fn webm(dir: &Path) -> PathBuf {
    make(dir, "clip.webm", &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=3", "-c:v", "libvpx-vp9", "-b:v", "500k", "-deadline", "realtime", "-cpu-used", "8"])
}

fn wav(dir: &Path, secs: u32) -> PathBuf {
    make(dir, "tone.wav", &["-f", "lavfi", "-i", &format!("sine=frequency=330:duration={secs}"), "-ac", "2", "-ar", "44100"])
}

fn gif(dir: &Path) -> PathBuf {
    make(dir, "anim.gif", &["-f", "lavfi", "-i", "testsrc2=size=320x241:rate=10:duration=2"])
}

fn big_1080p(dir: &Path, secs: u32) -> PathBuf {
    make(
        dir,
        "big.mp4",
        &[
            "-f", "lavfi", "-i", &format!("testsrc2=size=1920x1080:rate=30:duration={secs}"), "-f", "lavfi", "-i", &format!("sine=duration={secs}"), "-c:v", &h264(), "-b:v",
            "20M", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
        ],
    )
}

fn run(input: &Path, target: &str, options: Options, out: &Path) -> anyhow::Result<PathBuf> {
    let options = Options { out_dir: Some(out.to_path_buf()), ..options };
    convert_blocking(&JobSpec { input: input.to_path_buf(), target: target.into(), options }, 1, &AtomicBool::new(false), &|_| {})
}

fn ok(input: &Path, target: &str) -> (PathBuf, MediaInfo, tempfile::TempDir) {
    ok_with(input, target, Options::default())
}

fn ok_with(input: &Path, target: &str, options: Options) -> (PathBuf, MediaInfo, tempfile::TempDir) {
    let out = tempfile::tempdir().unwrap();
    let path = run(input, target, options, out.path()).unwrap_or_else(|e| panic!("{target}: {e:#}"));
    let info = analyze::probe(&path).unwrap_or_else(|| panic!("{target}: Ausgabe nicht lesbar"));
    (path, info, out)
}

fn vcodec(i: &MediaInfo) -> &str {
    i.video.as_ref().map(|v| v.codec.as_str()).unwrap_or("")
}

fn acodec(i: &MediaInfo) -> &str {
    i.audio.as_ref().map(|a| a.codec.as_str()).unwrap_or("")
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn mov_to_mp4_remuxes() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let t = Instant::now();
    let (path, info, _o) = ok(&src, "mp4");
    assert_eq!(path.extension().unwrap(), "mp4");
    assert_eq!(vcodec(&info), "h264");
    assert_eq!(acodec(&info), "aac");
    assert!(t.elapsed().as_secs_f64() < 3.0, "Remux dauerte {:?}", t.elapsed());
    assert!(close(info.duration_s, 4.0, 0.3));
}

#[test]
fn webm_to_mp4_reencodes() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (_, info, _o) = ok(&webm(d.path()), "mp4");
    assert_eq!(vcodec(&info), "h264");
    assert!(info.audio.is_none());
    assert_eq!(info.video.as_ref().unwrap().width, 640);
    assert!(close(info.duration_s, 3.0, 0.3));
}

#[test]
fn video_containers() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let (_, i, _o) = ok(&src, "mp4_hevc");
    assert_eq!(vcodec(&i), "hevc");
    let (_, i, _o) = ok(&src, "mkv");
    assert_eq!((vcodec(&i), acodec(&i)), ("h264", "aac"));
    let (_, i, _o) = ok(&src, "webm");
    assert_eq!((vcodec(&i), acodec(&i)), ("vp9", "opus"));
    assert!(close(i.duration_s, 4.0, 0.3));
    let (_, i, _o) = ok(&src, "avi");
    assert_eq!((vcodec(&i), acodec(&i)), ("mpeg4", "mp3"));
}

#[test]
fn audio_targets_from_video() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    for (target, codec) in [
        ("mp3", "mp3"),
        ("m4a", "aac"),
        ("wav", "pcm_s16le"),
        ("flac", "flac"),
        ("opus", "opus"),
        ("ogg", "vorbis"),
        ("aiff", "pcm_s16be"),
        ("wma", "wmav2"),
        ("alac", "alac"),
        ("aac", "aac"),
    ] {
        let (path, info, _o) = ok(&src, target);
        assert!(info.video.is_none(), "{target}: Video sollte fehlen");
        assert_eq!(acodec(&info), codec, "{target}");
        assert!(close(info.duration_s, 4.0, 0.4), "{target}: Dauer {}", info.duration_s);
        assert!(path.metadata().unwrap().len() > 1000);
    }
}

#[test]
fn wav_to_mp3() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (_, i, _o) = ok(&wav(d.path(), 10), "mp3");
    assert_eq!(acodec(&i), "mp3");
    assert!(close(i.duration_s, 10.0, 0.3));
}

#[test]
fn gif_poster_frames() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let (path, i, _o) = ok(&src, "gif_video");
    assert_eq!(path.extension().unwrap(), "gif");
    assert_eq!(vcodec(&i), "gif");
    assert!(i.video.as_ref().unwrap().width <= 480);

    let (path, i, _o) = ok(&src, "poster");
    assert_eq!(&std::fs::read(&path).unwrap()[..2], &[0xFF, 0xD8]);
    assert_eq!(i.video.as_ref().unwrap().width, 1280);

    let (path, _, _o) = ok(&src, "frames");
    let n = std::fs::read_dir(&path).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("Bild_")).count();
    assert!(n >= 3, "nur {n} Bilder");
}

#[test]
fn compress_and_resize() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let (path, i, _o) = ok(&src, "compress_video");
    assert_eq!(vcodec(&i), "h264");
    assert!(path.metadata().unwrap().len() < src.metadata().unwrap().len());

    let big = big_1080p(d.path(), 3);
    let (_, i, _o) = ok(&big, "p720");
    assert!(i.video.as_ref().unwrap().height <= 720);
    assert_eq!(i.video.as_ref().unwrap().height % 2, 0);
}

#[test]
fn mute_and_trim_and_rotate() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let (_, i, _o) = ok(&src, "mute");
    assert!(i.audio.is_none());
    assert_eq!(vcodec(&i), "h264");

    let (_, i, _o) = ok_with(&src, "trim_video", Options { start_s: Some(1.0), end_s: Some(3.0), ..Default::default() });
    assert!(close(i.duration_s, 2.0, 0.6), "Dauer {}", i.duration_s);

    let (_, i, _o) = ok(&src, "rotate_video");
    let v = i.video.unwrap();
    assert_eq!((v.width, v.height), (720, 1280));
}

#[test]
fn reel_is_vertical() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (_, i, _o) = ok(&mov(d.path()), "reel");
    let v = i.video.unwrap();
    assert!(v.height <= 1920);
    assert!(close(v.width as f64 / v.height as f64, 9.0 / 16.0, 0.01), "{}x{}", v.width, v.height);
}

#[test]
fn discord_stays_under_limit() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let big = big_1080p(d.path(), 20);
    assert!(big.metadata().unwrap().len() > 9_500_000);
    let (path, i, _o) = ok(&big, "discord_video");
    let size = path.metadata().unwrap().len();
    assert!(size <= 9_500_000, "{size} Bytes");
    assert!(i.video.as_ref().unwrap().height <= 720);
    assert!(close(i.duration_s, 20.0, 0.5));
}

#[test]
fn audio_tools() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let long = wav(d.path(), 40);

    let (path, i, _o) = ok(&long, "ringtone");
    assert_eq!(path.extension().unwrap(), "m4r");
    assert!(i.duration_s <= 30.5 && i.duration_s > 29.0, "Dauer {}", i.duration_s);
    assert_eq!(acodec(&i), "aac");

    let (_, i, _o) = ok(&long, "loudnorm");
    assert_eq!(i.audio.as_ref().unwrap().sample_rate, 44100);
    assert!(close(i.duration_s, 40.0, 0.5));

    let (_, i, _o) = ok(&long, "louder");
    assert!(close(i.duration_s, 40.0, 0.5));

    let (_, i, _o) = ok(&long, "mono");
    assert_eq!(i.audio.as_ref().unwrap().channels, 1);

    let (_, i, _o) = ok_with(&long, "trim_audio", Options { start_s: Some(5.0), end_s: Some(15.0), ..Default::default() });
    assert!(close(i.duration_s, 10.0, 0.3));
}

#[test]
fn faster_changes_duration() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let (_, i, _o) = ok(&mov(d.path()), "faster");
    assert!(close(i.duration_s, 4.0 / 1.5, 0.4), "Dauer {}", i.duration_s);
    assert!(i.audio.is_some());
    let (_, i, _o) = ok(&mov(d.path()), "slower");
    assert!(close(i.duration_s, 8.0, 0.5), "Dauer {}", i.duration_s);
}

#[test]
fn gif_to_video() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = gif(d.path());
    let (_, i, _o) = ok(&src, "gif_to_mp4");
    let v = i.video.as_ref().unwrap();
    assert_eq!(vcodec(&i), "h264");
    assert_eq!((v.width % 2, v.height % 2), (0, 0));
    assert!(i.audio.is_none());
    let (_, i, _o) = ok(&src, "gif_to_webm");
    assert_eq!(vcodec(&i), "vp9");
}

#[test]
fn cancel_removes_temp_file() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = make(
        d.path(),
        "long.mov",
        &["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=25:duration=120", "-c:v", &h264(), "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-an"],
    );
    let out = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let spec = JobSpec { input: src, target: "webm".into(), options: Options { out_dir: Some(out.path().to_path_buf()), ..Default::default() } };
    let res = std::thread::scope(|s| {
        let h = s.spawn(|| convert_blocking(&spec, 7, &cancel, &|_| {}));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        cancel.store(true, Ordering::Relaxed);
        h.join().unwrap()
    });
    assert!(res.is_err());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[test]
fn size_budget_never_grows_small_videos() {
    need_ffmpeg!();
    let d = tempfile::tempdir().unwrap();
    let src = mov(d.path());
    let before = std::fs::metadata(&src).unwrap().len();
    let (path, _, _o) = ok(&src, "mail_video");
    let after = std::fs::metadata(&path).unwrap().len();
    assert!(after <= before * 11 / 10, "Mail-Video wuchs von {before} auf {after} Bytes");
}
