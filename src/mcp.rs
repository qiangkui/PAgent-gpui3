//! MCP servers stored next to the program.
//!
//! The `mcpserver` folder holds one of two shapes:
//!
//! - `<mcpserver>\mcp.json` with a `mcpServers` object, the same shape other
//!   MCP clients use;
//! - `<mcpserver>\<id>\mcp.json`, one folder per server.
//!
//! Reading is deliberately forgiving: a folder that cannot be parsed is still
//! listed, with the reason, because "it does not show up" is the hardest kind of
//! problem to diagnose. Nothing here starts a server; the engine does that when
//! a question asks for the tools.

use std::path::{Path, PathBuf};

use crate::config::{AiSettings, is_server_id};

/// Hide the console window when spawning the engine on Windows.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Most servers the window will list, so a stray folder cannot flood the page.
pub const MAX_MCP_SERVERS: usize = 64;

/// Folder name used by the generated example.
pub const EXAMPLE_MCP_ID: &str = "example-server";

/// One MCP server found on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    /// Stable identity: the folder name, or the key in `mcpServers`.
    pub id: String,
    /// Name shown on the card, falling back to the id.
    pub name: String,
    /// Version from the manifest, empty when it has none.
    pub version: String,
    /// One-line description from the manifest.
    pub description: String,
    /// `stdio` or `http`.
    pub transport: String,
    /// Executable for a stdio server.
    pub command: String,
    /// Arguments passed to `command`.
    pub args: Vec<String>,
    /// Extra environment variables, in manifest order.
    pub env: Vec<(String, String)>,
    /// Endpoint for an http server.
    pub url: String,
    /// Extra HTTP headers, in manifest order.
    pub headers: Vec<(String, String)>,
    /// Directory a stdio server starts in; empty means the conversation's.
    pub cwd: String,
    /// Why the entry cannot be used; empty when it looks usable.
    pub problem: String,
    /// The `mcp.json` the entry came from.
    pub source: PathBuf,
    /// Skills (tools) the engine wrote after its last successful connection.
    pub skills: Vec<McpSkill>,
}

/// One skill a connected server offered, as written by the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpSkill {
    /// Name the server knows the tool by.
    pub name: String,
    /// Name the model calls (`mcp_<server>_<tool>`).
    pub exposed: String,
    /// Description shown to the model.
    pub description: String,
    /// Pretty-printed JSON schema of the arguments; empty when it has none.
    pub schema: String,
    /// The `skills\...json` file this came from.
    pub source: PathBuf,
}

impl McpServer {
    /// The line after the name: transport plus what it would start or call.
    pub fn detail(&self) -> String {
        if self.transport == "stdio" {
            let mut line = format!("stdio · {}", self.command);
            if !self.args.is_empty() {
                line.push(' ');
                line.push_str(&self.args.join(" "));
            }
            line
        } else if self.url.is_empty() {
            format!("{} · 缺少 url", self.transport)
        } else {
            format!("{} · {}", self.transport, self.url)
        }
    }

    /// What the engine should be handed, or `None` for a broken entry.
    pub fn wire(&self) -> Option<serde_json::Value> {
        if !self.problem.is_empty() {
            return None;
        }
        let env: serde_json::Map<String, serde_json::Value> = self
            .env
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect();
        let headers: serde_json::Map<String, serde_json::Value> = self
            .headers
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect();
        Some(serde_json::json!({
            "id": self.id,
            "name": self.name,
            "transport": self.transport,
            "command": self.command,
            "args": self.args,
            "env": env,
            "url": self.url,
            "headers": headers,
            "cwd": self.cwd,
        }))
    }
}

