use crate::{core::handle::Handle, process::AsyncHandler};
use anyhow::{Context as _, Result, bail};
use clash_verge_logging::{Type, logging};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    net::{IpAddr, SocketAddr},
    path::Path,
    time::Instant,
};

mod overlay;
mod scene;

pub use overlay::start;

const LOG_ROOT: &str = "/Applications/Hearthstone/Logs";
const LOG_TAIL_BYTES: u64 = 64 * 1024;

fn match_endpoint(line: &str) -> Option<SocketAddr> {
    let (_, address) = line.split_once("Network.GotoGameServe() - address=")?;
    let address = address.split(',').next()?.trim();
    let (ip, port) = address.rsplit_once(':')?;
    let ip = ip.trim_matches(['[', ']']).parse::<IpAddr>().ok()?.to_canonical();
    let port = port.parse::<u16>().ok()?;
    matches!(port, 1119 | 3724).then_some(SocketAddr::new(ip, port))
}

fn read_match_endpoint(root: &Path) -> Option<SocketAddr> {
    let newest = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("Hearthstone_"))
        .max_by_key(|entry| entry.file_name())?;
    let mut file = File::open(newest.path().join("GameNetLogger.log")).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // Ignore a partially written final line and the first line when reading from mid-file.
    text.split_inclusive('\n')
        .skip(usize::from(start > 0))
        .filter(|line| line.ends_with('\n'))
        .filter_map(match_endpoint)
        .last()
}

fn destination_port(meta: &Value) -> Option<u16> {
    match &meta["destinationPort"] {
        Value::String(port) => port.parse().ok(),
        Value::Number(port) => port.as_u64()?.try_into().ok(),
        _ => None,
    }
}

fn select_connection(connections: &[Value], endpoint: Option<SocketAddr>) -> Result<&str> {
    let mut candidates = Vec::new();
    for connection in connections {
        let meta = &connection["metadata"];
        let path = meta["processPath"].as_str().unwrap_or_default();
        let is_game = path.ends_with("/Hearthstone.app/Contents/MacOS/Hearthstone")
            || (path.is_empty() && meta["process"].as_str() == Some("Hearthstone"));
        let host = meta["host"].as_str().unwrap_or_default();
        let port = destination_port(meta);
        let matches_target = match endpoint {
            Some(endpoint) => {
                let ip = meta["destinationIP"]
                    .as_str()
                    .and_then(|ip| ip.parse::<IpAddr>().ok())
                    .or_else(|| host.parse::<IpAddr>().ok());
                port == Some(endpoint.port()) && ip.is_some_and(|ip| ip.to_canonical() == endpoint.ip())
            }
            None => host.is_empty() || host.parse::<IpAddr>().is_ok(),
        };
        if is_game
            && meta["network"].as_str() == Some("tcp")
            && matches_target
            && matches!(port, Some(1119 | 3724))
            && let Some(id) = connection["id"].as_str().filter(|id| !id.is_empty())
        {
            candidates.push(id);
        }
    }
    // Without a logged endpoint, multiple sockets must be rejected instead of guessed by port.
    match candidates.as_slice() {
        [id] => Ok(id),
        [] if endpoint.is_some() => {
            bail!("The logged Hearthstone match connection was not found; no connection was closed.")
        }
        [] => bail!("No Hearthstone match connection. Check TUN and find-process-mode: always."),
        _ => bail!("Multiple possible match connections; no connection was closed."),
    }
}

