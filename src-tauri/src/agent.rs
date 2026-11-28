//! Local agent. The user picks an OpenAI-compatible, Anthropic, or Gemini endpoint.
//! Tools only read chats and messages already stored in the SQLite database.

use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::{json, Value as Json};
use url::Url;

use crate::line;
use crate::store;

const ROUNDS: usize = 5;
const HISTORY: usize = 24;
const TURN_CHARS: usize = 8_000;

#[derive(serde::Deserialize)]
pub(crate) struct TurnIn {
    role: String,
    content: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSettings {
    endpoint: String,
    model: String,
    api_key: String,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    temperature: String,
    #[serde(default)]
    top_p: String,
    #[serde(default)]
    max_tokens: String,
    #[serde(default)]
    thinking: String,
    #[serde(default)]
    extra: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Protocol {
    OpenAi,
    Anthropic,
    Gemini,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Thinking {
    Default,
    Off,
    Low,
    Medium,
    High,
}

struct Knobs {
    protocol: Protocol,
    model: String,
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<u64>,
    thinking: Thinking,
    extra: Json,
}

struct Step {
    text: String,
    calls: Vec<(String, String, Json)>,
    assistant: Json,
}

const SYSTEM: &str = "You are the assistant inside a LINE desktop app. Answer questions about chats saved in the local database on this computer. Use the tools to look up chats and messages. The database keeps only recent messages for each chat. Do not invent messages, names, or times. If a tool returns nothing, say that. You cannot send, delete, or change messages. Reply in the same language as the user's question.";

#[derive(serde::Serialize)]
pub(crate) struct AgentAnswer {
    text: String,
    calls: Vec<String>,
}

struct Tool {
    name: &'static str,
    description: &'static str,
    schema: Json,
}

fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "search_conversations",
            description: "Search saved conversations by chat name or by message text. Use this to find where something was said.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to find in a chat name or in message text." },
                    "limit": { "type": "integer", "description": "Maximum matches, from 1 to 20. Defaults to 12." }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "get_messages",
            description: "Retrieve messages saved for one chat. Pass a chat id, or the display name of a person or group.",
            schema: json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id, or the name of a person or group." },
                    "limit": { "type": "integer", "description": "How many of the newest messages to return, from 1 to 40. Defaults to 20." }
                },
                "required": ["chat"]
            }),
        },
        Tool {
            name: "list_chats",
            description: "List saved chats with a person or a group. Filter by kind and an optional name.",
            schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "person for a one-to-one chat, group for a group or room, or any." },
                    "name": { "type": "string", "description": "Optional part of the person's or group's name." },
                    "limit": { "type": "integer", "description": "Maximum chats, from 1 to 40. Defaults to 20." }
                }
            }),
        },
    ]
}

fn openai_tools() -> Json {
    Json::Array(
        tools()
            .into_iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.schema,
                    }
                })
            })
            .collect(),
    )
}

fn settings_file() -> std::path::PathBuf {
    line::config_path().with_file_name("lineapp.settings.json")
}

fn load_object() -> Result<Json, String> {
    let path = settings_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(json!({}));
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value = serde_json::from_str::<Json>(&text).map_err(|_| "Could not read settings.".to_string())?;
    if !value.is_object() {
        return Err("Could not read settings.".to_string());
    }
    Ok(value)
}

fn string_field(value: &Json, key: &str) -> String {
    value.get(key).and_then(|item| item.as_str()).unwrap_or("").trim().to_string()
}

fn write_object(value: &Json) -> Result<(), String> {
    let path = settings_file();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let body = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn check_endpoint(raw: &str) -> Result<String, String> {
    let endpoint = raw.trim();
    if endpoint.is_empty() {
        return Ok(String::new());
    }
    if endpoint.len() > 500 || endpoint.chars().any(|ch| ch.is_control()) {
        return Err("Enter an http or https URL.".to_string());
    }
    let parsed = Url::parse(endpoint).map_err(|_| "Enter an http or https URL.".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Enter an http or https URL.".to_string());
    }
    if parsed.host_str().unwrap_or("").is_empty() {
        return Err("Enter an http or https URL.".to_string());
    }
    Ok(endpoint.to_string())
}

fn check_model(raw: &str) -> Result<String, String> {
    let model = raw.trim();
    if model.chars().any(|ch| ch.is_control()) || model.len() > 200 {
        return Err("Enter a model name.".to_string());
    }
    Ok(model.to_string())
}

fn check_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if key.chars().any(|ch| ch.is_control()) || key.len() > 400 {
        return Err("That API key cannot be saved.".to_string());
    }
    Ok(key.to_string())
}

fn check_protocol(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "" | "openai" => Ok("openai".to_string()),
        "anthropic" => Ok("anthropic".to_string()),
        "gemini" => Ok("gemini".to_string()),
        _ => Err("Choose a protocol.".to_string()),
    }
}

fn check_thinking(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "" | "default" => Ok("default".to_string()),
        "off" => Ok("off".to_string()),
        "low" => Ok("low".to_string()),
        "medium" => Ok("medium".to_string()),
        "high" => Ok("high".to_string()),
        _ => Err("Choose a thinking level.".to_string()),
    }
}

fn check_optional_number(raw: &str, min: f64, max: f64) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    let value: f64 = text.parse().map_err(|_| format!("Enter a number from {min} to {max}."))?;
    if !value.is_finite() || value < min || value > max {
        return Err(format!("Enter a number from {min} to {max}."));
    }
    Ok(text.to_string())
}

fn check_optional_tokens(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    let value: u64 = text.parse().map_err(|_| "Enter a maximum from 1 to 200000.".to_string())?;
    if value == 0 || value > 200_000 {
        return Err("Enter a maximum from 1 to 200000.".to_string());
    }
    Ok(text.to_string())
}

