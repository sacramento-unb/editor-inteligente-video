use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    r_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<i64>,
    bit_rate: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub duration_ms: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fps_num: Option<i64>,
    pub fps_den: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bitrate: Option<i64>,
    pub raw_json: String,
}

pub async fn probe(binary: &str, input: &Path) -> anyhow::Result<Metadata> {
    let output = Command::new(binary)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(input)
        .output()
        .await
        .with_context(|| format!("executar {binary}"))?;
    if !output.status.success() {
        bail!(
            "ffprobe falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw_json = String::from_utf8(output.stdout).context("ffprobe devolveu JSON inválido")?;
    let parsed: ProbeOutput =
        serde_json::from_str(&raw_json).context("interpretar resposta do ffprobe")?;
    let video = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let (fps_num, fps_den) = video
        .and_then(|stream| stream.r_frame_rate.as_deref())
        .and_then(parse_ratio)
        .unwrap_or((0, 1));
    let duration = parsed
        .format
        .duration
        .as_deref()
        .and_then(parse_seconds)
        .unwrap_or_default();
    let bitrate = parsed
        .format
        .bit_rate
        .as_deref()
        .or_else(|| video.and_then(|stream| stream.bit_rate.as_deref()))
        .and_then(|value| value.parse().ok());

    Ok(Metadata {
        duration_ms: (duration * 1000.0).round() as i64,
        width: video.and_then(|stream| stream.width),
        height: video.and_then(|stream| stream.height),
        fps_num: Some(fps_num),
        fps_den: Some(fps_den),
        video_codec: video.and_then(|stream| stream.codec_name.clone()),
        audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        sample_rate: audio
            .and_then(|stream| stream.sample_rate.as_deref())
            .and_then(|value| value.parse().ok()),
        channels: audio.and_then(|stream| stream.channels),
        bitrate,
        raw_json,
    })
}

fn parse_seconds(value: &str) -> Option<f64> {
    value.parse().ok()
}

fn parse_ratio(value: &str) -> Option<(i64, i64)> {
    let (numerator, denominator) = value.split_once('/')?;
    Some((numerator.parse().ok()?, denominator.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::{parse_ratio, parse_seconds};

    #[test]
    fn parses_probe_numbers() {
        assert_eq!(parse_ratio("30000/1001"), Some((30000, 1001)));
        assert_eq!(parse_ratio("broken"), None);
        assert_eq!(parse_seconds("12.5"), Some(12.5));
    }
}
