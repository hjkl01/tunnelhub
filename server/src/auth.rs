use std::{env, time::Duration};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base32::Alphabet;
use chrono::{Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use rand_core::OsRng as PasswordOsRng;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::Row;
use subtle::ConstantTimeEq;

use crate::state::AppState;

type HmacSha1 = Hmac<Sha1>;

const SESSION_COOKIE: &str = "tunnelhub_session";
const SESSION_HOURS: i64 = 8;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/setup", post(setup))
        .route("/totp/pending", get(pending_totp))
        .route("/totp/confirm", post(confirm_totp))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route_layer(axum::middleware::from_fn(require_admin_host))
}

#[derive(Deserialize)]
struct SetupInput {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct CodeInput {
    code: String,
}

#[derive(Deserialize)]
struct LoginInput {
    username: String,
    password: String,
    code: String,
}

#[derive(Serialize)]
struct AuthStatus {
    setup_required: bool,
    authenticated: bool,
    totp_pending: bool,
}

#[derive(Serialize)]
struct SetupResponse {
    secret: String,
    otpauth_uri: String,
}

#[derive(Serialize)]
struct RecoveryResponse {
    recovery_codes: Vec<String>,
}

#[derive(Serialize)]
struct AuthError {
    error: String,
}

impl AuthError {
    fn response(status: StatusCode, message: &'static str) -> (StatusCode, Json<Self>) {
        (status, Json(Self { error: message.to_string() }))
    }
}


async fn require_admin_host(request: Request, next: Next) -> Response {
    let allowed = request
        .headers()
        .get("x-tunnelhub-admin")
        .and_then(|value| value.to_str().ok())
        == Some("1");
    if !allowed {
        return StatusCode::NOT_FOUND.into_response();
    }
    let is_write = request.method() != axum::http::Method::GET
        && request.method() != axum::http::Method::HEAD
        && request.method() != axum::http::Method::OPTIONS;
    if is_write
        && (!request.headers().contains_key(header::ORIGIN) || !same_origin(request.headers()))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthStatus>, (StatusCode, Json<AuthError>)> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users WHERE totp_enabled = 1")
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users WHERE totp_enabled = 0")
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    let authenticated = session_user(&state, &headers).await.is_some();
    Ok(Json(AuthStatus {
        setup_required: count == 0 && pending == 0,
        authenticated,
        totp_pending: count == 0 && pending > 0,
    }))
}

async fn setup(
    State(state): State<AppState>,
    Json(input): Json<SetupInput>,
) -> Result<Json<SetupResponse>, (StatusCode, Json<AuthError>)> {
    let username = input.username.trim();
    if username.len() < 3 || username.len() > 64 {
        return Err(AuthError::response(StatusCode::BAD_REQUEST, "用户名长度必须为 3 到 64 个字符"));
    }
    if input.password.len() < 12 || input.password.len() > 1024 {
        return Err(AuthError::response(StatusCode::BAD_REQUEST, "密码长度必须为 12 到 1024 个字符"));
    }

    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error)?;
    if exists > 0 {
        return Err(AuthError::response(StatusCode::CONFLICT, "管理员已初始化，不能重复设置"));
    }

    let salt = SaltString::generate(&mut PasswordOsRng);
    let password_hash = Argon2::default()
        .hash_password(input.password.as_bytes(), &salt)
        .map_err(|_| internal_error_message())?
        .to_string();
    let secret = generate_secret();
    let result = sqlx::query(
        "INSERT INTO admin_users (id, username, password_hash, totp_secret, totp_enabled, created_at) VALUES (1, ?, ?, ?, 0, ?)",
    )
    .bind(username)
    .bind(password_hash)
    .bind(&secret)
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await;
    if let Err(error) = result {
        if let sqlx::Error::Database(db) = &error {
            if db.is_unique_violation() {
                return Err(AuthError::response(StatusCode::CONFLICT, "管理员已初始化，不能重复设置"));
            }
        }
        return Err(internal_error(error));
    }

    let issuer = "TunnelHub";
    let account = format!("{issuer}:{}", username);
    let uri = format!(
        "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
        percent_encode(&account),
        secret,
        percent_encode(issuer),
    );
    Ok(Json(SetupResponse { secret, otpauth_uri: uri }))
}


async fn pending_totp(
    State(state): State<AppState>,
) -> Result<Json<SetupResponse>, (StatusCode, Json<AuthError>)> {
    let row = sqlx::query("SELECT username, totp_secret FROM admin_users WHERE totp_enabled = 0 ORDER BY id LIMIT 1")
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| AuthError::response(StatusCode::NOT_FOUND, "没有待确认的 TOTP 绑定"))?;
    let username: String = row.get("username");
    let secret: String = row.get("totp_secret");
    let issuer = "TunnelHub";
    let account = format!("{issuer}:{username}");
    let uri = format!(
        "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
        percent_encode(&account),
        secret,
        percent_encode(issuer),
    );
    Ok(Json(SetupResponse { secret, otpauth_uri: uri }))
}

