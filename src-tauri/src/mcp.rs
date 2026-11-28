//! Streamable HTTP MCP servers chosen by the user.
//! Enabled servers are offered as tools to the agent and to auto reply.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value as Json};
use url::Url;

use crate::agent::{load_object, write_object};

const MAX_SERVERS: usize = 16;
const LIST_TIMEOUT: u64 = 8;
const CALL_TIMEOUT: u64 = 30;
const PROTOCOL: &str = "2025-03-26";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServer {
    url: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

pub(crate) struct McpTool {
    pub exposed: String,
    pub remote: String,
    pub description: String,
    pub schema: Json,
    pub link: usize,
}

struct RemoteTool {
    name: String,
    description: String,
    schema: Json,
}

pub(crate) struct Link {
    url: String,
    session: Option<String>,
    version: String,
    next_id: i64,
    agent: ureq::Agent,
}

#[tauri::command]
pub fn get_mcp_servers() -> Result<Vec<McpServer>, String> {
    let root = load_object()?;
    Ok(servers_from(&root))
}

#[tauri::command]
pub fn set_mcp_servers(servers: Vec<McpServer>) -> Result<(), String> {
    let saved = check_servers(servers)?;
    let mut root = load_object()?;
    let object = root.as_object_mut().ok_or_else(|| "Could not read settings.".to_string())?;
    object.insert("mcpServers".to_string(), serde_json::to_value(&saved).map_err(|err| err.to_string())?);
    write_object(&root)
}

pub(crate) fn servers_from(root: &Json) -> Vec<McpServer> {
    let Some(list) = root.get("mcpServers").and_then(Json::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let url = item.get("url").and_then(Json::as_str).unwrap_or("").trim();
            if url.is_empty() {
                return None;
            }
            let enabled = item.get("enabled").and_then(Json::as_bool).unwrap_or(true);
            Some(McpServer { url: url.to_string(), enabled })
        })
        .collect()
}

fn check_servers(servers: Vec<McpServer>) -> Result<Vec<McpServer>, String> {
    if servers.len() > MAX_SERVERS {
        return Err(format!("You can add up to {MAX_SERVERS} MCP servers."));
    }
    let mut seen = HashSet::new();
    let mut saved = Vec::new();
    for server in servers {
        let url = check_mcp_url(&server.url)?;
        if !seen.insert(url.clone()) {
            return Err("That MCP server is already in the list.".to_string());
        }
        saved.push(McpServer { url, enabled: server.enabled });
    }
    Ok(saved)
}

fn check_mcp_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > 500 || url.chars().any(|ch| ch.is_control()) {
        return Err("Enter an http or https URL.".to_string());
    }
    let parsed = Url::parse(url).map_err(|_| "Enter an http or https URL.".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Enter an http or https URL.".to_string());
    }
    if parsed.host_str().unwrap_or("").is_empty() {
        return Err("Enter an http or https URL.".to_string());
    }
    Ok(url.to_string())
}

/// Connect to each enabled server and list its tools. A server that does not
/// answer is skipped so the agent can still reply.
pub(crate) fn prepare() -> (Vec<Link>, Vec<McpTool>) {
    let servers = match load_object() {
        Ok(root) => servers_from(&root),
        Err(err) => {
            eprintln!("lineapp: MCP settings were not read: {err}");
            return (Vec::new(), Vec::new());
        }
    };
    let mut links = Vec::new();
    for server in servers.into_iter().filter(|server| server.enabled) {
        match Link::connect(&server.url) {
            Ok(link) => links.push(link),
            Err(err) => eprintln!("lineapp: MCP server skipped: {err}"),
        }
    }
    let mut tools = Vec::new();
    let mut used = HashSet::new();
    for (index, link) in links.iter_mut().enumerate() {
        let found = match link.list_tools() {
            Ok(found) => found,
            Err(err) => {
                eprintln!("lineapp: MCP tools were not listed: {err}");
                continue;
            }
        };
        for remote in found {
            tools.push(McpTool {
                exposed: expose_name(index, &remote.name, &mut used),
                remote: remote.name,
                description: remote.description,
                schema: remote.schema,
                link: index,
            });
        }
    }
    (links, tools)
}

pub(crate) fn invoke(links: &mut [Link], tools: &[McpTool], name: &str, arguments: &Json) -> Option<String> {
    let tool = tools.iter().find(|tool| tool.exposed == name)?;
    let arguments = object_args(arguments);
    let Some(link) = links.get_mut(tool.link) else {
        return Some(json!({ "error": "That MCP server is not connected." }).to_string());
    };
    Some(match link.call(&tool.remote, &arguments) {
        Ok(text) => text,
        Err(err) => json!({ "error": err }).to_string(),
    })
}

pub(crate) fn chip_name(tools: &[McpTool], exposed: &str) -> Option<String> {
    tools.iter().find(|tool| tool.exposed == exposed).map(|tool| format!("MCP {}", tool.remote))
}