fn check_extra(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    if text.len() > 8192 {
        return Err("That extra JSON is too long.".to_string());
    }
    let value = serde_json::from_str::<Json>(text).map_err(|_| "Extra JSON must be an object.".to_string())?;
    if !value.is_object() {
        return Err("Extra JSON must be an object.".to_string());
    }
    Ok(text.to_string())
}

fn stored_protocol(value: &Json) -> String {
    match string_field(value, "agentProtocol").as_str() {
        "anthropic" => "anthropic".to_string(),
        "gemini" => "gemini".to_string(),
        _ => "openai".to_string(),
    }
}

fn stored_thinking(value: &Json) -> String {
    match string_field(value, "agentThinking").as_str() {
        "off" => "off".to_string(),
        "low" => "low".to_string(),
        "medium" => "medium".to_string(),
        "high" => "high".to_string(),
        _ => "default".to_string(),
    }
}

#[tauri::command]
pub fn get_agent_settings() -> Result<AgentSettings, String> {
    let value = load_object()?;
    Ok(AgentSettings {
        endpoint: string_field(&value, "agentEndpoint"),
        model: string_field(&value, "agentModel"),
        api_key: string_field(&value, "agentApiKey"),
        protocol: stored_protocol(&value),
        temperature: string_field(&value, "agentTemperature"),
        top_p: string_field(&value, "agentTopP"),
        max_tokens: string_field(&value, "agentMaxTokens"),
        thinking: stored_thinking(&value),
        extra: string_field(&value, "agentExtra"),
    })
}

#[tauri::command]
pub fn set_agent_settings(settings: AgentSettings) -> Result<(), String> {
    let endpoint = check_endpoint(&settings.endpoint)?;
    let model = check_model(&settings.model)?;
    let api_key = check_key(&settings.api_key)?;
    let protocol = check_protocol(&settings.protocol)?;
    let temperature = check_optional_number(&settings.temperature, 0.0, 2.0)?;
    let top_p = check_optional_number(&settings.top_p, 0.0, 1.0)?;
    let max_tokens = check_optional_tokens(&settings.max_tokens)?;
    let thinking = check_thinking(&settings.thinking)?;
    let extra = check_extra(&settings.extra)?;
    let mut value = load_object()?;
    let Some(object) = value.as_object_mut() else {
        return Err("Could not read settings.".to_string());
    };
    object.insert("agentEndpoint".to_string(), Json::String(endpoint));
    object.insert("agentModel".to_string(), Json::String(model));
    object.insert("agentApiKey".to_string(), Json::String(api_key));
    object.insert("agentProtocol".to_string(), Json::String(protocol));
    object.insert("agentTemperature".to_string(), Json::String(temperature));
    object.insert("agentTopP".to_string(), Json::String(top_p));
    object.insert("agentMaxTokens".to_string(), Json::String(max_tokens));
    object.insert("agentThinking".to_string(), Json::String(thinking));
    object.insert("agentExtra".to_string(), Json::String(extra));
    write_object(&value)
}

#[tauri::command]
pub async fn agent_ask(messages: Vec<TurnIn>) -> Result<AgentAnswer, String> {
    tauri::async_runtime::spawn_blocking(move || ask(messages))
        .await
        .map_err(|err| err.to_string())?
}

fn ask(messages: Vec<TurnIn>) -> Result<AgentAnswer, String> {
    let settings = get_agent_settings()?;
    if settings.endpoint.is_empty() {
        return Err("Set the agent endpoint in Settings.".to_string());
    }
    if settings.model.is_empty() {
        return Err("Set the agent model in Settings.".to_string());
    }
    let knobs = resolve_knobs(&settings)?;
    let mut kept = Vec::new();
    for turn in messages.iter().rev().filter(|turn| turn.role == "user" || turn.role == "assistant") {
        let content = clip(turn.content.trim(), TURN_CHARS);
        if content.is_empty() {
            continue;
        }
        kept.push((turn.role.clone(), content));
        if kept.len() == HISTORY {
            break;
        }
    }
    kept.reverse();
    if kept.is_empty() {
        return Err("Write a message first.".to_string());
    }
    let mut transcript = seed_transcript(knobs.protocol, &kept);
    let mut calls = Vec::new();
    for _ in 0..ROUNDS {
        let body = build_request(&knobs, &transcript);
        let payload = post_chat(&settings.endpoint, &settings.api_key, &body, knobs.protocol, timeout_secs(knobs.thinking))?;
        let step = parse_reply(knobs.protocol, &payload)?;
        if step.calls.is_empty() {
            if step.text.trim().is_empty() {
                return Err("The endpoint returned an empty reply.".to_string());
            }
            return Ok(AgentAnswer { text: step.text.trim().to_string(), calls });
        }
        transcript.push(step.assistant);
        let mut results = Vec::new();
        for (_id, name, args) in &step.calls {
            calls.push(name.clone());
            results.push(run_tool(name, args));
        }
        append_tool_results(knobs.protocol, &mut transcript, &step.calls, &results);
    }
    Err("The agent stopped after several lookups without a reply.".to_string())
}

fn resolve_knobs(settings: &AgentSettings) -> Result<Knobs, String> {
    let protocol = match settings.protocol.as_str() {
        "anthropic" => Protocol::Anthropic,
        "gemini" => Protocol::Gemini,
        "openai" | "" => Protocol::OpenAi,
        _ => return Err("Choose a protocol.".to_string()),
    };
    let thinking = match settings.thinking.as_str() {
        "" | "default" => Thinking::Default,
        "off" => Thinking::Off,
        "low" => Thinking::Low,
        "medium" => Thinking::Medium,
        "high" => Thinking::High,
        _ => return Err("Choose a thinking level.".to_string()),
    };
    let extra = if settings.extra.trim().is_empty() {
        json!({})
    } else {
        let value = serde_json::from_str::<Json>(&settings.extra).map_err(|_| "Extra JSON must be an object.".to_string())?;
        if !value.is_object() {
            return Err("Extra JSON must be an object.".to_string());
        }
        value
    };
    Ok(Knobs {
        protocol,
        model: settings.model.clone(),
        temperature: optional_unit(&settings.temperature, 0.0, 2.0)?,
        top_p: optional_unit(&settings.top_p, 0.0, 1.0)?,
        max_tokens: optional_tokens(&settings.max_tokens)?,
        thinking,
        extra,
    })
}

