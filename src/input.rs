use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug)]
pub enum Source {
    Youtube(String),
    Local { path: PathBuf, id: String },
    Archive { path: PathBuf, id: String },
}

pub fn video_id(value: &str) -> Result<String> {
    if valid_video_id(value) {
        return Ok(value.to_owned());
    }
    let url = Url::parse(value).context("Input bukan URL YouTube atau file lokal yang valid")?;
    ensure!(
        matches!(url.scheme(), "https" | "http"),
        "Skema URL tidak didukung"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URL dengan credential tidak diterima"
    );
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    let candidate = match url.host_str().unwrap_or("") {
        "youtu.be" | "www.youtu.be" => parts.first().copied().unwrap_or("").to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            if url.path() == "/watch" {
                url.query_pairs()
                    .find(|(key, _)| key == "v")
                    .map(|(_, value)| value.into_owned())
                    .unwrap_or_default()
            } else if parts.len() == 2 && matches!(parts[0], "shorts" | "embed" | "live") {
                parts[1].to_owned()
            } else {
                String::new()
            }
        }
        _ => bail!("Host harus YouTube atau youtu.be"),
    };
    ensure!(valid_video_id(&candidate), "Video ID YouTube tidak valid");
    Ok(candidate)
}

fn valid_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

pub fn read_json(path: &Path) -> Result<Value> {
    let file = File::open(path).context("Berkas JSON tidak bisa dibaca")?;
    ensure!(
        file.metadata()?.len() <= 32 * 1024 * 1024,
        "JSON melewati batas 32 MiB"
    );
    serde_json::from_reader(file).context("JSON tidak valid")
}

pub fn source(value: &str) -> Result<Source> {
    let path = Path::new(value);
    if path.is_dir() {
        let metadata = read_json(&path.join("metadata.json"))?;
        let id = metadata["id"]
            .as_str()
            .context("Arsip tidak mempunyai video ID")?;
        ensure!(safe_id(id), "Video ID arsip tidak aman untuk nama folder");
        return Ok(Source::Archive {
            path: path.canonicalize()?,
            id: id.to_owned(),
        });
    }
    if path.is_file() {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 65536];
        loop {
            let length = file.read(&mut buffer)?;
            if length == 0 {
                break;
            }
            hasher.update(&buffer[..length]);
        }
        let hash: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let id = format!("local-{hash}");
        return Ok(Source::Local {
            path: path.canonicalize()?,
            id,
        });
    }
    Ok(Source::Youtube(video_id(value)?))
}

pub fn clean_metadata(raw: &Value, id: &str) -> Value {
    let mut result = serde_json::Map::new();
    for key in [
        "title",
        "channel",
        "channel_id",
        "uploader",
        "upload_date",
        "duration",
        "description",
        "language",
        "live_status",
        "availability",
        "chapters",
        "categories",
        "tags",
    ] {
        if let Some(value) = raw.get(key).filter(|value| !value.is_null()) {
            result.insert(key.to_owned(), value.clone());
        }
    }
    result.insert("id".into(), Value::String(id.to_owned()));
    if valid_video_id(id) {
        result.insert(
            "webpage_url".into(),
            Value::String(format!("https://www.youtube.com/watch?v={id}")),
        );
    }
    Value::Object(result)
}