/// The folder the window reads.
///
/// `DSH_MCP_HOME` wins. Otherwise it is `mcpserver` in the executable's own
/// directory, so the servers travel with the program exactly like `config.json`
/// and `plugins`. If that directory cannot be written (for example a copy under
/// `Program Files`), it falls back to the settings folder.
pub fn locate_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DSH_MCP_HOME") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(AiSettings::home);
    let dir = program.join("mcpserver");
    let _ = std::fs::create_dir_all(&dir);
    if dir.is_dir() {
        return dir;
    }
    let fallback = AiSettings::home().join("mcpserver");
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Every server in `dir`, sorted by id.
pub fn scanned(dir: &Path) -> Vec<McpServer> {
    let mut found: Vec<McpServer> = Vec::new();
    let shared = dir.join("mcp.json");
    if let Ok(text) = std::fs::read_to_string(&shared)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
    {
        read_entries(&value, &shared, None, None, &mut found);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let folder = entry.file_name().to_string_lossy().to_string();
            if folder.starts_with('.') || folder == "skills" {
                continue;
            }
            let manifest = entry.path().join("mcp.json");
            let text = match std::fs::read_to_string(&manifest) {
                Ok(text) => text,
                Err(_) => {
                    if is_server_id(&folder) {
                        found.push(McpServer {
                            id: folder.clone(),
                            name: folder,
                            version: String::new(),
                            description: String::new(),
                            transport: "stdio".to_string(),
                            command: String::new(),
                            args: Vec::new(),
                            env: Vec::new(),
                            url: String::new(),
                            headers: Vec::new(),
                            cwd: entry.path().display().to_string(),
                            problem: "缺少 mcp.json".to_string(),
                            source: manifest,
                            skills: Vec::new(),
                        });
                    }
                    continue;
                }
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                if is_server_id(&folder) {
                    found.push(problem_server(
                        &folder,
                        &folder,
                        &manifest,
                        Some(entry.path().as_path()),
                        "mcp.json 不是合法 JSON",
                    ));
                }
                continue;
            };
            read_entries(
                &value,
                &manifest,
                Some(&folder),
                Some(&entry.path()),
                &mut found,
            );
        }
    }
    found.sort_by(|left, right| left.id.cmp(&right.id));
    found.dedup_by(|later, earlier| later.id == earlier.id);
    let mut skill_sets = read_skills(dir);
    for server in &mut found {
        if let Some(list) = skill_sets.remove(&server.id) {
            server.skills = list;
        }
    }
    found.truncate(MAX_MCP_SERVERS);
    found
}