fn optional_unit(raw: &str, min: f64, max: f64) -> Result<Option<f64>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: f64 = text.parse().map_err(|_| format!("Enter a number from {min} to {max}."))?;
    if !value.is_finite() || value < min || value > max {
        return Err(format!("Enter a number from {min} to {max}."));
    }
    Ok(Some(value))
}

fn optional_tokens(raw: &str) -> Result<Option<u64>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: u64 = text.parse().map_err(|_| "Enter a maximum from 1 to 200000.".to_string())?;
    if value == 0 || value > 200_000 {
        return Err("Enter a maximum from 1 to 200000.".to_string());
    }
    Ok(Some(value))
}

fn timeout_secs(thinking: Thinking) -> u64 {
    match thinking {
        Thinking::Off => 120,
        Thinking::Default | Thinking::Low | Thinking::Medium | Thinking::High => 180,
    }
}

fn seed_transcript(protocol: Protocol, turns: &[(String, String)]) -> Vec<Json> {
    let mut messages = Vec::new();
    if protocol == Protocol::OpenAi {
        messages.push(json!({ "role": "system", "content": SYSTEM }));
    }
    for (role, content) in turns {
        messages.push(encode_turn(protocol, role, content));
    }
    messages
}

fn encode_turn(protocol: Protocol, role: &str, content: &str) -> Json {
    match protocol {
        Protocol::Gemini => json!({
            "role": if role == "assistant" { "model" } else { "user" },
            "parts": [{ "text": content }],
        }),
        Protocol::OpenAi | Protocol::Anthropic => json!({ "role": role, "content": content }),
    }
}

fn build_request(knobs: &Knobs, messages: &[Json]) -> Json {
    let mut body = match knobs.protocol {
        Protocol::OpenAi => openai_body(knobs, messages),
        Protocol::Anthropic => anthropic_body(knobs, messages),
        Protocol::Gemini => gemini_body(knobs, messages),
    };
    deep_merge(&mut body, &knobs.extra);
    body
}

fn openai_body(knobs: &Knobs, messages: &[Json]) -> Json {
    let mut body = json!({
        "model": knobs.model,
        "messages": messages,
        "tools": openai_tools(),
        "tool_choice": "auto",
    });
    if let Some(value) = knobs.temperature {
        body["temperature"] = json!(value);
    }
    if let Some(value) = knobs.top_p {
        body["top_p"] = json!(value);
    }
    if let Some(value) = knobs.max_tokens {
        let key = match knobs.thinking {
            Thinking::Low | Thinking::Medium | Thinking::High => "max_completion_tokens",
            Thinking::Default | Thinking::Off => "max_tokens",
        };
        body[key] = json!(value);
    }
    if let Some(effort) = thinking_name(knobs.thinking) {
        body["reasoning_effort"] = json!(effort);
    }
    body
}

fn anthropic_body(knobs: &Knobs, messages: &[Json]) -> Json {
    let thinking_on = thinking_name(knobs.thinking).is_some();
    let budget = thinking_budget(knobs.thinking);
    let mut max_tokens = knobs.max_tokens.unwrap_or(4096);
    if let Some(budget) = budget {
        if max_tokens <= budget {
            max_tokens = budget + 1;
        }
    }
    let mut body = json!({
        "model": knobs.model,
        "max_tokens": max_tokens,
        "system": SYSTEM,
        "messages": messages,
        "tools": anthropic_tools(),
    });
    if let Some(value) = knobs.temperature {
        if !thinking_on {
            body["temperature"] = json!(value);
        }
    }
    if let Some(value) = knobs.top_p {
        body["top_p"] = json!(value);
    }
    if let Some(budget) = budget {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
    }
    body
}

fn gemini_body(knobs: &Knobs, messages: &[Json]) -> Json {
    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": SYSTEM }] },
        "contents": messages,
        "tools": gemini_tools(),
    });
    let mut generation = json!({});
    if let Some(value) = knobs.temperature {
        generation["temperature"] = json!(value);
    }
    if let Some(value) = knobs.top_p {
        generation["topP"] = json!(value);
    }
    if let Some(value) = knobs.max_tokens {
        generation["maxOutputTokens"] = json!(value);
    }
    match knobs.thinking {
        Thinking::Off => generation["thinkingConfig"] = json!({ "thinkingBudget": 0 }),
        Thinking::Low | Thinking::Medium | Thinking::High => {
            generation["thinkingConfig"] = json!({ "thinkingLevel": thinking_name(knobs.thinking).unwrap_or("low") });
        }
        Thinking::Default => {}
    }
    if generation.as_object().is_some_and(|object| !object.is_empty()) {
        body["generationConfig"] = generation;
    }
    body
}

fn thinking_name(thinking: Thinking) -> Option<&'static str> {
    match thinking {
        Thinking::Low => Some("low"),
        Thinking::Medium => Some("medium"),
        Thinking::High => Some("high"),
        Thinking::Default | Thinking::Off => None,
    }
}

fn thinking_budget(thinking: Thinking) -> Option<u64> {
    match thinking {
        Thinking::Low => Some(1024),
        Thinking::Medium => Some(4096),
        Thinking::High => Some(16000),
        Thinking::Default | Thinking::Off => None,
    }
}

fn anthropic_tools() -> Json {
    Json::Array(
        tools()
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.schema,
                })
            })
            .collect(),
    )
}

fn gemini_tools() -> Json {
    json!([{
        "functionDeclarations": tools().into_iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.schema,
        })).collect::<Vec<_>>(),
    }])
}

