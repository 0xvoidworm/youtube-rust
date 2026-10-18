use clap::Parser;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use youtube_rust::{Options, input, tools, transcript};

fn archive(root: &Path, duration: f64) -> PathBuf {
    let source = root.join("archive");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("metadata.json"),
        serde_json::to_vec(
            &json!({"id":"abcdefghijk","title":"Contoh Indonesia","duration":duration}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(
        source.join("transcript.json"),
        serde_json::to_vec(&json!([
            {"text":"Halo dunia ✓","start":0.0,"duration":1.0},
            {"text":"Ini contoh.","start":1.0,"duration":1.0}
        ]))
        .unwrap(),
    )
    .unwrap();
    source
}

fn options(source: &Path, output: &Path) -> Options {
    Options::try_parse_from([
        "youtube-rust",
        source.to_str().unwrap(),
        "--output-root",
        output.to_str().unwrap(),
        "--yt-dlp",
        "/missing-downloader",
    ])
    .unwrap()
}

fn ffmpeg() -> PathBuf {
    let default = PathBuf::from(tools::DEFAULT_FFMPEG);
    if default.is_file() {
        default
    } else {
        PathBuf::from("ffmpeg")
    }
}

fn video(root: &Path) -> PathBuf {
    let path = root.join("video with spaces.mp4");
    let status = Command::new(ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=10",
            "-t",
            "6",
            "-c:v",
            "mpeg4",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    path
}

#[test]
fn accepts_supported_youtube_urls() {
    for url in [
        "abcdefghijk",
        "https://youtu.be/abcdefghijk?t=3",
        "https://www.youtube.com/watch?v=abcdefghijk&list=unused",
        "https://m.youtube.com/shorts/abcdefghijk",
        "https://youtube.com/embed/abcdefghijk",
        "https://youtube.com/live/abcdefghijk",
    ] {
        assert_eq!(input::video_id(url).unwrap(), "abcdefghijk");
    }
}

#[test]
fn rejects_spoofed_hosts_credentials_and_invalid_ids() {
    for url in [
        "https://evilyoutube.com/watch?v=abcdefghijk",
        "https://youtube.com.evil.test/watch?v=abcdefghijk",
        "https://user:pass@youtube.com/watch?v=abcdefghijk",
        "file:///abcdefghijk",
        "https://youtube.com/watch?v=../../oops",
        "short",
    ] {
        assert!(input::video_id(url).is_err(), "{url}");
    }
}

#[test]
fn json3_retains_words_and_timestamps() {
    let data = json!({"events":[{"wWinId":1}, {"tStartMs":1250,"dDurationMs":2500,"segs":[{"utf8":"Halo "},{"utf8":"dunia ✓"}]},{"tStartMs":4000,"segs":[{"utf8":"\n"}]}]});
    let result = transcript::parse(&data).unwrap();
    assert_eq!(
        result,
        vec![transcript::Segment {
            text: "Halo dunia ✓".into(),
            start: 1.25,
            duration: 2.5
        }]
    );
    assert_eq!(transcript::timestamp(3661.9), "01:01:01");
    assert_eq!(transcript::timestamp(61.2), "01:01");
}

#[test]
fn rejects_empty_and_invalid_transcripts() {
    for data in [
        json!([]),
        json!([{"text":"a","start":-1,"duration":1}]),
        json!([{"text":"a","start":0,"duration":-1}]),
        json!([{"text":"","start":0,"duration":1}]),
        json!([{"text":"a","start":2,"duration":1},{"text":"b","start":1,"duration":1}]),
    ] {
        assert!(transcript::parse(&data).is_err());
    }
}

#[test]
fn metadata_excludes_download_urls_and_headers() {
    let raw = json!({"title":"Judul", "duration":6, "formats":[{"url":"sensitive-fixture"}],"http_headers":{"Cookie":"sensitive-fixture"},"webpage_url":"untrusted-location"});
    let clean = input::clean_metadata(&raw, "abcdefghijk");
    assert_eq!(
        clean["webpage_url"],
        "https://www.youtube.com/watch?v=abcdefghijk"
    );
    assert!(!clean.to_string().contains("sensitive-fixture"));
}

#[test]
fn caption_selection_respects_language_and_manual_preference() {
    let raw = json!({"subtitles":{"en":[],"id":[]},"automatic_captions":{"id":[]}});
    assert_eq!(
        tools::choose_caption(&raw, &["id".into(), "en".into()]),
        Some(("id".into(), false))
    );
    let raw = json!({"subtitles":{"en":[]},"automatic_captions":{"id":[]}});
    assert_eq!(
        tools::choose_caption(&raw, &["id".into(), "en".into()]),
        Some(("id".into(), true))
    );
}

#[test]
fn archive_roundtrip_is_offline_and_preserves_original() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 6.0);
    let original = fs::read(source.join("transcript.json")).unwrap();
    let result = youtube_rust::run(&options(&source, &root.path().join("out"))).unwrap();
    assert_eq!(result["segments"], 2);
    assert_eq!(result["frames"], 0);
    assert_eq!(result["visual_evidence"], false);
    let exported =
        input::read_json(&Path::new(result["output"].as_str().unwrap()).join("transcript.json"))
            .unwrap();
    assert_eq!(
        exported,
        serde_json::from_slice::<Value>(&original).unwrap()
    );
    assert_eq!(fs::read(source.join("transcript.json")).unwrap(), original);
}

#[test]
fn repeated_exports_do_not_overwrite_each_other() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 6.0);
    let args = options(&source, &root.path().join("out"));
    let a = youtube_rust::run(&args).unwrap();
    let b = youtube_rust::run(&args).unwrap();
    assert_ne!(a["output"], b["output"]);
    for result in [a, b] {
        assert!(
            Path::new(result["output"].as_str().unwrap())
                .join("manifest.json")
                .is_file()
        );
    }
}