/// Read the skill files the engine wrote under `skills\<server id>\`.
///
/// Each file carries its server id; the folder name is only a fallback. A file
/// that cannot be read is skipped rather than making the whole card fail.
fn read_skills(dir: &Path) -> std::collections::HashMap<String, Vec<McpSkill>> {
    let mut found: std::collections::HashMap<String, Vec<McpSkill>> =
        std::collections::HashMap::new();
    let root = dir.join("skills");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return found;
    };
    for folder in entries.flatten() {
        if !folder
            .file_type()
            .map(|kind| kind.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let fallback = folder.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(folder.path()) else {
            continue;
        };
        for file in files.flatten() {
            if !file.file_type().map(|kind| kind.is_file()).unwrap_or(false) {
                continue;
            }
            if file.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(file.path()) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let server = value
                .get("server")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| fallback.clone());
            let name = value
                .get("name")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    file.path()
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let text_field = |key: &str| {
                value
                    .get(key)
                    .and_then(|item| item.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            let schema = value
                .get("inputSchema")
                .map(|schema| serde_json::to_string_pretty(schema).unwrap_or_default())
                .unwrap_or_default();
            found.entry(server).or_default().push(McpSkill {
                name,
                exposed: text_field("exposed"),
                description: text_field("description"),
                schema,
                source: file.path(),
            });
        }
    }
    for list in found.values_mut() {
        list.sort_by(|left, right| left.name.cmp(&right.name));
    }
    found
}

/// Every enabled server that can be used, ready for the engine.
pub fn enabled(settings: &AiSettings) -> Vec<McpServer> {
    scanned(&locate_dir())
        .into_iter()
        .filter(|server| server.problem.is_empty() && settings.mcp_server_enabled(&server.id))
        .collect()
}

/// What one on-demand skill fetch produced.
pub struct FetchedSkills {
    /// How many tools the server reported.
    pub count: usize,
    /// The engine's one-line summary (already user-facing).
    pub text: String,
}

/// Ask the engine to connect one server, write its skills, and report the tools.
///
/// The window never speaks MCP itself: this is a one-shot `mcp-skills` process.
/// It is only used by the card's 「获取技能」 item, and is blocked while a
/// question is running so two connections never write the same folder at once.
pub fn fetch_skills(engine_dir: &Path, dir: &Path, id: &str) -> Result<FetchedSkills, String> {
    let cli = engine_dir.join("src").join("cli.ts");
    if !cli.exists() {
        return Err(format!(
            "找不到引擎：{}\n设置 DSH_FLOW_HOME 指向 dsh-flow 目录后重启界面。",
            cli.display()
        ));
    }
    let payload = serde_json::json!({
        "dir": dir.to_string_lossy(),
        "id": id,
    })
    .to_string();
    let mut command = std::process::Command::new("node");
    command
        .arg(&cli)
        .arg("mcp-skills")
        .current_dir(engine_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    crate::engine::apply_proxy_env(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| crate::engine::node_error(&error))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if let Err(error) = stdin.write_all(payload.as_bytes()) {
            let _ = child.kill();
            return Err(format!("无法把请求交给引擎：{error}"));
        }
        let _ = stdin.write_all(b"\n");
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("等待引擎失败：{error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let value = serde_json::from_str::<serde_json::Value>(&stdout).map_err(|_| {
        let mut detail = stdout.clone();
        if !stderr.is_empty() {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(&stderr);
        }
        format!("获取技能失败：{}", detail.trim())
    })?;
    if value.get("ok").and_then(|item| item.as_bool()) != Some(true) {
        let text = value
            .get("text")
            .and_then(|item| item.as_str())
            .unwrap_or("获取技能失败")
            .trim()
            .to_string();
        return Err(if text.is_empty() {
            "获取技能失败".to_string()
        } else {
            text
        });
    }
    let count = value
        .get("tools")
        .and_then(|item| item.as_array())
        .map_or(0, |items| items.len());
    let text = value
        .get("text")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(FetchedSkills { count, text })
}

/// Read either a shared `mcpServers` map or one folder's direct manifest.
fn read_entries(
    value: &serde_json::Value,
    source: &Path,
    folder_id: Option<&str>,
    default_cwd: Option<&Path>,
    into: &mut Vec<McpServer>,
) {
    if let Some(map) = value.get("mcpServers").and_then(|item| item.as_object()) {
        for (id, item) in map {
            if let Some(server) = read_server(id.trim(), item, source, default_cwd) {
                into.push(server);
            }
        }
        return;
    }
    if let Some(id) = folder_id
        && let Some(server) = read_server(id, value, source, default_cwd)
    {
        into.push(server);
    }
}

/// One `mcpServers` entry, or a placeholder when it is not usable.
fn read_server(
    id: &str,
    value: &serde_json::Value,
    source: &Path,
    default_cwd: Option<&Path>,
) -> Option<McpServer> {
    if !is_server_id(id) {
        return None;
    }
    if !value.is_object() {
        return Some(problem_server(
            id,
            id,
            source,
            default_cwd,
            "服务器配置必须是对象",
        ));
    }
    let text = |name: &str| {
        value
            .get(name)
            .and_then(|item| item.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let name = match text("name") {
        printed if printed.is_empty() => id.to_string(),
        printed => printed,
    };
    let description = text("description");
    let command = text("command");
    let url = text("url");
    let transport = {
        let declared = text("transport");
        let kind = if declared.is_empty() {
            text("type")
        } else {
            declared
        };
        let kind = kind.to_ascii_lowercase();
        if kind.is_empty() {
            if url.is_empty() {
                "stdio".to_string()
            } else {
                "http".to_string()
            }
        } else {
            kind
        }
    };
    let args = read_args(value.get("args"));
    let env = read_pairs(value.get("env"));
    let headers = read_pairs(value.get("headers"));
    let cwd = {
        let configured = text("cwd");
        if configured.is_empty() {
            default_cwd
                .map(|dir| dir.display().to_string())
                .unwrap_or_default()
        } else if Path::new(&configured).is_absolute() || default_cwd.is_none() {
            configured
        } else {
            default_cwd
                .map(|dir| dir.join(&configured).display().to_string())
                .unwrap_or(configured)
        }
    };
    let problem = if transport == "sse" {
        "暂不支持 sse 传输，请改用 stdio 或 http".to_string()
    } else if !matches!(transport.as_str(), "stdio" | "http") {
        format!("transport 只支持 stdio / http，这里是「{transport}」")
    } else if transport == "stdio" && command.is_empty() {
        "缺少 command".to_string()
    } else if transport == "http" && url.is_empty() {
        "缺少 url".to_string()
    } else {
        String::new()
    };
    Some(McpServer {
        id: id.to_string(),
        name,
        version: text("version"),
        description,
        transport,
        command,
        args,
        env,
        url,
        headers,
        cwd,
        problem,
        source: source.to_path_buf(),
        skills: Vec::new(),
    })
}

/// A placeholder card for an entry that could not even be read.
fn problem_server(
    id: &str,
    name: &str,
    source: &Path,
    cwd: Option<&Path>,
    problem: &str,
) -> McpServer {
    McpServer {
        id: id.to_string(),
        name: name.to_string(),
        version: String::new(),
        description: String::new(),
        transport: "stdio".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        url: String::new(),
        headers: Vec::new(),
        cwd: cwd.map(|dir| dir.display().to_string()).unwrap_or_default(),
        problem: problem.to_string(),
        source: source.to_path_buf(),
        skills: Vec::new(),
    }
}

/// Arguments: an array of strings, or one string kept whole.
fn read_args(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(item)) if !item.trim().is_empty() => vec![item.clone()],
        _ => Vec::new(),
    }
}

/// `env` / `headers`: string values, numbers stringified so a port still works.
fn read_pairs(value: Option<&serde_json::Value>) -> Vec<(String, String)> {
    let Some(map) = value.and_then(|item| item.as_object()) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, item)| {
            let value = match item {
                serde_json::Value::String(text) => text.clone(),
                serde_json::Value::Number(number) => number.to_string(),
                serde_json::Value::Bool(flag) => flag.to_string(),
                _ => return None,
            };
            Some((key.clone(), value))
        })
        .collect()
}

