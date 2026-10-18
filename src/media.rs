use crate::tools;
use crate::transcript::Segment;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::fs;
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Serialize)]
pub struct Frame {
    pub file: String,
    pub timestamp: f64,
}

pub fn duration(ffmpeg: &Path, media: &Path) -> Result<f64> {
    let mut args = tools::args(&["-hide_banner", "-nostdin", "-i"]);
    args.push(media.as_os_str().to_owned());
    let output = tools::raw(ffmpeg, &args, 30)?;
    let text = String::from_utf8_lossy(&output.stderr);
    let duration = text
        .split("Duration: ")
        .nth(1)
        .and_then(|part| part.split(',').next())
        .context("Durasi media lokal tidak bisa dibaca")?;
    let fields: Vec<_> = duration
        .trim()
        .split(':')
        .map(str::parse::<f64>)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(fields.len() == 3, "Format durasi media tidak valid");
    let value = fields[0] * 3600.0 + fields[1] * 60.0 + fields[2];
    ensure!(value.is_finite() && value > 0.0, "Durasi media tidak valid");
    Ok(value)
}

pub fn frames(
    ffmpeg: &Path,
    media: &Path,
    directory: &Path,
    interval: u32,
    limit: u32,
    duration: f64,
) -> Result<(Vec<Frame>, u32)> {
    fs::create_dir_all(directory)?;
    let effective = if limit > 1 {
        interval.max((duration / f64::from(limit)).ceil() as u32)
    } else {
        interval
    };
    let filter = format!(
        "setpts=PTS-STARTPTS,select='isnan(prev_selected_t)+gte(t-prev_selected_t,{effective})',scale='min(1280,iw)':-2,showinfo"
    );
    let mut args = tools::args(&["-hide_banner", "-nostdin", "-y", "-i"]);
    args.push(media.as_os_str().to_owned());
    args.extend(tools::args(&[
        "-an",
        "-vf",
        &filter,
        "-fps_mode",
        "vfr",
        "-frames:v",
        &limit.to_string(),
        "-q:v",
        "3",
    ]));
    args.push(directory.join("frame-%05d.jpg").into_os_string());
    let output = tools::raw(ffmpeg, &args, 1800)?;
    ensure!(
        output.status.success(),
        "Ekstraksi frame gagal; media harus mempunyai stream video"
    );
    let mut times = Vec::new();
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        if line.contains("Parsed_showinfo")
            && let Some(value) = line
                .split("pts_time:")
                .nth(1)
                .and_then(|part| part.split_whitespace().next())
        {
            times.push(
                value
                    .parse::<f64>()
                    .context("Timestamp frame tidak valid")?,
            );
        }
    }
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.extension().is_some_and(|extension| extension == "jpg"));
    paths.sort();
    ensure!(
        !paths.is_empty() && times.len() >= paths.len(),
        "Frame atau timestamp tidak tersedia"
    );
    let frames = paths
        .iter()
        .zip(times)
        .map(|(path, timestamp)| Frame {
            file: format!(
                "frames/{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            timestamp,
        })
        .collect();
    Ok((frames, effective))
}

pub fn transcribe(
    ffmpeg: &Path,
    media: &Path,
    work: &Path,
    options: &crate::Options,
) -> Result<Vec<Segment>> {
    let model = &options.model;
    let language = &options.asr_language;
    ensure!(
        model.is_file(),
        "Model ASR belum tersedia; tentukan berkas GGML dengan --model"
    );
    ensure!(
        language == "auto" || whisper_rs::get_lang_id(language).is_some(),
        "Bahasa ASR tidak dikenal"
    );
    let pcm = work.join("audio.f32");
    let mut args = tools::args(&["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]);
    args.push(media.as_os_str().to_owned());
    args.extend(tools::args(&[
        "-vn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-t",
        &options.max_duration.to_string(),
        "-f",
        "f32le",
    ]));
    args.push(pcm.as_os_str().to_owned());
    tools::output(ffmpeg, &args, 600)?;
    let data = fs::read(pcm)?;
    ensure!(
        !data.is_empty() && data.len() % 4 == 0,
        "Audio PCM kosong atau rusak"
    );
    let samples: Vec<_> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    ensure!(
        samples.iter().all(|sample| sample.is_finite()),
        "Audio mengandung sampel tidak valid"
    );
    ensure!(
        samples.iter().any(|sample| sample.abs() > 0.0001),
        "Audio hening; transkrip tidak dibuat"
    );
    whisper_rs::install_logging_hooks();
    let mut context_params = WhisperContextParameters::default();
    context_params.use_gpu(false);
    let context = WhisperContext::new_with_params(
        model.to_str().context("Path model bukan UTF-8")?,
        context_params,
    )?;
    let mut state = context.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(options.threads as i32);
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);
    state.full(params, &samples)?;
    let mut segments = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str()?.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        let start = segment.start_timestamp() as f64 / 100.0;
        let end = segment.end_timestamp() as f64 / 100.0;
        segments.push(Segment {
            text,
            start,
            duration: (end - start).max(0.0),
        });
    }
    crate::transcript::validate(&segments)?;
    Ok(segments)
}
