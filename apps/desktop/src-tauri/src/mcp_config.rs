//! 一键把 imgtool MCP Server 写入各 Agent 客户端的用户级配置

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const SERVER_KEY: &str = "imgtool";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    ClaudeCode,
    Cursor,
    Kiro,
}

impl Client {
    pub const ALL: [Client; 3] = [Client::ClaudeCode, Client::Cursor, Client::Kiro];

    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::Cursor => "Cursor",
            Client::Kiro => "Kiro",
        }
    }

    pub fn config_path(self, home: &Path) -> PathBuf {
        match self {
            Client::ClaudeCode => home.join(".claude.json"),
            Client::Cursor => home.join(".cursor").join("mcp.json"),
            Client::Kiro => home.join(".kiro").join("settings").join("mcp.json"),
        }
    }

    pub fn entry(self, command: &str) -> Value {
        match self {
            Client::ClaudeCode => json!({ "type": "stdio", "command": command, "args": ["mcp"], "env": {} }),
            Client::Cursor => json!({ "command": command, "args": ["mcp"] }),
            Client::Kiro => json!({ "command": command, "args": ["mcp"], "disabled": false, "autoApprove": [] }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ClientStatus {
    pub id: Client,
    pub name: &'static str,
    pub config_path: String,
    /// 已写入 imgtool 配置
    pub configured: bool,
    /// 已写入且命令指向当前 sidecar
    pub up_to_date: bool,
}

fn read_config(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(format!("{} 不是 JSON 对象，已跳过以免覆盖", path.display())),
        Err(e) => Err(format!("{} 不是合法 JSON（{e}），已跳过以免覆盖", path.display())),
    }
}

pub fn status(home: &Path, command: &str) -> Vec<ClientStatus> {
    Client::ALL
        .iter()
        .map(|&c| {
            let path = c.config_path(home);
            let entry = read_config(&path).ok().and_then(|m| m.get("mcpServers")?.get(SERVER_KEY).cloned());
            ClientStatus {
                id: c,
                name: c.name(),
                config_path: path.display().to_string(),
                configured: entry.is_some(),
                up_to_date: entry.and_then(|e| e.get("command")?.as_str().map(|s| s == command)).unwrap_or(false),
            }
        })
        .collect()
}

/// 写入（或更新）mcpServers.imgtool，保留文件其余内容；首次修改前备份原文件
pub fn install(home: &Path, client: Client, command: &str) -> Result<PathBuf, String> {
    let path = client.config_path(home);
    let mut root = read_config(&path)?;
    let servers = root.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| format!("{} 中 mcpServers 不是对象", path.display()))?;
    servers.insert(SERVER_KEY.to_string(), client.entry(command));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let backup = path.with_extension(format!(
            "{}imgtool.bak",
            path.extension().and_then(|e| e.to_str()).map(|e| format!("{e}.")).unwrap_or_default()
        ));
        if !backup.exists() {
            std::fs::copy(&path, &backup).map_err(|e| format!("备份失败: {e}"))?;
        }
    }
    let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("imgtool-tmp");
    std::fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 供用户手动粘贴的通用配置片段
pub fn snippet(command: &str) -> String {
    let v = json!({ "mcpServers": { SERVER_KEY: { "command": command, "args": ["mcp"] } } });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_preserves_existing_content() {
        let home = tempfile::tempdir().unwrap();
        let path = Client::ClaudeCode.config_path(home.path());
        std::fs::write(&path, r#"{"numStartups": 3, "mcpServers": {"other": {"command": "x"}}}"#).unwrap();

        install(home.path(), Client::ClaudeCode, "/app/imgtool").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["numStartups"], 3);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["imgtool"]["command"], "/app/imgtool");
        assert!(home.path().join(".claude.json.imgtool.bak").exists());

        let s = status(home.path(), "/app/imgtool");
        assert!(s[0].configured && s[0].up_to_date);
        assert!(!status(home.path(), "/new/imgtool")[0].up_to_date);
    }

    #[test]
    fn creates_missing_config_dirs() {
        let home = tempfile::tempdir().unwrap();
        install(home.path(), Client::Kiro, "imgtool").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(Client::Kiro.config_path(home.path())).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["imgtool"]["args"][0], "mcp");
    }

    #[test]
    fn refuses_to_overwrite_invalid_json() {
        let home = tempfile::tempdir().unwrap();
        let path = Client::Cursor.config_path(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ broken").unwrap();
        assert!(install(home.path(), Client::Cursor, "imgtool").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
    }
}
