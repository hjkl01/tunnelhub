use std::{env, path::Path};

use argon2::{password_hash::{PasswordHash, PasswordVerifier}, Argon2};
use rust_embed::RustEmbed;
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header::HOST, HeaderMap, HeaderValue, Request, StatusCode},
    response::Response,
};
use reqwest::Url;
use chrono::{Duration as ChronoDuration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "../frontend/dist/"]
struct FrontendAssets;

pub async fn authorize_proxy(
    State(state): State<AppState>,
    axum::extract::Path(service_id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    let service = sqlx::query_scalar::<_, String>(
        "SELECT access_mode FROM services WHERE id = ? AND enabled = 1",
    )
    .bind(&service_id)
    .fetch_optional(&state.pool)
    .await;
    let access_mode = match service {
        Ok(Some(value)) => value,
        Ok(None) => {
            record_access(&state, &service_id, None, &headers, StatusCode::NOT_FOUND).await;
            return plain(StatusCode::NOT_FOUND, "服务不存在或已停用");
        }
        Err(error) => {
            tracing::error!(?error, "Caddy 代理授权检查失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "授权检查失败");
        }
    };
    if access_mode == "public" {
        record_access(&state, &service_id, None, &headers, StatusCode::OK).await;
        return plain(StatusCode::OK, "ok");
    }

    let Some(member_id) = member_from_request(&state, &headers).await else {
        record_access(&state, &service_id, None, &headers, StatusCode::FOUND).await;
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::FOUND;
        if let Ok(location) = HeaderValue::from_str(&format!("/proxy/{service_id}/.tunnelhub/login")) {
            response.headers_mut().insert(axum::http::header::LOCATION, location);
        }
        return response;
    };
    let allowed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM service_acl a WHERE a.service_id = ? AND ((a.subject_type = 'user' AND a.subject_id = ?) OR (a.subject_type = 'group' AND a.subject_id IN (SELECT group_id FROM group_members WHERE member_id = ?)))",
    )
    .bind(&service_id)
    .bind(member_id)
    .bind(member_id)
    .fetch_one(&state.pool)
    .await;
    match allowed {
        Ok(count) if count > 0 => {
            record_access(&state, &service_id, Some(member_id), &headers, StatusCode::OK).await;
            plain(StatusCode::OK, "ok")
        }
        Ok(_) => {
            record_access(&state, &service_id, Some(member_id), &headers, StatusCode::FORBIDDEN).await;
            plain(StatusCode::FORBIDDEN, "你的账户没有访问此服务的权限")
        }
        Err(error) => {
            tracing::error!(?error, "读取服务 ACL 失败");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "授权检查失败")
        }
    }
}

fn first_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Caddy 在转发鉴权请求时会附带原始请求的 X-Forwarded-* 头，这里把它们落库，
/// 供“访问日志”页面展示。状态码表示鉴权结果（200 放行 / 302 需登录 /
/// 403 无权限 / 404 服务不存在）。
async fn record_access(
    state: &AppState,
    service_id: &str,
    member_id: Option<i64>,
    headers: &HeaderMap,
    status: StatusCode,
) {
    let method = first_header(headers, "x-forwarded-method");
    let path = first_header(headers, "x-forwarded-uri");
    let client_ip = first_header(headers, "x-forwarded-for")
        .map(|value| value.split(',').next().unwrap_or_default().trim().to_string())
        .filter(|value| !value.is_empty());
    let user_agent = first_header(headers, "user-agent");
    let created_at = Utc::now().to_rfc3339();
    if let Err(error) = sqlx::query(
        "INSERT INTO access_logs (service_id, member_id, method, path, client_ip, user_agent, status, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(service_id)
    .bind(member_id)
    .bind(method)
    .bind(path)
    .bind(client_ip)
    .bind(user_agent)
    .bind(i64::from(status.as_u16()))
    .bind(created_at)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(?error, "写入访问日志失败");
        return;
    }
    // 只保留最近 5000 条，避免日志表无限增长。
    let _ = sqlx::query("DELETE FROM access_logs WHERE id <= (SELECT MAX(id) FROM access_logs) - 5000")
        .execute(&state.pool)
        .await;
}

