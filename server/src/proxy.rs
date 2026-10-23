use axum::{
    body::Body,
    extract::State,
    http::{header::HOST, HeaderMap, HeaderName, Request, StatusCode},
    response::Response,
};
use reqwest::Url;

use crate::state::AppState;

pub async fn handle(State(state): State<AppState>, request: Request<Body>) -> Response {
    let Some(host) = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_request_host)
        .filter(|value| !value.is_empty())
    else {
        return plain(StatusCode::BAD_REQUEST, "缺少有效的 Host 请求头");
    };

    let target: Option<String> = match sqlx::query_scalar(
        "SELECT target_url FROM services WHERE hostname = ? AND enabled = 1",
    )
    .bind(host)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(?error, "读取服务路由失败");
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取服务路由失败");
        }
    };

    let Some(target) = target else {
        return plain(StatusCode::NOT_FOUND, "没有找到该域名对应的已启用服务");
    };

    forward(&state, request, &target).await
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
    strip_hop_by_hop_headers(&mut headers);
    headers.remove(HOST);

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

fn normalize_request_host(value: &str) -> &str {
    let host = value.trim();
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or_default().trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or_default()
    };
    host.trim_end_matches('.')
}

fn strip_hop_by_hop_headers(headers: &mut HeaderMap) {
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

    // The Connection header can nominate additional hop-by-hop headers.
    // It is removed above; standard hop-by-hop headers are stripped regardless.
    let _ = HeaderName::from_static("proxy-connection");
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
