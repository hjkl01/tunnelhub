use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use url::Url;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Serialize, FromRow)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub target_url: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateService {
    pub name: String,
    pub hostname: String,
    pub target_url: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateService {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub target_url: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

impl ApiError {
    fn new(message: impl Into<String>) -> Json<Self> {
        Json(Self { error: message.into() })
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_services).post(create_service))
        .route("/{id}", get(get_service).put(update_service).delete(delete_service))
}

async fn list_services(State(state): State<AppState>) -> Result<Json<Vec<Service>>, (StatusCode, Json<ApiError>)> {
    let rows = sqlx::query_as::<_, Service>(
        "SELECT id, name, hostname, target_url, enabled, created_at, updated_at FROM services ORDER BY name COLLATE NOCASE"
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows))
}

async fn get_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Service>, (StatusCode, Json<ApiError>)> {
    let row = sqlx::query_as::<_, Service>(
        "SELECT id, name, hostname, target_url, enabled, created_at, updated_at FROM services WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    Ok(Json(row))
}

async fn create_service(
    State(state): State<AppState>,
    Json(input): Json<CreateService>,
) -> Result<(StatusCode, Json<Service>), (StatusCode, Json<ApiError>)> {
    let name = required(input.name, "name")?;
    let hostname = normalize_hostname(&input.hostname)?;
    let target_url = validate_target(&input.target_url)?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let id = Uuid::new_v4().to_string();
    let enabled = input.enabled.unwrap_or(true);

    sqlx::query("INSERT INTO services (id, name, hostname, target_url, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(&name).bind(&hostname).bind(&target_url).bind(enabled).bind(&now).bind(&now)
        .execute(&state.pool).await.map_err(conflict_or_internal)?;

    let service = fetch_by_id(&state, id).await?;
    Ok((StatusCode::CREATED, Json(service)))
}

async fn update_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateService>,
) -> Result<Json<Service>, (StatusCode, Json<ApiError>)> {
    let current = fetch_optional(&state, &id).await?.ok_or_else(not_found)?;
    let name = match input.name { Some(value) => required(value, "name")?, None => current.name };
    let hostname = match input.hostname { Some(value) => normalize_hostname(&value)?, None => current.hostname };
    let target_url = match input.target_url { Some(value) => validate_target(&value)?, None => current.target_url };
    let enabled = input.enabled.unwrap_or(current.enabled);
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    sqlx::query("UPDATE services SET name = ?, hostname = ?, target_url = ?, enabled = ?, updated_at = ? WHERE id = ?")
        .bind(name).bind(hostname).bind(target_url).bind(enabled).bind(now).bind(&id)
        .execute(&state.pool).await.map_err(conflict_or_internal)?;
    Ok(Json(fetch_by_id(&state, id).await?))
}

async fn delete_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM services WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn fetch_optional(state: &AppState, id: &str) -> Result<Option<Service>, (StatusCode, Json<ApiError>)> {
    sqlx::query_as::<_, Service>(
        "SELECT id, name, hostname, target_url, enabled, created_at, updated_at FROM services WHERE id = ?"
    )
    .bind(id).fetch_optional(&state.pool).await.map_err(internal_error)
}

async fn fetch_by_id(state: &AppState, id: String) -> Result<Service, (StatusCode, Json<ApiError>)> {
    fetch_optional(state, &id).await?.ok_or_else(not_found)
}

fn required(value: String, field: &str) -> Result<String, (StatusCode, Json<ApiError>)> {
    let value = value.trim();
    if value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, ApiError::new(format!("{field} 不能为空"))));
    }
    if value.len() > 200 {
        return Err((StatusCode::BAD_REQUEST, ApiError::new(format!("{field} 长度不能超过 200 个字符"))));
    }
    Ok(value.to_string())
}

fn normalize_hostname(value: &str) -> Result<String, (StatusCode, Json<ApiError>)> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value.len() > 253 || value.contains('/') || value.contains(':') || value.chars().any(char::is_whitespace) {
        return Err((StatusCode::BAD_REQUEST, ApiError::new("hostname 必须是有效的主机名，不含协议、端口或路径")));
    }
    let valid = value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    if !valid {
        return Err((StatusCode::BAD_REQUEST, ApiError::new("hostname 格式无效")));
    }
    Ok(value)
}

fn validate_target(value: &str) -> Result<String, (StatusCode, Json<ApiError>)> {
    let value = value.trim();
    let parsed = Url::parse(value)
        .map_err(|_| (StatusCode::BAD_REQUEST, ApiError::new("target_url 必须是完整的 http:// 或 https:// URL")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err((StatusCode::BAD_REQUEST, ApiError::new("target_url 仅支持 http 和 https 地址")));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err((StatusCode::BAD_REQUEST, ApiError::new("target_url 不允许在 URL 中嵌入用户名或密码")));
    }
    Ok(parsed.to_string())
}

fn not_found() -> (StatusCode, Json<ApiError>) {
    (StatusCode::NOT_FOUND, ApiError::new("服务不存在"))
}

fn internal_error(error: sqlx::Error) -> (StatusCode, Json<ApiError>) {
    tracing::error!(?error, "database operation failed");
    (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new("数据库操作失败"))
}

fn conflict_or_internal(error: sqlx::Error) -> (StatusCode, Json<ApiError>) {
    if let sqlx::Error::Database(database_error) = &error {
        if database_error.is_unique_violation() {
            return (StatusCode::CONFLICT, ApiError::new("该 hostname 已被使用"));
        }
    }
    internal_error(error)
}