fn deep_merge(base: &mut Json, extra: &Json) {
    let (Some(base_map), Some(extra_map)) = (base.as_object_mut(), extra.as_object()) else {
        *base = extra.clone();
        return;
    };
    for (key, value) in extra_map {
        match base_map.get_mut(key) {
            Some(slot) if slot.is_object() && value.is_object() => deep_merge(slot, value),
            _ => {
                base_map.insert(key.clone(), value.clone());
            }
        }
    }
}

fn parse_reply(protocol: Protocol, payload: &Json) -> Result<Step, String> {
    if let Some(message) = payload.pointer("/error/message").and_then(|value| value.as_str()).map(str::trim).filter(|text| !text.is_empty()) {
        let has_reply = match protocol {
            Protocol::OpenAi => payload.get("choices").is_some(),
            Protocol::Anthropic => payload.get("content").is_some(),
            Protocol::Gemini => payload.get("candidates").is_some(),
        };
        if !has_reply {
            return Err(clip(message, 300));
        }
    }
    match protocol {
        Protocol::OpenAi => parse_openai(payload),
        Protocol::Anthropic => parse_anthropic(payload),
        Protocol::Gemini => parse_gemini(payload),
    }
}

fn parse_openai(payload: &Json) -> Result<Step, String> {
    let mut message = payload.pointer("/choices/0/message").cloned().filter(|value| value.is_object()).ok_or_else(|| "The endpoint did not return a reply.".to_string())?;
    if message.get("content").map(Json::is_null).unwrap_or(true) {
        message["content"] = json!("");
    }
    let text = content_text(message.get("content").unwrap_or(&Json::Null));
    let listed = message.get("tool_calls").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let mut calls = Vec::new();
    for (index, call) in listed.iter().enumerate() {
        let mut id = call.get("id").and_then(|value| value.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            id = format!("call-{}", index + 1);
            if let Some(slot) = message.get_mut("tool_calls").and_then(|value| value.get_mut(index)) {
                slot["id"] = json!(id);
            }
        }
        let name = call.pointer("/function/name").and_then(|value| value.as_str()).unwrap_or("").to_string();
        let args = call.pointer("/function/arguments").cloned().unwrap_or(Json::Null);
        calls.push((id, name, args));
    }
    Ok(Step { text, calls, assistant: message })
}

fn parse_anthropic(payload: &Json) -> Result<Step, String> {
    let content = payload.get("content").ok_or_else(|| "The endpoint did not return a reply.".to_string())?;
    let mut blocks = match content {
        Json::String(text) => vec![json!({ "type": "text", "text": text })],
        Json::Array(items) => items.clone(),
        _ => return Err("The endpoint did not return a reply.".to_string()),
    };
    let mut text = String::new();
    let mut calls = Vec::new();
    for block in &mut blocks {
        match block.get("type").and_then(|value| value.as_str()).unwrap_or("") {
            "text" => push_text(&mut text, block.get("text").and_then(|value| value.as_str()).unwrap_or("")),
            "tool_use" => {
                let mut id = block.get("id").and_then(|value| value.as_str()).unwrap_or("").to_string();
                if id.is_empty() {
                    id = format!("call-{}", calls.len() + 1);
                    block["id"] = json!(id);
                }
                let name = block.get("name").and_then(|value| value.as_str()).unwrap_or("").to_string();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                calls.push((id, name, input));
            }
            _ => {}
        }
    }
    Ok(Step { text, calls, assistant: json!({ "role": "assistant", "content": blocks }) })
}

fn parse_gemini(payload: &Json) -> Result<Step, String> {
    if payload.get("candidates").is_none() {
        if let Some(reason) = payload.pointer("/promptFeedback/blockReason").and_then(|value| value.as_str()).filter(|reason| !reason.is_empty()) {
            return Err(format!("The endpoint refused the request ({reason})."));
        }
    }
    let mut content = payload.pointer("/candidates/0/content").cloned().filter(|value| value.is_object()).ok_or_else(|| "The endpoint did not return a reply.".to_string())?;
    if content.get("role").and_then(|value| value.as_str()).unwrap_or("").is_empty() {
        content["role"] = json!("model");
    }
    let parts = content.get("parts").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let mut text = String::new();
    let mut calls = Vec::new();
    for part in &parts {
        if part.get("thought").and_then(|value| value.as_bool()).unwrap_or(false) {
            continue;
        }
        if let Some(piece) = part.get("text").and_then(|value| value.as_str()) {
            push_text(&mut text, piece);
        }
        if let Some(call) = part.get("functionCall") {
            let name = call.get("name").and_then(|value| value.as_str()).unwrap_or("").to_string();
            let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
            calls.push((format!("call-{}", calls.len() + 1), name, args));
        }
    }
    Ok(Step { text, calls, assistant: content })
}

fn push_text(text: &mut String, piece: &str) {
    if piece.is_empty() {
        return;
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(piece);
}

fn append_tool_results(protocol: Protocol, messages: &mut Vec<Json>, calls: &[(String, String, Json)], results: &[String]) {
    match protocol {
        Protocol::OpenAi => {
            for (call, result) in calls.iter().zip(results) {
                messages.push(json!({ "role": "tool", "tool_call_id": call.0, "content": result }));
            }
        }
        Protocol::Anthropic => {
            let blocks: Vec<Json> = calls
                .iter()
                .zip(results)
                .map(|(call, result)| json!({ "type": "tool_result", "tool_use_id": call.0, "content": result }))
                .collect();
            messages.push(json!({ "role": "user", "content": blocks }));
        }
        Protocol::Gemini => {
            let parts: Vec<Json> = calls
                .iter()
                .zip(results)
                .map(|(call, result)| {
                    let response = serde_json::from_str::<Json>(result).ok().filter(|value| value.is_object()).unwrap_or_else(|| json!({ "result": result }));
                    json!({ "functionResponse": { "name": call.1, "response": response } })
                })
                .collect();
            messages.push(json!({ "role": "user", "parts": parts }));
        }
    }
}

fn content_text(content: &Json) -> String {
    match content {
        Json::String(text) => text.clone(),
        Json::Array(parts) => parts.iter().filter_map(|part| part.get("text").and_then(|value| value.as_str())).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn post_chat(endpoint: &str, key: &str, body: &Json, protocol: Protocol, timeout: u64) -> Result<Json, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout)).redirects(0).build();
    let mut request = agent.post(endpoint).set("Content-Type", "application/json").set("Accept", "application/json");
    if protocol == Protocol::Anthropic {
        request = request.set("anthropic-version", "2023-06-01");
    }
    if !key.is_empty() {
        request = match protocol {
            Protocol::OpenAi => request.set("Authorization", &format!("Bearer {key}")),
            Protocol::Anthropic => request.set("x-api-key", key),
            Protocol::Gemini => request.set("x-goog-api-key", key),
        };
    }
    let bytes = serde_json::to_vec(body).map_err(|err| err.to_string())?;
    match request.send_bytes(&bytes) {
        Ok(response) => read_json(response),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            Err(http_failure(code, &text))
        }
        Err(_) => Err("The endpoint did not answer.".to_string()),
    }
}

