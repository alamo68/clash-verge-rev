use crate::core::handle::Handle;
use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use std::net::IpAddr;

mod overlay;
mod scene;

pub use overlay::start;

fn select_connection(connections: &[Value]) -> Result<&str> {
    let mut candidates = Vec::new();
    for connection in connections {
        let meta = &connection["metadata"];
        let path = meta["processPath"].as_str().unwrap_or_default();
        let is_game = path.ends_with("/Hearthstone.app/Contents/MacOS/Hearthstone")
            || (path.is_empty() && meta["process"].as_str() == Some("Hearthstone"));
        let host = meta["host"].as_str().unwrap_or_default();
        let port = meta["destinationPort"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| meta["destinationPort"].as_u64().map(|value| value.to_string()));
        if is_game
            && meta["network"].as_str() == Some("tcp")
            && (host.is_empty() || host.parse::<IpAddr>().is_ok())
            && matches!(port.as_deref(), Some("1119" | "3724"))
            && let Some(id) = connection["id"].as_str().filter(|id| !id.is_empty())
        {
            candidates.push((id, port));
        }
    }
    let has_preferred = candidates.iter().any(|(_, port)| port.as_deref() == Some("1119"));
    let ids: Vec<_> = candidates
        .iter()
        .filter(|(_, port)| !has_preferred || port.as_deref() == Some("1119"))
        .map(|(id, _)| *id)
        .collect();
    match ids.as_slice() {
        [id] => Ok(id),
        [] => bail!("No Hearthstone match connection. Check TUN and find-process-mode: always."),
        _ => bail!("Multiple possible match connections; no connection was closed."),
    }
}

async fn disconnect() -> Result<()> {
    let mihomo = Handle::mihomo();
    let response = mihomo.get_connections().await?;
    let value = serde_json::to_value(response)?;
    let connections = value["connections"]
        .as_array()
        .context("Invalid connections response")?;
    let id = select_connection(connections)?;
    mihomo.close_connection(id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn connection(id: &str, host: &str, port: u16) -> Value {
        json!({"id":id, "metadata":{"processPath":"/Applications/Hearthstone/Hearthstone.app/Contents/MacOS/Hearthstone", "network":"tcp", "host":host,"destinationPort":port}})
    }
    #[test]
    fn excludes_login_and_selects_match() {
        let values = [
            connection("login", "cn.actual.battlenet.com.cn", 1119),
            connection("match", "", 3724),
        ];
        assert_eq!(select_connection(&values).unwrap(), "match");
    }
    #[test]
    fn refuses_missing_or_ambiguous_targets() {
        assert!(select_connection(&[connection("login", "battle.net", 1119)]).is_err());
        assert!(select_connection(&[connection("a", "", 3724), connection("b", "", 3724)]).is_err());
    }
}
