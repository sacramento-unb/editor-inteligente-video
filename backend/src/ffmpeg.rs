use std::{path::Path, process::Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisItem {
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub suggestion: String,
    pub evidence_json: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioStats {
    pub mean_volume_db: Option<f64>,
    pub max_volume_db: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AnalysisReport {
    pub items: Vec<AnalysisItem>,
    pub audio: AudioStats,
}

pub async fn analyze_file(
    binary: &str,
    input: &Path,
    has_audio: bool,
    has_video: bool,
) -> anyhow::Result<AnalysisReport> {
    let mut report = AnalysisReport::default();
    if has_audio {
        let silence = run_filter(binary, input, &["-af", "silencedetect=noise=-35dB:d=2"])
            .await
            .context("analisar silêncio")?;
        report.items.extend(parse_intervals(
            &silence,
            "silence",
            "silence_start:",
            "silence_end:",
            0.93,
        ));

        let volume = run_filter(binary, input, &["-af", "volumedetect"])
            .await
            .context("analisar volume")?;
        report.audio = parse_volume(&volume);
        if let Some(mean) = report.audio.mean_volume_db {
            if mean < -24.0 {
                report.items.push(AnalysisItem {
                    kind: "low_volume".into(),
                    start_ms: 0,
                    end_ms: 0,
                    confidence: 0.88,
                    suggestion: "normalize_audio".into(),
                    evidence_json: serde_json::json!({"mean_volume_db": mean}).to_string(),
                });
            }
        }
    }
    if has_video {
        let black = run_filter(binary, input, &["-vf", "blackdetect=d=1:pix_th=0.10"])
            .await
            .context("analisar frames pretos")?;
        report.items.extend(parse_intervals(
            &black,
            "black",
            "black_start:",
            "black_end:",
            0.90,
        ));

        let frozen = run_filter(binary, input, &["-vf", "freezedetect=n=-60dB:d=2"])
            .await
            .context("analisar frames congelados")?;
        report.items.extend(parse_intervals(
            &frozen,
            "freeze",
            "freeze_start:",
            "freeze_end:",
            0.84,
        ));
    }
    Ok(report)
}

async fn run_filter(binary: &str, input: &Path, filter: &[&str]) -> anyhow::Result<String> {
    let mut command = Command::new(binary);
    command
        .arg("-hide_banner")
        .arg("-nostats")
        .arg("-i")
        .arg(input);
    for argument in filter {
        command.arg(argument);
    }
    let output = command
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("executar {binary}"))?;
    if !output.status.success() {
        bail!(
            "ffmpeg falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

pub async fn render_command(binary: &str, args: &[String]) -> anyhow::Result<String> {
    let output = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("executar {binary}"))?;
    if !output.status.success() {
        bail!(
            "ffmpeg falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stderr).into_owned())
}

pub async fn measure_loudnorm(binary: &str, input: &Path) -> anyhow::Result<LoudnormMeasure> {
    let output = Command::new(binary)
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args([
            "-af",
            "loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json",
            "-f",
            "null",
            "-",
        ])
        .output()
        .await
        .with_context(|| format!("medir loudnorm com {binary}"))?;
    if !output.status.success() {
        bail!(
            "medição loudnorm falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let start = stderr.rfind('{').context("JSON loudnorm não encontrado")?;
    let end = stderr
        .rfind('}')
        .context("fim do JSON loudnorm não encontrado")?
        + 1;
    Ok(serde_json::from_str(&stderr[start..end]).context("interpretar medição loudnorm")?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoudnormMeasure {
    pub input_i: String,
    pub input_tp: String,
    pub input_lra: String,
    pub input_thresh: String,
    pub target_offset: String,
}

fn parse_intervals(
    log: &str,
    kind: &str,
    start_marker: &str,
    end_marker: &str,
    confidence: f64,
) -> Vec<AnalysisItem> {
    let mut result = Vec::new();
    let mut start = None;
    for line in log.lines() {
        if let Some(value) = value_after(line, start_marker) {
            start = Some(value);
        }
        if let Some(end) = value_after(line, end_marker) {
            if let Some(begin) = start.take() {
                if end > begin {
                    result.push(AnalysisItem {
                        kind: kind.into(),
                        start_ms: (begin * 1000.0).round() as i64,
                        end_ms: (end * 1000.0).round() as i64,
                        confidence,
                        suggestion: "review_remove".into(),
                        evidence_json: serde_json::json!({"start": begin, "end": end}).to_string(),
                    });
                }
            }
        }
    }
    result
}

fn parse_volume(log: &str) -> AudioStats {
    let mut stats = AudioStats::default();
    for line in log.lines() {
        if let Some(value) = value_after(line, "mean_volume:") {
            stats.mean_volume_db = Some(value);
        }
        if let Some(value) = value_after(line, "max_volume:") {
            stats.max_volume_db = Some(value);
        }
    }
    stats
}

fn value_after(line: &str, marker: &str) -> Option<f64> {
    let value = line
        .split_once(marker)?
        .1
        .trim()
        .split_whitespace()
        .next()?;
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{parse_intervals, parse_volume};

    #[test]
    fn parses_ffmpeg_detections() {
        let result = parse_intervals(
            "silence_start: 1.2\nsilence_end: 4.5",
            "silence",
            "silence_start:",
            "silence_end:",
            0.9,
        );
        assert_eq!(result[0].start_ms, 1200);
        assert_eq!(result[0].end_ms, 4500);
    }

    #[test]
    fn parses_volume_summary() {
        let result = parse_volume("mean_volume: -25.0 dB\nmax_volume: -2.0 dB");
        assert_eq!(result.mean_volume_db, Some(-25.0));
        assert_eq!(result.max_volume_db, Some(-2.0));
    }
}