pub async fn handle(State(state): State<AppState>, request: Request<Body>) -> Response {
    if let Some((service_id, service_path)) = embedded_service_path(request.uri().path()) {
        return handle_embedded_service(&state, request, &service_id, &service_path).await;
    }

    let Some(host) = request.headers().get(HOST).and_then(|value| value.to_str().ok())
        .map(normalize_request_host).filter(|value| !value.is_empty()) else {
        return plain(StatusCode::BAD_REQUEST, "缺少有效的 Host 请求头");
    };

    // Each service is also reachable at `<id>.<service-domain>` where it runs at
    // the origin root, so apps that hardcode absolute paths work unmodified. This
    // only applies when a service domain is configured explicitly; path-based
    // access via `/proxy/{id}/` works on any host.
    if let Some(service_domain) = configured_service_domain() {
        if let Some(service_id) = service_id_from_host(&host, &service_domain) {
            let enabled = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM services WHERE id = ? AND enabled = 1",
            )
            .bind(&service_id)
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
            if enabled > 0 {
                let service_path = request.uri().path().to_string();
                return handle_embedded_service(&state, request, &service_id, &service_path).await;
            }
        }
    }

    // Apps that hardcode root-absolute URLs (Next.js `/_next/...`, `/api/...`,
    // client-side routes) escape the `/proxy/{id}/` prefix. The browser still
    // sends a Referer pointing at the embedded page, so forward the request to
    // the same service as if it were served at the origin root.
    if let Some(service_id) = escape_service_id(request.headers()) {
        let enabled = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM services WHERE id = ? AND enabled = 1",
        )
        .bind(&service_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        if enabled > 0 {
            let service_path = request.uri().path().to_string();
            return handle_embedded_service(&state, request, &service_id, &service_path).await;
        }
    }

    // Any other host serves the management UI. When TUNNELHUB_UI_HOST is set it
    // acts as an allowlist; leaving it unset lets TunnelHub run behind FRP on an
    // arbitrary domain without extra configuration.
    if admin_host_allowed(Some(&host)) {
        return serve_frontend(&request);
    }

    plain(StatusCode::NOT_FOUND, "该域名不在允许的管理域名列表中")
}

/// Explicit `TUNNELHUB_SERVICE_DOMAIN`, used to expose each service at
/// `<id>.<service-domain>`. When unset, services are only reachable through the
/// same-origin path `/proxy/{id}/`, which works on any host.
pub fn service_domain() -> Option<String> {
    configured_service_domain()
}

fn configured_service_domain() -> Option<String> {
    env::var("TUNNELHUB_SERVICE_DOMAIN")
        .ok()
        .map(|value| value.trim().trim_matches('.').to_string())
        .filter(|value| !value.is_empty())
}

/// Optional management-domain allowlist. When `TUNNELHUB_UI_HOST` is unset any
/// host is accepted, so TunnelHub works behind FRP without configuring a domain.
pub fn admin_host_allowed(host: Option<&str>) -> bool {
    match env::var("TUNNELHUB_UI_HOST")
        .ok()
        .map(|value| value.trim().trim_matches('.').to_string())
        .filter(|value| !value.is_empty())
    {
        None => true,
        Some(configured) => host
            .map(normalize_request_host)
            .is_some_and(|actual| actual == normalize_request_host(&configured)),
    }
}

fn service_id_from_host(host: &str, service_domain: &str) -> Option<String> {
    let service_domain = normalize_request_host(service_domain);
    let id = host.strip_suffix(&format!(".{service_domain}"))?;
    if id.is_empty() || id.contains('.') || id == "www" {
        return None;
    }
    Some(id.to_string())
}

fn embedded_service_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/proxy/")?;
    let (id, suffix) = match rest.split_once('/') {
        Some((id, suffix)) => (id, format!("/{}", suffix)),
        None => (rest, "/".to_string()),
    };
    if id.is_empty() || id.contains('/') || id == "." || id == ".." {
        return None;
    }
    Some((id.to_string(), suffix))
}

/// Extract the embedded service id from a same-origin `Referer` such as
/// `http://localhost:8088/proxy/<id>/login`.
fn escape_service_id(headers: &HeaderMap) -> Option<String> {
    let referer = headers.get("referer")?.to_str().ok()?;
    let referer_url = Url::parse(referer).ok()?;
    let request_host = headers.get(HOST)?.to_str().ok()?;
    if referer_url.host_str()? != normalize_request_host(request_host) {
        return None;
    }
    let rest = referer_url.path().strip_prefix("/proxy/")?;
    let id = rest.split('/').next().unwrap_or_default();
    if id.is_empty() || id.contains('/') {
        return None;
    }
    Some(id.to_string())
}