async fn confirm_totp(
    State(state): State<AppState>,
    Json(input): Json<CodeInput>,
) -> Result<(HeaderMap, Json<RecoveryResponse>), (StatusCode, Json<AuthError>)> {
    let row = sqlx::query("SELECT id, username, totp_secret, totp_enabled FROM admin_users ORDER BY id LIMIT 1")
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| AuthError::response(StatusCode::BAD_REQUEST, "请先初始化管理员"))?;
    if row.get::<i64, _>("totp_enabled") != 0 {
        return Err(AuthError::response(StatusCode::CONFLICT, "TOTP 已启用"));
    }
    let user_id: i64 = row.get("id");
    let username: String = row.get("username");
    let since = (Utc::now() - ChronoDuration::minutes(15)).to_rfc3339();
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_login_attempts WHERE username = ? AND attempted_at > ?",
    )
    .bind(&username)
    .bind(since)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)?;
    if attempts >= 10 {
        return Err(AuthError::response(StatusCode::TOO_MANY_REQUESTS, "验证码尝试次数过多，请 15 分钟后重试"));
    }
    let secret: String = row.get("totp_secret");
    if !verify_totp(&secret, &input.code) {
        record_login_failure(&state, &username).await?;
        return Err(AuthError::response(StatusCode::UNAUTHORIZED, "验证码无效或已过期"));
    }
    sqlx::query("DELETE FROM auth_login_attempts WHERE username = ?")
        .bind(&username)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    let recovery_codes = generate_recovery_codes();
    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    sqlx::query("UPDATE admin_users SET totp_enabled = 1 WHERE id = ? AND totp_enabled = 0")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    for code in &recovery_codes {
        sqlx::query("INSERT INTO recovery_codes (code_hash, created_at) VALUES (?, ?)")
            .bind(hash_token(code))
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(internal_error)?;
    }
    tx.commit().await.map_err(internal_error)?;

    let token = create_session(&state, user_id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(&token));
    Ok((headers, Json(RecoveryResponse { recovery_codes })))
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Result<(HeaderMap, Json<serde_json::Value>), (StatusCode, Json<AuthError>)> {
    if !same_origin(&headers) {
        return Err(AuthError::response(StatusCode::FORBIDDEN, "请求来源校验失败"));
    }
    let username = input.username.trim();
    let since = (Utc::now() - ChronoDuration::minutes(15)).to_rfc3339();
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_login_attempts WHERE username = ? AND attempted_at > ?",
    )
    .bind(username)
    .bind(since)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)?;
    if attempts >= 10 {
        return Err(AuthError::response(StatusCode::TOO_MANY_REQUESTS, "登录失败次数过多，请 15 分钟后重试"));
    }

    let row = sqlx::query(
        "SELECT id, username, password_hash, totp_secret, totp_enabled FROM admin_users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    let Some(row) = row else {
        record_login_failure(&state, username).await?;
        tokio::time::sleep(Duration::from_millis(350)).await;
        return Err(AuthError::response(StatusCode::UNAUTHORIZED, "用户名或密码错误"));
    };

    let password_hash: String = row.get("password_hash");
    let parsed = PasswordHash::new(&password_hash)
        .map_err(|_| AuthError::response(StatusCode::INTERNAL_SERVER_ERROR, "管理员凭据数据损坏"))?;
    if Argon2::default().verify_password(input.password.as_bytes(), &parsed).is_err() {
        record_login_failure(&state, username).await?;
        tokio::time::sleep(Duration::from_millis(350)).await;
        return Err(AuthError::response(StatusCode::UNAUTHORIZED, "用户名或密码错误"));
    }
    if row.get::<i64, _>("totp_enabled") != 1 {
        return Err(AuthError::response(StatusCode::FORBIDDEN, "请先完成 TOTP 绑定"));
    }

    let user_id: i64 = row.get("id");
    let secret: String = row.get("totp_secret");
    if !verify_totp(&secret, &input.code) && !consume_recovery_code(&state, &input.code).await? {
        record_login_failure(&state, username).await?;
        tokio::time::sleep(Duration::from_millis(350)).await;
        return Err(AuthError::response(StatusCode::UNAUTHORIZED, "动态验证码或恢复码无效"));
    }
    sqlx::query("DELETE FROM auth_login_attempts WHERE username = ?")
        .bind(username)
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;

    let token = create_session(&state, user_id).await?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::SET_COOKIE, session_cookie(&token));
    Ok((response_headers, Json(serde_json::json!({"authenticated": true}))))
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(HeaderMap, StatusCode), (StatusCode, Json<AuthError>)> {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(token))
            .execute(&state.pool)
            .await
            .map_err(internal_error)?;
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("tunnelhub_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok((response_headers, StatusCode::NO_CONTENT))
}