/// Delete one server: a shared-file entry is removed from the map, a folder
/// server has its folder removed (or its entry removed when that one manifest
/// holds several servers).
pub fn delete_server(dir: &Path, server: &McpServer) -> Result<(), String> {
    let source = &server.source;
    let result = if source.parent() == Some(dir) {
        remove_entry(source, &server.id)
    } else if let Ok(text) = std::fs::read_to_string(source)
        && let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(map) = value
            .get_mut("mcpServers")
            .and_then(|item| item.as_object_mut())
        && map.remove(&server.id).is_some()
    {
        write_json(source, &value)
    } else {
        let Some(folder) = source.parent() else {
            return Err(format!("找不到 {} 所在的文件夹", server.id));
        };
        if folder == dir {
            return Err("不能删除 mcpserver 根目录".to_string());
        }
        if !folder.exists() {
            Ok(())
        } else {
            std::fs::remove_dir_all(folder)
                .map_err(|error| format!("无法删除 {}：{error}", folder.display()))
        }
    };
    if result.is_ok() {
        // Skills live in their own folder; a deleted server must not leave
        // stale cards behind.
        let _ = std::fs::remove_dir_all(dir.join("skills").join(&server.id));
    }
    result
}

/// Remove one key from a shared `mcpServers` file.
fn remove_entry(file: &Path, id: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("读取 {} 失败：{error}", file.display()))?;
    let mut value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|error| format!("{} 不是合法 JSON：{error}", file.display()))?;
    let removed = value
        .get_mut("mcpServers")
        .and_then(|item| item.as_object_mut())
        .and_then(|map| map.remove(id))
        .is_some();
    if !removed {
        return Err(format!("{} 里没有「{id}」", file.display()));
    }
    write_json(file, &value)
}

/// Replace one manifest in a step, so a crash cannot leave half a file.
fn write_json(file: &Path, value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    crate::store::write_atomic(file, &text)
        .map_err(|error| format!("写入 {} 失败：{error}", file.display()))
}

/// The manifest written by 「生成示例服务器」.
pub const EXAMPLE_MCP_MANIFEST: &str = r#"{
  "name": "示例 MCP 服务器",
  "version": "1.0.0",
  "description": "一个最小的 stdio MCP 服务器，提供 echo 工具；不需要网络。",
  "transport": "stdio",
  "command": "node",
  "args": ["server.js"]
}
"#;

