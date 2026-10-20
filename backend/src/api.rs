use std::{convert::Infallible, time::Duration};

use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Request, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use tokio::time::sleep;
use tracing::warn;

use crate::{AppState, jobs, render, storage};

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        warn!(%error, "erro de API");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        ApiError::from(anyhow!(error))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct CreateProject {
    name: String,
}

#[derive(Debug, Serialize, FromRow)]
struct Project {
    id: String,
    name: String,
    slug: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, FromRow)]
struct MediaFile {
    id: String,
    project_id: String,
    original_name: String,
    order_index: i64,
    size_bytes: i64,
    sha256: String,
    status: String,
    storage_path: String,
    content_type: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
struct MediaMetadata {
    media_file_id: String,
    duration_ms: i64,
    width: Option<i64>,
    height: Option<i64>,
    fps_num: Option<i64>,
    fps_den: Option<i64>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    sample_rate: Option<i64>,
    channels: Option<i64>,
    bitrate: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct AnalysisResult {
    id: String,
    media_file_id: String,
    kind: String,
    start_ms: i64,
    end_ms: i64,
    confidence: f64,
    suggestion: String,
    evidence_json: String,
    rule_version: String,
}

#[derive(Debug, Deserialize, Default)]
struct RenderRequest {
    #[serde(default)]
    remove_analysis_ids: Vec<String>,
    #[serde(default)]
    normalize_audio: bool,
    #[serde(default)]
    manual_cuts: Vec<render::ManualCut>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/projects", post(create_project).get(list_projects))
        .route(
            "/api/projects/{project_id}",
            get(get_project).patch(rename_project),
        )
        .route("/api/projects/{project_id}/media/order", patch(order_media))
        .route(
            "/api/projects/{project_id}/media/{media_id}/content",
            get(media_content),
        )
        .route("/api/projects/{project_id}/renders", get(list_renders))
        .route(
            "/api/projects/{project_id}/render-plans/{plan_id}/download",
            get(download_render),
        )
        .route(
            "/api/projects/{project_id}/media",
            post(upload_media).get(list_media),
        )
        .route("/api/projects/{project_id}/analyze", post(analyze_project))
        .route("/api/projects/{project_id}/timeline", get(get_timeline))
        .route("/api/projects/{project_id}/render", post(render_project))
        .route(
            "/api/projects/{project_id}/render-plans/{plan_id}",
            get(get_render_plan),
        )
        .route("/api/jobs/{job_id}", get(get_job))
        .route("/api/jobs/{job_id}/events", get(job_events))
        .layer(DefaultBodyLimit::disable())
        .with_state(state)
}

async fn rename_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<CreateProject>,
) -> Result<StatusCode, ApiError> {
    ensure_project(&state, &id).await?;
    let name = input.name.trim();
    if name.is_empty() || name.len() > 200 {
        return Err(ApiError::bad_request("Nome deve ter entre 1 e 200 bytes"));
    }
    sqlx::query(
        "UPDATE projects SET name = ?, slug = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(name)
    .bind(slugify(name))
    .bind(id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MediaOrder {
    media_ids: Vec<String>,
}

async fn order_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<MediaOrder>,
) -> Result<StatusCode, ApiError> {
    ensure_project(&state, &id).await?;
    let mut tx = state.db.begin().await?;
    let mut expected: Vec<String> =
        sqlx::query_scalar("SELECT id FROM media_files WHERE project_id = ? ORDER BY id")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
    let mut received = input.media_ids.clone();
    expected.sort();
    received.sort();
    if expected != received {
        return Err(ApiError::bad_request(
            "Envie todos os arquivos do projeto exatamente uma vez",
        ));
    }
    for (index, media_id) in input.media_ids.iter().enumerate() {
        sqlx::query("UPDATE media_files SET order_index = ? WHERE id = ? AND project_id = ?")
            .bind(index as i64)
            .bind(media_id)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn media_content(
    State(state): State<AppState>,
    Path((project_id, media_id)): Path<(String, String)>,
    request: Request,
) -> Result<axum::response::Response, ApiError> {
    let path: String =
        sqlx::query_scalar("SELECT storage_path FROM media_files WHERE project_id = ? AND id = ?")
            .bind(project_id)
            .bind(media_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::not_found("Mídia não encontrada"))?;
    serve_file(&state, &path, request).await
}

async fn serve_file(
    state: &AppState,
    path: &str,
    request: Request,
) -> Result<axum::response::Response, ApiError> {
    // The path comes only from the database; ServeFile handles streaming and byte ranges.
    let response = tower_http::services::ServeFile::new(state.storage_root.join(path))
        .try_call(request)
        .await
        .map_err(|error| anyhow!(error))?;
    Ok(response.into_response())
}

async fn list_renders(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_project(&state, &id).await?;
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as("SELECT id, status, rendered_at FROM render_plans WHERE project_id = ? ORDER BY created_at DESC, rowid DESC")
        .bind(id).fetch_all(&state.db).await?;
    Ok(Json(json!(rows.into_iter().map(|(id, status, rendered_at)| json!({"id": id, "status": status, "rendered_at": rendered_at})).collect::<Vec<_>>())))
}

async fn download_render(
    State(state): State<AppState>,
    Path((project_id, plan_id)): Path<(String, String)>,
    request: Request,
) -> Result<axum::response::Response, ApiError> {
    let path: String = sqlx::query_scalar("SELECT output_path FROM render_plans WHERE project_id = ? AND id = ? AND status = 'COMPLETED'")
        .bind(project_id).bind(plan_id).fetch_optional(&state.db).await?
        .ok_or_else(|| ApiError::not_found("Render concluído não encontrado"))?;
    let mut response = serve_file(&state, &path, request).await?;
    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        "attachment; filename=video-editado.mp4".parse().unwrap(),
    );
    Ok(response)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

async fn create_project(
    State(state): State<AppState>,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 200 {
        return Err(ApiError::bad_request(
            "name deve ter entre 1 e 200 caracteres",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let slug = slugify(name);
    sqlx::query("INSERT INTO projects (id, name, slug) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(&slug)
        .execute(&state.db)
        .await?;
    storage::project_dir(&state.storage_root, &id).await?;
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, name, slug, status, created_at, updated_at FROM projects WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn list_projects(State(state): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(sqlx::query_as::<_, Project>("SELECT id, name, slug, status, created_at, updated_at FROM projects ORDER BY created_at DESC")
        .fetch_all(&state.db).await?))
}

async fn get_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let project = sqlx::query_as::<_, Project>(
        "SELECT id, name, slug, status, created_at, updated_at FROM projects WHERE id = ?",
    )
    .bind(&project_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("projeto não encontrado"))?;
    let media = sqlx::query_as::<_, MediaFile>("SELECT id, project_id, original_name, order_index, size_bytes, sha256, status, storage_path, content_type FROM media_files WHERE project_id = ? ORDER BY order_index")
        .bind(&project_id).fetch_all(&state.db).await?;
    Ok(Json(json!({"project": project, "media": media})))
}

async fn list_media(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    ensure_project(&state, &project_id).await?;
    let files = sqlx::query_as::<_, MediaFile>("SELECT id, project_id, original_name, order_index, size_bytes, sha256, status, storage_path, content_type FROM media_files WHERE project_id = ? ORDER BY order_index")
        .bind(&project_id).fetch_all(&state.db).await?;
    let mut result = Vec::with_capacity(files.len());
    for file in files {
        let metadata = sqlx::query_as::<_, MediaMetadata>("SELECT media_file_id, duration_ms, width, height, fps_num, fps_den, video_codec, audio_codec, sample_rate, channels, bitrate FROM media_metadata WHERE media_file_id = ?")
            .bind(&file.id).fetch_optional(&state.db).await?;
        result.push(json!({"file": file, "metadata": metadata}));
    }
    Ok(Json(result))
}

async fn upload_media(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Vec<serde_json::Value>>), ApiError> {
    ensure_project(&state, &project_id).await?;
    let mut uploaded = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let original_name = field.file_name().unwrap_or("upload.bin").to_string();
        let content_type = field.content_type().map(ToOwned::to_owned);
        let (storage_path, size_bytes, sha256) = storage::write_upload(
            &state.storage_root,
            &project_id,
            &original_name,
            field,
            state.max_upload_bytes,
        )
        .await?;
        let media_id = uuid::Uuid::new_v4().to_string();
        let order_index: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(order_index), -1) + 1 FROM media_files WHERE project_id = ?",
        )
        .bind(&project_id)
        .fetch_one(&state.db)
        .await?;
        sqlx::query("INSERT INTO media_files (id, project_id, original_name, storage_path, content_type, order_index, size_bytes, sha256) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&media_id).bind(&project_id).bind(&original_name).bind(&storage_path).bind(&content_type).bind(order_index).bind(size_bytes as i64).bind(&sha256)
            .execute(&state.db).await?;
        let job_id = jobs::create(
            &state.db,
            &project_id,
            "PROBE",
            json!({"media_id": media_id}),
        )
        .await?;
        state
            .job_tx
            .send(job_id.clone())
            .await
            .map_err(|_| anyhow!("worker de jobs indisponível"))?;
        uploaded.push(json!({"media_id": media_id, "job_id": job_id, "original_name": original_name, "status": "UPLOADED"}));
    }
    if uploaded.is_empty() {
        return Err(ApiError::bad_request(
            "envie ao menos um campo multipart chamado file",
        ));
    }
    Ok((StatusCode::CREATED, Json(uploaded)))
}

async fn analyze_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    ensure_project(&state, &project_id).await?;
    let job_id = jobs::create(&state.db, &project_id, "ANALYZE_PROJECT", json!({})).await?;
    state
        .job_tx
        .send(job_id.clone())
        .await
        .map_err(|_| anyhow!("worker de jobs indisponível"))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job_id": job_id, "status": "QUEUED"})),
    ))
}

async fn get_timeline(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<AnalysisResult>>, ApiError> {
    ensure_project(&state, &project_id).await?;
    Ok(Json(
        sqlx::query_as::<_, AnalysisResult>(
            "SELECT id, media_file_id, kind, start_ms, end_ms, confidence, suggestion, evidence_json, rule_version FROM analysis_results WHERE project_id = ? ORDER BY media_file_id, start_ms",
        )
        .bind(&project_id)
        .fetch_all(&state.db)
        .await?,
    ))
}

async fn render_project(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    Json(input): Json<RenderRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    ensure_project(&state, &project_id).await?;
    let plan = render::build_plan(
        &state,
        &project_id,
        &input.remove_analysis_ids,
        input.normalize_audio,
        &input.manual_cuts,
    )
    .await
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let plan_json = serde_json::to_string(&plan).map_err(|error| anyhow!(error))?;
    sqlx::query(
        "INSERT INTO render_plans (id, project_id, plan_json, output_path) VALUES (?, ?, ?, ?)",
    )
    .bind(&plan.id)
    .bind(&project_id)
    .bind(plan_json)
    .bind(&plan.output_path)
    .execute(&state.db)
    .await?;
    let job_id = jobs::create(
        &state.db,
        &project_id,
        "RENDER",
        json!({"render_plan_id": plan.id}),
    )
    .await?;
    state
        .job_tx
        .send(job_id.clone())
        .await
        .map_err(|_| anyhow!("worker de jobs indisponível"))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "render_plan_id": plan.id,
            "job_id": job_id,
            "segment_count": plan.segments.len(),
            "status": "QUEUED"
        })),
    ))
}