fn object_args(arguments: &Json) -> Json {
    match arguments {
        Json::String(text) if text.trim().is_empty() => json!({}),
        Json::String(text) => serde_json::from_str::<Json>(text).ok().filter(Json::is_object).unwrap_or_else(|| json!({})),
        Json::Object(_) => arguments.clone(),
        _ => json!({}),
    }
}

impl Link {
    fn connect(url: &str) -> Result<Self, String> {
        let mut link = Self {
            url: url.to_string(),
            session: None,
            version: PROTOCOL.to_string(),
            next_id: 1,
            agent: agent_with(LIST_TIMEOUT),
        };
        let result = link.rpc("initialize", json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": "lineapp", "version": "0.1.0" }
        }))?;
        if let Some(version) = result.get("protocolVersion").and_then(Json::as_str).map(str::trim).filter(|text| !text.is_empty()) {
            link.version = version.to_string();
        }
        let _ = link.notify("notifications/initialized", json!({}));
        link.agent = agent_with(CALL_TIMEOUT);
        Ok(link)
    }

    fn list_tools(&mut self) -> Result<Vec<RemoteTool>, String> {
        let mut found = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let mut params = json!({});
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let result = self.rpc("tools/list", params)?;
            if let Some(list) = result.get("tools").and_then(Json::as_array) {
                for item in list {
                    if found.len() == 32 {
                        break;
                    }
                    let Some(name) = item.get("name").and_then(Json::as_str).map(str::trim).filter(|text| !text.is_empty()) else {
                        continue;
                    };
                    let description = clip(item.get("description").and_then(Json::as_str).unwrap_or("").trim(), 400);
                    let schema = object_schema(item.get("inputSchema").unwrap_or(&Json::Null));
                    found.push(RemoteTool { name: name.to_string(), description, schema });
                }
            }
            cursor = result.get("nextCursor").and_then(Json::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
            if cursor.is_none() || found.len() == 32 {
                break;
            }
        }
        Ok(found)
    }

    fn call(&mut self, name: &str, arguments: &Json) -> Result<String, String> {
        let result = self.rpc("tools/call", json!({ "name": name, "arguments": arguments }))?;
        Ok(render_tool_result(&result))
    }

    fn notify(&mut self, method: &str, params: Json) -> Result<(), String> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let response = self.post(&body)?;
        if response.status == 202 || response.status == 204 || response.body.trim().is_empty() {
            return Ok(());
        }
        let _ = parse_rpc_payload(&response.content_type, &response.body, 0);
        Ok(())
    }

    fn rpc(&mut self, method: &str, params: Json) -> Result<Json, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = self.post(&body)?;
        parse_rpc_payload(&response.content_type, &response.body, id)
    }

    fn post(&mut self, body: &Json) -> Result<RpcResponse, String> {
        let bytes = serde_json::to_vec(body).map_err(|err| err.to_string())?;
        let mut request = self.agent.post(&self.url).set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream");
        request = request.set("MCP-Protocol-Version", &self.version);
        if let Some(session) = &self.session {
            request = request.set("Mcp-Session-Id", session);
        }
        let response = match request.send_bytes(&bytes) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) if code == 202 || code == 204 => response,
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                return Err(http_failure(code, &text));
            }
            Err(_) => return Err("The MCP server did not answer.".to_string()),
        };
        if let Some(session) = response.header("mcp-session-id").map(str::trim).filter(|text| !text.is_empty()) {
            self.session = Some(session.to_string());
        }
        let content_type = response.header("content-type").unwrap_or("").to_string();
        let status = response.status();
        let body = response.into_string().unwrap_or_default();
        Ok(RpcResponse { status, content_type, body })
    }
}

struct RpcResponse {
    status: u16,
    content_type: String,
    body: String,
}

fn agent_with(timeout: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout)).redirects(0).build()
}

fn http_failure(code: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<Json>(body).ok();
    let message = parsed.as_ref().and_then(|value| value.pointer("/error/message")).and_then(Json::as_str).unwrap_or("").trim();
    if message.is_empty() {
        format!("The MCP server returned {code}.")
    } else {
        format!("The MCP server returned {code}: {}", clip(message, 240))
    }
}

pub(crate) fn parse_rpc_payload(content_type: &str, body: &str, id: i64) -> Result<Json, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("The MCP server returned no result.".to_string());
    }
    if content_type.contains("text/event-stream") || trimmed.starts_with("event:") || trimmed.starts_with("data:") {
        return parse_sse(trimmed, id);
    }
    let value = serde_json::from_str::<Json>(trimmed).map_err(|_| "The MCP server returned data this app could not read.".to_string())?;
    rpc_result(value, id)
}

