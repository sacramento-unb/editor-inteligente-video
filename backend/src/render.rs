use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{AppState, ffmpeg};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSegment {
    pub media_id: String,
    pub source_path: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPlan {
    pub id: String,
    pub project_id: String,
    pub segments: Vec<RenderSegment>,
    pub normalize_audio: bool,
    pub output_path: String,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SourceMedia {
    pub id: String,
    pub storage_path: String,
    pub duration_ms: i64,
}

#[derive(Debug, Deserialize)]
pub struct ManualCut {
    pub media_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

pub async fn build_plan(
    state: &AppState,
    project_id: &str,
    remove_analysis_ids: &[String],
    normalize_audio: bool,
    manual_cuts: &[ManualCut],
) -> anyhow::Result<RenderPlan> {
    let sources: Vec<SourceMedia> = sqlx::query_as("SELECT m.id, m.storage_path, COALESCE(mm.duration_ms, 0) AS duration_ms FROM media_files m LEFT JOIN media_metadata mm ON mm.media_file_id = m.id WHERE m.project_id = ? ORDER BY m.order_index")
        .bind(project_id)
        .fetch_all(&state.db)
        .await?;
    if sources.is_empty() {
        bail!("projeto não possui vídeos");
    }

    let mut removals: Vec<(String, i64, i64)> = Vec::new();
    for cut in manual_cuts {
        let source = sources
            .iter()
            .find(|source| source.id == cut.media_id)
            .context("Mídia do corte não pertence ao projeto")?;
        if cut.start_ms < 0 || cut.end_ms <= cut.start_ms || cut.end_ms > source.duration_ms {
            bail!("Intervalo de corte inválido");
        }
        removals.push((cut.media_id.clone(), cut.start_ms, cut.end_ms));
    }
    for id in remove_analysis_ids {
        if let Some(item) = sqlx::query_as::<_, (String, String, i64, i64)>("SELECT id, media_file_id, start_ms, end_ms FROM analysis_results WHERE id = ? AND project_id = ? AND kind IN ('silence', 'black', 'freeze') AND end_ms > start_ms")
            .bind(id)
            .bind(project_id)
            .fetch_optional(&state.db)
            .await?
        {
            removals.push((item.1, item.2, item.3));
        } else {
            bail!("Sugestão de corte inválida ou desatualizada; analise novamente");
        }
    }

    if sources.len() > 1 && !removals.is_empty() {
        bail!(
            "Renderização com cortes disponível para projetos com um arquivo. Múltiplos arquivos aceitam concatenação sem cortes nesta versão."
        );
    }
    let unsupported: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_files m LEFT JOIN media_metadata mm ON mm.media_file_id = m.id WHERE m.project_id = ? AND (mm.video_codec IS NULL OR mm.audio_codec IS NULL OR COALESCE(mm.duration_ms, 0) <= 0)")
        .bind(project_id).fetch_one(&state.db).await?;
    if unsupported > 0 {
        bail!(
            "Renderização requer arquivos inspecionados com vídeo e áudio. Arquivos somente de áudio ou vídeo podem ser reproduzidos e analisados."
        );
    }

    let mut segments = Vec::new();
    for source in &sources {
        let mut cuts = removals
            .iter()
            .filter(|(media_id, _, _)| media_id == &source.id)
            .map(|(_, start, end)| ((*start).max(0), (*end).min(source.duration_ms)))
            .filter(|(start, end)| end > start)
            .collect::<Vec<_>>();
        cuts.sort_unstable();
        let mut cursor = 0_i64;
        for (start, end) in merge_ranges(cuts) {
            if start > cursor {
                segments.push(segment(source, cursor, start));
            }
            cursor = cursor.max(end);
        }
        if source.duration_ms > cursor {
            segments.push(segment(source, cursor, source.duration_ms));
        }
    }
    if segments.is_empty() {
        bail!("os cortes removem todo o conteúdo");
    }

    let id = uuid::Uuid::new_v4().to_string();
    let output_path = format!("projects/{project_id}/output/master-{id}.mp4");
    Ok(RenderPlan {
        id,
        project_id: project_id.into(),
        segments,
        normalize_audio,
        output_path,
        command: Vec::new(),
    })
}

pub async fn execute(state: &AppState, plan: &mut RenderPlan) -> anyhow::Result<ffprobe::Metadata> {
    let output = state.storage_root.join(&plan.output_path);
    let raw = output.with_extension("raw.mp4");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).await?;
    }
    let args = build_raw_args(state, plan, &raw).await?;
    plan.command = args.clone();
    ffmpeg::render_command(&state.ffmpeg, &args).await?;

    if plan.normalize_audio {
        let measure = ffmpeg::measure_loudnorm(&state.ffmpeg, &raw).await?;
        let final_tmp = output.with_extension("normalized.mp4");
        let audio_filter = format!(
            "loudnorm=I=-16:TP=-1.5:LRA=11:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
            measure.input_i,
            measure.input_tp,
            measure.input_lra,
            measure.input_thresh,
            measure.target_offset
        );
        let args = vec![
            "-y".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-i".into(),
            raw.to_string_lossy().into_owned(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a:0".into(),
            "-c:v".into(),
            "copy".into(),
            "-af".into(),
            audio_filter,
            "-c:a".into(),
            "aac".into(),
            "-movflags".into(),
            "+faststart".into(),
            final_tmp.to_string_lossy().into_owned(),
        ];
        plan.command.extend(args.clone());
        ffmpeg::render_command(&state.ffmpeg, &args).await?;
        fs::rename(&final_tmp, &output).await?;
        let _ = fs::remove_file(&raw).await;
    } else {
        fs::rename(&raw, &output).await?;
    }
    crate::ffprobe::probe(&state.ffprobe, &output).await
}

async fn build_raw_args(
    state: &AppState,
    plan: &RenderPlan,
    output: &Path,
) -> anyhow::Result<Vec<String>> {
    let mut has_cuts = false;
    for media_id in plan
        .segments
        .iter()
        .map(|segment| segment.media_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let duration: i64 = sqlx::query_scalar(
            "SELECT COALESCE(duration_ms, 0) FROM media_metadata WHERE media_file_id = ?",
        )
        .bind(media_id)
        .fetch_one(&state.db)
        .await?;
        let segments = plan
            .segments
            .iter()
            .filter(|segment| segment.media_id == media_id)
            .collect::<Vec<_>>();
        if segments.len() != 1 || segments[0].start_ms != 0 || segments[0].end_ms != duration {
            has_cuts = true;
        }
    }

    if !has_cuts && plan.segments.len() > 1 {
        let list_path = state
            .storage_root
            .join("projects")
            .join(&plan.project_id)
            .join("analysis")
            .join(format!("render-{}.txt", plan.id));
        let content = plan
            .segments
            .iter()
            .map(|segment| {
                format!(
                    "file '{}'\n",
                    state.storage_root.join(&segment.source_path).display()
                )
            })
            .collect::<String>();
        fs::write(&list_path, content).await?;
        return Ok(vec![
            "-y".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list_path.to_string_lossy().into_owned(),
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "medium".into(),
            "-crf".into(),
            "23".into(),
            "-c:a".into(),
            "aac".into(),
            "-movflags".into(),
            "+faststart".into(),
            output.to_string_lossy().into_owned(),
        ]);
    }

    let first = plan.segments.first().context("RenderPlan vazio")?;
    if plan
        .segments
        .iter()
        .any(|segment| segment.media_id != first.media_id)
    {
        bail!("cortes em múltiplos arquivos ainda exigem normalização prévia");
    }
    let video = plan
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "[0:v]trim=start={}:end={},setpts=PTS-STARTPTS[v{index}];",
                seconds(segment.start_ms),
                seconds(segment.end_ms)
            )
        })
        .collect::<String>();
    let audio = plan
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "[0:a]atrim=start={}:end={},asetpts=PTS-STARTPTS[a{index}];",
                seconds(segment.start_ms),
                seconds(segment.end_ms)
            )
        })
        .collect::<String>();
    let inputs = (0..plan.segments.len())
        .map(|index| format!("[v{index}][a{index}]"))
        .collect::<String>();
    let filter = format!(
        "{video}{audio}{inputs}concat=n={}:v=1:a=1[v][a]",
        plan.segments.len()
    );
    Ok(vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        state
            .storage_root
            .join(&first.source_path)
            .to_string_lossy()
            .into_owned(),
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "[v]".into(),
        "-map".into(),
        "[a]".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "medium".into(),
        "-crf".into(),
        "23".into(),
        "-c:a".into(),
        "aac".into(),
        "-movflags".into(),
        "+faststart".into(),
        output.to_string_lossy().into_owned(),
    ])
}

fn segment(source: &SourceMedia, start_ms: i64, end_ms: i64) -> RenderSegment {
    RenderSegment {
        media_id: source.id.clone(),
        source_path: source.storage_path.clone(),
        start_ms,
        end_ms,
    }
}

fn merge_ranges(mut ranges: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in ranges.drain(..) {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn seconds(value: i64) -> String {
    format!("{:.3}", value as f64 / 1000.0)
}

mod ffprobe {
    pub use crate::ffprobe::Metadata;
}

#[cfg(test)]
mod tests {
    use super::merge_ranges;

    #[test]
    fn merges_overlapping_cuts() {
        assert_eq!(
            merge_ranges(vec![(0, 100), (90, 200), (300, 400)]),
            vec![(0, 200), (300, 400)]
        );
    }
}
