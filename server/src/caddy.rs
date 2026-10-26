use std::{path::PathBuf, time::Duration};

use sqlx::SqlitePool;
use tokio::{process::{Child, Command}, time::sleep};
use tracing::{info, warn};
use url::Url;

pub async fn start(
    pool: SqlitePool,
    binary: String,
    listen: String,
    config_path: PathBuf,
    service_domain: Option<String>,
) -> anyhow::Result<Child> {
    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let initial = render_config(&pool, &listen, service_domain.as_deref()).await?;
    tokio::fs::write(&config_path, &initial).await?;

    let child = Command::new(&binary)
        .args(["run", "--config"])
        .arg(&config_path)
        .args(["--adapter", "caddyfile"])
        .kill_on_drop(true)
        .spawn()?;

    let sync_pool = pool.clone();
    let sync_binary = binary.clone();
    let sync_path = config_path.clone();
    let sync_listen = listen.clone();
    let sync_domain = service_domain.clone();
    tokio::spawn(async move {
        let mut applied = initial;
        loop {
            sleep(Duration::from_secs(2)).await;
            let desired = match render_config(&sync_pool, &sync_listen, sync_domain.as_deref()).await {
                Ok(value) => value,
                Err(error) => {
                    warn!(?error, "无法生成 Caddy 配置");
                    continue;
                }
            };
            if desired == applied {
                continue;
            }
            let temp_path = sync_path.with_extension("Caddyfile.next");
            if let Err(error) = tokio::fs::write(&temp_path, &desired).await {
                warn!(?error, "无法写入待应用的 Caddy 配置");
                continue;
            }

            // Validate the candidate before replacing the active Caddyfile. A bad
            // generated config must never overwrite the last known-good config.
            match Command::new(&sync_binary)
                .args(["validate", "--config"])
                .arg(&temp_path)
                .args(["--adapter", "caddyfile"])
                .output()
                .await
            {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    warn!(status = ?output.status, %stderr, %stdout, "Caddy 配置校验失败；保留旧配置");
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    continue;
                }
                Err(error) => {
                    warn!(?error, "无法执行 Caddy 配置校验；保留旧配置");
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    continue;
                }
            }

            if let Err(error) = tokio::fs::rename(&temp_path, &sync_path).await {
                warn!(?error, "无法替换 Caddy 配置文件");
                continue;
            }
            match Command::new(&sync_binary)
                .args(["reload", "--config"])
                .arg(&sync_path)
                .args(["--adapter", "caddyfile"])
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    applied = desired;
                    info!("Caddy 代理配置已同步");
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    warn!(status = ?output.status, %stderr, %stdout, "Caddy 拒绝新配置；恢复旧配置并重试");
                    if let Err(error) = tokio::fs::write(&sync_path, &applied).await {
                        warn!(?error, "无法恢复上一次有效的 Caddy 配置");
                    }
                }
                Err(error) => {
                    warn!(?error, "无法重载 Caddy；恢复旧配置并重试");
                    if let Err(write_error) = tokio::fs::write(&sync_path, &applied).await {
                        warn!(?write_error, "无法恢复上一次有效的 Caddy 配置");
                    }
                }
            }
        }
    });

    Ok(child)
}

