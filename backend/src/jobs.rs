use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{AppState, ffmpeg, ffprobe, render};

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct Job {
    pub id: String,
    pub project_id: String,
    #[serde(rename = "type")]
    pub job_type: String,
    pub status: String,
    pub progress: f64,
    pub current_step: Option<String>,
    pub input_json: String,
    pub result_json: Option<String>,
    pub error: Option<String>,
    pub log_path: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

pub async fn create(
    pool: &SqlitePool,
    project_id: &str,
    job_type: &str,
    input: Value,
) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO jobs (id, project_id, type, input_json) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(project_id)
        .bind(job_type)
        .bind(serde_json::to_string(&input)?)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn get(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<Job>> {
    Ok(sqlx::query_as::<_, Job>("SELECT id, project_id, type AS job_type, status, progress, current_step, input_json, result_json, error, log_path, created_at, started_at, finished_at FROM jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn resume_queued(pool: &SqlitePool, tx: &mpsc::Sender<String>) -> anyhow::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'QUEUED', started_at = NULL WHERE status = 'RUNNING'")
        .execute(pool)
        .await?;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM jobs WHERE status = 'QUEUED' ORDER BY created_at")
            .fetch_all(pool)
            .await?;
    for id in ids {
        tx.send(id).await.context("reenfileirar job")?;
    }
    Ok(())
}

pub async fn run_worker(state: AppState, mut rx: mpsc::Receiver<String>) {
    while let Some(job_id) = rx.recv().await {
        if let Err(error) = run_one(&state, &job_id).await {
            error!(%job_id, %error, "job falhou");
            let _ = sqlx::query("UPDATE jobs SET status = 'FAILED', error = ?, finished_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind(error.to_string())
                .bind(&job_id)
                .execute(&state.db)
                .await;
        }
    }
}

async fn run_one(state: &AppState, job_id: &str) -> anyhow::Result<()> {
    let job = get(&state.db, job_id)
        .await?
        .ok_or_else(|| anyhow!("job não encontrado"))?;
    sqlx::query("UPDATE jobs SET status = 'RUNNING', started_at = CURRENT_TIMESTAMP, current_step = 'starting' WHERE id = ? AND status = 'QUEUED'")
        .bind(job_id)
        .execute(&state.db)
        .await?;

    match job.job_type.as_str() {
        "PROBE" => run_probe(state, &job).await?,
        "ANALYZE_PROJECT" => {
            run_project_probe(state, &job).await?;
            run_project_analysis(state, &job).await?;
        }
        "RENDER" => run_render(state, &job).await?,
        other => return Err(anyhow!("tipo de job não implementado: {other}")),
    }
    Ok(())
}

async fn run_project_analysis(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let media: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT m.id, m.storage_path, mm.audio_codec, mm.video_codec FROM media_files m LEFT JOIN media_metadata mm ON mm.media_file_id = m.id WHERE m.project_id = ? ORDER BY m.order_index",
    )
    .bind(&job.project_id)
    .fetch_all(&state.db)
    .await?;
    for (index, (media_id, path, audio_codec, video_codec)) in media.iter().enumerate() {
        let report = ffmpeg::analyze_file(
            &state.ffmpeg,
            &state.storage_root.join(path),
            audio_codec.is_some(),
            video_codec.is_some(),
        )
        .await?;
        sqlx::query("DELETE FROM analysis_results WHERE media_file_id = ?")
            .bind(media_id)
            .execute(&state.db)
            .await?;
        for item in &report.items {
            sqlx::query("INSERT INTO analysis_results (id, project_id, media_file_id, kind, start_ms, end_ms, confidence, suggestion, evidence_json, rule_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&job.project_id)
                .bind(media_id)
                .bind(&item.kind)
                .bind(item.start_ms)
                .bind(item.end_ms)
                .bind(item.confidence)
                .bind(&item.suggestion)
                .bind(&item.evidence_json)
                .bind("phase2-v1")
                .execute(&state.db)
                .await?;
        }
        sqlx::query("UPDATE jobs SET progress = ?, current_step = ? WHERE id = ?")
            .bind((index + 1) as f64 / media.len().max(1) as f64)
            .bind(format!("análise {}/{}", index + 1, media.len()))
            .bind(&job.id)
            .execute(&state.db)
            .await?;
    }
    finish(&state.db, &job.id, json!({"media_count": media.len()})).await
}

async fn run_render(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let input: Value = serde_json::from_str(&job.input_json)?;
    let plan_id = input
        .get("render_plan_id")
        .and_then(Value::as_str)
        .context("RENDER sem render_plan_id")?;
    let plan_json: String =
        sqlx::query_scalar("SELECT plan_json FROM render_plans WHERE id = ? AND project_id = ?")
            .bind(plan_id)
            .bind(&job.project_id)
            .fetch_one(&state.db)
            .await?;
    let mut plan: render::RenderPlan = serde_json::from_str(&plan_json)?;
    sqlx::query("UPDATE render_plans SET status = 'RUNNING' WHERE id = ?")
        .bind(plan_id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE jobs SET progress = 0.1, current_step = 'rendering' WHERE id = ?")
        .bind(&job.id)
        .execute(&state.db)
        .await?;
    let metadata = match render::execute(state, &mut plan).await {
        Ok(metadata) => metadata,
        Err(error) => {
            sqlx::query("UPDATE render_plans SET status = 'FAILED' WHERE id = ?")
                .bind(plan_id)
                .execute(&state.db)
                .await?;
            return Err(error);
        }
    };
    sqlx::query("UPDATE render_plans SET plan_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&plan)?)
        .bind(plan_id)
        .execute(&state.db)
        .await?;
    sqlx::query("UPDATE render_plans SET status = 'COMPLETED', rendered_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(plan_id)
        .execute(&state.db)
        .await?;
    finish(
        &state.db,
        &job.id,
        json!({"render_plan_id": plan_id, "output_path": plan.output_path, "duration_ms": metadata.duration_ms}),
    )
    .await
}

async fn run_probe(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let input: Value = serde_json::from_str(&job.input_json)?;
    let media_id = input
        .get("media_id")
        .and_then(Value::as_str)
        .context("PROBE sem media_id")?;
    let path: String =
        sqlx::query_scalar("SELECT storage_path FROM media_files WHERE id = ? AND project_id = ?")
            .bind(media_id)
            .bind(&job.project_id)
            .fetch_one(&state.db)
            .await?;
    let metadata = ffprobe::probe(&state.ffprobe, &state.storage_root.join(&path)).await?;
    save_metadata(&state.db, media_id, &metadata).await?;
    sqlx::query("UPDATE media_files SET status = 'READY' WHERE id = ?")
        .bind(media_id)
        .execute(&state.db)
        .await?;
    finish(&state.db, &job.id, json!({"media_id": media_id})).await
}

async fn run_project_probe(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let media_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM media_files WHERE project_id = ? ORDER BY order_index")
            .bind(&job.project_id)
            .fetch_all(&state.db)
            .await?;
    for (index, media_id) in media_ids.iter().enumerate() {
        let child_id = create(
            &state.db,
            &job.project_id,
            "PROBE",
            json!({"media_id": media_id}),
        )
        .await?;
        run_probe(state, &get(&state.db, &child_id).await?.unwrap()).await?;
        sqlx::query("UPDATE jobs SET progress = ?, current_step = ? WHERE id = ?")
            .bind((index + 1) as f64 / media_ids.len().max(1) as f64)
            .bind(format!("media {}/{}", index + 1, media_ids.len()))
            .bind(&job.id)
            .execute(&state.db)
            .await?;
    }
    // Project analysis is still running; only its final stage may complete the job.
    Ok(())
}

async fn save_metadata(
    pool: &SqlitePool,
    media_id: &str,
    metadata: &ffprobe::Metadata,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO media_metadata (media_file_id, duration_ms, width, height, fps_num, fps_den, video_codec, audio_codec, sample_rate, channels, bitrate, raw_probe_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(media_file_id) DO UPDATE SET duration_ms = excluded.duration_ms, width = excluded.width, height = excluded.height, fps_num = excluded.fps_num, fps_den = excluded.fps_den, video_codec = excluded.video_codec, audio_codec = excluded.audio_codec, sample_rate = excluded.sample_rate, channels = excluded.channels, bitrate = excluded.bitrate, raw_probe_json = excluded.raw_probe_json")
        .bind(media_id)
        .bind(metadata.duration_ms)
        .bind(metadata.width)
        .bind(metadata.height)
        .bind(metadata.fps_num)
        .bind(metadata.fps_den)
        .bind(&metadata.video_codec)
        .bind(&metadata.audio_codec)
        .bind(metadata.sample_rate)
        .bind(metadata.channels)
        .bind(metadata.bitrate)
        .bind(&metadata.raw_json)
        .execute(pool)
        .await?;
    Ok(())
}

async fn finish(pool: &SqlitePool, job_id: &str, result: Value) -> anyhow::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'COMPLETED', progress = 1, current_step = 'completed', result_json = ?, finished_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(serde_json::to_string(&result)?)
        .bind(job_id)
        .execute(pool)
        .await?;
    info!(%job_id, "job concluído");
    Ok(())
}
