use argon2::{password_hash::{PasswordHasher, SaltString}, Argon2};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::env;
use url::Url;
use uuid::Uuid;

use crate::state::AppState;

#[derive(Debug, Serialize, FromRow)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub target_url: String,
    pub enabled: bool,
    pub access_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateService {
    pub name: String,
    pub target_url: String,
    pub enabled: Option<bool>,
    pub access_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateService {
    pub name: Option<String>,
    pub target_url: Option<String>,
    pub enabled: Option<bool>,
    pub access_mode: Option<String>,
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

#[derive(Debug, Deserialize)]
struct TestTargetInput {
    target_url: String,
}

#[derive(Debug, Serialize)]
struct TestTargetResult {
    reachable: bool,
    status: u16,
    final_url: String,
    content_type: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/test-target", post(test_target))
        .route("/network", get(network_info))
        .route("/logs", get(list_logs))
        .route("/", get(list_services).post(create_service))
        .route("/{id}", get(get_service).put(update_service).delete(delete_service))
        .route("/{id}/acl", get(get_acl).put(update_acl))
        .route("/access/users", get(list_users).post(create_user))
        .route("/access/users/{id}", axum::routing::put(update_user).delete(delete_user_account))
        .route("/access/groups", get(list_groups).post(create_group))
        .route("/access/groups/{id}", axum::routing::put(update_group).delete(delete_group))
        
}

#[derive(Debug, Deserialize)]
struct LogQuery {
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct AccessLogRow {
    id: i64,
    service_id: String,
    service_name: Option<String>,
    method: Option<String>,
    path: Option<String>,
    client_ip: Option<String>,
    user_agent: Option<String>,
    username: Option<String>,
    status: i64,
    created_at: String,
}

async fn list_logs(
    State(state): State<AppState>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<AccessLogRow>>, (StatusCode, Json<ApiError>)> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows = sqlx::query_as::<_, AccessLogRow>(
        "SELECT l.id, l.service_id, s.name AS service_name, l.method, l.path, l.client_ip, l.user_agent, m.username, l.status, l.created_at \
         FROM access_logs l \
         LEFT JOIN services s ON s.id = l.service_id \
         LEFT JOIN members m ON m.id = l.member_id \
         ORDER BY l.id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize)]
struct NetworkInfo {
    version: String,
    listen: String,
    caddy_enabled: bool,
    caddy_listen: Option<String>,
    caddy_config_path: Option<String>,
    caddy_config_exists: bool,
    ui_host: Option<String>,
    service_domain: Option<String>,
    cookie_secure: bool,
    database_url: Option<String>,
    total_services: i64,
    enabled_services: i64,
    caddyfile: Option<String>,
}

fn env_non_empty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_flag(name: &str) -> bool {
    env_non_empty(name)
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false)
}

async fn network_info(
    State(state): State<AppState>,
) -> Result<Json<NetworkInfo>, (StatusCode, Json<ApiError>)> {
    let total_services: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM services")
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let enabled_services: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM services WHERE enabled = 1")
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let caddy_enabled = env_flag("TUNNELHUB_CADDY_ENABLED");
    let caddy_config_path = env_non_empty("TUNNELHUB_CADDY_CONFIG");
    let caddy_config_exists = match &caddy_config_path {
        Some(path) => tokio::fs::metadata(path).await.is_ok(),
        None => false,
    };
    let caddyfile = match &caddy_config_path {
        Some(path) if caddy_config_exists => tokio::fs::read_to_string(path).await.ok(),
        _ => None,
    };
    let listen = env_non_empty("TUNNELHUB_LISTEN").unwrap_or_else(|| {
        if caddy_enabled {
            "127.0.0.1:8081".to_string()
        } else {
            "0.0.0.0:8080".to_string()
        }
    });
    Ok(Json(NetworkInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        listen,
        caddy_enabled,
        caddy_listen: env_non_empty("TUNNELHUB_CADDY_LISTEN"),
        caddy_config_path,
        caddy_config_exists,
        ui_host: env_non_empty("TUNNELHUB_UI_HOST"),
        service_domain: crate::proxy::service_domain(),
        cookie_secure: env_non_empty("TUNNELHUB_COOKIE_SECURE")
            .map(|value| !value.eq_ignore_ascii_case("false"))
            .unwrap_or(true),
        database_url: env_non_empty("TUNNELHUB_DATABASE_URL"),
        total_services,
        enabled_services,
        caddyfile,
    }))
}

async fn test_target(
    Json(input): Json<TestTargetInput>,
) -> Result<Json<TestTargetResult>, (StatusCode, Json<ApiError>)> {
    let target_url = validate_target(&input.target_url)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| {
            tracing::error!(?error, "创建 HTTP 连通性检测客户端失败");
            (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new("无法初始化 HTTP 检测客户端"))
        })?;
    let response = client.get(&target_url).send().await.map_err(|error| {
        tracing::warn!(target_url = %target_url, ?error, "目标服务 HTTP 连通性检测失败");
        (
            StatusCode::BAD_GATEWAY,
            ApiError::new(format!("TunnelHub 无法通过 HTTP 访问目标服务：{error}")),
        )
    })?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    Ok(Json(TestTargetResult {
        reachable: true,
        status,
        final_url,
        content_type,
    }))
}

async fn list_services(State(state): State<AppState>) -> Result<Json<Vec<Service>>, (StatusCode, Json<ApiError>)> {
    let rows = sqlx::query_as::<_, Service>(
        "SELECT id, name, target_url, enabled, access_mode, created_at, updated_at FROM services ORDER BY name COLLATE NOCASE"
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
        "SELECT id, name, target_url, enabled, access_mode, created_at, updated_at FROM services WHERE id = ?"
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
    let target_url = validate_target(&input.target_url)?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let id = Uuid::new_v4().to_string();
    let enabled = input.enabled.unwrap_or(true);
    let access_mode = validate_access_mode(input.access_mode.as_deref().unwrap_or("public"))?;

    sqlx::query("INSERT INTO services (id, name, target_url, enabled, access_mode, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(&name).bind(&target_url).bind(enabled).bind(access_mode).bind(&now).bind(&now)
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
    let target_url = match input.target_url { Some(value) => validate_target(&value)?, None => current.target_url };
    let enabled = input.enabled.unwrap_or(current.enabled);
    let access_mode = validate_access_mode(input.access_mode.as_deref().unwrap_or(&current.access_mode))?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    sqlx::query("UPDATE services SET name = ?, target_url = ?, enabled = ?, access_mode = ?, updated_at = ? WHERE id = ?")
        .bind(name).bind(target_url).bind(enabled).bind(access_mode).bind(now).bind(&id)
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
        "SELECT id, name, target_url, enabled, access_mode, created_at, updated_at FROM services WHERE id = ?"
    )
    .bind(id).fetch_optional(&state.pool).await.map_err(internal_error)
}

async fn fetch_by_id(state: &AppState, id: String) -> Result<Service, (StatusCode, Json<ApiError>)> {
    fetch_optional(state, &id).await?.ok_or_else(not_found)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct AccessUser { id: i64, username: String, enabled: bool, created_at: String }
#[derive(Debug, Deserialize)]
struct CreateAccessUser { username: String, password: String }
#[derive(Debug, Deserialize)]
struct UpdateAccessUser { password: Option<String>, enabled: Option<bool> }
#[derive(Debug, Serialize, sqlx::FromRow)]
struct AccessGroup { id: i64, name: String, created_at: String }
#[derive(Debug, Deserialize)]
struct CreateAccessGroup { name: String }
#[derive(Debug, Deserialize)]
struct UpdateAccessGroup { name: Option<String>, members: Option<Vec<i64>> }
#[derive(Debug, Deserialize)]
struct UpdateServiceAcl { access_mode: String, users: Vec<i64>, groups: Vec<i64> }
#[derive(Debug, Serialize)]
struct ServiceAcl { access_mode: String, users: Vec<i64>, groups: Vec<i64> }

async fn list_users(State(state): State<AppState>) -> Result<Json<Vec<AccessUser>>, (StatusCode, Json<ApiError>)> {
    let rows = sqlx::query_as::<_, AccessUser>("SELECT id, username, enabled, created_at FROM members ORDER BY username COLLATE NOCASE")
        .fetch_all(&state.pool).await.map_err(internal_error)?;
    Ok(Json(rows))
}
async fn create_user(State(state): State<AppState>, Json(input): Json<CreateAccessUser>) -> Result<(StatusCode, Json<AccessUser>), (StatusCode, Json<ApiError>)> {
    let username = input.username.trim();
    if username.len() < 3 || username.len() > 64 { return Err((StatusCode::BAD_REQUEST, ApiError::new("用户名长度必须为 3 到 64 个字符"))); }
    if input.password.len() < 12 || input.password.len() > 1024 { return Err((StatusCode::BAD_REQUEST, ApiError::new("密码长度必须为 12 到 1024 个字符"))); }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(input.password.as_bytes(), &salt).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new("无法生成密码哈希")))?.to_string();
    let result = sqlx::query("INSERT INTO members (username, password_hash, enabled, created_at) VALUES (?, ?, 1, ?)")
        .bind(username).bind(hash).bind(Utc::now().to_rfc3339()).execute(&state.pool).await;
    if let Err(e) = result { return Err(conflict_or_internal(e)); }
    let row = sqlx::query_as::<_, AccessUser>("SELECT id, username, enabled, created_at FROM members WHERE username = ?")
        .bind(username).fetch_one(&state.pool).await.map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(row)))
}
async fn update_user(State(state): State<AppState>, Path(id): Path<i64>, Json(input): Json<UpdateAccessUser>) -> Result<Json<AccessUser>, (StatusCode, Json<ApiError>)> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE id = ?").bind(id).fetch_one(&state.pool).await.map_err(internal_error)?;
    if exists == 0 { return Err((StatusCode::NOT_FOUND, ApiError::new("用户不存在"))); }
    if let Some(password) = input.password {
        if password.len() < 12 || password.len() > 1024 { return Err((StatusCode::BAD_REQUEST, ApiError::new("密码长度必须为 12 到 1024 个字符"))); }
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, ApiError::new("无法生成密码哈希")))?.to_string();
        sqlx::query("UPDATE members SET password_hash = ? WHERE id = ?").bind(hash).bind(id).execute(&state.pool).await.map_err(internal_error)?;
        sqlx::query("DELETE FROM member_sessions WHERE member_id = ?").bind(id).execute(&state.pool).await.map_err(internal_error)?;
    }
    if let Some(enabled) = input.enabled {
        sqlx::query("UPDATE members SET enabled = ? WHERE id = ?").bind(enabled).bind(id).execute(&state.pool).await.map_err(internal_error)?;
        if !enabled { sqlx::query("DELETE FROM member_sessions WHERE member_id = ?").bind(id).execute(&state.pool).await.map_err(internal_error)?; }
    }
    let row = sqlx::query_as::<_, AccessUser>("SELECT id, username, enabled, created_at FROM members WHERE id = ?").bind(id).fetch_one(&state.pool).await.map_err(internal_error)?;
    Ok(Json(row))
}
async fn delete_user_account(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM members WHERE id = ?").bind(id).execute(&state.pool).await.map_err(internal_error)?;
    if result.rows_affected() == 0 { return Err((StatusCode::NOT_FOUND, ApiError::new("用户不存在"))); }
    Ok(StatusCode::NO_CONTENT)
}