fn read_json(response: ureq::Response) -> Result<Json, String> {
    let text = response.into_string().map_err(|_| "The endpoint returned data this app could not read.".to_string())?;
    serde_json::from_str(&text).map_err(|_| "The endpoint returned data this app could not read.".to_string())
}

fn http_failure(code: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<Json>(body).ok();
    let message = parsed.as_ref().and_then(|value| value.pointer("/error/message")).and_then(|value| value.as_str()).unwrap_or("").trim();
    if message.is_empty() {
        format!("The endpoint returned {code}.")
    } else {
        format!("The endpoint returned {code}: {}", clip(message, 240))
    }
}

fn run_tool(name: &str, arguments: &Json) -> String {
    let args = match arguments {
        Json::String(text) if text.trim().is_empty() => json!({}),
        Json::String(text) => serde_json::from_str::<Json>(text).unwrap_or(Json::Null),
        Json::Object(_) => arguments.clone(),
        Json::Null => json!({}),
        _ => Json::Null,
    };
    if !args.is_object() && arguments.as_str().is_some() {
        return json!({ "error": "Could not read the tool arguments." }).to_string();
    }
    let value = store::with_db(|conn| Ok(call_tool(conn, name, &args))).unwrap_or_else(|err| json!({ "error": err }));
    value.to_string()
}

fn call_tool(conn: &Connection, name: &str, args: &Json) -> Json {
    match name {
        "search_conversations" => search_conversations(conn, args),
        "get_messages" => get_messages(conn, args),
        "list_chats" => list_chats(conn, args),
        _ => json!({ "error": "Unknown tool." }),
    }
}

fn field(args: &Json, key: &str) -> String {
    args.get(key).and_then(|value| value.as_str()).unwrap_or("").trim().to_string()
}

fn limit_field(args: &Json, default: usize, max: usize) -> usize {
    let Some(raw) = args.get("limit") else {
        return default;
    };
    let parsed = raw
        .as_u64()
        .or_else(|| raw.as_i64().filter(|value| *value > 0).map(|value| value as u64))
        .or_else(|| raw.as_str().and_then(|text| text.trim().parse::<u64>().ok()));
    parsed.unwrap_or(default as u64).clamp(1, max as u64) as usize
}

fn like_pattern(query: &str) -> String {
    let mut pattern = String::from('%');
    for ch in query.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index == max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn iso_utc(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000) as u32;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

fn display_text(body: &str, kind: &str, sealed: bool, max: usize) -> String {
    let text = body.trim();
    if !text.is_empty() {
        return clip(text, max);
    }
    if sealed {
        return "Letter-sealed message".to_string();
    }
    match kind {
        "" => String::new(),
        "sticker" => "[sticker]".to_string(),
        "image" | "photo" => "[photo]".to_string(),
        "video" => "[video]".to_string(),
        "audio" => "[audio]".to_string(),
        "file" => "[file]".to_string(),
        "unsent" => "Unsent a message.".to_string(),
        other => format!("[{other}]"),
    }
}

fn chat_json(id: &str, name: &str, kind: &str, members: Option<i64>, last: Option<&str>, time: i64, unread: Option<i64>) -> Json {
    json!({
        "id": id,
        "name": name,
        "kind": chat_kind(kind),
        "members": members,
        "lastMessage": last.map(|text| clip(text.trim(), 180)).unwrap_or_default(),
        "time": iso_utc(time),
        "unread": unread,
    })
}

fn chat_kind(kind: &str) -> &str {
    match kind {
        "friend" => "person",
        "group" => "group",
        "room" => "group",
        "official" => "official",
        other => other,
    }
}

enum KindFilter {
    Any,
    Friend,
    Group,
    Official,
}

fn kind_filter(kind: &str) -> Result<KindFilter, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "" | "any" | "all" => Ok(KindFilter::Any),
        "person" | "friend" | "people" | "direct" => Ok(KindFilter::Friend),
        "group" | "groups" | "room" | "rooms" => Ok(KindFilter::Group),
        "official" | "officials" | "oa" => Ok(KindFilter::Official),
        _ => Err("kind must be person, group, or any.".to_string()),
    }
}