async fn handle_embedded_service(
    state: &AppState,
    request: Request<Body>,
    service_id: &str,
    service_path: &str,
) -> Response {
    let service = sqlx::query_as::<_, (String, String)>(
        "SELECT target_url, access_mode FROM services WHERE id = ? AND enabled = 1",
    )
    .bind(service_id)
    .fetch_optional(&state.pool)
    .await;
    let (target, access_mode) = match service {
        Ok(Some(value)) => value,
        Ok(None) => return plain(StatusCode::NOT_FOUND, "服务不存在或已停用"),
        Err(error) => {
            tracing::error!(?error, "读取嵌入式服务配置失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取服务配置失败");
        }
    };

    if service_path == "/.tunnelhub/login" && request.method() == axum::http::Method::POST {
        return member_login(state, request, service_id, &access_mode).await;
    }

    if access_mode == "restricted" {
        let member_id = member_from_request(state, request.headers()).await;
        let Some(member_id) = member_id else {
            if request.headers().get("accept").and_then(|v| v.to_str().ok()).unwrap_or("").contains("text/html")
                && request.method() == axum::http::Method::GET {
                return login_page_at(&format!("/proxy/{service_id}/.tunnelhub/login"));
            }
            return plain(StatusCode::UNAUTHORIZED, "该服务需要登录后访问");
        };
        let allowed = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM service_acl a WHERE a.service_id = ? AND ((a.subject_type = 'user' AND a.subject_id = ?) OR (a.subject_type = 'group' AND a.subject_id IN (SELECT group_id FROM group_members WHERE member_id = ?)))",
        )
        .bind(service_id)
        .bind(member_id)
        .bind(member_id)
        .fetch_one(&state.pool)
        .await;
        match allowed {
            Ok(count) if count > 0 => {}
            Ok(_) => return plain(StatusCode::FORBIDDEN, "你的账户没有访问此服务的权限"),
            Err(error) => {
                tracing::error!(?error, "检查嵌入式服务访问权限失败");
                return plain(StatusCode::INTERNAL_SERVER_ERROR, "检查服务访问权限失败");
            }
        }
    }

    let (mut parts, body) = request.into_parts();
    let path_and_query = match parts.uri.query() {
        Some(query) => format!("{service_path}?{query}"),
        None => service_path.to_string(),
    };
    parts.uri = match path_and_query.parse() {
        Ok(uri) => uri,
        Err(_) => return plain(StatusCode::BAD_REQUEST, "请求路径无效"),
    };
    let response = forward(state, Request::from_parts(parts, body), &target).await;
    rewrite_embedded_response(response, service_id, &target)
}

fn rewrite_embedded_response(mut response: Response, service_id: &str, target: &str) -> Response {
    if let Some(location) = response.headers().get(axum::http::header::LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string) {
        if let Ok(base) = Url::parse(target) {
            if let Ok(resolved) = base.join(&location) {
                if resolved.origin() == base.origin() {
                    let mut path = format!("/proxy/{service_id}{}", resolved.path());
                    if let Some(query) = resolved.query() {
                        path.push('?');
                        path.push_str(query);
                    }
                    if let Some(fragment) = resolved.fragment() {
                        path.push('#');
                        path.push_str(fragment);
                    }
                    if let Ok(value) = HeaderValue::from_str(&path) {
                        response.headers_mut().insert(axum::http::header::LOCATION, value);
                    }
                }
            }
        }
    }

    let cookies: Vec<String> = response.headers().get_all(axum::http::header::SET_COOKIE)
        .iter().filter_map(|value| value.to_str().ok().map(str::to_string)).collect();
    if !cookies.is_empty() {
        response.headers_mut().remove(axum::http::header::SET_COOKIE);
        for cookie in cookies {
            let rewritten = if cookie.to_ascii_lowercase().contains("path=") {
                cookie.split(';').map(|part| {
                    let trimmed = part.trim();
                    if trimmed.to_ascii_lowercase().starts_with("path=") {
                        format!(" Path=/proxy/{service_id}{}", trimmed.trim_start_matches("Path=").trim_start_matches("path="))
                    } else { part.to_string() }
                }).collect::<Vec<_>>().join(";")
            } else {
                format!("{cookie}; Path=/proxy/{service_id}/")
            };
            if let Ok(value) = HeaderValue::from_str(&rewritten) {
                response.headers_mut().append(axum::http::header::SET_COOKIE, value);
            }
        }
    }
    response
}