async fn get_render_plan(
    State(state): State<AppState>,
    Path((project_id, plan_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    ensure_project(&state, &project_id).await?;
    let row: Option<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, plan_json, output_path, rendered_at FROM render_plans WHERE id = ? AND project_id = ?",
    )
    .bind(&plan_id)
    .bind(&project_id)
    .fetch_optional(&state.db)
    .await?;
    let (id, status, plan_json, output_path, rendered_at) =
        row.ok_or_else(|| ApiError::not_found("RenderPlan não encontrado"))?;
    let plan: serde_json::Value =
        serde_json::from_str(&plan_json).map_err(|error| anyhow!(error))?;
    Ok(Json(json!({
        "id": id,
        "status": status,
        "output_path": output_path,
        "rendered_at": rendered_at,
        "plan": plan
    })))
}

async fn get_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<jobs::Job>, ApiError> {
    Ok(Json(jobs::get(&state.db, &job_id).await?.ok_or_else(
        || ApiError::not_found("job não encontrado"),
    )?))
}

async fn job_events(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    if jobs::get(&state.db, &job_id).await?.is_none() {
        return Err(ApiError::not_found("job não encontrado"));
    }
    let stream = async_stream::stream! {
        loop {
            match jobs::get(&state.db, &job_id).await {
                Ok(Some(job)) => {
                    let terminal = matches!(job.status.as_str(), "COMPLETED" | "FAILED" | "CANCELED");
                    let payload = serde_json::to_string(&job).unwrap_or_else(|_| "{}".into());
                    yield Ok(Event::default().event("job").data(payload));
                    if terminal { break; }
                }
                Ok(None) => {
                    yield Ok(Event::default().event("error").data("job não encontrado"));
                    break;
                }
                Err(error) => {
                    yield Ok(Event::default().event("error").data(error.to_string()));
                    break;
                }
            }
            sleep(Duration::from_secs(1)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn ensure_project(state: &AppState, id: &str) -> Result<(), ApiError> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("projeto não encontrado"));
    }
    Ok(())
}

fn slugify(input: &str) -> String {
    let slug = input
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    slug.split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn creates_stable_url_slug() {
        assert_eq!(slugify(" Aula 04 — PostGIS "), "aula-04-postgis");
    }
}