fn list_chats(conn: &Connection, args: &Json) -> Json {
    let filter = match kind_filter(&field(args, "kind")) {
        Ok(filter) => filter,
        Err(err) => return json!({ "error": err }),
    };
    let name = field(args, "name");
    let pattern = if name.is_empty() { String::new() } else { like_pattern(&name) };
    let limit = limit_field(args, 20, 40);
    let sql = match filter {
        KindFilter::Any => "SELECT id, name, kind, last_message, time, members, unread FROM chats WHERE (?1 = '' OR name LIKE ?1 ESCAPE '\\') ORDER BY time DESC LIMIT ?2",
        KindFilter::Friend => "SELECT id, name, kind, last_message, time, members, unread FROM chats WHERE kind = 'friend' AND (?1 = '' OR name LIKE ?1 ESCAPE '\\') ORDER BY time DESC LIMIT ?2",
        KindFilter::Group => "SELECT id, name, kind, last_message, time, members, unread FROM chats WHERE kind IN ('group', 'room') AND (?1 = '' OR name LIKE ?1 ESCAPE '\\') ORDER BY time DESC LIMIT ?2",
        KindFilter::Official => "SELECT id, name, kind, last_message, time, members, unread FROM chats WHERE kind = 'official' AND (?1 = '' OR name LIKE ?1 ESCAPE '\\') ORDER BY time DESC LIMIT ?2",
    };
    match read_chats(conn, sql, &pattern, limit) {
        Ok(chats) => json!({ "chats": chats }),
        Err(err) => json!({ "error": err }),
    }
}

fn read_chats(conn: &Connection, sql: &str, pattern: &str, limit: usize) -> Result<Vec<Json>, String> {
    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit as i64], |row| {
            Ok(chat_json(
                &row.get::<_, String>(0)?,
                &row.get::<_, String>(1)?,
                &row.get::<_, String>(2)?,
                row.get(5)?,
                Some(&row.get::<_, String>(3)?),
                row.get(4)?,
                row.get(6)?,
            ))
        })
        .map_err(|err| err.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|err| err.to_string())
}

fn search_conversations(conn: &Connection, args: &Json) -> Json {
    let query = field(args, "query");
    if query.is_empty() {
        return json!({ "error": "query is required." });
    }
    let limit = limit_field(args, 12, 20);
    let pattern = like_pattern(&query);
    let chats = read_chats(
        conn,
        "SELECT id, name, kind, last_message, time, members, unread FROM chats WHERE name LIKE ?1 ESCAPE '\\' ORDER BY time DESC LIMIT ?2",
        &pattern,
        limit,
    );
    let messages = search_message_rows(conn, &pattern, limit);
    match (chats, messages) {
        (Ok(chats), Ok(messages)) => json!({ "query": query, "chats": chats, "messages": messages }),
        (Err(err), _) | (_, Err(err)) => json!({ "error": err }),
    }
}

fn search_message_rows(conn: &Connection, pattern: &str, limit: usize) -> Result<Vec<Json>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.chat_id, COALESCE(c.name, ''), COALESCE(c.kind, ''), m.msg_id, m.mine, m.name, m.body, m.sealed, m.time, m.kind
             FROM messages m
             LEFT JOIN chats c ON c.id = m.chat_id
             WHERE m.body LIKE ?1 ESCAPE '\\'
             ORDER BY m.time DESC
             LIMIT ?2",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit as i64], |row| {
            let mine = row.get::<_, i64>(4)? != 0;
            let name: String = row.get(5)?;
            let body: String = row.get(6)?;
            let sealed = row.get::<_, i64>(7)? != 0;
            let kind: String = row.get(9)?;
            Ok(json!({
                "chatId": row.get::<_, String>(0)?,
                "chatName": row.get::<_, String>(1)?,
                "chatKind": chat_kind(&row.get::<_, String>(2)?),
                "messageId": row.get::<_, String>(3)?,
                "from": speaker(mine, &name),
                "time": iso_utc(row.get(8)?),
                "text": display_text(&body, &kind, sealed, 240),
            }))
        })
        .map_err(|err| err.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|err| err.to_string())
}

fn speaker(mine: bool, name: &str) -> String {
    if mine {
        "You".to_string()
    } else if name.trim().is_empty() {
        "Unknown".to_string()
    } else {
        name.trim().to_string()
    }
}

fn get_messages(conn: &Connection, args: &Json) -> Json {
    let chat = field(args, "chat");
    if chat.is_empty() {
        return json!({ "error": "chat is required." });
    }
    let limit = limit_field(args, 20, 40);
    let found = match resolve_chat(conn, &chat) {
        Ok(found) => found,
        Err(err) => return json!({ "error": err }),
    };
    let Some(found) = found else {
        return json!({ "error": "No saved chat has that id or name." });
    };
    match &found {
        Resolved::One(chat) => match message_rows(conn, &chat.id, limit) {
            Ok(messages) => json!({ "chat": chat_json(&chat.id, &chat.name, &chat.kind, chat.members, None, chat.time, None), "messages": messages }),
            Err(err) => json!({ "error": err }),
        },
        Resolved::Many(chats) => json!({
            "error": "Several chats match that name. Call again with a chat id.",
            "chats": chats,
        }),
    }
}

struct ChatHit {
    id: String,
    name: String,
    kind: String,
    members: Option<i64>,
    time: i64,
}

enum Resolved {
    One(ChatHit),
    Many(Vec<Json>),
}

fn resolve_chat(conn: &Connection, chat: &str) -> Result<Option<Resolved>, String> {
    let exact = chat_hits(conn, "SELECT id, name, kind, members, time FROM chats WHERE id = ?1", params![chat])?;
    if let Some(hit) = exact.into_iter().next() {
        return Ok(Some(Resolved::One(hit)));
    }
    let named = chat_hits(conn, "SELECT id, name, kind, members, time FROM chats WHERE lower(name) = lower(?1) ORDER BY time DESC", params![chat])?;
    if !named.is_empty() {
        return Ok(Some(collapse(named)));
    }
    let pattern = like_pattern(chat);
    let fuzzy = chat_hits(conn, "SELECT id, name, kind, members, time FROM chats WHERE name LIKE ?1 ESCAPE '\\' ORDER BY time DESC LIMIT 8", params![pattern])?;
    if fuzzy.is_empty() {
        return Ok(None);
    }
    Ok(Some(collapse(fuzzy)))
}

