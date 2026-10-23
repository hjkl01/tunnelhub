use argon2::{password_hash::{PasswordHash, PasswordVerifier}, Argon2};
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

pub async fn handle(State(state): State<AppState>, request: Request<Body>) -> Response {
    let Some(host) = request.headers().get(HOST).and_then(|value| value.to_str().ok())
        .map(normalize_request_host).filter(|value| !value.is_empty()) else {
        return plain(StatusCode::BAD_REQUEST, "缺少有效的 Host 请求头");
    };

    let service = match sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, target_url, access_mode FROM services WHERE hostname = ? AND enabled = 1",
    ).bind(&host).fetch_optional(&state.pool).await {
        Ok(Some(value)) => value,
        Ok(None) => return plain(StatusCode::NOT_FOUND, "没有找到该域名对应的已启用服务"),
        Err(error) => {
            tracing::error!(?error, "读取服务路由失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取服务路由失败");
        }
    };
    let (service_id, target, access_mode) = service;

    if request.uri().path() == "/.tunnelhub/login" && request.method() == axum::http::Method::POST {
        return member_login(&state, request, &service_id, &access_mode).await;
    }

    if access_mode == "restricted" {
        let member_id = member_from_request(&state, request.headers()).await;
        let Some(member_id) = member_id else {
            if request.headers().get("accept").and_then(|v| v.to_str().ok()).unwrap_or("").contains("text/html")
                && request.method() == axum::http::Method::GET {
                return login_page();
            }
            return plain(StatusCode::UNAUTHORIZED, "该服务需要登录后访问");
        };
        let allowed = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM service_acl a WHERE a.service_id = ? AND ((a.subject_type = 'user' AND a.subject_id = ?) OR (a.subject_type = 'group' AND a.subject_id IN (SELECT group_id FROM group_members WHERE member_id = ?)))",
        ).bind(&service_id).bind(member_id).bind(member_id).fetch_one(&state.pool).await;
        match allowed {
            Ok(count) if count > 0 => {},
            Ok(_) => return plain(StatusCode::FORBIDDEN, "你的账户没有访问此服务的权限"),
            Err(error) => {
                tracing::error!(?error, "检查服务访问权限失败");
                return plain(StatusCode::INTERNAL_SERVER_ERROR, "检查服务访问权限失败");
            }
        }
    }
    forward(&state, request, &target).await
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
    let row = match sqlx::query("SELECT id, password_hash FROM members WHERE username = ? AND enabled = 1")
        .bind(input.username.trim()).fetch_optional(&state.pool).await {
        Ok(Some(value)) => value,
        Ok(None) => {
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
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        return plain(StatusCode::UNAUTHORIZED, "用户名或密码错误");
    }
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

async fn member_from_request(state: &AppState, headers: &HeaderMap) -> Option<i64> {
    let token = headers.get(axum::http::header::COOKIE)?.to_str().ok()?.split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == "tunnelhub_member_session").then_some(value))?;
    let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    sqlx::query_scalar::<_, i64>(
        "SELECT ms.member_id FROM member_sessions ms JOIN members m ON m.id = ms.member_id WHERE ms.token_hash = ? AND ms.expires_at > ? AND m.enabled = 1",
    ).bind(token_hash).bind(Utc::now().to_rfc3339()).fetch_optional(&state.pool).await.ok().flatten()
}

fn login_page() -> Response {
    let html = r#"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>服务访问验证</title><style>body{font:15px system-ui;background:#f5f6f8;display:grid;place-items:center;min-height:100vh;margin:0;color:#20283a}.card{width:min(360px,calc(100% - 48px));padding:28px;background:white;border:1px solid #e4e7ed;border-radius:12px;box-shadow:0 18px 50px #1720330d}h1{font-size:22px;margin:0 0 8px}p{color:#778196;font-size:13px;line-height:1.6}label{display:block;font-size:12px;margin:16px 0 6px}input{box-sizing:border-box;width:100%;height:40px;border:1px solid #d9dee8;border-radius:6px;padding:0 10px}button{width:100%;margin-top:20px;height:42px;background:#29344b;color:white;border:0;border-radius:6px;font-weight:600}#error{color:#b4232f}</style><div class="card"><h1>需要授权访问</h1><p>登录 TunnelHub 账户以继续访问此服务。</p><form id="form"><label>用户名</label><input name="username" autocomplete="username" required><label>密码</label><input name="password" type="password" autocomplete="current-password" required><button>登录并继续</button><p id="error"></p></form></div><script>document.querySelector('#form').addEventListener('submit',async e=>{e.preventDefault();const f=new FormData(e.target);const r=await fetch('/.tunnelhub/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({username:f.get('username'),password:f.get('password'),return_to:location.pathname+location.search})});if(r.ok){const d=await r.json();location.replace(d.return_to||'/')}else{document.querySelector('#error').textContent=(await r.text())||'登录失败'}})</script></html>"#;
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
    use super::normalize_request_host;

    #[test]
    fn normalizes_host_case_and_port() {
        assert_eq!(normalize_request_host("App.Example.COM:8088"), "app.example.com");
        assert_eq!(normalize_request_host("app.example.com."), "app.example.com");
    }
}
