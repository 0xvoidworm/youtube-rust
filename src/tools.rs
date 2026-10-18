use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub const DEFAULT_YTDLP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/.tools/bin/yt-dlp");
pub const DEFAULT_FFMPEG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/.tools/bin/ffmpeg");

pub fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

pub fn raw(tool: &Path, arguments: &[OsString], seconds: u32) -> Result<Output> {
    Command::new("timeout")
        .args(["--kill-after=5s", &format!("{seconds}s")])
        .arg(tool)
        .args(arguments)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .context("Tidak dapat menjalankan tool eksternal; periksa --self-test")
}

pub fn output(tool: &Path, arguments: &[OsString], seconds: u32) -> Result<Vec<u8>> {
    let output = raw(tool, arguments, seconds)?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).to_lowercase();
        let reason = if output.status.code() == Some(124) || output.status.code() == Some(137) {
            "batas waktu terlampaui"
        } else if error.contains("sign in") || error.contains("confirm you're not a bot") {
            "YouTube meminta verifikasi akun/bot"
        } else if error.contains("429") {
            "YouTube membatasi jumlah permintaan (429)"
        } else if error.contains("403") {
            "server menolak akses (403)"
        } else {
            "proses gagal; periksa input dan instalasi tool"
        };
        // stderr can contain signed media URLs; never expose it.
        bail!(
            "{}: {reason} ({})",
            tool.file_name().unwrap_or_default().to_string_lossy(),
            output.status
        );
    }
    Ok(output.stdout)
}

pub fn yt_args() -> Vec<OsString> {
    args(&[
        "--ignore-config",
        "--no-playlist",
        "--no-progress",
        "--no-warnings",
        "--no-cache-dir",
        "--socket-timeout",
        "20",
        "--retries",
        "1",
        "--extractor-retries",
        "1",
        "--js-runtimes",
        "node",
        "--no-remote-components",
    ])
}

pub fn metadata(tool: &Path, url: &str) -> Result<Value> {
    let mut arguments = yt_args();
    arguments.extend(args(&["--skip-download", "--dump-single-json", "--", url]));
    serde_json::from_slice(&output(tool, &arguments, 180)?).context("Metadata yt-dlp bukan JSON")
}

pub fn download(
    tool: &Path,
    ffmpeg: &Path,
    url: &str,
    directory: &Path,
    video: bool,
) -> Result<PathBuf> {
    let mut arguments = yt_args();
    arguments.extend(args(&["--no-overwrites", "--ffmpeg-location"]));
    arguments.push(ffmpeg.as_os_str().to_owned());
    arguments.extend(args(&[
        "--format",
        if video {
            "bv*[height<=720]+ba/b[height<=720]/b"
        } else {
            "bestaudio/best"
        },
        "--print",
        "after_move:filepath",
        "--output",
    ]));
    arguments.push(directory.join("media.%(ext)s").into_os_string());
    arguments.extend(args(&["--", url]));
    let text = String::from_utf8(output(tool, &arguments, 1800)?)
        .context("Nama file media bukan UTF-8")?;
    let path = PathBuf::from(
        text.lines()
            .last()
            .context("Lokasi unduhan tidak tersedia")?,
    )
    .canonicalize()
    .context("Unduhan media tidak ditemukan")?;
    if !path.starts_with(directory.canonicalize()?) {
        bail!("Lokasi unduhan di luar folder kerja");
    }
    Ok(path)
}

pub fn choose_caption(raw: &Value, languages: &[String]) -> Option<(String, bool)> {
    for preferred in languages {
        for (key, automatic) in [("subtitles", false), ("automatic_captions", true)] {
            if let Some(tracks) = raw[key].as_object() {
                let selected = tracks
                    .keys()
                    .find(|language| *language == preferred)
                    .or_else(|| {
                        tracks
                            .keys()
                            .find(|language| language.starts_with(&format!("{preferred}-")))
                    });
                if let Some(language) = selected.filter(|language| {
                    language
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                }) {
                    return Some((language.clone(), automatic));
                }
            }
        }
    }
    None
}

pub fn caption(
    tool: &Path,
    url: &str,
    directory: &Path,
    language: &str,
    automatic: bool,
) -> Result<Value> {
    let mut arguments = yt_args();
    arguments.extend(args(&[
        "--skip-download",
        if automatic {
            "--write-auto-subs"
        } else {
            "--write-subs"
        },
        "--sub-format",
        "json3",
        "--sub-langs",
        &format!("^{language}$"),
        "--output",
    ]));
    arguments.push(directory.join("captions.%(ext)s").into_os_string());
    arguments.extend(args(&["--", url]));
    output(tool, &arguments, 180)?;
    crate::input::read_json(&directory.join(format!("captions.{language}.json3")))
}