fn collapse(hits: Vec<ChatHit>) -> Resolved {
    if hits.len() == 1 {
        return Resolved::One(hits.into_iter().next().expect("one chat"));
    }
    Resolved::Many(hits.iter().map(|hit| chat_json(&hit.id, &hit.name, &hit.kind, hit.members, None, hit.time, None)).collect())
}

fn chat_hits(conn: &Connection, sql: &str, values: impl rusqlite::Params) -> Result<Vec<ChatHit>, String> {
    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(values, |row| {
            Ok(ChatHit {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                members: row.get(3)?,
                time: row.get(4)?,
            })
        })
        .map_err(|err| err.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|err| err.to_string())
}

fn message_rows(conn: &Connection, chat: &str, limit: usize) -> Result<Vec<Json>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT msg_id, mine, name, body, sealed, time, kind
             FROM messages
             WHERE chat_id = ?1
             ORDER BY time ASC, seq ASC",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![chat], |row| {
            let mine = row.get::<_, i64>(1)? != 0;
            let name: String = row.get(2)?;
            let body: String = row.get(3)?;
            let sealed = row.get::<_, i64>(4)? != 0;
            let kind: String = row.get(6)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "from": speaker(mine, &name),
                "time": iso_utc(row.get(5)?),
                "text": display_text(&body, &kind, sealed, 700),
            }))
        })
        .map_err(|err| err.to_string())?;
    let mut messages = rows.collect::<Result<Vec<_>, _>>().map_err(|err| err.to_string())?;
    if messages.len() > limit {
        messages = messages.split_off(messages.len() - limit);
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE chats (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                last_message TEXT NOT NULL,
                kind TEXT NOT NULL,
                time INTEGER NOT NULL,
                members INTEGER,
                unread INTEGER
            );
            CREATE TABLE messages (
                chat_id TEXT NOT NULL,
                msg_id TEXT NOT NULL,
                mine INTEGER NOT NULL,
                name TEXT NOT NULL,
                body TEXT NOT NULL,
                sealed INTEGER NOT NULL,
                time INTEGER NOT NULL,
                kind TEXT NOT NULL,
                seq INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    fn chat(conn: &Connection, id: &str, name: &str, kind: &str, last: &str, time: i64, members: Option<i64>) {
        conn.execute(
            "INSERT INTO chats (id, name, last_message, kind, time, members, unread) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![id, name, last, kind, time, members],
        )
        .unwrap();
    }

    fn message(conn: &Connection, chat: &str, id: &str, mine: bool, name: &str, body: &str, time: i64, seq: i64) {
        conn.execute(
            "INSERT INTO messages (chat_id, msg_id, mine, name, body, sealed, time, kind, seq) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, '', ?7)",
            params![chat, id, if mine { 1 } else { 0 }, name, body, time, seq],
        )
        .unwrap();
    }

    #[test]
    fn tools_are_named_and_described() {
        let names: Vec<_> = tools().into_iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["search_conversations", "get_messages", "list_chats"]);
        let exported = openai_tools();
        assert_eq!(exported.as_array().unwrap().len(), 3);
        assert_eq!(exported[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn search_finds_a_message_and_does_not_treat_wildcards_as_everything() {
        let conn = db();
        chat(&conn, "u1", "Ada", "friend", "hello", 1_700_000_000_000, None);
        chat(&conn, "c1", "Picnic", "group", "100%", 1_700_000_100_000, Some(3));
        message(&conn, "u1", "1", false, "Ada", "hello from the park", 1_700_000_000_000, 1);
        message(&conn, "c1", "2", true, "", "100% coming", 1_700_000_100_000, 1);
        let found = call_tool(&conn, "search_conversations", &json!({ "query": "park" }));
        assert_eq!(found["messages"][0]["chatName"], "Ada");
        assert_eq!(found["messages"][0]["from"], "Ada");
        let percent = call_tool(&conn, "search_conversations", &json!({ "query": "%" }));
        assert_eq!(percent["messages"].as_array().unwrap().len(), 1);
        assert_eq!(percent["messages"][0]["text"], "100% coming");
        let injected = call_tool(&conn, "search_conversations", &json!({ "query": "' OR '1'='1" }));
        assert!(injected["messages"].as_array().unwrap().is_empty());
        assert!(injected["chats"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_chats_keeps_people_and_groups_apart() {
        let conn = db();
        chat(&conn, "u1", "Ada Lovelace", "friend", "hi", 20, None);
        chat(&conn, "c1", "Ada club", "group", "yo", 30, Some(4));
        chat(&conn, "r1", "Ada room", "room", "room", 10, Some(2));
        let people = call_tool(&conn, "list_chats", &json!({ "kind": "person", "name": "Ada" }));
        assert_eq!(people["chats"].as_array().unwrap().len(), 1);
        assert_eq!(people["chats"][0]["id"], "u1");
        assert_eq!(people["chats"][0]["kind"], "person");
        let groups = call_tool(&conn, "list_chats", &json!({ "kind": "group", "name": "Ada" }));
        assert_eq!(groups["chats"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn get_messages_uses_a_name_and_asks_when_two_match() {
        let conn = db();
        chat(&conn, "u1", "Sam", "friend", "", 10, None);
        chat(&conn, "u2", "Sam", "friend", "", 20, None);
        message(&conn, "u1", "m1", false, "Sam", "first", 10, 1);
        message(&conn, "u1", "m2", true, "", "second", 11, 2);
        message(&conn, "u1", "m3", false, "Sam", "third", 12, 3);
        let ambiguous = call_tool(&conn, "get_messages", &json!({ "chat": "Sam" }));
        assert!(ambiguous["error"].as_str().unwrap().contains("Several"));
        assert_eq!(ambiguous["chats"].as_array().unwrap().len(), 2);
        let messages = call_tool(&conn, "get_messages", &json!({ "chat": "u1", "limit": 2 }));
        assert_eq!(messages["chat"]["name"], "Sam");
        assert_eq!(messages["messages"].as_array().unwrap().len(), 2);
        assert_eq!(messages["messages"][0]["text"], "second");
        assert_eq!(messages["messages"][0]["from"], "You");
        assert_eq!(messages["messages"][1]["text"], "third");
    }

    #[test]
    fn times_are_utc_and_endpoints_stay_http() {
        assert_eq!(iso_utc(0), "");
        assert_eq!(iso_utc(1_577_836_800_000), "2020-01-01T00:00:00.000Z");
        assert!(check_endpoint("http://127.0.0.1:11434/v1/chat/completions").is_ok());
        assert!(check_endpoint("file:///tmp/model").is_err());
        assert!(check_endpoint("").is_ok());
    }

    #[test]
    fn saved_agent_fields_do_not_drop_other_settings() {
        let mut value = json!({ "minimizeToTray": false, "alwaysOnTop": true });
        value.as_object_mut().unwrap().insert("agentEndpoint".into(), json!("http://127.0.0.1:11434/v1/chat/completions"));
        assert_eq!(value["minimizeToTray"], false);
        assert_eq!(value["alwaysOnTop"], true);
        assert!(check_model("local\nmodel").is_err());
    }

    fn knobs(protocol: Protocol) -> Knobs {
        Knobs {
            protocol,
            model: "demo".to_string(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            thinking: Thinking::Default,
            extra: json!({}),
        }
    }

    fn hi(protocol: Protocol) -> Vec<Json> {
        seed_transcript(protocol, &[("user".to_string(), "Hi".to_string())])
    }

    #[test]
    fn openai_omits_blank_knobs_and_uses_completion_tokens_when_thinking() {
        let blank = build_request(&knobs(Protocol::OpenAi), &hi(Protocol::OpenAi));
        assert!(blank.get("temperature").is_none());
        assert!(blank.get("top_p").is_none());
        assert!(blank.get("max_tokens").is_none());
        assert!(blank.get("max_completion_tokens").is_none());
        assert!(blank.get("reasoning_effort").is_none());
        assert_eq!(blank["messages"][0]["role"], "system");
        let mut thinking = knobs(Protocol::OpenAi);
        thinking.thinking = Thinking::High;
        thinking.max_tokens = Some(100);
        thinking.temperature = Some(0.5);
        let body = build_request(&thinking, &hi(Protocol::OpenAi));
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["max_completion_tokens"], 100);
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn anthropic_splits_the_system_prompt_and_raises_the_thinking_budget() {
        let mut knobs = knobs(Protocol::Anthropic);
        knobs.thinking = Thinking::High;
        knobs.temperature = Some(0.2);
        knobs.top_p = Some(0.9);
        let body = build_request(&knobs, &hi(Protocol::Anthropic));
        assert_eq!(body["system"], SYSTEM);
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body.get("temperature").is_none());
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["thinking"]["budget_tokens"], 16000);
        assert_eq!(body["max_tokens"], 16001);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn gemini_sets_a_thinking_level_and_keeps_sampling_under_generation_config() {
        let mut knobs = knobs(Protocol::Gemini);
        knobs.thinking = Thinking::High;
        knobs.temperature = Some(0.4);
        let body = build_request(&knobs, &hi(Protocol::Gemini));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], SYSTEM);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"], "high");
        assert!(body["generationConfig"]["thinkingConfig"].get("thinkingBudget").is_none());
        assert_eq!(body["generationConfig"]["temperature"], 0.4);
        assert_eq!(body["tools"][0]["functionDeclarations"].as_array().unwrap().len(), 3);
        knobs.thinking = Thinking::Off;
        knobs.temperature = None;
        let off = build_request(&knobs, &hi(Protocol::Gemini));
        assert_eq!(off["generationConfig"]["thinkingConfig"]["thinkingBudget"], 0);
    }

    #[test]
    fn extra_json_overrides_a_knob() {
        let mut knobs = knobs(Protocol::OpenAi);
        knobs.temperature = Some(0.4);
        knobs.extra = json!({ "temperature": 0.2, "reasoning_effort": "minimal" });
        let body = build_request(&knobs, &hi(Protocol::OpenAi));
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["reasoning_effort"], "minimal");
        assert!(check_extra("[1]").is_err());
        assert!(check_extra("").is_ok());
    }

    #[test]
    fn each_protocol_parses_a_tool_call_and_skips_thinking_text() {
        let openai = parse_reply(Protocol::OpenAi, &json!({
            "choices": [{ "message": {
                "role": "assistant",
                "content": "hello",
                "reasoning_content": "secret",
                "tool_calls": [{ "id": "c1", "function": { "name": "list_chats", "arguments": "{\"kind\":\"group\"}" } }]
            }}]
        })).unwrap();
        assert_eq!(openai.text, "hello");
        assert_eq!(openai.calls[0].1, "list_chats");
        assert!(openai.text.contains("secret") == false);

        let anthropic = parse_reply(Protocol::Anthropic, &json!({
            "content": [
                { "type": "thinking", "thinking": "secret" },
                { "type": "text", "text": "Looking." },
                { "type": "tool_use", "id": "t1", "name": "get_messages", "input": { "chat": "Ada" } }
            ]
        })).unwrap();
        assert_eq!(anthropic.text, "Looking.");
        assert_eq!(anthropic.calls[0].1, "get_messages");
        assert_eq!(anthropic.assistant["content"][0]["type"], "thinking");

        let gemini = parse_reply(Protocol::Gemini, &json!({
            "candidates": [{ "content": { "role": "model", "parts": [
                { "text": "secret", "thought": true },
                { "text": "Here." },
                { "functionCall": { "name": "search_conversations", "args": { "query": "park" } } }
            ]}}]
        })).unwrap();
        assert_eq!(gemini.text, "Here.");
        assert_eq!(gemini.calls[0].1, "search_conversations");
        assert_eq!(gemini.calls[0].2["query"], "park");
        assert_eq!(timeout_secs(Thinking::Off), 120);
        assert_eq!(timeout_secs(Thinking::Default), 180);
    }
}