/// A real, dependency-free MCP server: enough for the engine to initialize,
/// list tools and call one.
pub const EXAMPLE_MCP_SERVER: &str = r#"#!/usr/bin/env node
// 最小 MCP 服务器：换行分隔的 JSON-RPC，只提供 echo。
process.stdin.setEncoding('utf8')
let buffer = ''
const tools = [
  {
    name: 'echo',
    description: '把输入原样返回，用来确认 MCP 链路可用。',
    inputSchema: {
      type: 'object',
      properties: { text: { type: 'string', description: '要返回的文字' } },
      required: ['text']
    }
  }
]
function send(message) {
  process.stdout.write(JSON.stringify(message) + '\n')
}
function handle(message) {
  if (message.id === undefined) return
  if (message.method === 'initialize') {
    send({
      jsonrpc: '2.0',
      id: message.id,
      result: {
        protocolVersion: '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'example-server', version: '1.0.0' }
      }
    })
    return
  }
  if (message.method === 'tools/list') {
    send({ jsonrpc: '2.0', id: message.id, result: { tools } })
    return
  }
  if (message.method === 'tools/call') {
    const params = message.params || {}
    if (params.name !== 'echo') {
      send({ jsonrpc: '2.0', id: message.id, error: { code: -32602, message: '未知工具：' + params.name } })
      return
    }
    const text = String((params.arguments || {}).text || '')
    send({ jsonrpc: '2.0', id: message.id, result: { content: [{ type: 'text', text }] } })
    return
  }
  if (message.method === 'ping') {
    send({ jsonrpc: '2.0', id: message.id, result: {} })
    return
  }
  send({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: '不支持：' + message.method } })
}
process.stdin.on('data', (chunk) => {
  buffer += chunk
  let end
  while ((end = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, end).trim()
    buffer = buffer.slice(end + 1)
    if (!line) continue
    try {
      handle(JSON.parse(line))
    } catch {
      // 坏行忽略，保持服务继续读取。
    }
  }
})
process.stdin.on('end', () => process.exit(0))
"#;