#[derive(Deserialize)]
struct MemberLogin {
    username: String,
    password: String,
    return_to: Option<String>,
}

async fn member_login(state: &AppState, request: Request<Body>, service_id: &str, access_mode: &str) -> Response {
    if access_mode != "restricted" {
        return plain(StatusCode::NOT_FOUND, "登录入口不存在");
    }
    let headers = request.headers();
    if !same_origin(headers) {
        return plain(StatusCode::FORBIDDEN, "请求来源校验失败");
    }
    let bytes = match to_bytes(request.into_body(), 16 * 1024).await {
        Ok(value) => value,
        Err(_) => return plain(StatusCode::BAD_REQUEST, "登录请求格式无效"),
    };
    let input: MemberLogin = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return plain(StatusCode::BAD_REQUEST, "登录请求格式无效"),
    };
    let username = input.username.trim();
    let since = (Utc::now() - ChronoDuration::minutes(15)).to_rfc3339();
    let attempts: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM member_login_attempts WHERE username = ? AND attempted_at > ?")
        .bind(username).bind(since).fetch_one(&state.pool).await {
        Ok(value) => value,
        Err(error) => { tracing::error!(?error, "读取登录失败记录失败"); return plain(StatusCode::INTERNAL_SERVER_ERROR, "登录失败"); }
    };
    if attempts >= 10 { return plain(StatusCode::TOO_MANY_REQUESTS, "登录失败次数过多，请 15 分钟后重试"); }
    let row = match sqlx::query("SELECT id, password_hash FROM members WHERE username = ? AND enabled = 1")
        .bind(username).fetch_optional(&state.pool).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            record_member_failure(state, username).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            return plain(StatusCode::UNAUTHORIZED, "用户名或密码错误");
        }
        Err(error) => {
            tracing::error!(?error, "读取用户凭据失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "登录失败");
        }
    };
    let password_hash: String = row.get("password_hash");
    let parsed = match PasswordHash::new(&password_hash) {
        Ok(value) => value,
        Err(_) => return plain(StatusCode::INTERNAL_SERVER_ERROR, "用户凭据数据损坏"),
    };
    if Argon2::default().verify_password(input.password.as_bytes(), &parsed).is_err() {
        record_member_failure(state, username).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        return plain(StatusCode::UNAUTHORIZED, "用户名或密码错误");
    }
    sqlx::query("DELETE FROM member_login_attempts WHERE username = ?").bind(username).execute(&state.pool).await.ok();
    let member_id: i64 = row.get("id");
    let allowed = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM service_acl a WHERE a.service_id = ? AND ((a.subject_type = 'user' AND a.subject_id = ?) OR (a.subject_type = 'group' AND a.subject_id IN (SELECT group_id FROM group_members WHERE member_id = ?)))",
    ).bind(service_id).bind(member_id).bind(member_id).fetch_one(&state.pool).await;
    match allowed {
        Ok(count) if count > 0 => {},
        Ok(_) => return plain(StatusCode::FORBIDDEN, "你的账户没有访问此服务的权限"),
        Err(error) => {
            tracing::error!(?error, "检查服务访问权限失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "检查服务访问权限失败");
        }
    }

    let mut random = [0_u8; 32];
    OsRng.fill_bytes(&mut random);
    let token = random.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let now = Utc::now();
    if let Err(error) = sqlx::query("INSERT INTO member_sessions (token_hash, member_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(token_hash).bind(member_id).bind(now.to_rfc3339())
        .bind((now + ChronoDuration::hours(8)).to_rfc3339()).execute(&state.pool).await {
        tracing::error!(?error, "创建用户访问会话失败");
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "登录失败");
    }
    let secure = if std::env::var("TUNNELHUB_COOKIE_SECURE").as_deref() == Ok("false") { "" } else { "; Secure" };
    let cookie = format!("tunnelhub_member_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=28800{secure}");
    let mut response = Response::new(Body::from(serde_json::json!({
        "authenticated": true,
        "return_to": safe_return_path(input.return_to.as_deref().unwrap_or("/"))
    }).to_string()));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(axum::http::header::SET_COOKIE, value);
    }
    response
}


