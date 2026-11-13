use crate::{core::handle::Handle, process::AsyncHandler};
use anyhow::{Context, Result, bail};
use clash_verge_logging::{Type, logging};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::IpAddr,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager};

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

pub fn start(app: &AppHandle) -> Result<()> {
    let directory = app.path().app_cache_dir()?.join("hearthstone-overlay");
    std::fs::create_dir_all(&directory)?;
    let executable = directory.join(concat!("overlay-", env!("CARGO_PKG_VERSION")));
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/hearthstone-overlay"));
    if std::fs::read(&executable).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::write(&executable, bytes)?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(errors) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                logging!(info, Type::Core, "{}", line);
            }
        });
    }
    let input = Arc::new(Mutex::new(child.stdin.take().context("Missing overlay input")?));
    let output = child.stdout.take().context("Missing overlay output")?;
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            match line {
                Ok(line) if line == "skip" => {
                    let input = input.clone();
                    AsyncHandler::spawn(move || async move {
                        let result = disconnect().await;
                        let response = match result {
                            Ok(()) => {
                                json!({"ok": true, "message": "Match connection closed; waiting for Hearthstone to reconnect."})
                            }
                            Err(error) => json!({"ok": false, "message": error.to_string()}),
                        };
                        if let Ok(mut writer) = input.lock() {
                            let _ = writeln!(writer, "{response}");
                        }
                    });
                }
                Ok(_) => (),
                Err(_) => break,
            }
        }
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