#[test]
fn duration_limit_blocks_processing() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 7201.0);
    let output = root.path().join("out");
    assert!(youtube_rust::run(&options(&source, &output)).is_err());
    assert!(!output.exists());
}

#[test]
fn bad_transcript_leaves_no_completed_export() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 6.0);
    fs::write(source.join("transcript.json"), "[]").unwrap();
    let output = root.path().join("out");
    assert!(youtube_rust::run(&options(&source, &output)).is_err());
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn archive_id_cannot_escape_output_directory() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 6.0);
    fs::write(
        source.join("metadata.json"),
        r#"{"id":"../escape","duration":6}"#,
    )
    .unwrap();
    assert!(youtube_rust::run(&options(&source, &root.path().join("out"))).is_err());
}

#[test]
fn full_mode_extracts_real_frames_with_bounded_timestamps() {
    let root = tempfile::tempdir().unwrap();
    let source = archive(root.path(), 6.0);
    let media = video(root.path());
    let mut args = options(&source, &root.path().join("out"));
    args.mode = youtube_rust::Mode::Full;
    args.media = Some(media);
    args.ffmpeg = ffmpeg();
    args.frame_interval = 1;
    args.max_frames = 3;
    let result = youtube_rust::run(&args).unwrap();
    assert_eq!(result["frames"], 3);
    assert_eq!(result["frame_interval"], 2);
    assert_eq!(result["visual_evidence"], true);
    for (index, frame) in result["frame_index"].as_array().unwrap().iter().enumerate() {
        assert!((frame["timestamp"].as_f64().unwrap() - index as f64 * 2.0).abs() < 0.05);
        let bytes = fs::read(
            Path::new(result["output"].as_str().unwrap()).join(frame["file"].as_str().unwrap()),
        )
        .unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8]));
    }
}

#[test]
fn local_metadata_does_not_require_a_model_or_downloader() {
    let root = tempfile::tempdir().unwrap();
    let media = video(root.path());
    let mut args = options(&media, &root.path().join("out"));
    args.mode = youtube_rust::Mode::Metadata;
    args.ffmpeg = ffmpeg();
    args.model = root.path().join("missing-model");
    let result = youtube_rust::run(&args).unwrap();
    assert!(result["video_id"].as_str().unwrap().starts_with("local-"));
    assert_eq!(result["duration"], 6.0);
}

#[test]
fn no_asr_fails_without_fabricating_a_transcript() {
    let root = tempfile::tempdir().unwrap();
    let media = video(root.path());
    let mut args = options(&media, &root.path().join("out"));
    args.ffmpeg = ffmpeg();
    args.no_asr = true;
    let error = youtube_rust::run(&args).unwrap_err().to_string();
    assert!(error.contains("ASR dinonaktifkan"));
}

#[test]
fn external_errors_do_not_expose_raw_stderr() {
    let arguments = tools::args(&["/missing/sensitive-fixture"]);
    let error = tools::output(Path::new("/usr/bin/env"), &arguments, 5)
        .unwrap_err()
        .to_string();
    assert!(!error.contains("sensitive-fixture"));
}