async fn record_member_failure(state: &AppState, username: &str) {
    if let Err(error) = sqlx::query("INSERT INTO member_login_attempts (username, attempted_at) VALUES (?, ?)")
        .bind(username).bind(Utc::now().to_rfc3339()).execute(&state.pool).await {
        tracing::warn!(?error, "记录业务用户登录失败次数失败");
    }
    let cutoff = (Utc::now() - ChronoDuration::hours(24)).to_rfc3339();
    let _ = sqlx::query("DELETE FROM member_login_attempts WHERE attempted_at < ?").bind(cutoff).execute(&state.pool).await;
}

async fn member_from_request(state: &AppState, headers: &HeaderMap) -> Option<i64> {
    let token = headers.get(axum::http::header::COOKIE)?.to_str().ok()?.split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == "tunnelhub_member_session").then_some(value))?;
    let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    sqlx::query_scalar::<_, i64>(
        "SELECT ms.member_id FROM member_sessions ms JOIN members m ON m.id = ms.member_id WHERE ms.token_hash = ? AND ms.expires_at > ? AND m.enabled = 1",
    ).bind(token_hash).bind(Utc::now().to_rfc3339()).fetch_optional(&state.pool).await.ok().flatten()
}

fn login_page_at(login_endpoint: &str) -> Response {
    let html = r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>服务访问验证</title><style>body{font:15px system-ui;background:#f5f6f8;display:grid;place-items:center;min-height:100vh;margin:0;color:#20283a}.card{width:min(360px,calc(100% - 48px));padding:28px;background:white;border:1px solid #e4e7ed;border-radius:12px;box-shadow:0 18px 50px #1720330d}h1{font-size:22px;margin:0 0 8px}p{color:#778196;font-size:13px;line-height:1.6}label{display:block;font-size:12px;margin:16px 0 6px}input{box-sizing:border-box;width:100%;height:40px;border:1px solid #d9dee8;border-radius:6px;padding:0 10px}button{width:100%;margin-top:20px;height:42px;background:#29344b;color:white;border:0;border-radius:6px;font-weight:600}#error{color:#b4232f}</style><div class="card"><h1>需要授权访问</h1><p>登录 TunnelHub 账户以继续访问此服务。</p><form id="form"><label>用户名</label><input name="username" autocomplete="username" required><label>密码</label><input name="password" type="password" autocomplete="current-password" required><button>登录并继续</button><p id="error"></p></form></div><script>document.querySelector('#form').addEventListener('submit',async e=>{e.preventDefault();const f=new FormData(e.target);const r=await fetch('__LOGIN_ENDPOINT__',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({username:f.get('username'),password:f.get('password'),return_to:location.pathname+location.search})});if(r.ok){const d=await r.json();location.replace(d.return_to||'/')}else{document.querySelector('#error').textContent=(await r.text())||'登录失败'}})</script></html>"#;
    let html = html.replace("__LOGIN_ENDPOINT__", login_endpoint);
    let mut response = Response::new(Body::from(html));
    response.headers_mut().insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else { return false; };
    let Some(host) = headers.get(HOST).and_then(|v| v.to_str().ok()) else { return false; };
    let Ok(url) = reqwest::Url::parse(origin) else { return false; };
    let Ok(expected) = reqwest::Url::parse(&format!("{}://{}", url.scheme(), host)) else { return false; };
    url.origin().ascii_serialization() == expected.origin().ascii_serialization()
}

fn safe_return_path(value: &str) -> &str {
    if value.starts_with('/') && !value.starts_with("//") && !value.contains('\\') { value } else { "/" }
}

async fn forward(state: &AppState, request: Request<Body>, target: &str) -> Response {
    let (parts, body) = request.into_parts();
    let mut target_url = match Url::parse(target) {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(?error, "数据库中的目标 URL 无效");
            return plain(StatusCode::BAD_GATEWAY, "目标服务 URL 无效");
        }
    };

    let base_path = target_url.path().trim_end_matches('/');
    let request_path = parts.uri.path();
    let combined_path = format!("{base_path}{request_path}");
    target_url.set_path(&combined_path);
    target_url.set_query(parts.uri.query());

    let mut headers = parts.headers;
    let original_host = headers.get(HOST).cloned();
    strip_hop_by_hop_headers(&mut headers);
    headers.remove(HOST);
    if let Some(host) = original_host {
        headers.insert("x-forwarded-host", host);
    }
    if !headers.contains_key("x-forwarded-proto") {
        headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    }

    let upstream = match state
        .client
        .request(parts.method, target_url)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(?error, "请求目标服务失败");
            return plain(StatusCode::BAD_GATEWAY, "无法连接目标服务，请检查目标地址和 Docker 网络");
        }
    };

    let status = upstream.status();
    let mut response_headers = upstream.headers().clone();
    strip_hop_by_hop_headers(&mut response_headers);

    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}

