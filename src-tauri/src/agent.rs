//! Local agent. The user picks an OpenAI-compatible, Anthropic, or Gemini endpoint.
//! Tools only read chats and messages already stored in the SQLite database.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::{json, Value as Json};
use url::Url;

use tauri::Emitter;

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

#[derive(Clone, serde::Serialize, serde::Deserialize)]
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
    #[serde(default = "default_stream")]
    stream: bool,
    #[serde(default)]
    auto_context: String,
    #[serde(default)]
    reply_temperature: String,
    #[serde(default)]
    reply_top_p: String,
    #[serde(default)]
    reply_max_tokens: String,
    #[serde(default = "default_reply_thinking")]
    reply_thinking: String,
    #[serde(default)]
    reply_extra: String,
    #[serde(default)]
    reply_language: String,
}

fn default_reply_thinking() -> String {
    "off".to_string()
}

fn default_stream() -> bool {
    true
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
    stream: bool,
}

struct Step {
    text: String,
    calls: Vec<(String, String, Json)>,
    assistant: Json,
}

const SYSTEM: &str = "You are the assistant inside a LINE desktop app. Answer questions about chats saved in the local database on this computer. Use the tools to look up chats and messages. When the user asks for the latest or recent messages in a chat, call latest_messages once with that chat's name, then answer. The database keeps only recent messages for each chat. Do not invent messages, names, or times. If a tool returns nothing, say that. You cannot send, delete, or change messages. Reply in the same language as the user's question.";

const ANSWER_NOW: &str = "Answer the user's last question now from the tool results above. Do not call tools.";

#[derive(serde::Serialize)]
pub(crate) struct AgentAnswer {
    text: String,
    calls: Vec<String>,
}

struct Tool {
    name: String,
    description: String,
    schema: Json,
}

fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "search_conversations".into(),
            description: "Search saved conversations by chat name or by message text. Use this to find where something was said.".into(),
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
            name: "get_messages".into(),
            description: "Retrieve messages saved for one chat. Pass a chat id, or the display name of a person or group.".into(),
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
            name: "latest_messages".into(),
            description: "Read the newest messages in one chat. Use this once when the user asks for the latest, recent, or last messages, or for a summary of a named chat. Then answer without calling another tool.".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "Chat id, or the name of a person or group." },
                    "count": { "type": "integer", "description": "How many of the newest messages to return, from 1 to 40. Defaults to 12." }
                },
                "required": ["chat"]
            }),
        },
        Tool {
            name: "list_chats".into(),
            description: "List saved chats with a person or a group. Filter by kind and an optional name.".into(),
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

fn openai_tools(tools: &[Tool]) -> Json {
    Json::Array(
        tools.iter()
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

pub(crate) fn load_object() -> Result<Json, String> {
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

pub(crate) fn write_object(value: &Json) -> Result<(), String> {
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

fn check_auto_context(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.len() > 4000 {
        return Err("That auto reply context is too long.".to_string());
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

fn stored_stream(value: &Json) -> bool {
    match value.get("agentStream") {
        Some(Json::Bool(on)) => *on,
        Some(Json::String(text)) => !text.eq_ignore_ascii_case("false"),
        _ => true,
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

fn stored_reply_thinking(value: &Json) -> String {
    match string_field(value, "agentReplyThinking").as_str() {
        "default" => "default".to_string(),
        "low" => "low".to_string(),
        "medium" => "medium".to_string(),
        "high" => "high".to_string(),
        "off" => "off".to_string(),
        _ => "off".to_string(),
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
        stream: stored_stream(&value),
        auto_context: string_field(&value, "agentAutoContext"),
        reply_temperature: string_field(&value, "agentReplyTemperature"),
        reply_top_p: string_field(&value, "agentReplyTopP"),
        reply_max_tokens: string_field(&value, "agentReplyMaxTokens"),
        reply_thinking: stored_reply_thinking(&value),
        reply_extra: string_field(&value, "agentReplyExtra"),
        reply_language: String::new(),
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
    object.insert("agentStream".to_string(), Json::Bool(settings.stream));
    write_object(&value)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplySettings {
    temperature: String,
    top_p: String,
    max_tokens: String,
    thinking: String,
    extra: String,
    auto_context: String,
}

fn check_reply_thinking(raw: &str) -> Result<String, String> {
    match raw.trim() {
        "" | "off" => Ok("off".to_string()),
        "default" => Ok("default".to_string()),
        "low" => Ok("low".to_string()),
        "medium" => Ok("medium".to_string()),
        "high" => Ok("high".to_string()),
        _ => Err("Choose a thinking level.".to_string()),
    }
}

#[tauri::command]
pub fn set_reply_settings(settings: ReplySettings) -> Result<(), String> {
    let temperature = check_optional_number(&settings.temperature, 0.0, 2.0)?;
    let top_p = check_optional_number(&settings.top_p, 0.0, 1.0)?;
    let max_tokens = check_optional_tokens(&settings.max_tokens)?;
    let thinking = check_reply_thinking(&settings.thinking)?;
    let extra = check_extra(&settings.extra)?;
    let auto_context = check_auto_context(&settings.auto_context)?;
    let mut value = load_object()?;
    let Some(object) = value.as_object_mut() else {
        return Err("Could not read settings.".to_string());
    };
    object.insert("agentReplyTemperature".to_string(), Json::String(temperature));
    object.insert("agentReplyTopP".to_string(), Json::String(top_p));
    object.insert("agentReplyMaxTokens".to_string(), Json::String(max_tokens));
    object.insert("agentReplyThinking".to_string(), Json::String(thinking));
    object.insert("agentReplyExtra".to_string(), Json::String(extra));
    object.insert("agentAutoContext".to_string(), Json::String(auto_context));
    write_object(&value)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatLlm {
    endpoint: String,
    model: String,
    api_key: String,
    protocol: String,
    temperature: String,
    top_p: String,
    max_tokens: String,
    thinking: String,
    extra: String,
    context: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    mark_ai: bool,
    #[serde(default)]
    interrupt: bool,
}

fn check_chat_key(id: &str) -> Result<(), String> {
    if id.len() > 64 || id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("That chat cannot store model settings.".to_string());
    }
    Ok(())
}

fn chat_llm_map(value: &Json) -> Option<&serde_json::Map<String, Json>> {
    value.get("chatLlm").and_then(Json::as_object)
}

fn chat_llm_from_json(value: &Json) -> ChatLlm {
    let thinking = match string_field(value, "thinking").as_str() {
        "default" => "default",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "off" => "off",
        _ => "off",
    };
    let protocol = match string_field(value, "protocol").as_str() {
        "anthropic" => "anthropic",
        "gemini" => "gemini",
        _ => "openai",
    };
    ChatLlm {
        endpoint: string_field(value, "endpoint"),
        model: string_field(value, "model"),
        api_key: string_field(value, "apiKey"),
        protocol: protocol.to_string(),
        temperature: string_field(value, "temperature"),
        top_p: string_field(value, "topP"),
        max_tokens: string_field(value, "maxTokens"),
        thinking: thinking.to_string(),
        extra: string_field(value, "extra"),
        context: string_field(value, "context"),
        language: reply_language_name(&string_field(value, "language")).map(|_| string_field(value, "language")).unwrap_or_default(),
        mark_ai: bool_field(value, "markAi"),
        interrupt: bool_field(value, "interrupt"),
    }
}

fn bool_field(value: &Json, key: &str) -> bool {
    match value.get(key) {
        Some(Json::Bool(on)) => *on,
        Some(Json::String(text)) => text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn read_chat_llm(root: &Json, id: &str) -> Option<ChatLlm> {
    chat_llm_map(root).and_then(|map| map.get(id)).map(chat_llm_from_json)
}

fn upsert_chat_llm(root: &mut Json, id: &str, settings: ChatLlm) {
    let map = root.as_object_mut().expect("settings object").entry("chatLlm").or_insert_with(|| json!({}));
    if !map.is_object() {
        *map = json!({});
    }
    map.as_object_mut().expect("chat map").insert(id.to_string(), serde_json::to_value(settings).expect("chat llm"));
}

fn remove_chat_llm(root: &mut Json, id: &str) {
    if let Some(map) = root.get_mut("chatLlm").and_then(Json::as_object_mut) {
        map.remove(id);
    }
}

fn chat_as_settings(chat: &ChatLlm) -> AgentSettings {
    let thinking = if chat.thinking.trim().is_empty() { "off".to_string() } else { chat.thinking.clone() };
    AgentSettings {
        endpoint: chat.endpoint.clone(),
        model: chat.model.clone(),
        api_key: chat.api_key.clone(),
        protocol: chat.protocol.clone(),
        temperature: chat.temperature.clone(),
        top_p: chat.top_p.clone(),
        max_tokens: chat.max_tokens.clone(),
        thinking: thinking.clone(),
        extra: chat.extra.clone(),
        stream: false,
        auto_context: chat.context.clone(),
        reply_temperature: chat.temperature.clone(),
        reply_top_p: chat.top_p.clone(),
        reply_max_tokens: chat.max_tokens.clone(),
        reply_thinking: thinking,
        reply_extra: chat.extra.clone(),
        reply_language: chat.language.clone(),
    }
}

fn load_chat_llm(id: &str) -> Result<Option<ChatLlm>, String> {
    if id.is_empty() {
        return Ok(None);
    }
    check_chat_key(id)?;
    let value = load_object()?;
    Ok(read_chat_llm(&value, id))
}

fn settings_for_chat(chat_id: &str) -> Result<AgentSettings, String> {
    if let Some(chat) = load_chat_llm(chat_id)? {
        return Ok(chat_as_settings(&chat));
    }
    get_agent_settings()
}

#[tauri::command]
pub fn get_chat_llm(id: String) -> Result<Option<ChatLlm>, String> {
    load_chat_llm(id.trim())
}

#[tauri::command]
pub fn list_chat_llm() -> Result<Vec<String>, String> {
    Ok(chat_llm_ids(&load_object()?))
}

fn chat_llm_ids(root: &Json) -> Vec<String> {
    let mut ids: Vec<String> = chat_llm_map(root)
        .map(|map| map.keys().filter(|id| check_chat_key(id).is_ok()).cloned().collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

#[tauri::command]
pub fn set_chat_llm(id: String, settings: ChatLlm) -> Result<(), String> {
    let id = id.trim();
    check_chat_key(id)?;
    let endpoint = check_endpoint(&settings.endpoint)?;
    let model = check_model(&settings.model)?;
    if endpoint.is_empty() || model.is_empty() {
        return Err("Enter an endpoint and a model for this chat.".to_string());
    }
    let saved = ChatLlm {
        endpoint,
        model,
        api_key: check_key(&settings.api_key)?,
        protocol: check_protocol(&settings.protocol)?,
        temperature: check_optional_number(&settings.temperature, 0.0, 2.0)?,
        top_p: check_optional_number(&settings.top_p, 0.0, 1.0)?,
        max_tokens: check_optional_tokens(&settings.max_tokens)?,
        thinking: check_reply_thinking(&settings.thinking)?,
        extra: check_extra(&settings.extra)?,
        context: check_auto_context(&settings.context)?,
        language: check_reply_language(&settings.language)?,
        mark_ai: settings.mark_ai,
        interrupt: settings.interrupt,
    };
    let mut value = load_object()?;
    upsert_chat_llm(&mut value, id, saved);
    write_object(&value)
}

#[tauri::command]
pub fn clear_chat_llm(id: String) -> Result<(), String> {
    let id = id.trim();
    check_chat_key(id)?;
    let mut value = load_object()?;
    remove_chat_llm(&mut value, id);
    write_object(&value)
}

#[derive(serde::Deserialize)]
pub(crate) struct ComposeTurn {
    mine: bool,
    name: String,
    text: String,
}

const REPLY_SYSTEM: &str = "You are Me, a person texting in a LINE chat. The other people read your next message. Answer their last message in the first person, the way a friend texts. If they ask what you did and the chat never says, give a short ordinary answer as yourself and you may ask them back. Do not invent a long story. Do not tell anyone what they should say. Do not start with Suppose, Imagine, You could, or You can. Output only the message. Match the language and tone of the chat.";

const REWRITE_AS_ME: &str = "That was advice about what to say. Send Me's own text instead. Answer the other person in the first person. Output only that text.";

fn reply_language_name(id: &str) -> Option<&'static str> {
    Some(match id.trim() {
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh-Hans" => "Simplified Chinese",
        "zh-Hant" => "Traditional Chinese",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        "ms" => "Malay",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "ru" => "Russian",
        "ar" => "Arabic",
        "tr" => "Turkish",
        "el" => "Greek",
        "hi" => "Hindi",
        _ => return None,
    })
}

fn check_reply_language(raw: &str) -> Result<String, String> {
    let language = raw.trim();
    if language.is_empty() {
        return Ok(String::new());
    }
    if reply_language_name(language).is_none() {
        return Err("Choose a reply language.".to_string());
    }
    Ok(language.to_string())
}

fn reply_transcript(messages: &[ComposeTurn], language: &str) -> String {
    let mut lines = Vec::new();
    let start = messages.len().saturating_sub(20);
    for turn in &messages[start..] {
        let text = clip(turn.text.trim(), 500);
        if text.is_empty() {
            continue;
        }
        let who = if turn.mine {
            "Me".to_string()
        } else if turn.name.trim().is_empty() {
            "Them".to_string()
        } else {
            turn.name.trim().to_string()
        };
        lines.push(format!("{who}: {text}"));
    }
    if lines.is_empty() {
        return String::new();
    }
    let other = last_other_name(messages);
    format!(
        "Messages:\n{}\n\nYou are Me. Text {other} back. Answer their last message in the first person, as a friend texting. If they asked what you did and the messages above do not say, answer briefly as yourself. Output only the text you send them. {}",
        lines.join("\n"),
        language_order(language)
    )
}

fn language_order(name: &str) -> String {
    if name.is_empty() {
        "Reply in the same language as their last message.".to_string()
    } else {
        format!("Reply in {name}. Write only in {name}, even if earlier messages used another language.")
    }
}

fn reply_language_for(messages: &[ComposeTurn], configured: &str) -> String {
    if let Some(name) = reply_language_name(configured) {
        return name.to_string();
    }
    let last = messages
        .iter()
        .rev()
        .find(|turn| !turn.mine && !turn.text.trim().is_empty())
        .map(|turn| turn.text.as_str())
        .unwrap_or("");
    detect_language_name(last).unwrap_or_default().to_string()
}

fn detect_language_name(text: &str) -> Option<&'static str> {
    let mut hangul = 0usize;
    let mut kana = 0usize;
    let mut han = 0usize;
    let mut thai = 0usize;
    let mut arabic = 0usize;
    let mut cyrillic = 0usize;
    let mut greek = 0usize;
    let mut devanagari = 0usize;
    let mut latin = 0usize;
    for ch in text.chars() {
        match ch {
            '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' => hangul += 1,
            '\u{3040}'..='\u{30FF}' => kana += 1,
            '\u{4E00}'..='\u{9FFF}' => han += 1,
            '\u{0E00}'..='\u{0E7F}' => thai += 1,
            '\u{0600}'..='\u{06FF}' => arabic += 1,
            '\u{0400}'..='\u{04FF}' => cyrillic += 1,
            '\u{0370}'..='\u{03FF}' => greek += 1,
            '\u{0900}'..='\u{097F}' => devanagari += 1,
            'A'..='Z' | 'a'..='z' => latin += 1,
            _ => {}
        }
    }
    let mut best = 0usize;
    let mut name = "";
    for (count, label) in [
        (hangul, "Korean"),
        (kana, "Japanese"),
        (han, "Chinese"),
        (thai, "Thai"),
        (arabic, "Arabic"),
        (cyrillic, "Russian"),
        (greek, "Greek"),
        (devanagari, "Hindi"),
    ] {
        if count > best {
            best = count;
            name = label;
        }
    }
    if best >= 2 {
        if kana > 0 {
            return Some("Japanese");
        }
        return Some(name);
    }
    if latin < 3 {
        return None;
    }
    let words: Vec<String> = text
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_lowercase())
        .collect();
    const ENGLISH: &[&str] = &[
        "the", "a", "an", "i", "you", "your", "we", "what", "did", "do", "does", "last", "night", "how", "are", "is", "was", "were", "to", "of", "and", "my", "me", "coming", "hi", "hello", "hey", "sup",
    ];
    const MALAY: &[&str] = &["aku", "saya", "kau", "awak", "anda", "lah", "je", "tak", "apa", "buat", "yang", "ini", "itu", "nak", "dah", "pun", "kat"];
    let english = words.iter().filter(|word| ENGLISH.contains(&word.as_str())).count();
    let malay = words.iter().filter(|word| MALAY.contains(&word.as_str())).count();
    if malay > english && malay > 0 {
        return Some("Malay");
    }
    if english > 0 {
        return Some("English");
    }
    None
}

fn last_other_name(messages: &[ComposeTurn]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|turn| {
            if turn.mine || turn.text.trim().is_empty() {
                return None;
            }
            let name = turn.name.trim();
            Some(if name.is_empty() { "them".to_string() } else { name.to_string() })
        })
        .unwrap_or_else(|| "them".to_string())
}

fn sounds_like_advice(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    const STARTS: &[&str] = &["suppose ", "imagine ", "you could ", "you can ", "you should ", "you might ", "perhaps you", "here's ", "here is "];
    if STARTS.iter().any(|prefix| lower.starts_with(prefix)) {
        return true;
    }
    const PHRASES: &[&str] = &["you can reply", "you could say", "you could reply", "what to say", "the best move", "side of the story"];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

fn reply_system(guidance: &str, language: &str) -> String {
    let base = match reply_language_name(language) {
        Some(name) => {
            let rule = language_order(name);
            format!("{rule}\n\n{}", REPLY_SYSTEM.replacen("Match the language and tone of the chat.", "Match the tone of the chat.", 1))
        }
        None => REPLY_SYSTEM.to_string(),
    };
    let guidance = guidance.trim();
    if guidance.is_empty() {
        return base;
    }
    format!("{base}\n\nHow Me should sound when talking to the other people:\n{guidance}")
}

fn empty_reply_message(thinking: Thinking) -> String {
    if thinking == Thinking::Off {
        return "The model returned only its thinking and no message to send.".to_string();
    }
    "The model returned only its thinking and no message. Turn thinking off for this chat.".to_string()
}

fn last_is_mine(messages: &[ComposeTurn]) -> bool {
    messages.iter().rev().find(|turn| !turn.text.trim().is_empty()).is_none_or(|turn| turn.mine)
}

#[tauri::command]
pub async fn agent_probe() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(probe_model).await.map_err(|err| err.to_string())?
}

fn probe_model() -> Result<(), String> {
    let settings = get_agent_settings()?;
    if settings.endpoint.is_empty() || settings.model.is_empty() {
        return Err("Set the agent endpoint in Settings.".to_string());
    }
    let mut knobs = resolve_knobs(&settings)?;
    knobs.stream = false;
    knobs.thinking = Thinking::Off;
    knobs.max_tokens = Some(16);
    let transcript = seed_transcript(knobs.protocol, &[("user".to_string(), "Reply with ok.".to_string())]);
    let mut body = build_request(&knobs, &transcript);
    strip_tools(&mut body);
    let payload = post_chat(&settings.endpoint, &settings.api_key, &body, knobs.protocol, 20)?;
    let step = parse_reply(knobs.protocol, &payload)?;
    if !probe_answered(knobs.protocol, &payload, &step) {
        return Err("The endpoint returned an empty reply.".to_string());
    }
    Ok(())
}

/// A thinking model can spend a short probe on hidden reasoning and leave the
/// visible text empty. That is still a response, so auto reply can be offered.
fn probe_answered(protocol: Protocol, payload: &Json, step: &Step) -> bool {
    if !step.text.trim().is_empty() || !step.calls.is_empty() {
        return true;
    }
    match protocol {
        Protocol::OpenAi => {
            let message = payload.pointer("/choices/0/message");
            filled(message.and_then(|item| item.get("reasoning"))) || filled(message.and_then(|item| item.get("reasoning_content")))
        }
        Protocol::Anthropic => payload.get("content").and_then(Json::as_array).is_some_and(|blocks| {
            blocks.iter().any(|block| matches!(block.get("type").and_then(Json::as_str), Some("thinking" | "text")))
        }),
        Protocol::Gemini => payload
            .pointer("/candidates/0/content/parts")
            .and_then(Json::as_array)
            .is_some_and(|parts| parts.iter().any(|part| part.get("text").and_then(Json::as_str).is_some_and(|text| !text.trim().is_empty()) || part.get("thought") == Some(&Json::Bool(true)))),
    }
}

fn filled(value: Option<&Json>) -> bool {
    value.and_then(Json::as_str).is_some_and(|text| !text.trim().is_empty())
}

#[tauri::command]
pub async fn agent_compose(messages: Vec<ComposeTurn>, chat_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || compose_reply(messages, &chat_id)).await.map_err(|err| err.to_string())?
}

fn compose_reply(messages: Vec<ComposeTurn>, chat_id: &str) -> Result<String, String> {
    if last_is_mine(&messages) {
        return Ok(String::new());
    }
    let settings = settings_for_chat(chat_id)?;
    if settings.endpoint.is_empty() || settings.model.is_empty() {
        return Err("Set the endpoint and model for this chat.".to_string());
    }
    let language = reply_language_for(&messages, &settings.reply_language);
    let transcript = reply_transcript(&messages, &language);
    if transcript.is_empty() {
        return Ok(String::new());
    }
    let knobs = reply_knobs(&settings)?;
    let mut system = reply_system(&settings.auto_context, &settings.reply_language);
    if reply_language_name(&settings.reply_language).is_none() && !language.is_empty() {
        system = system.replacen("Match the language and tone of the chat.", &format!("Reply in {language}. Match the tone of the chat."), 1);
    }
    let (mut links, remote) = crate::mcp::prepare();
    let offered = mcp_tools(&remote);
    if !offered.is_empty() {
        system.push_str("\n\nYou may call MCP tools before writing the message. After the tools return, write only Me's next message to the other people in the chat.");
    }
    let mut transcript = match knobs.protocol {
        Protocol::OpenAi => vec![
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": transcript }),
        ],
        Protocol::Anthropic => vec![json!({ "role": "user", "content": transcript })],
        Protocol::Gemini => vec![json!({ "role": "user", "parts": [{ "text": transcript }] })],
    };
    let mut retried = false;
    let mut round = 0usize;
    let tool_limit = if offered.is_empty() { 0 } else { 3 };
    loop {
        let answering = retried || round >= tool_limit;
        let use_tools: &[Tool] = if answering { &[] } else { &offered };
        let mut body = protocol_body(&knobs, &transcript, use_tools);
        match knobs.protocol {
            Protocol::Anthropic => body["system"] = json!(system),
            Protocol::Gemini => body["systemInstruction"] = json!({ "parts": [{ "text": system }] }),
            Protocol::OpenAi => {}
        }
        silence_thinking(knobs.protocol, knobs.thinking, &mut body);
        finish_body(&mut body, &knobs, use_tools);
        let payload = post_chat(&settings.endpoint, &settings.api_key, &body, knobs.protocol, timeout_secs(knobs.thinking))?;
        let step = parse_reply(knobs.protocol, &payload)?;
        if answering || step.calls.is_empty() {
            let text = clip(step.text.trim(), 2000);
            if text.is_empty() {
                return Err(empty_reply_message(knobs.thinking));
            }
            if !retried && sounds_like_advice(&text) {
                retried = true;
                transcript.push(step.assistant);
                nudge_rewrite(knobs.protocol, &mut transcript, &language_order(&language));
                continue;
            }
            return Ok(text);
        }
        transcript.push(step.assistant);
        let mut results = Vec::new();
        for (_id, name, args) in &step.calls {
            results.push(dispatch_tool(name, args, &mut links, &remote));
        }
        append_tool_results(knobs.protocol, &mut transcript, &step.calls, &results);
        round += 1;
        if round > tool_limit {
            return Err(empty_reply_message(knobs.thinking));
        }
    }
}

fn nudge_rewrite(protocol: Protocol, messages: &mut Vec<Json>, language: &str) {
    let text = format!("{REWRITE_AS_ME} {language}");
    match protocol {
        Protocol::OpenAi | Protocol::Anthropic => messages.push(json!({ "role": "user", "content": text })),
        Protocol::Gemini => messages.push(json!({ "role": "user", "parts": [{ "text": text }] })),
    }
}

#[tauri::command]
pub async fn agent_ask(app: tauri::AppHandle, messages: Vec<TurnIn>) -> Result<AgentAnswer, String> {
    tauri::async_runtime::spawn_blocking(move || ask(&app, messages))
        .await
        .map_err(|err| err.to_string())?
}

fn emit_agent(app: &tauri::AppHandle, payload: Json) {
    let _ = app.emit("agent-stream", payload);
}

fn ask(app: &tauri::AppHandle, messages: Vec<TurnIn>) -> Result<AgentAnswer, String> {
    ask_with(messages, &mut |payload| emit_agent(app, payload))
}

fn ask_with(messages: Vec<TurnIn>, emit: &mut dyn FnMut(Json)) -> Result<AgentAnswer, String> {
    let settings = get_agent_settings()?;
    if settings.endpoint.is_empty() {
        return Err("Set the agent endpoint in Settings.".to_string());
    }
    if settings.model.is_empty() {
        return Err("Set the agent model in Settings.".to_string());
    }
    let knobs = resolve_knobs(&settings)?;
    let (mut links, remote) = crate::mcp::prepare();
    let offered = {
        let mut combined = tools();
        combined.extend(mcp_tools(&remote));
        combined
    };
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
    if !remote.is_empty() && knobs.protocol == Protocol::OpenAi {
        if let Some(Json::String(text)) = transcript.first_mut().and_then(|message| message.get_mut("content")) {
            text.push_str("\n\nMCP tools from servers the user added are also available. Call one when it helps, then answer from its result.");
        }
    }
    let mut calls = Vec::new();
    for round in 0..=ROUNDS {
        let answering = round == ROUNDS;
        if answering {
            nudge_answer(knobs.protocol, &mut transcript);
        }
        let use_tools: &[Tool] = if answering { &[] } else { &offered };
        let body = request_with(&knobs, &transcript, use_tools);
        let step = if knobs.stream {
            let endpoint = stream_endpoint(knobs.protocol, &settings.endpoint);
            post_stream(&endpoint, &settings.api_key, &body, knobs.protocol, timeout_secs(knobs.thinking), emit)?
        } else {
            let payload = post_chat(&settings.endpoint, &settings.api_key, &body, knobs.protocol, timeout_secs(knobs.thinking))?;
            parse_reply(knobs.protocol, &payload)?
        };
        if answering || step.calls.is_empty() {
            if step.text.trim().is_empty() {
                return Err(if answering {
                    "The agent stopped after several lookups without a reply.".to_string()
                } else {
                    "The endpoint returned an empty reply.".to_string()
                });
            }
            return Ok(AgentAnswer { text: step.text.trim().to_string(), calls });
        }
        transcript.push(step.assistant);
        let mut results = Vec::new();
        for (_id, name, args) in &step.calls {
            calls.push(crate::mcp::chip_name(&remote, name).unwrap_or_else(|| name.clone()));
            results.push(dispatch_tool(name, args, &mut links, &remote));
        }
        append_tool_results(knobs.protocol, &mut transcript, &step.calls, &results);
    }
    Err("The agent stopped after several lookups without a reply.".to_string())
}

fn mcp_tools(remote: &[crate::mcp::McpTool]) -> Vec<Tool> {
    remote
        .iter()
        .map(|tool| {
            let description = if tool.description.is_empty() {
                format!("MCP tool {}.", tool.remote)
            } else {
                format!("{} (MCP)", tool.description)
            };
            Tool { name: tool.exposed.clone(), description, schema: tool.schema.clone() }
        })
        .collect()
}

fn dispatch_tool(name: &str, arguments: &Json, links: &mut [crate::mcp::Link], remote: &[crate::mcp::McpTool]) -> String {
    if let Some(result) = crate::mcp::invoke(links, remote, name, arguments) {
        return result;
    }
    run_tool(name, arguments)
}

fn strip_tools(body: &mut Json) {
    if let Some(object) = body.as_object_mut() {
        object.remove("tools");
        object.remove("tool_choice");
    }
}

fn nudge_answer(protocol: Protocol, messages: &mut Vec<Json>) {
    match protocol {
        Protocol::OpenAi => messages.push(json!({ "role": "user", "content": ANSWER_NOW })),
        Protocol::Anthropic => {
            if let Some(last) = messages.last_mut().filter(|message| message.get("role").and_then(|value| value.as_str()) == Some("user")) {
                match last.get_mut("content") {
                    Some(Json::Array(blocks)) => blocks.push(json!({ "type": "text", "text": ANSWER_NOW })),
                    Some(Json::String(text)) => {
                        let prior = text.clone();
                        last["content"] = json!([{ "type": "text", "text": prior }, { "type": "text", "text": ANSWER_NOW }]);
                    }
                    _ => last["content"] = json!(ANSWER_NOW),
                }
                return;
            }
            messages.push(json!({ "role": "user", "content": ANSWER_NOW }));
        }
        Protocol::Gemini => {
            if let Some(last) = messages.last_mut().filter(|message| message.get("role").and_then(|value| value.as_str()) == Some("user")) {
                if let Some(parts) = last.get_mut("parts").and_then(|value| value.as_array_mut()) {
                    parts.push(json!({ "text": ANSWER_NOW }));
                    return;
                }
            }
            messages.push(json!({ "role": "user", "parts": [{ "text": ANSWER_NOW }] }));
        }
    }
}

fn reply_knobs(settings: &AgentSettings) -> Result<Knobs, String> {
    let mut shaped = settings.clone();
    shaped.temperature = settings.reply_temperature.clone();
    shaped.top_p = settings.reply_top_p.clone();
    shaped.max_tokens = settings.reply_max_tokens.clone();
    shaped.thinking = if settings.reply_thinking.trim().is_empty() { "off".to_string() } else { settings.reply_thinking.clone() };
    shaped.extra = settings.reply_extra.clone();
    shaped.stream = false;
    resolve_knobs(&shaped)
}

/// Ollama's OpenAI-compatible endpoint ignores `think` and turns thinking on
/// unless `reasoning_effort` is `none`. Off means the visible text is the message.
fn silence_thinking(protocol: Protocol, thinking: Thinking, body: &mut Json) {
    if thinking == Thinking::Off && protocol == Protocol::OpenAi {
        body["think"] = json!(false);
        body["reasoning_effort"] = json!("none");
    }
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
        stream: settings.stream,
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
    request_with(knobs, messages, &tools())
}

fn request_with(knobs: &Knobs, messages: &[Json], tools: &[Tool]) -> Json {
    let mut body = protocol_body(knobs, messages, tools);
    finish_body(&mut body, knobs, tools);
    body
}

fn protocol_body(knobs: &Knobs, messages: &[Json], tools: &[Tool]) -> Json {
    match knobs.protocol {
        Protocol::OpenAi => openai_body(knobs, messages, tools),
        Protocol::Anthropic => anthropic_body(knobs, messages, tools),
        Protocol::Gemini => gemini_body(knobs, messages, tools),
    }
}

fn finish_body(body: &mut Json, knobs: &Knobs, tools: &[Tool]) {
    deep_merge(body, &knobs.extra);
    if tools.is_empty() {
        strip_tools(body);
    } else {
        attach_tools(body, knobs.protocol, tools);
    }
    if knobs.protocol != Protocol::Gemini {
        if knobs.stream {
            body["stream"] = json!(true);
        } else if let Some(object) = body.as_object_mut() {
            object.remove("stream");
        }
    }
}

fn attach_tools(body: &mut Json, protocol: Protocol, tools: &[Tool]) {
    match protocol {
        Protocol::OpenAi => {
            body["tools"] = openai_tools(tools);
            body["tool_choice"] = json!("auto");
        }
        Protocol::Anthropic => {
            body["tools"] = anthropic_tools(tools);
        }
        Protocol::Gemini => {
            body["tools"] = gemini_tools(tools);
        }
    }
}

fn system_text(tools: &[Tool]) -> String {
    if tools.iter().any(|tool| tool.name.starts_with("mcp")) {
        format!("{SYSTEM}\n\nMCP tools from servers the user added are also available. Call one when it helps, then answer from its result.")
    } else {
        SYSTEM.to_string()
    }
}

fn stream_endpoint(protocol: Protocol, endpoint: &str) -> String {
    if protocol != Protocol::Gemini {
        return endpoint.to_string();
    }
    let mut url = if endpoint.contains(":streamGenerateContent") {
        endpoint.to_string()
    } else {
        endpoint.replacen(":generateContent", ":streamGenerateContent", 1)
    };
    if !url.contains("alt=sse") {
        url.push(if url.contains('?') { '&' } else { '?' });
        url.push_str("alt=sse");
    }
    url
}

fn openai_body(knobs: &Knobs, messages: &[Json], tools: &[Tool]) -> Json {
    let mut body = json!({
        "model": knobs.model,
        "messages": messages,
        "tools": openai_tools(tools),
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

fn anthropic_body(knobs: &Knobs, messages: &[Json], tools: &[Tool]) -> Json {
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
        "system": system_text(tools),
        "messages": messages,
        "tools": anthropic_tools(tools),
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

fn gemini_body(knobs: &Knobs, messages: &[Json], tools: &[Tool]) -> Json {
    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": system_text(tools) }] },
        "contents": messages,
        "tools": gemini_tools(tools),
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

fn anthropic_tools(tools: &[Tool]) -> Json {
    Json::Array(
        tools
            .iter()
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

fn gemini_tools(tools: &[Tool]) -> Json {
    json!([{
        "functionDeclarations": tools.iter().map(|tool| json!({
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
    let sentence = error_sentence(payload);
    if !sentence.is_empty() {
        let has_reply = match protocol {
            Protocol::OpenAi => payload.get("choices").is_some(),
            Protocol::Anthropic => payload.get("content").is_some(),
            Protocol::Gemini => payload.get("candidates").is_some(),
        };
        if !has_reply {
            return Err(present_endpoint_error(None, &sentence));
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

struct OpenAiTool {
    id: String,
    name: String,
    args: String,
    announced: bool,
}

struct AnthropicBlock {
    kind: String,
    id: String,
    name: String,
    text: String,
    signature: String,
    announced: bool,
}

struct StreamFold {
    protocol: Protocol,
    text: String,
    openai_tools: Vec<OpenAiTool>,
    anthropic: Vec<AnthropicBlock>,
    gemini_parts: Vec<Json>,
    error: Option<String>,
}

fn post_stream(endpoint: &str, key: &str, body: &Json, protocol: Protocol, timeout: u64, emit: &mut dyn FnMut(Json)) -> Result<Step, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout)).redirects(0).build();
    let mut request = agent.post(endpoint).set("Content-Type", "application/json").set("Accept", "text/event-stream, application/json");
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
    let response = match request.send_bytes(&bytes) {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return Err(http_failure(code, &text));
        }
        Err(_) => return Err("The endpoint did not answer.".to_string()),
    };
    let mut reader = BufReader::new(response.into_reader());
    let mut line = String::new();
    let mut fold = StreamFold::new(protocol);
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|_| "The endpoint returned data this app could not read.".to_string())?;
        if read == 0 {
            break;
        }
        let (delta, names) = fold.accept(&line);
        if !delta.is_empty() {
            emit(json!({ "type": "text", "text": delta }));
        }
        for name in names {
            emit(json!({ "type": "call", "name": name }));
        }
    }
    fold.finish()
}

impl StreamFold {
    fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            text: String::new(),
            openai_tools: Vec::new(),
            anthropic: Vec::new(),
            gemini_parts: Vec::new(),
            error: None,
        }
    }

    fn accept(&mut self, line: &str) -> (String, Vec<String>) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') || trimmed.starts_with("event:") {
            return (String::new(), Vec::new());
        }
        let payload = trimmed.strip_prefix("data:").map(str::trim).unwrap_or(trimmed);
        if payload.is_empty() || payload == "[DONE]" {
            return (String::new(), Vec::new());
        }
        let Ok(value) = serde_json::from_str::<Json>(payload) else {
            return (String::new(), Vec::new());
        };
        self.consume(value)
    }

    fn consume(&mut self, value: Json) -> (String, Vec<String>) {
        match self.protocol {
            Protocol::OpenAi => self.consume_openai(value),
            Protocol::Anthropic => self.consume_anthropic(value),
            Protocol::Gemini => self.consume_gemini(value),
        }
    }

    fn note_error(&mut self, message: &str) {
        let message = message.trim();
        if !message.is_empty() && self.error.is_none() {
            self.error = Some(clip(message, 300));
        }
    }

    fn consume_openai(&mut self, value: Json) -> (String, Vec<String>) {
        if value.get("choices").is_none() {
            let sentence = error_sentence(&value);
            if !sentence.is_empty() {
                self.note_error(&present_endpoint_error(None, &sentence));
            }
        }
        if value.pointer("/choices/0/message").is_some() {
            return self.consume_openai_message(value);
        }
        let Some(delta) = value.pointer("/choices/0/delta").cloned() else {
            return (String::new(), Vec::new());
        };
        let mut shown = String::new();
        if let Some(piece) = delta.get("content") {
            shown = self.push_visible(&content_text(piece));
        }
        let mut names = Vec::new();
        let listed = delta.get("tool_calls").and_then(|item| item.as_array()).cloned().unwrap_or_default();
        for call in listed {
            let index = call.get("index").and_then(|item| item.as_u64()).unwrap_or(self.openai_tools.len() as u64) as usize;
            while self.openai_tools.len() <= index {
                self.openai_tools.push(OpenAiTool { id: String::new(), name: String::new(), args: String::new(), announced: false });
            }
            let tool = &mut self.openai_tools[index];
            if let Some(id) = call.get("id").and_then(|item| item.as_str()).filter(|id| !id.is_empty()) {
                tool.id = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(|item| item.as_str()).filter(|name| !name.is_empty()) {
                tool.name = name.to_string();
            }
            if let Some(arguments) = call.pointer("/function/arguments") {
                match arguments {
                    Json::String(text) => tool.args.push_str(text),
                    Json::Null => {}
                    other => tool.args.push_str(&other.to_string()),
                }
            }
            if !tool.announced && !tool.name.is_empty() {
                tool.announced = true;
                names.push(tool.name.clone());
            }
        }
        (shown, names)
    }

    fn consume_openai_message(&mut self, value: Json) -> (String, Vec<String>) {
        let Some(message) = value.pointer("/choices/0/message").cloned() else {
            return (String::new(), Vec::new());
        };
        let shown = self.push_visible(&content_text(message.get("content").unwrap_or(&Json::Null)));
        let mut names = Vec::new();
        for call in message.get("tool_calls").and_then(|item| item.as_array()).cloned().unwrap_or_default() {
            let name = call.pointer("/function/name").and_then(|item| item.as_str()).unwrap_or("").to_string();
            let args = match call.pointer("/function/arguments") {
                Some(Json::String(text)) => text.clone(),
                Some(Json::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            let announced = name.is_empty();
            if !announced {
                names.push(name.clone());
            }
            self.openai_tools.push(OpenAiTool {
                id: call.get("id").and_then(|item| item.as_str()).unwrap_or("").to_string(),
                name,
                args,
                announced: !announced,
            });
        }
        (shown, names)
    }

    fn consume_anthropic(&mut self, value: Json) -> (String, Vec<String>) {
        if value.get("type").and_then(|item| item.as_str()) == Some("error") {
            let sentence = error_sentence(&value);
            if !sentence.is_empty() {
                self.note_error(&present_endpoint_error(None, &sentence));
            }
            return (String::new(), Vec::new());
        }
        let whole_message = value.get("type").and_then(|item| item.as_str()) == Some("message") || (value.get("content").is_some_and(|item| item.is_array()) && value.get("role").is_some());
        if whole_message {
            return self.consume_anthropic_message(&value);
        }
        let kind = value.get("type").and_then(|item| item.as_str()).unwrap_or("");
        if kind == "content_block_start" {
            let index = value.get("index").and_then(|item| item.as_u64()).unwrap_or(self.anthropic.len() as u64) as usize;
            let block = self.anthropic_block(index);
            let content = value.get("content_block").cloned().unwrap_or_else(|| json!({}));
            block.kind = content.get("type").and_then(|item| item.as_str()).unwrap_or("").to_string();
            block.id = content.get("id").and_then(|item| item.as_str()).unwrap_or("").to_string();
            block.name = content.get("name").and_then(|item| item.as_str()).unwrap_or("").to_string();
            let mut names = Vec::new();
            if block.kind == "tool_use" && !block.name.is_empty() && !block.announced {
                block.announced = true;
                names.push(block.name.clone());
            }
            return (String::new(), names);
        }
        if kind == "content_block_delta" {
            let index = value.get("index").and_then(|item| item.as_u64()).unwrap_or(0) as usize;
            let block = self.anthropic_block(index);
            let delta = value.get("delta").cloned().unwrap_or_else(|| json!({}));
            let delta_type = delta.get("type").and_then(|item| item.as_str()).unwrap_or("");
            return match delta_type {
                "text_delta" => {
                    if block.kind.is_empty() {
                        block.kind = "text".to_string();
                    }
                    let piece = delta.get("text").and_then(|item| item.as_str()).unwrap_or("");
                    block.text.push_str(piece);
                    (self.push_visible(piece), Vec::new())
                }
                "thinking_delta" => {
                    if block.kind.is_empty() {
                        block.kind = "thinking".to_string();
                    }
                    block.text.push_str(delta.get("thinking").and_then(|item| item.as_str()).unwrap_or(""));
                    (String::new(), Vec::new())
                }
                "signature_delta" => {
                    block.signature.push_str(delta.get("signature").and_then(|item| item.as_str()).unwrap_or(""));
                    (String::new(), Vec::new())
                }
                "input_json_delta" => {
                    if block.kind.is_empty() {
                        block.kind = "tool_use".to_string();
                    }
                    block.text.push_str(delta.get("partial_json").and_then(|item| item.as_str()).unwrap_or(""));
                    (String::new(), Vec::new())
                }
                _ => (String::new(), Vec::new()),
            };
        }
        (String::new(), Vec::new())
    }

    fn consume_anthropic_message(&mut self, value: &Json) -> (String, Vec<String>) {
        let content = value.get("content").cloned().unwrap_or(Json::Null);
        let mut blocks = match content {
            Json::String(text) => vec![json!({ "type": "text", "text": text })],
            Json::Array(items) => items,
            _ => Vec::new(),
        };
        let mut shown = String::new();
        let mut names = Vec::new();
        for block in &mut blocks {
            let kind = block.get("type").and_then(|item| item.as_str()).unwrap_or("").to_string();
            let id = block.get("id").and_then(|item| item.as_str()).unwrap_or("").to_string();
            let name = block.get("name").and_then(|item| item.as_str()).unwrap_or("").to_string();
            let text = match kind.as_str() {
                "text" => block.get("text").and_then(|item| item.as_str()).unwrap_or("").to_string(),
                "thinking" => block.get("thinking").and_then(|item| item.as_str()).unwrap_or("").to_string(),
                "tool_use" => block.get("input").map(|item| item.to_string()).unwrap_or_default(),
                _ => String::new(),
            };
            if kind == "text" {
                shown.push_str(&self.push_visible(&text));
            }
            if kind == "tool_use" && !name.is_empty() {
                names.push(name.clone());
            }
            self.anthropic.push(AnthropicBlock {
                kind,
                id,
                name,
                text,
                signature: block.get("signature").and_then(|item| item.as_str()).unwrap_or("").to_string(),
                announced: true,
            });
        }
        (shown, names)
    }

    fn anthropic_block(&mut self, index: usize) -> &mut AnthropicBlock {
        while self.anthropic.len() <= index {
            self.anthropic.push(AnthropicBlock {
                kind: String::new(),
                id: String::new(),
                name: String::new(),
                text: String::new(),
                signature: String::new(),
                announced: false,
            });
        }
        &mut self.anthropic[index]
    }

    fn consume_gemini(&mut self, value: Json) -> (String, Vec<String>) {
        if value.get("candidates").is_none() {
            let sentence = error_sentence(&value);
            if !sentence.is_empty() {
                self.note_error(&present_endpoint_error(None, &sentence));
            }
        }
        let parts = value.pointer("/candidates/0/content/parts").and_then(|item| item.as_array()).cloned().unwrap_or_default();
        let mut shown = String::new();
        let mut names = Vec::new();
        for part in parts {
            if part.get("thought").and_then(|item| item.as_bool()).unwrap_or(false) {
                self.gemini_parts.push(part);
                continue;
            }
            if let Some(piece) = part.get("text").and_then(|item| item.as_str()) {
                shown.push_str(&self.push_visible(piece));
                self.push_gemini_text(piece);
            }
            if let Some(call) = part.get("functionCall") {
                let name = call.get("name").and_then(|item| item.as_str()).unwrap_or("").to_string();
                if !name.is_empty() {
                    names.push(name);
                }
                self.gemini_parts.push(part);
            }
        }
        (shown, names)
    }

    fn push_gemini_text(&mut self, piece: &str) {
        if let Some(last) = self.gemini_parts.last_mut() {
            let thought = last.get("thought").and_then(|item| item.as_bool()).unwrap_or(false);
            if !thought && last.get("functionCall").is_none() && last.get("text").is_some() {
                let current = last.get("text").and_then(|item| item.as_str()).unwrap_or("").to_string();
                last["text"] = json!(format!("{current}{piece}"));
                return;
            }
        }
        self.gemini_parts.push(json!({ "text": piece }));
    }

    fn push_visible(&mut self, piece: &str) -> String {
        if piece.is_empty() {
            return String::new();
        }
        self.text.push_str(piece);
        piece.to_string()
    }

    fn finish(self) -> Result<Step, String> {
        let error = self.error.clone();
        let step = match self.protocol {
            Protocol::OpenAi => self.finish_openai(),
            Protocol::Anthropic => self.finish_anthropic(),
            Protocol::Gemini => self.finish_gemini(),
        };
        if step.calls.is_empty() && step.text.trim().is_empty() {
            if let Some(message) = error {
                return Err(message);
            }
        }
        Ok(step)
    }

    fn finish_openai(self) -> Step {
        let mut message = json!({ "role": "assistant", "content": self.text });
        let mut calls = Vec::new();
        if !self.openai_tools.is_empty() {
            let mut listed = Vec::new();
            for (index, tool) in self.openai_tools.iter().enumerate() {
                if tool.name.is_empty() {
                    continue;
                }
                let id = if tool.id.is_empty() { format!("call-{}", index + 1) } else { tool.id.clone() };
                listed.push(json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": tool.name, "arguments": tool.args },
                }));
                calls.push((id, tool.name.clone(), Json::String(tool.args.clone())));
            }
            if !listed.is_empty() {
                message["tool_calls"] = Json::Array(listed);
            }
        }
        Step { text: self.text, calls, assistant: message }
    }

    fn finish_anthropic(self) -> Step {
        let mut blocks = Vec::new();
        let mut calls = Vec::new();
        let mut text = String::new();
        for (index, block) in self.anthropic.iter().enumerate() {
            match block.kind.as_str() {
                "text" => {
                    push_text(&mut text, &block.text);
                    blocks.push(json!({ "type": "text", "text": block.text }));
                }
                "thinking" => {
                    let mut saved = json!({ "type": "thinking", "thinking": block.text });
                    if !block.signature.is_empty() {
                        saved["signature"] = json!(block.signature);
                    }
                    blocks.push(saved);
                }
                "tool_use" => {
                    let id = if block.id.is_empty() { format!("call-{}", index + 1) } else { block.id.clone() };
                    let input = serde_json::from_str::<Json>(&block.text).ok().filter(|value| value.is_object()).unwrap_or_else(|| json!({}));
                    blocks.push(json!({ "type": "tool_use", "id": id, "name": block.name, "input": input }));
                    calls.push((id, block.name.clone(), input));
                }
                _ => {}
            }
        }
        Step { text, calls, assistant: json!({ "role": "assistant", "content": blocks }) }
    }

    fn finish_gemini(self) -> Step {
        let mut calls = Vec::new();
        for part in &self.gemini_parts {
            if part.get("thought").and_then(|item| item.as_bool()).unwrap_or(false) {
                continue;
            }
            if let Some(call) = part.get("functionCall") {
                let name = call.get("name").and_then(|item| item.as_str()).unwrap_or("").to_string();
                let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                calls.push((format!("call-{}", calls.len() + 1), name, args));
            }
        }
        let assistant = json!({ "role": "model", "parts": self.gemini_parts });
        Step { text: self.text, calls, assistant }
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
    let sentence = parsed.as_ref().map(error_sentence).unwrap_or_default();
    present_endpoint_error(Some(code), &sentence)
}

fn error_sentence(value: &Json) -> String {
    let nested = value.pointer("/error/message").and_then(Json::as_str).unwrap_or("").trim();
    if !nested.is_empty() {
        return nested.to_string();
    }
    if let Some(text) = value.get("error").and_then(Json::as_str).map(str::trim).filter(|text| !text.is_empty()) {
        return text.to_string();
    }
    value.get("message").and_then(Json::as_str).unwrap_or("").trim().to_string()
}

fn context_overflow(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "context length",
        "context window",
        "context size",
        "maximum context",
        "prompt is too long",
        "too many tokens",
        "exceeds the available context",
        "exceed_context",
        "input token count",
        "input tokens exceed",
        "n_ctx",
    ]
    .iter()
    .any(|mark| lower.contains(mark))
}

fn present_endpoint_error(code: Option<u16>, sentence: &str) -> String {
    let sentence = sentence.trim();
    if context_overflow(sentence) {
        let detail = clip(sentence, 240);
        if detail.is_empty() {
            return "The request is larger than the model's context window.".to_string();
        }
        return format!("The request is larger than the model's context window. {detail}");
    }
    if sentence.is_empty() {
        return match code {
            Some(code) => format!("The endpoint returned {code}."),
            None => "The endpoint did not return a reply.".to_string(),
        };
    }
    match code {
        Some(code) => format!("The endpoint returned {code}: {}", clip(sentence, 240)),
        None => clip(sentence, 300),
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
        "latest_messages" => latest_messages(conn, args),
        "list_chats" => list_chats(conn, args),
        _ => json!({ "error": "Unknown tool." }),
    }
}

fn field(args: &Json, key: &str) -> String {
    args.get(key).and_then(|value| value.as_str()).unwrap_or("").trim().to_string()
}

fn limit_field(args: &Json, default: usize, max: usize) -> usize {
    limit_named(args, "limit", default, max)
}

fn limit_named(args: &Json, key: &str, default: usize, max: usize) -> usize {
    let Some(raw) = args.get(key) else {
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

fn latest_messages(conn: &Connection, args: &Json) -> Json {
    let count = limit_named(args, "count", 12, 40);
    let mut forwarded = args.clone();
    if let Some(object) = forwarded.as_object_mut() {
        object.insert("limit".to_string(), json!(count));
    }
    let mut value = get_messages(conn, &forwarded);
    if value.get("messages").is_some() {
        value["note"] = json!("These are the newest messages. Answer the user from them and do not call another tool.");
    }
    value
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
        let local = tools();
        let names: Vec<_> = local.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, ["search_conversations", "get_messages", "latest_messages", "list_chats"]);
        let exported = openai_tools(&local);
        assert_eq!(exported.as_array().unwrap().len(), 4);
        assert_eq!(exported[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn a_context_overflow_is_told_apart_from_other_endpoint_errors() {
        let ollama = http_failure(400, r#"{"error":"the request exceeds the available context size (4096 tokens)"}"#);
        assert!(ollama.starts_with("The request is larger than the model's context window."));
        assert!(ollama.contains("4096"));
        let openai = http_failure(400, r#"{"error":{"message":"This model's maximum context length is 8192 tokens."}}"#);
        assert!(openai.starts_with("The request is larger than the model's context window."));
        let other = http_failure(401, r#"{"error":{"message":"Invalid API key."}}"#);
        assert_eq!(other, "The endpoint returned 401: Invalid API key.");
        let bare = http_failure(500, "nope");
        assert_eq!(bare, "The endpoint returned 500.");
    }

    #[test]
    fn an_mcp_tool_is_offered_beside_the_local_tools() {
        let mut offered = tools();
        offered.push(Tool {
            name: "mcp0_ping".into(),
            description: "Ping a server. (MCP)".into(),
            schema: json!({ "type": "object", "properties": {} }),
        });
        let body = request_with(&knobs(Protocol::OpenAi), &hi(Protocol::OpenAi), &offered);
        let names: Vec<_> = body["tools"].as_array().unwrap().iter().map(|tool| tool["function"]["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search_conversations"));
        assert!(names.contains(&"mcp0_ping"));
        let anthropic = request_with(&knobs(Protocol::Anthropic), &hi(Protocol::Anthropic), &offered);
        assert!(anthropic["system"].as_str().unwrap().contains("MCP tools"));
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
    fn latest_messages_returns_the_newest_count() {
        let conn = db();
        chat(&conn, "c1", "Bangkok Pool League Captains", "group", "later", 30, Some(4));
        chat(&conn, "c2", "Bangkok Pool League", "group", "other", 20, Some(4));
        message(&conn, "c1", "m1", false, "Ada", "first", 10, 1);
        message(&conn, "c1", "m2", false, "Bea", "second", 20, 2);
        message(&conn, "c1", "m3", true, "", "third", 30, 3);
        let latest = call_tool(&conn, "latest_messages", &json!({ "chat": "captains", "count": 2 }));
        assert_eq!(latest["chat"]["id"], "c1");
        assert_eq!(latest["messages"].as_array().unwrap().len(), 2);
        assert_eq!(latest["messages"][0]["text"], "second");
        assert_eq!(latest["messages"][1]["text"], "third");
        assert!(latest["note"].as_str().unwrap().contains("newest"));
        let ambiguous = call_tool(&conn, "latest_messages", &json!({ "chat": "Bangkok" }));
        assert!(ambiguous["error"].as_str().unwrap().contains("Several"));
    }

    #[test]
    fn final_answer_request_drops_tools_and_asks_for_the_reply() {
        let mut body = build_request(&knobs(Protocol::OpenAi), &hi(Protocol::OpenAi));
        body["tools"] = json!([{ "type": "function" }]);
        body["tool_choice"] = json!("auto");
        strip_tools(&mut body);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());

        let mut openai = vec![json!({ "role": "tool", "tool_call_id": "c", "content": "{}" })];
        nudge_answer(Protocol::OpenAi, &mut openai);
        assert_eq!(openai.last().unwrap()["content"], ANSWER_NOW);

        let mut anthropic = vec![json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t", "content": "{}" }] })];
        nudge_answer(Protocol::Anthropic, &mut anthropic);
        assert_eq!(anthropic[0]["content"][1]["text"], ANSWER_NOW);

        let mut gemini = vec![json!({ "role": "user", "parts": [{ "functionResponse": { "name": "latest_messages", "response": {} } }] })];
        nudge_answer(Protocol::Gemini, &mut gemini);
        assert_eq!(gemini[0]["parts"][1]["text"], ANSWER_NOW);
    }

    #[test]
    fn auto_reply_skips_our_own_last_message_and_keeps_the_guidance() {
        let thread = vec![
            ComposeTurn { mine: true, name: String::new(), text: "hello".to_string() },
            ComposeTurn { mine: false, name: "Ada".to_string(), text: "are you coming?".to_string() },
        ];
        assert!(!last_is_mine(&thread));
        assert_eq!(
            reply_transcript(&thread, "English"),
            "Messages:\nMe: hello\nAda: are you coming?\n\nYou are Me. Text Ada back. Answer their last message in the first person, as a friend texting. If they asked what you did and the messages above do not say, answer briefly as yourself. Output only the text you send them. Reply in English. Write only in English, even if earlier messages used another language."
        );
        assert_eq!(reply_language_for(&thread, ""), "English");
        assert_eq!(reply_language_for(&thread, "ja"), "Japanese");
        assert_eq!(detect_language_name("What did you do last night?"), Some("English"));
        assert!(reply_system("Be brief and friendly.", "").contains("Be brief and friendly."));
        assert_eq!(reply_system("  ", ""), REPLY_SYSTEM);
        let japanese = reply_system("", "ja");
        assert!(japanese.contains("Reply in Japanese."));
        assert!(!japanese.contains("Match the language"));
        let own = vec![ComposeTurn { mine: false, name: "Ada".to_string(), text: "hi".to_string() }, ComposeTurn { mine: true, name: String::new(), text: "on my way".to_string() }];
        assert!(last_is_mine(&own));
        assert!(sounds_like_advice("Suppose you enjoyed something special and want to hear your side of the story!"));
        assert!(!sounds_like_advice("Not much, I stayed in. How was your night?"));
        assert!(check_auto_context(&"x".repeat(4001)).is_err());
        assert!(check_auto_context("Bring snacks.").is_ok());
    }

    #[test]
    fn each_chat_keeps_its_own_endpoint_and_context() {
        let mut root = json!({ "minimizeToTray": true });
        let first = ChatLlm {
            endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
            model: "qwen".into(),
            api_key: String::new(),
            protocol: "openai".into(),
            temperature: String::new(),
            top_p: String::new(),
            max_tokens: String::new(),
            thinking: "off".into(),
            extra: String::new(),
            context: "Reply in English.".into(),
            language: "en".into(),
            mark_ai: false,
            interrupt: false,
        };
        let second = ChatLlm { context: "Reply in Thai.".into(), model: "other".into(), ..first.clone() };
        upsert_chat_llm(&mut root, "u111", first);
        upsert_chat_llm(&mut root, "u222", second);
        assert_eq!(root["minimizeToTray"], true);
        assert_eq!(read_chat_llm(&root, "u111").unwrap().context, "Reply in English.");
        assert_eq!(read_chat_llm(&root, "u222").unwrap().model, "other");
        assert!(read_chat_llm(&root, "u333").is_none());
        remove_chat_llm(&mut root, "u111");
        assert!(read_chat_llm(&root, "u111").is_none());
        assert_eq!(read_chat_llm(&root, "u222").unwrap().context, "Reply in Thai.");
    }

    #[test]
    fn saved_chat_ids_are_listed_in_order() {
        let root = json!({
            "minimizeToTray": true,
            "chatLlm": {
                "u20": { "endpoint": "http://127.0.0.1/v1", "model": "a" },
                "u10": { "endpoint": "http://127.0.0.1/v1", "model": "b" }
            }
        });
        assert_eq!(chat_llm_ids(&root), vec!["u10".to_string(), "u20".to_string()]);
        assert!(chat_llm_ids(&json!({})).is_empty());
    }

    #[test]
    fn auto_reply_thinking_off_asks_ollama_not_to_think() {
        let mut body = json!({ "model": "qwen3.5:4b" });
        silence_thinking(Protocol::OpenAi, Thinking::Off, &mut body);
        assert_eq!(body["think"], false);
        assert_eq!(body["reasoning_effort"], "none");
        silence_thinking(Protocol::OpenAi, Thinking::Default, &mut body);
        assert_eq!(body["think"], false);
        assert_eq!(body["reasoning_effort"], "none");
        let mut left = json!({});
        silence_thinking(Protocol::OpenAi, Thinking::Default, &mut left);
        assert!(left.get("think").is_none());
        assert!(left.get("reasoning_effort").is_none());
    }

    #[test]
    fn a_thinking_model_with_blank_text_still_counts_as_responding() {
        let payload = json!({
            "choices": [{
                "message": { "role": "assistant", "content": "", "reasoning": "Thinking Process:" }
            }]
        });
        let step = parse_reply(Protocol::OpenAi, &payload).expect("reply");
        assert!(step.text.is_empty());
        assert!(probe_answered(Protocol::OpenAi, &payload, &step));
        let blank = json!({ "choices": [{ "message": { "role": "assistant", "content": "" } }] });
        let empty = parse_reply(Protocol::OpenAi, &blank).expect("reply");
        assert!(!probe_answered(Protocol::OpenAi, &blank, &empty));
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
            stream: false,
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
        assert_eq!(body["tools"][0]["functionDeclarations"].as_array().unwrap().len(), 4);
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

    fn played(protocol: Protocol, raw: &str) -> (Step, String, Vec<String>) {
        let mut fold = StreamFold::new(protocol);
        let mut shown = String::new();
        let mut names = Vec::new();
        for line in raw.lines() {
            let (delta, calls) = fold.accept(line);
            shown.push_str(&delta);
            names.extend(calls);
        }
        (fold.finish().expect("stream"), shown, names)
    }

    #[test]
    fn stream_toggle_sets_the_request_and_keeps_the_gemini_url() {
        let mut off = knobs(Protocol::OpenAi);
        off.extra = json!({ "stream": true });
        assert!(build_request(&off, &hi(Protocol::OpenAi)).get("stream").is_none());
        let mut on = knobs(Protocol::OpenAi);
        on.stream = true;
        assert_eq!(build_request(&on, &hi(Protocol::OpenAi))["stream"], true);
        assert_eq!(stream_endpoint(Protocol::OpenAi, "http://vr.local.net:11434/v1/chat/completions"), "http://vr.local.net:11434/v1/chat/completions");
        assert_eq!(
            stream_endpoint(Protocol::Gemini, "https://example.test/v1beta/models/gemini:generateContent"),
            "https://example.test/v1beta/models/gemini:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            stream_endpoint(Protocol::Gemini, "https://example.test/models/gemini:streamGenerateContent?key=1"),
            "https://example.test/models/gemini:streamGenerateContent?key=1&alt=sse"
        );
    }

    #[test]
    fn each_protocol_streams_text_and_a_tool_call() {
        let (step, shown, names) = played(
            Protocol::OpenAi,
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
             data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_chats\",\"arguments\":\"\"}}]}}]}\n\
             data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"kind\\\":\\\"group\\\"}\"}}]}}]}\n\
             data: [DONE]\n",
        );
        assert_eq!(shown, "Hello");
        assert_eq!(step.text, "Hello");
        assert_eq!(names, vec!["list_chats".to_string()]);
        assert_eq!(step.calls[0].1, "list_chats");
        assert_eq!(step.calls[0].2, json!("{\"kind\":\"group\"}"));

        let (step, shown, names) = played(
            Protocol::Anthropic,
            "event: content_block_start\n\
             data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"secret\"}}\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig\"}}\n\
             data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
             data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\
             data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"get_messages\",\"input\":{}}}\n\
             data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"chat\\\":\\\"Ada\\\"}\"}}\n",
        );
        assert_eq!(shown, "Hi");
        assert_eq!(step.text, "Hi");
        assert!(!step.text.contains("secret"));
        assert_eq!(names, vec!["get_messages".to_string()]);
        assert_eq!(step.calls[0].2["chat"], "Ada");
        assert_eq!(step.assistant["content"][0]["thinking"], "secret");
        assert_eq!(step.assistant["content"][0]["signature"], "sig");

        let (step, shown, names) = played(
            Protocol::Gemini,
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"secret\",\"thought\":true}]}}]}\n\
             data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Look\"}]}}]}\n\
             data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" here\"}]}}]}\n\
             data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"search_conversations\",\"args\":{\"query\":\"park\"}}}]}}]}\n",
        );
        assert_eq!(shown, "Look here");
        assert_eq!(step.text, "Look here");
        assert_eq!(names, vec!["search_conversations".to_string()]);
        assert_eq!(step.calls[0].2["query"], "park");
        assert_eq!(step.assistant["parts"][1]["text"], "Look here");

        let mut fold = StreamFold::new(Protocol::OpenAi);
        fold.accept("data: {\"error\":{\"message\":\"bad key\"}}");
        match fold.finish() {
            Err(message) => assert!(message.contains("bad key")),
            Ok(_) => panic!("expected the stream error"),
        }
    }
}