async fn render_config(pool: &SqlitePool, listen: &str, service_domain: Option<&str>) -> anyhow::Result<String> {
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, target_url, enabled FROM services ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    // Use an explicit HTTP scheme. A bare site address can be subject to
    // Caddy's automatic HTTPS behavior depending on how the address is parsed.
    // TunnelHub terminates HTTP here; TLS should be provided by an external proxy.
    //
    // The site address must NOT contain the bind IP (e.g. "0.0.0.0"): Caddy would
    // treat it as a Host matcher, so requests carrying a real host ("localhost:8088")
    // would not match any site block and Caddy would reply with an empty 200.
    // Instead, match any host on the port and use the `bind` directive for the socket.
    let listen = listen.strip_prefix("http://").unwrap_or(listen);
    let (bind_host, port) = split_listen(listen);
    let site_address = format!("http://:{port}");
    let mut config = format!(
        "{{\n    admin 127.0.0.1:2019\n    auto_https off\n}}\n\n{} {{\n",
        caddy_quote(&site_address)
    );
    if let Some(bind_host) = bind_host {
        config.push_str(&format!("    bind {bind_host}\n"));
    }
    config.push_str("    route {\n        @internal path /_internal/*\n        respond @internal \"Not Found\" 404\n");

    let mut upstreams = Vec::new();
    for (id, target, enabled) in rows {
        if enabled == 0 {
            continue;
        }
        let Some((scheme, dial, base_path)) = upstream_parts(&target)? else {
            warn!(service_id = %id, "跳过无法解析的服务目标地址");
            continue;
        };
        upstreams.push((id, scheme, dial, base_path));
    }

    // Each service is additionally served at `<id>.<service-domain>` with the
    // app running at the origin root, so apps that emit absolute URLs work as-is.
    // Only generated when a service domain is configured explicitly.
    if let Some(service_domain) = service_domain
        .map(|value| value.trim().trim_matches('.'))
        .filter(|value| !value.is_empty())
    {
        for (id, scheme, dial, _base_path) in &upstreams {
            let safe = id.replace('-', "_");
            config.push_str(&format!("        @host_{safe} host {id}.{service_domain}\n"));
            config.push_str(&format!(
                "        handle @host_{safe} {{\n            route {{\n                @login_{safe} path /proxy/{id}/.tunnelhub/login\n                handle @login_{safe} {{\n                    reverse_proxy 127.0.0.1:8081\n                }}\n                forward_auth 127.0.0.1:8081 {{\n                    uri /_internal/proxy-auth/{id}\n                }}\n                reverse_proxy {scheme}://{dial} {{\n                    header_down -X-Frame-Options\n                    header_down Content-Security-Policy \"frame-ancestors 'none'\" \"frame-ancestors *\"\n                }}\n            }}\n        }}\n"
            ));
        }
    }

    // Path-based proxying is registered first so prefixed requests always win
    // over the root-path fallback below.
    for (id, scheme, dial, base_path) in &upstreams {
        let prefix = format!("/proxy/{id}");
        // Keep the login endpoint on Rust; all ordinary traffic is proxied by Caddy.
        config.push_str(&format!(
            "        handle {prefix}/.tunnelhub/login {{\n            reverse_proxy 127.0.0.1:8081\n        }}\n"
        ));
        config.push_str(&format!(
            "        handle_path {prefix}/* {{\n"
        ));
        // Always authorize at request time so ACL changes take effect immediately,
        // without waiting for a Caddy configuration reload.
        config.push_str(&format!(
            "            forward_auth 127.0.0.1:8081 {{\n                uri /_internal/proxy-auth/{id}\n            }}\n"
        ));
        if !base_path.is_empty() {
            config.push_str(&format!(
                "            rewrite * {}{{uri}}\n",
                base_path
            ));
        }
        config.push_str(&format!(
            "            reverse_proxy {scheme}://{dial} {{\n                header_up X-Forwarded-Prefix {prefix}\n                header_down -X-Frame-Options\n                header_down Content-Security-Policy \"frame-ancestors 'none'\" \"frame-ancestors 'self'\"\n                header_down Location ^/(.*)$ {prefix}/$1\n                header_down Set-Cookie \"Path=/\" \"Path={prefix}/\"\n            }}\n        }}\n"
        ));
    }

    // Many web apps emit root-absolute URLs (for example Next.js `/_next/...`,
    // `/api/...` or client-side route changes). Those requests escape the
    // `/proxy/{id}/` prefix, but the browser still sends a Referer pointing at
    // the embedded page, so route them back to the same upstream as if the app
    // were served at the origin root.
    for (id, scheme, dial, _base_path) in &upstreams {
        let prefix = format!("/proxy/{id}");
        let safe = id.replace('-', "_");

        // A root-absolute document navigation can be redirected back under the
        // service prefix. This keeps subsequent page resources associated with
        // the same service instead of falling through to TunnelHub's UI.
        // SPA history.pushState() URLs cannot be identified reliably this way;
        // arbitrary apps need an origin-per-service mode or app-specific rewrites.
        config.push_str(&format!(
            "        @escape_nav_{safe} {{\n            method GET\n            header_regexp escape_nav_ref_{safe} Referer \"://[^/]+/proxy/{id}(/|$)\"\n            header Sec-Fetch-Mode navigate\n            header Sec-Fetch-Dest document\n        }}\n        handle @escape_nav_{safe} {{\n            redir {prefix}{{uri}} 302\n        }}\n"
        ));
        config.push_str(&format!(
            "        @escape_{safe} header_regexp escape_ref_{safe} Referer \"://[^/]+/proxy/{id}(/|$)\"\n"
        ));
        config.push_str(&format!(
            "        handle @escape_{safe} {{\n            forward_auth 127.0.0.1:8081 {{\n                uri /_internal/proxy-auth/{id}\n            }}\n            reverse_proxy {scheme}://{dial} {{\n                header_up X-Forwarded-Prefix {prefix}\n                header_down -X-Frame-Options\n                header_down Content-Security-Policy \\"frame-ancestors 'none'\\" \\"frame-ancestors 'self'\\"\n                header_down Location ^/(.*)$ {prefix}/$1\n            }}\n        }}\n"
        ));
    }

    config.push_str("        reverse_proxy 127.0.0.1:8081\n    }\n}\n");
    Ok(config)
}