fn parse_sse(body: &str, id: i64) -> Result<Json, String> {
    let mut data = String::new();
    let mut found: Option<Json> = None;
    let flush = |data: &mut String, found: &mut Option<Json>| {
        let text = data.trim();
        if text.is_empty() || text == "[DONE]" {
            data.clear();
            return;
        }
        if let Ok(value) = serde_json::from_str::<Json>(text) {
            let matches = rpc_id(&value) == Some(id);
            if matches || found.is_none() {
                *found = Some(value);
            }
        }
        data.clear();
    };
    for line in body.lines() {
        if line.is_empty() {
            flush(&mut data, &mut found);
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    flush(&mut data, &mut found);
    let Some(value) = found else {
        return Err("The MCP server returned no result.".to_string());
    };
    rpc_result(value, id)
}

fn rpc_id(value: &Json) -> Option<i64> {
    value.get("id").and_then(Json::as_i64).or_else(|| value.get("id").and_then(Json::as_str).and_then(|text| text.parse().ok()))
}

fn rpc_result(value: Json, _id: i64) -> Result<Json, String> {
    if let Some(error) = value.get("error") {
        let message = error.get("message").and_then(Json::as_str).unwrap_or("The MCP server returned an error.");
        return Err(clip(message.trim(), 300));
    }
    if let Some(result) = value.get("result") {
        return Ok(result.clone());
    }
    if value.get("tools").is_some() || value.get("content").is_some() {
        return Ok(value);
    }
    Err("The MCP server returned no result.".to_string())
}

pub(crate) fn render_tool_result(result: &Json) -> String {
    let error = result.get("isError") == Some(&Json::Bool(true));
    let mut text = String::new();
    if let Some(parts) = result.get("content").and_then(Json::as_array) {
        for part in parts {
            let piece = match part.get("type").and_then(Json::as_str) {
                Some("text") => part.get("text").and_then(Json::as_str).unwrap_or("").trim().to_string(),
                Some("resource") => part.pointer("/resource/text").and_then(Json::as_str).unwrap_or("").trim().to_string(),
                _ => String::new(),
            };
            if piece.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&piece);
        }
    }
    if text.is_empty() {
        text = clip(&result.to_string(), 4000);
    }
    let text = clip(&text, 6000);
    if error {
        format!("Tool error: {text}")
    } else {
        text
    }
}

pub(crate) fn object_schema(value: &Json) -> Json {
    let mut schema = if value.is_object() { value.clone() } else { json!({ "type": "object", "properties": {} }) };
    if schema.get("type").and_then(Json::as_str) != Some("object") {
        schema["type"] = json!("object");
    }
    if !schema.get("properties").is_some_and(Json::is_object) {
        schema["properties"] = json!({});
    }
    if schema.to_string().len() > 12_000 {
        return json!({ "type": "object", "properties": {} });
    }
    schema
}

pub(crate) fn expose_name(index: usize, remote: &str, used: &mut HashSet<String>) -> String {
    let mut raw = String::new();
    for ch in remote.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            raw.push(ch);
        } else {
            raw.push('_');
        }
    }
    if raw.is_empty() {
        raw.push_str("tool");
    }
    let prefix = format!("mcp{index}_");
    let room = 64usize.saturating_sub(prefix.len());
    raw.truncate(room.max(1));
    let mut name = format!("{prefix}{raw}");
    name.truncate(64);
    let base = name.clone();
    let mut extra = 2u32;
    while !used.insert(name.clone()) {
        let suffix = format!("_{extra}");
        let keep = 64usize.saturating_sub(suffix.len());
        name = format!("{}{suffix}", &base[..base.len().min(keep)]);
        extra += 1;
    }
    name
}

fn clip(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_toggle_stays_on_and_other_settings_remain() {
        let root = json!({
            "minimizeToTray": true,
            "mcpServers": [
                { "url": "https://tools.example/mcp" },
                { "url": "http://127.0.0.1:3000/mcp", "enabled": false }
            ]
        });
        let servers = servers_from(&root);
        assert!(servers[0].enabled);
        assert!(!servers[1].enabled);
        assert_eq!(servers[0].url, "https://tools.example/mcp");
        assert!(check_servers(vec![McpServer { url: "https://tools.example/mcp".into(), enabled: true }]).is_ok());
        assert!(check_servers(vec![
            McpServer { url: "https://tools.example/mcp".into(), enabled: true },
            McpServer { url: "https://tools.example/mcp".into(), enabled: false },
        ]).is_err());
        assert!(check_mcp_url("file:///tmp/mcp").is_err());
    }

    #[test]
    fn tool_names_are_safe_and_results_keep_text() {
        let mut used = HashSet::new();
        let first = expose_name(0, "search docs", &mut used);
        let second = expose_name(0, "search docs", &mut used);
        assert_eq!(first, "mcp0_search_docs");
        assert_ne!(first, second);
        assert!(second.starts_with("mcp0_"));
        assert!(second.len() <= 64);
        let result = render_tool_result(&json!({
            "content": [{ "type": "text", "text": "hello" }, { "type": "text", "text": "there" }],
            "isError": false
        }));
        assert_eq!(result, "hello\nthere");
        let failed = render_tool_result(&json!({ "content": [{ "type": "text", "text": "nope" }], "isError": true }));
        assert_eq!(failed, "Tool error: nope");
        let payload = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n\n";
        let parsed = parse_rpc_payload("text/event-stream", payload, 2).unwrap();
        assert!(parsed.get("tools").unwrap().as_array().unwrap().is_empty());
    }
}