async fn disconnect(requested_at: Instant) -> Result<()> {
    let started = Instant::now();
    let mihomo = Handle::mihomo();
    let version = mihomo.get_version().await?;
    if version.extra.get("connectionReset").and_then(Value::as_bool) != Some(true) {
        bail!("当前内核不支持快速拔线。请使用新版稳定内核，替换应用后重新安装 Clash 服务。");
    }
    let endpoint = AsyncHandler::spawn_blocking(|| read_match_endpoint(Path::new(LOG_ROOT)))
        .await
        .context("Failed to read Hearthstone match endpoint")?;
    let response = mihomo.get_connections().await?;
    let value = serde_json::to_value(response)?;
    let connections = value["connections"]
        .as_array()
        .context("Invalid connections response")?;
    let id = select_connection(connections, endpoint)?;
    let selected = Instant::now();
    let result: Result<()> = async {
        let request = mihomo
            .load_ctx()
            .build_request(reqwest::Method::DELETE, &format!("/connections/{id}/reset"))?;
        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_IMPLEMENTED {
            bail!("此连接不支持立即重置，请使用 gVisor 或 System TUN 栈。");
        }
        let response = response.error_for_status()?;
        if response
            .headers()
            .get("X-Mihomo-Connection-Reset")
            .and_then(|header| header.to_str().ok())
            != Some("true")
        {
            bail!("内核未确认游戏侧连接已重置；请检查当前运行的内核版本。");
        }
        Ok(())
    }
    .await;
    logging!(
        info,
        Type::Core,
        "Hearthstone disconnect: mode=inbound-reset, source={}, port={}, queue_ms={}, lookup_ms={}, close_ms={}, total_ms={}, ok={}",
        if endpoint.is_some() {
            "game-log"
        } else {
            "port-fallback"
        },
        connections
            .iter()
            .find(|connection| connection["id"].as_str() == Some(id))
            .and_then(|connection| destination_port(&connection["metadata"]))
            .unwrap_or_default(),
        started.duration_since(requested_at).as_millis(),
        selected.duration_since(started).as_millis(),
        selected.elapsed().as_millis(),
        requested_at.elapsed().as_millis(),
        result.is_ok(),
    );
    result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn connection(id: &str, host: &str, port: u16) -> Value {
        json!({"id":id, "metadata":{"processPath":"/Applications/Hearthstone/Hearthstone.app/Contents/MacOS/Hearthstone", "network":"tcp", "host":host,"destinationPort":port, "destinationIP":"192.0.2.10"}})
    }

    #[test]
    fn excludes_login_and_selects_match() {
        let values = [
            connection("login", "cn.actual.battlenet.com.cn", 1119),
            connection("match", "", 3724),
        ];
        assert_eq!(select_connection(&values, None).unwrap(), "match");
    }

    #[test]
    fn selects_logged_game_socket_instead_of_guessing_by_port() {
        let values = [connection("login", "", 1119), connection("match", "", 3724)];
        assert!(select_connection(&values, None).is_err());
        assert_eq!(
            select_connection(&values, Some("192.0.2.10:3724".parse().unwrap())).unwrap(),
            "match"
        );
        assert_eq!(
            select_connection(&values, Some("192.0.2.10:1119".parse().unwrap())).unwrap(),
            "login"
        );
    }

    #[test]
    fn matches_logged_address_and_refuses_to_close_another_socket() {
        let mut other = connection("other-game", "", 3724);
        other["metadata"]["destinationIP"] = json!("192.0.2.20");
        let values = [connection("match", "", 3724), other];
        assert_eq!(
            select_connection(&values, Some("192.0.2.10:3724".parse().unwrap())).unwrap(),
            "match"
        );
        assert!(select_connection(&values, Some("192.0.2.30:3724".parse().unwrap())).is_err());
        assert!(select_connection(&values, None).is_err());
    }

    #[test]
    fn refuses_missing_or_ambiguous_targets() {
        assert!(select_connection(&[connection("login", "battle.net", 1119)], None).is_err());
        let values = [
            connection("a", "", 3724),
            connection("b", "", 3724),
            connection("login", "", 1119),
        ];
        assert!(select_connection(&values, None).is_err());
        assert!(select_connection(&values, Some("192.0.2.10:3724".parse().unwrap())).is_err());
        assert_eq!(
            select_connection(&[connection("legacy", "", 1119)], None).unwrap(),
            "legacy"
        );
    }

    #[test]
    fn parses_game_endpoint_without_account_or_handshake_fields() {
        assert_eq!(
            match_endpoint(
                "I 21:03:39.5132210 Network.GotoGameServe() - address= 192.0.2.10:3724, game=1, reconnecting=True\r\n"
            ),
            Some("192.0.2.10:3724".parse().unwrap())
        );
        assert_eq!(
            match_endpoint("Network.GotoGameServe() - address= [2001:db8::1]:1119, game=1\n"),
            Some("[2001:db8::1]:1119".parse().unwrap())
        );
        assert!(match_endpoint("Login() - address= 192.0.2.10:3724, game=1\n").is_none());
        assert!(match_endpoint("Network.GotoGameServe() - address= example.com:3724, game=1\n").is_none());
    }

    #[test]
    fn reads_latest_session_and_ignores_partial_endpoint() {
        let root = std::env::temp_dir().join(format!("hearthstone-endpoint-{}", std::process::id()));
        let old = root.join("Hearthstone_2026_11_19");
        let latest = root.join("Hearthstone_2026_11_20");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&latest).unwrap();
        std::fs::write(
            old.join("GameNetLogger.log"),
            "Network.GotoGameServe() - address= 192.0.2.20:3724, game=1\n",
        )
        .unwrap();
        std::fs::write(latest.join("GameNetLogger.log"), "Network.GotoGameServe() - address= 192.0.2.10:3724, game=1\nNetwork.GotoGameServe() - address= 192.0.2.30:1119").unwrap();
        assert_eq!(read_match_endpoint(&root), Some("192.0.2.10:3724".parse().unwrap()));
        std::fs::remove_file(latest.join("GameNetLogger.log")).unwrap();
        assert_eq!(read_match_endpoint(&root), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