fn serve_frontend(request: &Request<Body>) -> Response {
    let requested_path = request.uri().path().trim_start_matches('/');
    let requested_path = if requested_path.is_empty() { "index.html" } else { requested_path };
    let safe_path = Path::new(requested_path).components().all(|part| {
        matches!(part, std::path::Component::Normal(_))
    });
    let asset_path = if safe_path && FrontendAssets::get(requested_path).is_some() {
        requested_path
    } else {
        "index.html"
    };

    let Some(asset) = FrontendAssets::get(asset_path) else {
        return plain(StatusCode::NOT_FOUND, "前端资源不存在，请重新构建应用");
    };
    let mut response = Response::new(Body::from(asset.data.into_owned()));
    let content_type = mime_guess::from_path(asset_path).first_or_octet_stream();
    if let Ok(value) = HeaderValue::from_str(content_type.as_ref()) {
        response.headers_mut().insert(axum::http::header::CONTENT_TYPE, value);
    }
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static(if asset_path == "index.html" { "no-cache" } else { "public, max-age=31536000, immutable" }),
    );
    response
}

fn normalize_request_host(value: &str) -> String {
    let host = value.trim();
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or_default().trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or_default()
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn strip_hop_by_hop_headers(headers: &mut HeaderMap) {
    let connection_headers: Vec<String> = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(|name| name.trim().to_ascii_lowercase())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    const HOP_BY_HOP: [&str; 8] = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    for name in HOP_BY_HOP {
        headers.remove(name);
    }

    for name in connection_headers {
        headers.remove(name.as_str());
    }
    headers.remove("proxy-connection");
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    let mut response = Response::new(Body::from(message));
    *response.status_mut() = status;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().expect("static content type"),
    );
    response
}


#[cfg(test)]
mod tests {
    use super::{escape_service_id, normalize_request_host, service_id_from_host};
    use axum::http::{HeaderMap, HeaderValue};

    fn headers(referer: Option<&str>, host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(referer) = referer {
            headers.insert("referer", HeaderValue::from_str(referer).unwrap());
        }
        headers
    }

    #[test]
    fn parses_embedded_service_path() {
        assert_eq!(
            super::embedded_service_path("/proxy/abc-123/assets/app.js"),
            Some(("abc-123".to_string(), "/assets/app.js".to_string()))
        );
        assert_eq!(
            super::embedded_service_path("/proxy/abc-123"),
            Some(("abc-123".to_string(), "/".to_string()))
        );
        assert_eq!(super::embedded_service_path("/proxy/"), None);
    }

    #[test]
    fn extracts_service_from_same_origin_referer() {
        assert_eq!(
            escape_service_id(&headers(Some("http://localhost:8088/proxy/abc-123/login"), "localhost:8088")),
            Some("abc-123".to_string())
        );
        assert_eq!(
            escape_service_id(&headers(Some("http://localhost:8088/proxy/abc-123"), "localhost:8088")),
            Some("abc-123".to_string())
        );
        // Cross-origin referers are ignored.
        assert_eq!(
            escape_service_id(&headers(Some("http://evil.example/proxy/abc-123/login"), "localhost:8088")),
            None
        );
        assert_eq!(escape_service_id(&headers(None, "localhost:8088")), None);
    }

    #[test]
    fn extracts_service_from_subdomain() {
        assert_eq!(service_id_from_host("abc-123.localhost", "localhost"), Some("abc-123".to_string()));
        assert_eq!(service_id_from_host("abc-123.example.com", "example.com"), Some("abc-123".to_string()));
        assert_eq!(service_id_from_host("localhost", "localhost"), None);
        assert_eq!(service_id_from_host("a.b.localhost", "localhost"), None);
        assert_eq!(service_id_from_host("evil-example.com", "example.com"), None);
    }

    #[test]
    fn normalizes_host_case_and_port() {
        assert_eq!(normalize_request_host("App.Example.COM:8088"), "app.example.com");
        assert_eq!(normalize_request_host("app.example.com."), "app.example.com");
    }
}
