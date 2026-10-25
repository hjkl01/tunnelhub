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
) -> anyhow::Result<Child> {
    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let initial = render_config(&pool, &listen).await?;
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
    tokio::spawn(async move {
        let mut applied = initial;
        loop {
            sleep(Duration::from_secs(2)).await;
            let desired = match render_config(&sync_pool, &sync_listen).await {
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
            if let Err(error) = tokio::fs::rename(&temp_path, &sync_path).await {
                warn!(?error, "无法替换 Caddy 配置文件");
                continue;
            }
            match Command::new(&sync_binary)
                .args(["reload", "--config"])
                .arg(&sync_path)
                .args(["--adapter", "caddyfile"])
                .status()
                .await
            {
                Ok(status) if status.success() => {
                    applied = desired;
                    info!("Caddy 代理配置已同步");
                }
                Ok(status) => warn!(?status, "Caddy 拒绝新配置；将重试"),
                Err(error) => warn!(?error, "无法重载 Caddy；将重试"),
            }
        }
    });

    Ok(child)
}

async fn render_config(pool: &SqlitePool, listen: &str) -> anyhow::Result<String> {
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT id, target_url, enabled FROM services ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mut config = format!(
        "{{\n    admin 127.0.0.1:2019\n    auto_https off\n}}\n\n{} {{\n    route {{\n        @internal path /_internal/*\n        respond @internal \"Not Found\" 404\n",
        caddy_quote(listen)
    );

    for (id, target, enabled) in rows {
        if enabled == 0 {
            continue;
        }
        let Some((scheme, dial, base_path)) = upstream_parts(&target)? else {
            warn!(service_id = %id, "跳过无法解析的服务目标地址");
            continue;
        };
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
            "            reverse_proxy {scheme}://{dial} {{\n                header_up X-Forwarded-Prefix {prefix}\n                header_down Location *^/(.*)$ {prefix}/$1\n                header_down Set-Cookie \"Path=/\" \"Path={prefix}/\"\n            }}\n        }}\n"
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

fn caddy_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{caddy_quote, upstream_parts};

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
