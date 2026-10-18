use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Segment {
    pub text: String,
    pub start: f64,
    pub duration: f64,
}

pub fn parse(value: &Value) -> Result<Vec<Segment>> {
    let segments = if value.is_array() {
        serde_json::from_value(value.clone()).context("Format transkrip arsip tidak valid")?
    } else {
        let events = value["events"]
            .as_array()
            .context("Subtitle harus berupa JSON3 atau array transkrip")?;
        let mut result = Vec::new();
        for event in events {
            let Some(parts) = event["segs"].as_array() else {
                continue;
            };
            let text: String = parts
                .iter()
                .filter_map(|part| part["utf8"].as_str())
                .collect();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                continue;
            }
            let start = event["tStartMs"]
                .as_f64()
                .context("Timestamp subtitle tidak tersedia")?
                / 1000.0;
            let duration = event
                .get("dDurationMs")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                / 1000.0;
            result.push(Segment {
                text,
                start,
                duration,
            });
        }
        result
    };
    validate(&segments)?;
    Ok(segments)
}

pub fn validate(segments: &[Segment]) -> Result<()> {
    ensure!(!segments.is_empty(), "Transkrip kosong");
    ensure!(segments.len() <= 100_000, "Segmen transkrip terlalu banyak");
    let mut previous = 0.0;
    for segment in segments {
        ensure!(!segment.text.trim().is_empty(), "Segmen transkrip kosong");
        ensure!(
            segment.start.is_finite()
                && segment.start >= previous
                && segment.duration.is_finite()
                && segment.duration >= 0.0,
            "Timestamp transkrip tidak valid atau tidak berurutan"
        );
        previous = segment.start;
    }
    Ok(())
}

pub fn timestamp(seconds: f64) -> String {
    let seconds = seconds.max(0.0).floor() as u64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let remainder = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{remainder:02}")
    } else {
        format!("{minutes:02}:{remainder:02}")
    }
}

pub fn markdown(metadata: &Value, segments: &[Segment], source: &str) -> String {
    let title = metadata["title"]
        .as_str()
        .unwrap_or("Video")
        .replace(['\r', '\n'], " ");
    let mut text = format!(
        "# {title}\n\n- Sumber: {}\n- Transkrip: {source}\n\n## Transkrip\n\n",
        metadata["webpage_url"].as_str().unwrap_or("media lokal")
    );
    for segment in segments {
        text.push_str(&format!(
            "[{}] {}\n",
            timestamp(segment.start),
            segment.text.replace(['\r', '\n'], " ")
        ));
    }
    text
}