#[derive(Debug, Serialize)]
struct AccessGroupWithMembers { id: i64, name: String, created_at: String, members: Vec<i64> }

async fn list_groups(State(state): State<AppState>) -> Result<Json<Vec<AccessGroupWithMembers>>, (StatusCode, Json<ApiError>)> {
    let groups = sqlx::query_as::<_, AccessGroup>("SELECT id, name, created_at FROM user_groups ORDER BY name COLLATE NOCASE")
        .fetch_all(&state.pool).await.map_err(internal_error)?;
    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        let members = sqlx::query_scalar("SELECT member_id FROM group_members WHERE group_id = ? ORDER BY member_id")
            .bind(group.id).fetch_all(&state.pool).await.map_err(internal_error)?;
        result.push(AccessGroupWithMembers { id: group.id, name: group.name, created_at: group.created_at, members });
    }
    Ok(Json(result))
}

async fn create_group(State(state): State<AppState>, Json(input): Json<CreateAccessGroup>) -> Result<(StatusCode, Json<AccessGroup>), (StatusCode, Json<ApiError>)> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 { return Err((StatusCode::BAD_REQUEST, ApiError::new("用户组名称不能为空且不能超过 100 个字符"))); }
    let result = sqlx::query("INSERT INTO user_groups (name, created_at) VALUES (?, ?)").bind(name).bind(Utc::now().to_rfc3339()).execute(&state.pool).await;
    if let Err(e) = result { return Err(conflict_or_internal(e)); }
    let row = sqlx::query_as::<_, AccessGroup>("SELECT id, name, created_at FROM user_groups WHERE name = ?").bind(name).fetch_one(&state.pool).await.map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(row)))
}
async fn update_group(State(state): State<AppState>, Path(id): Path<i64>, Json(input): Json<UpdateAccessGroup>) -> Result<Json<AccessGroup>, (StatusCode, Json<ApiError>)> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_groups WHERE id = ?").bind(id).fetch_one(&state.pool).await.map_err(internal_error)?;
    if exists == 0 { return Err((StatusCode::NOT_FOUND, ApiError::new("用户组不存在"))); }
    if let Some(name) = input.name {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 { return Err((StatusCode::BAD_REQUEST, ApiError::new("用户组名称不能为空且不能超过 100 个字符"))); }
        let result = sqlx::query("UPDATE user_groups SET name = ? WHERE id = ?").bind(name).bind(id).execute(&state.pool).await;
        if let Err(e) = result { return Err(conflict_or_internal(e)); }
    }
    if let Some(members) = input.members {
        let mut tx = state.pool.begin().await.map_err(internal_error)?;
        sqlx::query("DELETE FROM group_members WHERE group_id = ?").bind(id).execute(&mut *tx).await.map_err(internal_error)?;
        for member_id in members {
            sqlx::query("INSERT OR IGNORE INTO group_members (group_id, member_id) SELECT ?, id FROM members WHERE id = ? AND enabled = 1")
                .bind(id).bind(member_id).execute(&mut *tx).await.map_err(internal_error)?;
        }
        tx.commit().await.map_err(internal_error)?;
    }
    let row = sqlx::query_as::<_, AccessGroup>("SELECT id, name, created_at FROM user_groups WHERE id = ?").bind(id).fetch_one(&state.pool).await.map_err(internal_error)?;
    Ok(Json(row))
}
async fn delete_group(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let result = sqlx::query("DELETE FROM user_groups WHERE id = ?").bind(id).execute(&state.pool).await.map_err(internal_error)?;
    if result.rows_affected() == 0 { return Err((StatusCode::NOT_FOUND, ApiError::new("用户组不存在"))); }
    Ok(StatusCode::NO_CONTENT)
}
async fn get_acl(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<ServiceAcl>, (StatusCode, Json<ApiError>)> {
    let access_mode: Option<String> = sqlx::query_scalar("SELECT access_mode FROM services WHERE id = ?").bind(&id).fetch_optional(&state.pool).await.map_err(internal_error)?;
    let access_mode = access_mode.ok_or_else(not_found)?;
    let users = sqlx::query_scalar("SELECT subject_id FROM service_acl WHERE service_id = ? AND subject_type = 'user' ORDER BY subject_id").bind(&id).fetch_all(&state.pool).await.map_err(internal_error)?;
    let groups = sqlx::query_scalar("SELECT subject_id FROM service_acl WHERE service_id = ? AND subject_type = 'group' ORDER BY subject_id").bind(&id).fetch_all(&state.pool).await.map_err(internal_error)?;
    Ok(Json(ServiceAcl { access_mode, users, groups }))
}
async fn update_acl(State(state): State<AppState>, Path(id): Path<String>, Json(input): Json<UpdateServiceAcl>) -> Result<Json<ServiceAcl>, (StatusCode, Json<ApiError>)> {
    let access_mode = validate_access_mode(&input.access_mode)?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM services WHERE id = ?").bind(&id).fetch_one(&state.pool).await.map_err(internal_error)?;
    if exists == 0 { return Err(not_found()); }
    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    sqlx::query("DELETE FROM service_acl WHERE service_id = ?").bind(&id).execute(&mut *tx).await.map_err(internal_error)?;
    for user_id in input.users {
        sqlx::query("INSERT OR IGNORE INTO service_acl (service_id, subject_type, subject_id) SELECT ?, 'user', id FROM members WHERE id = ? AND enabled = 1")
            .bind(&id).bind(user_id).execute(&mut *tx).await.map_err(internal_error)?;
    }
    for group_id in input.groups {
        sqlx::query("INSERT OR IGNORE INTO service_acl (service_id, subject_type, subject_id) SELECT ?, 'group', id FROM user_groups WHERE id = ?")
            .bind(&id).bind(group_id).execute(&mut *tx).await.map_err(internal_error)?;
    }
    sqlx::query("UPDATE services SET access_mode = ?, updated_at = ? WHERE id = ?").bind(access_mode).bind(Utc::now().to_rfc3339()).bind(&id)
        .execute(&mut *tx).await.map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;
    get_acl(State(state), Path(id)).await
}
fn validate_access_mode(value: &str) -> Result<String, (StatusCode, Json<ApiError>)> {
    match value {
        "public" | "restricted" => Ok(value.to_string()),
        _ => Err((StatusCode::BAD_REQUEST, ApiError::new("访问模式只能是 public 或 restricted"))),
    }
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
            return (StatusCode::CONFLICT, ApiError::new("数据已存在或与现有记录冲突"));
        }
    }
    internal_error(error)
}