fn upstream_parts(value: &str) -> anyhow::Result<Option<(String, String, String)>> {
    let url = Url::parse(value)?;
    let Some(host) = url.host_str() else { return Ok(None) };
    let port = url.port_or_known_default().ok_or_else(|| anyhow::anyhow!("目标 URL 缺少有效端口"))?;
    let dial = format!("{host}:{port}");
    let scheme = match url.scheme() {
        "http" => "http",
        "https" => "https",
        _ => return Ok(None),
    };
    let base_path = url.path().trim_end_matches('/').to_string();
    Ok(Some((scheme.to_string(), dial, base_path)))
}

/// Split a `host:port` listen address into an optional bind host and a port.
/// `::`/`[::]` IPv6 hosts are kept intact; an empty host (e.g. `:8080`) yields `None`.
fn split_listen(listen: &str) -> (Option<String>, String) {
    match listen.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if host.is_empty() {
                (None, port.to_string())
            } else {
                (Some(host.to_string()), port.to_string())
            }
        }
        _ => (None, listen.to_string()),
    }
}

fn caddy_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{caddy_quote, split_listen, upstream_parts};

    #[test]
    fn splits_listen_into_bind_and_port() {
        assert_eq!(split_listen("0.0.0.0:8080"), (Some("0.0.0.0".to_string()), "8080".to_string()));
        assert_eq!(split_listen("127.0.0.1:8080"), (Some("127.0.0.1".to_string()), "8080".to_string()));
        assert_eq!(split_listen(":8080"), (None, "8080".to_string()));
        assert_eq!(split_listen("[::]:8080"), (Some("::".to_string()), "8080".to_string()));
    }

    #[test]
    fn extracts_http_upstream_and_base_path() {
        assert_eq!(
            upstream_parts("http://127.0.0.1:3000/base/").unwrap(),
            Some(("http".to_string(), "127.0.0.1:3000".to_string(), "/base".to_string()))
        );
    }

    #[test]
    fn quotes_caddyfile_strings() {
        assert_eq!(caddy_quote("hello"), "\"hello\"");
    }
}