/// Write the example server unless that folder already exists.
///
/// @returns the folder and whether it was created.
pub fn create_example(dir: &Path) -> Result<(PathBuf, bool), String> {
    let folder = dir.join(EXAMPLE_MCP_ID);
    if folder.exists() {
        return Ok((folder, false));
    }
    std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::write(folder.join("mcp.json"), EXAMPLE_MCP_MANIFEST))
        .and_then(|()| std::fs::write(folder.join("server.js"), EXAMPLE_MCP_SERVER))
        .map_err(|error| format!("无法写入 {}：{error}", folder.display()))?;
    Ok((folder, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-flow-ui-mcp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_the_standard_shared_file() {
        let dir = temp_dir("shared");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"mcpServers":{"filesystem":{"command":"npx","args":["-y","server"],"env":{"HOME":"C:\\"}},"remote":{"url":"https://example.test/mcp","type":"http"}}}"#,
        )
        .unwrap();
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(found.len(), 2);
        let filesystem = &found[0];
        assert_eq!(filesystem.id, "filesystem");
        assert_eq!(filesystem.name, "filesystem");
        assert_eq!(filesystem.transport, "stdio");
        assert_eq!(filesystem.args, ["-y", "server"]);
        assert_eq!(filesystem.env, [("HOME".to_string(), "C:\\".to_string())]);
        assert!(filesystem.problem.is_empty());
        assert_eq!(found[1].transport, "http");
        assert_eq!(found[1].url, "https://example.test/mcp");
    }

    #[test]
    fn reads_a_folder_per_server_and_reports_broken_ones() {
        let dir = temp_dir("folders");
        let demo = dir.join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        std::fs::write(
            demo.join("mcp.json"),
            r#"{"name":"演示","description":"本地演示","command":"node","args":"server.js"}"#,
        )
        .unwrap();
        let broken = dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("mcp.json"), "{not json").unwrap();
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "broken");
        assert!(found[0].problem.contains("JSON"));
        assert_eq!(found[1].name, "演示");
        assert_eq!(found[1].args, ["server.js"]);
        assert_eq!(found[1].cwd, demo.display().to_string());
        assert!(found[1].problem.is_empty());
    }

    #[test]
    fn notes_a_missing_command_or_url() {
        let dir = temp_dir("missing");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"mcpServers":{"nocommand":{},"nourl":{"transport":"http"}}}"#,
        )
        .unwrap();
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|server| !server.problem.is_empty()));
    }

    #[test]
    fn rejects_the_legacy_sse_transport() {
        let dir = temp_dir("sse");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"mcpServers":{"legacy":{"transport":"sse","url":"https://example.test/sse"}}}"#,
        )
        .unwrap();
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 1);
        assert!(found[0].problem.contains("sse"), "{}", found[0].problem);
        assert!(found[0].wire().is_none());
    }

    #[test]
    fn deletes_a_shared_entry_without_touching_its_neighbour() {
        let dir = temp_dir("delete-shared");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"mcpServers":{"one":{"command":"node"},"two":{"command":"node"}}}"#,
        )
        .unwrap();
        let one = scanned(&dir)
            .into_iter()
            .find(|server| server.id == "one")
            .expect("one");
        delete_server(&dir, &one).unwrap();
        let after = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "two");
    }

    #[test]
    fn reads_skill_files_and_skips_the_skills_folder() {
        let dir = temp_dir("skills");
        let demo = dir.join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        std::fs::write(demo.join("mcp.json"), r#"{"command":"node"}"#).unwrap();
        let skill_dir = dir.join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("echo.json"),
            r#"{"server":"demo","name":"echo","exposed":"mcp_demo_echo","description":"返回输入","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}"#,
        )
        .unwrap();
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "demo");
        assert_eq!(found[0].skills.len(), 1);
        assert_eq!(found[0].skills[0].name, "echo");
        assert_eq!(found[0].skills[0].exposed, "mcp_demo_echo");
        assert!(found[0].skills[0].schema.contains("text"));
    }

    #[test]
    fn deletes_a_folder_server_and_only_one_entry_of_a_multi_entry_file() {
        let dir = temp_dir("delete-folder");
        let demo = dir.join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        std::fs::write(demo.join("mcp.json"), r#"{"command":"node"}"#).unwrap();
        let demo_skills = dir.join("skills").join("demo");
        std::fs::create_dir_all(&demo_skills).unwrap();
        std::fs::write(
            demo_skills.join("echo.json"),
            r#"{"server":"demo","name":"echo"}"#,
        )
        .unwrap();
        let multi = dir.join("multi");
        std::fs::create_dir_all(&multi).unwrap();
        std::fs::write(
            multi.join("mcp.json"),
            r#"{"mcpServers":{"a":{"command":"node"},"b":{"command":"node"}}}"#,
        )
        .unwrap();
        let found = scanned(&dir);
        let demo_server = found
            .iter()
            .find(|server| server.id == "demo")
            .unwrap()
            .clone();
        let a = found
            .iter()
            .find(|server| server.id == "a")
            .unwrap()
            .clone();
        delete_server(&dir, &demo_server).unwrap();
        delete_server(&dir, &a).unwrap();
        assert!(!demo.exists());
        assert!(!demo_skills.exists());
        assert!(multi.exists());
        let after = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "b");
    }

    #[test]
    fn the_example_is_written_once() {
        let dir = temp_dir("example");
        let (folder, created) = create_example(&dir).unwrap();
        assert!(created);
        assert!(folder.join("mcp.json").is_file());
        assert!(folder.join("server.js").is_file());
        let (_, again) = create_example(&dir).unwrap();
        assert!(!again);
        let found = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 1);
        assert!(found[0].problem.is_empty());
        assert_eq!(found[0].id, EXAMPLE_MCP_ID);
    }

    #[test]
    fn only_enabled_servers_reach_the_engine() {
        let dir = temp_dir("enabled");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"mcpServers":{"one":{"command":"node"},"two":{"command":"node"}}}"#,
        )
        .unwrap();
        let mut settings = AiSettings::default();
        assert!(settings.set_mcp_server("two", true));
        let all = scanned(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let picked: Vec<&McpServer> = all
            .iter()
            .filter(|server| settings.mcp_server_enabled(&server.id))
            .collect();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "two");
        assert!(picked[0].wire().is_some());
    }
}