pub async fn require_session(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers().clone();
    if !same_origin(&headers) || session_user(&state, &headers).await.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(AuthError { error: "请先登录管理界面".to_string() }),
        )
            .into_response();
    }
    next.run(request).await
}

async fn session_user(state: &AppState, headers: &HeaderMap) -> Option<i64> {
    let token = cookie_value(headers, SESSION_COOKIE)?;
    sqlx::query_scalar::<_, i64>(
        "SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(hash_token(token))
    .bind(Utc::now().to_rfc3339())
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
}

async fn create_session(
    state: &AppState,
    user_id: i64,
) -> Result<String, (StatusCode, Json<AuthError>)> {
    let mut random = [0_u8; 32];
    OsRng.fill_bytes(&mut random);
    let token = hex(&random);
    let now = Utc::now();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(now.to_rfc3339())
        .bind((now + ChronoDuration::hours(SESSION_HOURS)).to_rfc3339())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(token)
}

async fn record_login_failure(
    state: &AppState,
    username: &str,
) -> Result<(), (StatusCode, Json<AuthError>)> {
    sqlx::query("INSERT INTO auth_login_attempts (username, attempted_at) VALUES (?, ?)")
        .bind(username)
        .bind(Utc::now().to_rfc3339())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    sqlx::query("DELETE FROM auth_login_attempts WHERE attempted_at < ?")
        .bind((Utc::now() - ChronoDuration::hours(24)).to_rfc3339())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(())
}

async fn consume_recovery_code(
    state: &AppState,
    code: &str,
) -> Result<bool, (StatusCode, Json<AuthError>)> {
    let normalized = code.trim().to_ascii_uppercase();
    let result = sqlx::query("UPDATE recovery_codes SET used_at = ? WHERE code_hash = ? AND used_at IS NULL")
        .bind(Utc::now().to_rfc3339())
        .bind(hash_token(&normalized))
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(result.rows_affected() == 1)
}

fn session_cookie(token: &str) -> HeaderValue {
    let secure = if env::var("TUNNELHUB_COOKIE_SECURE").as_deref() == Ok("true") {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_HOURS * 3600,
        secure,
    )
    .parse::<HeaderValue>()
    .expect("generated session cookie is valid")
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    let Ok(expected) = url::Url::parse(&format!("{}://{}", url.scheme(), host)) else {
        return false;
    };
    url.origin().ascii_serialization() == expected.origin().ascii_serialization()
}

fn generate_secret() -> String {
    let mut bytes = [0_u8; 20];
    OsRng.fill_bytes(&mut bytes);
    base32::encode(Alphabet::RFC4648 { padding: false }, &bytes)
}

fn verify_totp(secret: &str, supplied: &str) -> bool {
    let supplied = supplied.trim();
    if supplied.len() != 6 || !supplied.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Ok(key) = base32::decode(Alphabet::RFC4648 { padding: false }, secret) else {
        return false;
    };
    let step = Utc::now().timestamp().div_euclid(30);
    (-1_i64..=1).any(|offset| {
        let counter = (step + offset) as u64;
        let Ok(mut mac) = HmacSha1::new_from_slice(&key) else {
            return false;
        };
        mac.update(&counter.to_be_bytes());
        let digest = mac.finalize().into_bytes();
        let index = (digest[digest.len() - 1] & 0x0f) as usize;
        let number = ((u32::from(digest[index] & 0x7f) << 24)
            | (u32::from(digest[index + 1]) << 16)
            | (u32::from(digest[index + 2]) << 8)
            | u32::from(digest[index + 3]))
            % 1_000_000;
        bool::from(format!("{number:06}").as_bytes().ct_eq(supplied.as_bytes()))
    })
}

fn generate_recovery_codes() -> Vec<String> {
    (0..8)
        .map(|_| {
            let mut bytes = [0_u8; 6];
            OsRng.fill_bytes(&mut bytes);
            hex(&bytes).to_ascii_uppercase()
        })
        .collect()
}

fn hash_token(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn percent_encode(value: &str) -> String {
    value.bytes().map(|byte| {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            (byte as char).to_string()
        } else {
            format!("%{byte:02X}")
        }
    }).collect()
}

fn internal_error(error: sqlx::Error) -> (StatusCode, Json<AuthError>) {
    tracing::error!(?error, "authentication database operation failed");
    AuthError::response(StatusCode::INTERNAL_SERVER_ERROR, "认证数据库操作失败")
}

fn internal_error_message() -> (StatusCode, Json<AuthError>) {
    AuthError::response(StatusCode::INTERNAL_SERVER_ERROR, "无法生成密码哈希")
}
