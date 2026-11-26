//! Talk, QR login, chats, history, send, and letter sealing.

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

use crate::e2ee::{self, open_text, seal_text};
use crate::legy::{needs_legy, LegyTransport, LEGY_HOST, LEGY_LAP, LEGY_PATH};
use crate::thrift::{self, bin, Field, Value};

const APP_VERSION: &str = "26.6.0.4004";
const SYSTEM_VERSION: &str = "10.0.26100-11NT";
const DEFAULT_ENDPOINT: &str = "legy-jp.line-apps.com";
const LOGIN_PATH: &str = "/acct/lgn/sq/v1";
const LOGIN_POLL_PATH: &str = "/acct/lp/lgn/sq/v1";
const TALK_PATH: &str = "/S4";
const HISTORY_PAGE: i32 = 30;
const HISTORY_TARGET: usize = 40;
const POLL_HOLD: Duration = Duration::from_secs(185);
const POLL_PROBE: Duration = Duration::from_secs(25);
const FALLBACK_EVERY: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, PartialEq, Eq)]
struct PollRoute {
    path: &'static str,
    method: &'static str,
}

const POLL_ROUTES: [PollRoute; 5] = [
    PollRoute { path: "/P4", method: "sync" },
    PollRoute { path: "/SYNC4", method: "sync" },
    PollRoute { path: "/S4", method: "sync" },
    PollRoute { path: "/S4", method: "fetchOps" },
    PollRoute { path: "/P4", method: "fetchOps" },
];

#[derive(Clone, Copy, Default)]
struct OpCursor {
    local: i64,
    global: i64,
    individual: i64,
}

enum InboxNotice {
    Message { id: String, message: BTreeMap<i16, Value>, incoming: bool },
    Read { id: String },
    Refresh { id: String },
}

#[derive(Debug)]
pub struct LineError {
    pub message: String,
    pub code: Option<i64>,
    pub reason: String,
}

impl std::fmt::Display for LineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

enum CallFail {
    Line(LineError),
    TimedOut(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredKey {
    #[serde(rename = "keyId")]
    key_id: i64,
    #[serde(rename = "privateKey")]
    private_key: String,
    #[serde(rename = "publicKey")]
    public_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Session {
    #[serde(rename = "accessToken", default)]
    access_token: String,
    #[serde(rename = "refreshToken", default)]
    refresh_token: String,
    #[serde(default)]
    certificate: String,
    #[serde(default)]
    mid: String,
    #[serde(default)]
    expire: Option<i64>,
    #[serde(rename = "sqrPrivateKey", default)]
    sqr_private_key: String,
    #[serde(default = "default_endpoint")]
    endpoint: String,
    #[serde(rename = "e2eePrivateKey", default)]
    e2ee_private_key: String,
    #[serde(rename = "e2eePublicKey", default)]
    e2ee_public_key: String,
    #[serde(rename = "e2eeKeyId", default)]
    e2ee_key_id: Option<i64>,
    #[serde(rename = "e2eeKeys", default, skip_serializing_if = "Vec::is_empty")]
    e2ee_keys: Vec<StoredKey>,
}

fn default_endpoint() -> String {
    DEFAULT_ENDPOINT.to_string()
}

impl Default for Session {
    fn default() -> Self {
        Self {
            access_token: String::new(),
            refresh_token: String::new(),
            certificate: String::new(),
            mid: String::new(),
            expire: None,
            sqr_private_key: String::new(),
            endpoint: std::env::var("LINE_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.into()),
            e2ee_private_key: String::new(),
            e2ee_public_key: String::new(),
            e2ee_key_id: None,
            e2ee_keys: Vec::new(),
        }
    }
}

pub(crate) fn config_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("LINE_CONFIG") {
        return PathBuf::from(override_path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("lineapp.local.json")
}

fn load_session() -> Session {
    let mut session = Session::default();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(parsed) = serde_json::from_str::<Session>(&text) {
            session = parsed;
        }
    }
    if let Ok(token) = std::env::var("LINE_AUTH_TOKEN") {
        session.access_token = token.trim().to_string();
    }
    if let Ok(token) = std::env::var("LINE_REFRESH_TOKEN") {
        session.refresh_token = token.trim().to_string();
    }
    if let Ok(certificate) = std::env::var("LINE_CERTIFICATE") {
        session.certificate = certificate.trim().to_string();
    }
    if std::env::var("LINE_ENDPOINT").is_ok() {
        session.endpoint = std::env::var("LINE_ENDPOINT").unwrap_or(session.endpoint);
    }
    session
}

pub fn clear_session() {
    let path = config_path();
    if path.is_file() {
        let _ = std::fs::remove_file(path);
    }
    crate::store::clear();
}

pub fn signed_in() -> bool {
    !load_session().access_token.is_empty()
}

fn save_session(session: &Session) -> Result<PathBuf, LineError> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| line(err.to_string()))?;
    }
    let payload = serde_json::to_string_pretty(session).map_err(|err| line(err.to_string()))? + "\n";
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|err| line(err.to_string()))?;
    file.write_all(payload.as_bytes()).map_err(|err| line(err.to_string()))?;
    let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    Ok(path)
}

fn line(message: impl Into<String>) -> LineError {
    LineError { message: message.into(), code: None, reason: String::new() }
}

fn line_code(message: impl Into<String>, code: Option<i64>, reason: impl Into<String>) -> LineError {
    LineError { message: message.into(), code, reason: reason.into() }
}

fn app_version() -> String {
    std::env::var("LINE_APP_VERSION").unwrap_or_else(|_| APP_VERSION.into())
}

fn system_version() -> String {
    std::env::var("LINE_SYSTEM_VERSION").unwrap_or_else(|_| SYSTEM_VERSION.into())
}

fn application_header() -> String {
    format!("DESKTOPWIN\t{}\tWINDOWS\t{}", app_version(), system_version())
}

fn user_agent() -> String {
    format!("DESKTOP:WINDOWS:{}({})", system_version(), app_version())
}

pub struct Client {
    session: Session,
    seq: i32,
    legy: Option<LegyTransport>,
    keys: Option<Vec<(Vec<u8>, i64)>>,
    listed: Option<Vec<(i64, Vec<u8>)>>,
    pubs: HashMap<(String, i64), Option<Vec<u8>>>,
    groups: HashMap<(String, i64), Option<Vec<u8>>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Client {
    pub fn new() -> Self {
        Self {
            session: load_session(),
            seq: 0,
            legy: None,
            keys: None,
            listed: None,
            pubs: HashMap::new(),
            groups: HashMap::new(),
            cancel: None,
        }
    }

    pub fn with_cancel(cancel: Arc<AtomicBool>) -> Self {
        let mut client = Self::new();
        client.cancel = Some(cancel);
        client
    }

    fn next_seq(&mut self) -> i32 {
        self.seq += 1;
        self.seq
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    fn require_token(&self) -> Result<(), LineError> {
        if self.session.access_token.is_empty() {
            return Err(line(format!(
                "No LINE session. Write accessToken to {}",
                config_path().display()
            )));
        }
        Ok(())
    }

    fn call(
        &mut self,
        method: &str,
        fields: &[(i16, Field)],
        path: &str,
        timeout: Duration,
        extra: &[(&str, String)],
        access_token: Option<&str>,
    ) -> Result<Option<Value>, CallFail> {
        let body = thrift::write_call(method, fields);
        let token = access_token
            .map(str::to_string)
            .unwrap_or_else(|| self.session.access_token.clone());
        let use_legy = !token.is_empty() && needs_legy(&token);
        let (status, payload, headers) = self.post(path, &body, &token, timeout, extra, use_legy)?;
        self.parse(method, path, status, &payload, &headers)
    }

    fn parse(
        &mut self,
        method: &str,
        path: &str,
        status: u16,
        payload: &[u8],
        headers: &HashMap<String, String>,
    ) -> Result<Option<Value>, CallFail> {
        if let Some(next_token) = headers.get("x-line-next-access") {
            self.session.access_token = next_token.clone();
            let _ = save_session(&self.session);
        }
        if payload.is_empty() {
            if status == 408 || status == 410 {
                return Err(CallFail::TimedOut(format!("{method} long-poll timed out (HTTP {status})")));
            }
            if status >= 400 {
                return Err(CallFail::Line(line(format!("{method} {path} failed with HTTP {status}"))));
            }
            return Ok(None);
        }
        if payload.first() != Some(&0x82) {
            let snippet: String = payload.iter().take(40).map(|byte| format!("{byte:02x}")).collect();
            return Err(CallFail::Line(line(format!("{method} {path} HTTP {status}, not thrift: {snippet}"))));
        }
        let message = thrift::read_message(payload).map_err(|err| CallFail::Line(line(format!("{method} {path} HTTP {status}: {err}"))))?;
        if message.mtype == 3 || (message.fields.contains_key(&1) && !message.fields.contains_key(&0)) {
            let (code, reason) = exception_parts(message.fields.get(&1));
            let detail = if reason.is_empty() {
                code.map(|value| format!("code {value}")).unwrap_or_else(|| "exception".into())
            } else {
                reason.clone()
            };
            return Err(CallFail::Line(line_code(
                format!("{method} {path} failed: {detail}"),
                code,
                reason,
            )));
        }
        if status >= 400 && !message.fields.contains_key(&0) {
            return Err(CallFail::Line(line(format!("{method} {path} failed with HTTP {status}"))));
        }
        Ok(message.fields.get(&0).cloned())
    }

    fn post(
        &mut self,
        path: &str,
        body: &[u8],
        token: &str,
        timeout: Duration,
        extra: &[(&str, String)],
        legy_on: bool,
    ) -> Result<(u16, Vec<u8>, HashMap<String, String>), CallFail> {
        if legy_on {
            return self.post_legy(path, body, token, timeout);
        }
        let mut headers = vec![
            ("accept", "application/x-thrift".into()),
            ("user-agent", user_agent()),
            ("x-line-application", application_header()),
            ("content-type", "application/x-thrift".into()),
            ("x-lal", std::env::var("LINE_LAL").unwrap_or_else(|_| "ja_JP".into())),
            ("x-lpv", "1".into()),
            ("x-lap", LEGY_LAP.into()),
            ("x-lhm", "POST".into()),
            ("accept-encoding", "gzip".into()),
        ];
        if !token.is_empty() {
            headers.push(("x-line-access", token.to_string()));
        }
        for (key, value) in extra {
            headers.push((key, value.clone()));
        }
        https(&self.session.endpoint, path, body, &headers, timeout)
    }

    fn post_legy(
        &mut self,
        path: &str,
        body: &[u8],
        token: &str,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>, HashMap<String, String>), CallFail> {
        if self.legy.is_none() {
            self.legy = Some(LegyTransport::new());
        }
        let transport = self.legy.as_mut().unwrap();
        let sealed = transport.seal(path, token, body).map_err(|err| CallFail::Line(line(err)))?;
        let x_lcs = transport.x_lcs().map_err(|err| CallFail::Line(line(err)))?;
        let headers = [
            ("x-line-application", application_header()),
            ("x-le", "7".into()),
            ("x-lap", LEGY_LAP.into()),
            ("x-lpv", "1".into()),
            ("x-lcs", x_lcs),
            ("user-agent", user_agent()),
            ("content-type", "application/x-thrift".into()),
            ("x-lal", std::env::var("LINE_LAL").unwrap_or_else(|_| "ja_JP".into())),
            ("x-lhm", "POST".into()),
            ("accept", "*/*".into()),
            ("accept-encoding", "gzip".into()),
        ];
        let (status, payload, response_headers) = https(LEGY_HOST, LEGY_PATH, &sealed, &headers, timeout)?;
        if payload.is_empty() {
            return Ok((status, payload, response_headers));
        }
        let opened = self
            .legy
            .as_ref()
            .unwrap()
            .open(&payload)
            .map_err(|err| CallFail::Line(line(err)))?;
        let mut merged = response_headers;
        for (key, value) in opened.headers {
            merged.insert(key.to_ascii_lowercase(), value);
        }
        let mut inner = status;
        if let Some(code) = merged.get("x-lc") {
            if code != "200" {
                if let Ok(parsed) = code.parse::<u16>() {
                    inner = parsed;
                }
            }
        }
        Ok((inner, opened.body, merged))
    }

    /// Chat list for the window.
    ///
    /// Names, photos, and sort times come from `getChats` and `getMessageBoxes`.
    /// A preview already stored locally is kept when the message box is not newer.
    /// The shell is emitted before any per-chat message fetch. Missing previews
    /// follow as `refresh` poll updates.
    pub fn list_chats_with<F: FnMut(Json)>(&mut self, mut emit: F) -> Result<Json, LineError> {
        self.require_token()?;
        let issued = self
            .talk(
                "getAllChatMids",
                &[
                    (1, Field::Struct(vec![(1, Field::Bool(true)), (2, Field::Bool(true))])),
                    (2, Field::I32(0)),
                ],
            )?;
        let issued = issued.unwrap_or(Value::Struct(BTreeMap::new()));
        let issued = as_struct(&issued)?;
        let mut mids = id_list(issued.get(&1));
        mids.extend(id_list(issued.get(&2)));
        let mut failures = Vec::new();
        let mut raw_chats = Vec::new();
        for batch in mids.chunks(50) {
            let items: Vec<Field> = batch.iter().cloned().map(bin).collect();
            match self.talk(
                "getChats",
                &[(
                    1,
                    Field::Struct(vec![
                        (1, Field::List(thrift::BINARY, items)),
                        (2, Field::Bool(true)),
                        (3, Field::Bool(false)),
                    ]),
                )],
            ) {
                Ok(Some(value)) => {
                    if let Ok(response) = as_struct(&value) {
                        if let Some(Value::List(chats)) = response.get(&1) {
                            for chat in chats {
                                if let Value::Struct(fields) = chat {
                                    raw_chats.push(fields.clone());
                                }
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => failures.push(json!({"name": "chat list", "error": err.message})),
            }
        }
        let box_rows = match self.message_box_rows() {
            Ok(rows) => rows,
            Err(err) => {
                failures.push(json!({"name": "unread", "error": err.message}));
                Vec::new()
            }
        };
        let (unread, unread_total, box_ids) = unread_from_rows(&box_rows);
        let box_when: HashMap<String, i64> = box_rows.into_iter().map(|(mid, _, when)| (mid, when)).collect();
        let known: std::collections::HashSet<String> = raw_chats.iter().map(|chat| text_field(chat, 2)).collect();
        let direct_mids: Vec<String> = box_ids
            .into_iter()
            .filter(|mid| mid.starts_with('u') && !known.contains(mid))
            .collect();
        let saved = crate::store::cached_rows();
        let cards = self.contact_cards(&direct_mids);
        let mut rows = Vec::new();
        let mut stale = Vec::new();
        for chat in &raw_chats {
            let mut name = {
                let raw = text_field(chat, 6);
                if raw.trim().is_empty() { "Unnamed chat".into() } else { raw.trim().to_string() }
            };
            let mid = text_field(chat, 2);
            if name == "Unnamed chat" {
                if let Some(prior) = saved.get(&mid) {
                    if !prior.name.is_empty() {
                        name = prior.name.clone();
                    }
                }
            }
            let prior = saved.get(&mid);
            let activity = box_when.get(&mid).copied().unwrap_or(0);
            let (preview, sealed, sticker, fallback, prior_time) = saved_preview(prior);
            if needs_fresh_preview(&preview, activity, prior_time) {
                stale.push(mid.clone());
            }
            rows.push(json!({
                "id": mid,
                "name": name,
                "lastMessage": preview,
                "sealed": sealed,
                "time": prior_time.max(activity),
                "members": member_count(chat),
                "picture": picture_url(&text_field(chat, 7)),
                "muted": bool_field(chat, 4),
                "kind": chat_kind(&mid),
                "unread": unread.get(&mid).copied(),
                "sticker": sticker,
                "stickerFallback": fallback,
            }));
        }
        for mid in &direct_mids {
            let prior = saved.get(mid);
            let card = cards.get(mid);
            let name = card
                .map(|item| item.0.clone())
                .filter(|item| !item.is_empty())
                .or_else(|| prior.map(|item| item.name.clone()).filter(|item| !item.is_empty()))
                .unwrap_or_else(|| "Direct chat".into());
            let picture = card
                .map(|item| item.1.clone())
                .filter(|item| !item.is_empty())
                .or_else(|| prior.map(|item| item.picture.clone()))
                .unwrap_or_default();
            let activity = box_when.get(mid).copied().unwrap_or(0);
            let (preview, sealed, sticker, fallback, prior_time) = saved_preview(prior);
            if needs_fresh_preview(&preview, activity, prior_time) {
                stale.push(mid.clone());
            }
            rows.push(json!({
                "id": mid,
                "name": name,
                "lastMessage": preview,
                "sealed": sealed,
                "time": prior_time.max(activity),
                "members": Json::Null,
                "picture": picture,
                "muted": false,
                "kind": chat_kind(mid),
                "unread": unread.get(mid).copied(),
                "sticker": sticker,
                "stickerFallback": fallback,
            }));
        }
        sort_chat_rows(&mut rows);
        let profile = self.list_profile();
        let shell = json!({
            "type": "chats",
            "profile": profile,
            "chats": rows,
            "failures": failures,
            "unreadTotal": unread_total,
        });
        emit(shell);
        for batch in stale.chunks(8) {
            let mut fetched = Vec::new();
            let mut senders = Vec::new();
            for mid in batch {
                let (message, when) = self.latest_message(mid, "chat", &mut failures);
                if let Some(message) = &message {
                    let sender = text_field(message, 1);
                    if !sender.is_empty() && sender != self.session.mid {
                        senders.push(sender);
                    }
                }
                fetched.push((mid.clone(), message, when));
            }
            let cards = self.contact_cards(&senders);
            let mut updates = Vec::new();
            for (mid, message, when) in fetched {
                let Some(message) = message else {
                    if when > 0 {
                        patch_time(&mut rows, &mid, when);
                        updates.push(json!({
                            "id": mid,
                            "lastMessage": "",
                            "time": when,
                            "sealed": false,
                            "keepPreview": true,
                            "refresh": true,
                            "incoming": false,
                        }));
                    }
                    continue;
                };
                let sender_id = text_field(&message, 1);
                let mut sender = cards.get(&sender_id).map(|card| card.0.clone()).unwrap_or_default();
                if !sender_id.is_empty() && sender_id == self.session.mid {
                    sender = "You".into();
                }
                let (preview, sealed) = self.visible_preview(&message, &sender);
                let (sticker, fallback) = sticker_pair(Some(&message));
                patch_preview(&mut rows, &mid, &preview, sealed, when, &sticker, &fallback);
                let mut item = self.message_update(&mid, &message, when, false, &cards);
                item["refresh"] = json!(true);
                item["incoming"] = json!(false);
                updates.push(item);
            }
            if !updates.is_empty() {
                emit(json!({
                    "type": "poll",
                    "unreadTotal": Json::Null,
                    "unreads": [],
                    "updates": updates,
                }));
            }
        }
        sort_chat_rows(&mut rows);
        Ok(json!({
            "type": "chats",
            "profile": profile,
            "chats": rows,
            "failures": failures,
            "unreadTotal": unread_total,
        }))
    }

    pub fn chat_history(&mut self, mid: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let page = self.message_page("getRecentMessagesV2", &[(2, bin(mid)), (3, Field::I32(HISTORY_PAGE))])?;
        let mut collected = take_messages(&[], &page);
        if page.len() >= HISTORY_PAGE as usize {
            while collected.len() < HISTORY_TARGET {
                let oldest = collected.iter().min_by_key(|message| message_time(message)).cloned();
                let Some(oldest) = oldest else { break };
                let ident = text_field(&oldest, 4);
                if !ident.chars().all(|ch| ch.is_ascii_digit()) || ident.is_empty() {
                    break;
                }
                let older = self.message_page(
                    "getPreviousMessagesV2WithRequest",
                    &[(
                        2,
                        Field::Struct(vec![
                            (1, bin(mid)),
                            (2, Field::Struct(vec![
                                (1, Field::I64(message_time(&oldest))),
                                (2, Field::I64(ident.parse().unwrap_or(0))),
                            ])),
                            (3, Field::I32(HISTORY_PAGE)),
                        ]),
                    )],
                )?;
                let before = collected.len();
                let older_len = older.len();
                collected = take_messages(&collected, &older);
                if collected.len() == before || older_len < HISTORY_PAGE as usize {
                    break;
                }
            }
        }
        collected.sort_by_key(message_time);
        if collected.len() > HISTORY_TARGET {
            collected = collected.split_off(collected.len() - HISTORY_TARGET);
        }
        let mut senders: Vec<String> = collected
            .iter()
            .map(|message| text_field(message, 1))
            .filter(|sender| !sender.is_empty() && sender != &self.session.mid)
            .collect();
        for message in &collected {
            senders.extend(event_mids(message).into_iter().filter(|mid| mid != &self.session.mid));
        }
        let cards = self.contact_cards(&senders);
        let mut messages = Vec::new();
        for message in &collected {
            let (text, sealed) = self.visible_text(message, &cards);
            if text.is_empty() {
                continue;
            }
            let sender = text_field(message, 1);
            let mine = !self.session.mid.is_empty() && sender == self.session.mid;
            let (sticker, fallback) = sticker_pair(Some(message));
            let kind = content_kind(int_field(message, 15));
            let media_id = if kind == "image" || kind == "video" { text_field(message, 4) } else { String::new() };
            messages.push(json!({
                "mine": mine,
                "name": if mine { "" } else { cards.get(&sender).map(|card| card.0.as_str()).unwrap_or("") },
                "picture": if mine { "" } else { cards.get(&sender).map(|card| card.1.as_str()).unwrap_or("") },
                "text": text,
                "sealed": sealed,
                "time": message_time(message),
                "sticker": sticker,
                "stickerFallback": fallback,
                "kind": kind,
                "gid": if media_id.is_empty() { String::new() } else { meta_value(message, "GID") },
                "seq": meta_value(message, "GSEQ").parse::<i64>().unwrap_or(0),
                "mediaId": media_id,
                "animated": !media_id.is_empty() && message_animated(message),
            }));
        }
        Ok(json!({"type": "history", "id": mid, "messages": messages}))
    }

    pub fn load_media(&mut self, chat: &str, ids: &[String], preview: bool, animated_ids: &[String]) -> Result<Json, LineError> {
        self.require_token()?;
        let mut wanted = Vec::new();
        for id in ids {
            if wanted.len() >= 24 {
                break;
            }
            if id.len() > 24 || id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit()) || wanted.iter().any(|have| have == id) {
                continue;
            }
            wanted.push(id.clone());
        }
        let marked: std::collections::HashSet<&str> = animated_ids.iter().map(String::as_str).collect();
        let mut located = HashMap::new();
        let pending: Vec<String> = wanted
            .iter()
            .filter(|id| read_media_cache(id, preview, marked.contains(id.as_str())).is_none())
            .cloned()
            .collect();
        if !pending.is_empty() {
            if let Ok(page) = self.message_page("getRecentMessagesV2", &[(2, bin(chat)), (3, Field::I32(50))]) {
                for message in page {
                    let id = text_field(&message, 4);
                    if pending.iter().any(|want| want == &id) {
                        located.insert(id, message);
                    }
                }
            }
        }
        let mut files = Vec::new();
        for id in wanted {
            let animated = marked.contains(id.as_str());
            match self.obtain_media(&id, preview, located.get(&id), animated) {
                Ok((mime, bytes)) => files.push(json!({"id": id, "mime": mime, "data": STANDARD.encode(bytes)})),
                Err(err) => files.push(json!({"id": id, "error": err.message})),
            }
        }
        Ok(Json::Array(files))
    }

    fn obtain_media(&mut self, id: &str, mut preview: bool, message: Option<&BTreeMap<i16, Value>>, mut animated: bool) -> Result<(String, Vec<u8>), LineError> {
        if message.is_some_and(message_animated) {
            animated = true;
        }
        if animated {
            preview = false;
        }
        if let Some(hit) = read_media_cache(id, preview, animated) {
            return Ok(hit);
        }
        let sid = message.map(|item| meta_value(item, "SID")).unwrap_or_default();
        let oid = message.map(|item| meta_value(item, "OID")).unwrap_or_default();
        if !sid.is_empty() && !oid.is_empty() {
            let object = if preview { format!("{oid}__ud-preview") } else { oid.clone() };
            let meta = talk_meta(id);
            let sealed = self.obs_get(&format!("/r/talk/{sid}/{}", path_segment(&object)), &[("x-talk-meta", meta.clone())]);
            let sealed = match sealed {
                Ok(hit) => Ok(hit),
                Err(err) if preview => self.obs_get(&format!("/r/talk/{sid}/{}", path_segment(&oid)), &[("x-talk-meta", meta)]).or(Err(err)),
                Err(err) => Err(err),
            };
            if let Ok((mime, mut bytes)) = sealed {
                if let Some(message) = message {
                    if bytes_list(message.get(&20)).is_some_and(|chunks| chunks.len() >= 5) {
                        let payload = self.sealed_payload(message).ok_or_else(|| line("could not open this photo"))?;
                        let material = payload.get("keyMaterial").and_then(|value| value.as_str()).ok_or_else(|| line("could not open this photo"))?;
                        bytes = e2ee::decrypt_file(&bytes, material).map_err(line)?;
                    }
                }
                let mime = prefer_mime(&mime, &bytes);
                write_media_cache(id, preview, animated, &mime, &bytes);
                return Ok((mime, bytes));
            }
        }
        // LINE stores the moving GIF at /original. The plain object and /preview are still JPEGs.
        let path = if animated {
            format!("/r/talk/m/{id}/original")
        } else if preview {
            format!("/r/talk/m/{id}/preview")
        } else {
            format!("/r/talk/m/{id}")
        };
        let (mime, bytes) = match self.obs_get(&path, &[]) {
            Ok(hit) => hit,
            Err(err) if animated || preview => self.obs_get(&format!("/r/talk/m/{id}"), &[]).map_err(|_| err)?,
            Err(err) => return Err(err),
        };
        let mime = prefer_mime(&mime, &bytes);
        write_media_cache(id, preview, animated, &mime, &bytes);
        Ok((mime, bytes))
    }

    fn obs_get(&self, path: &str, extra: &[(&str, String)]) -> Result<(String, Vec<u8>), LineError> {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(45)).build();
        let mut request = agent
            .get(&format!("https://obs.line-apps.com{path}"))
            .set("x-line-access", &self.session.access_token)
            .set("x-line-application", &application_header())
            .set("user-agent", &user_agent())
            .set("accept", "*/*");
        if !self.session.mid.is_empty() {
            request = request.set("x-line-mid", &self.session.mid);
        }
        for (key, value) in extra {
            request = request.set(key, value);
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => return Err(line(format!("photo download failed ({code})"))),
            Err(err) => return Err(line(format!("photo download failed: {err}"))),
        };
        let header = response.header("content-type").unwrap_or("").to_string();
        let mut bytes = Vec::new();
        response.into_reader().take(40_000_000).read_to_end(&mut bytes).map_err(|err| line(err.to_string()))?;
        if bytes.len() >= 40_000_000 {
            return Err(line("that file is too large to show here"));
        }
        if bytes.is_empty() {
            return Err(line("photo download was empty"));
        }
        Ok((header, bytes))
    }

    fn sealed_payload(&mut self, message: &BTreeMap<i16, Value>) -> Option<Json> {
        let chunks = bytes_list(message.get(&20))?;
        if chunks.len() < 5 {
            return None;
        }
        let sender = text_field(message, 1);
        let recipient = text_field(message, 2);
        let sender_key = i64::from(u32::from_be_bytes(chunks[3].as_slice().try_into().ok()?));
        let receiver_key = i64::from(u32::from_be_bytes(chunks[4].as_slice().try_into().ok()?));
        let spec = e2ee_spec(message);
        let kind = int_field(message, 15).unwrap_or(0);
        let me = self.session.mid.clone();
        let raw = if recipient.starts_with('c') || recipient.starts_with('r') {
            let group_priv = self.group_private(&recipient, receiver_key)?;
            let peer_pub = if sender == me { self.public_for_key(sender_key)? } else { self.peer_public(&sender, sender_key)? };
            e2ee::open_bytes(&chunks, &group_priv, &peer_pub, &sender, &recipient, spec, kind).ok()?
        } else {
            let mut opened = None;
            for (private, own_id) in self.owned_keys() {
                let (peer, peer_key) = if sender == me && sender_key == own_id {
                    (recipient.clone(), receiver_key)
                } else if receiver_key == own_id {
                    (sender.clone(), sender_key)
                } else {
                    continue;
                };
                let Some(public) = self.peer_public(&peer, peer_key) else { continue };
                if let Ok(bytes) = e2ee::open_bytes(&chunks, &private, &public, &sender, &recipient, spec, kind) {
                    opened = Some(bytes);
                    break;
                }
            }
            opened?
        };
        serde_json::from_slice(&raw).ok()
    }

    fn owned_keys(&mut self) -> Vec<(Vec<u8>, i64)> {
        let mut out = self.secret_keys();
        let stored = self.session.e2ee_keys.clone();
        let single_id = self.session.e2ee_key_id;
        let single = self.session.e2ee_private_key.clone();
        for key in stored {
            if key.key_id <= 0 {
                continue;
            }
            let Ok(private) = STANDARD.decode(&key.private_key) else { continue };
            if private.len() == 32 && !out.iter().any(|(_, id)| *id == key.key_id) {
                out.push((private, key.key_id));
            }
        }
        if let Some(id) = single_id {
            if id > 0 {
                if let Ok(private) = STANDARD.decode(single) {
                    if private.len() == 32 && !out.iter().any(|(_, have)| *have == id) {
                        out.push((private, id));
                    }
                }
            }
        }
        out
    }

    fn public_for_key(&mut self, key_id: i64) -> Option<Vec<u8>> {
        let private = self.owned_keys().into_iter().find(|(_, id)| *id == key_id)?.0;
        let secret = x25519_dalek::StaticSecret::from(<[u8; 32]>::try_from(private.as_slice()).ok()?);
        Some(x25519_dalek::PublicKey::from(&secret).as_bytes().to_vec())
    }

    fn group_private(&mut self, group: &str, key_id: i64) -> Option<Vec<u8>> {
        let cache = (group.to_string(), key_id);
        if let Some(found) = self.groups.get(&cache) {
            return found.clone();
        }
        let found = self.fetch_group_private(group, key_id);
        self.groups.insert(cache, found.clone());
        found
    }

    fn fetch_group_private(&mut self, group: &str, key_id: i64) -> Option<Vec<u8>> {
        let value = self.group_shared_key(group, key_id)?;
        let fields = as_struct(&value).ok()?;
        let group_key = int_field(fields, 2).unwrap_or(0);
        if key_id > 0 && group_key > 0 && group_key != key_id {
            return None;
        }
        let creator = text_field(fields, 3);
        let creator_kid = int_field(fields, 4)?;
        let receiver_kid = int_field(fields, 6)?;
        let encrypted = bytes_field(fields, 7);
        if encrypted.is_empty() || creator.is_empty() {
            return None;
        }
        let creator_pub = self.peer_public(&creator, creator_kid)?;
        let mut keys = self.owned_keys();
        keys.sort_by_key(|(_, id)| if *id == receiver_kid { 0 } else { 1 });
        for (private, _) in keys {
            if let Some(secret) = e2ee::open_group_private(&private, &creator_pub, &encrypted) {
                return Some(secret);
            }
        }
        None
    }

    fn group_shared_key(&mut self, group: &str, key_id: i64) -> Option<Value> {
        if let Ok(id) = i32::try_from(key_id) {
            if id > 0 {
                if let Ok(Some(value)) = self.talk(
                    "getE2EEGroupSharedKey",
                    &[(2, Field::I32(2)), (3, bin(group)), (4, Field::I32(id))],
                ) {
                    return Some(value);
                }
            }
        }
        self.talk("getLastE2EEGroupSharedKey", &[(2, Field::I32(2)), (3, bin(group))]).ok().flatten()
    }

    pub fn send_for_window(&mut self, to: &str, message: &str) -> Json {
        match self.send_text(to, message) {
            Ok(sent) => sent,
            Err(err) => {
                if err.code == Some(82) && to.starts_with('u') {
                    match self.send_sealed_text(to, message) {
                        Ok(sent) => sent,
                        Err(sealed) => send_failure(to, &sealed),
                    }
                } else {
                    send_failure(to, &err)
                }
            }
        }
    }

    pub fn poll_inbox(&mut self, known: &HashMap<String, i64>) -> Result<Json, LineError> {
        self.require_token()?;
        let boxes = self.message_box_rows()?;
        let mut unreads = Vec::new();
        let mut total = 0i64;
        let mut changed = Vec::new();
        for (mid, unread, when) in &boxes {
            if let Some(count) = unread {
                unreads.push(json!({"id": mid, "unread": count}));
                if *count > 0 {
                    total += count;
                }
            }
            let previous = known.get(mid);
            if previous.is_none() || when > previous.unwrap_or(&0) {
                changed.push((mid.clone(), *unread, *when));
            }
        }
        let mut pending = Vec::new();
        let mut senders = Vec::new();
        let mut failures = Vec::new();
        for (mid, unread, delivered) in changed.into_iter().take(40) {
            let (message, mut when) = self.latest_message(&mid, "inbox", &mut failures);
            if when == 0 {
                when = delivered;
            }
            if let Some(message) = &message {
                let sender = text_field(message, 1);
                if !sender.is_empty() && sender != self.session.mid {
                    senders.push(sender);
                }
            }
            pending.push((mid, unread, message, when));
        }
        let mut people = senders;
        for (mid, _, _, _) in &pending {
            if !known.contains_key(mid) && mid.starts_with('u') {
                people.push(mid.clone());
            }
        }
        let cards = self.contact_cards(&people);
        let mut updates = Vec::new();
        for (mid, unread, message, when) in pending {
            let (preview, sealed, sticker, fallback, keep_preview) = if let Some(message) = &message {
                let sender_id = text_field(&message.clone(), 1);
                let mut sender = cards.get(&sender_id).map(|card| card.0.clone()).unwrap_or_default();
                if !sender_id.is_empty() && sender_id == self.session.mid {
                    sender = "You".into();
                }
                let (preview, sealed) = self.visible_preview(message, &sender);
                let (sticker, fallback) = sticker_pair(Some(message));
                (preview, sealed, sticker, fallback, false)
            } else {
                (String::new(), false, String::new(), String::new(), when > 0)
            };
            let mut item = json!({
                "id": mid,
                "lastMessage": preview,
                "time": when,
                "sealed": sealed,
                "sticker": sticker,
                "stickerFallback": fallback,
                "unread": unread,
                "keepPreview": keep_preview,
            });
            if !known.contains_key(&mid) && mid.starts_with('u') {
                let card = cards.get(&mid);
                item["name"] = json!(card.map(|item| item.0.clone()).filter(|name| !name.is_empty()).unwrap_or_else(|| "Direct chat".into()));
                item["picture"] = json!(card.map(|item| item.1.clone()).unwrap_or_default());
                item["kind"] = json!("friend");
                item["members"] = Json::Null;
            }
            updates.push(item);
        }
        Ok(json!({
            "type": "poll",
            "unreadTotal": total,
            "unreads": unreads,
            "updates": updates,
        }))
    }

    /// Follow the desktop operation stream.
    ///
    /// The live route is `sync` on `/P4`: one request struct, and a reply whose
    /// operations sit under the operation response. The call stays open until
    /// LINE answers or the hold elapses. A route that answers at once is paced
    /// the way the desktop client paces a short-poll, so a quiet account does
    /// not spin. `fetchOps` is only tried after `sync` is rejected.
    pub fn watch_inbox<F: FnMut(Json)>(&mut self, mut emit: F) {
        if self.require_token().is_err() {
            return;
        }
        let mut cursor = match self.seed_cursor() {
            Ok(cursor) => cursor,
            Err(_) => OpCursor::default(),
        };
        let mut route: Option<PollRoute> = None;
        let mut skip: Option<PollRoute> = None;
        let mut proven = false;
        let mut quiet_rounds = 0u32;
        let mut last_health = Instant::now();
        let mut fallback_rev = cursor.local;
        let mut fallback_at = Instant::now().checked_sub(FALLBACK_EVERY).unwrap_or_else(Instant::now);
        loop {
            if self.cancelled() {
                return;
            }
            if cursor.local <= 0 {
                match self.seed_cursor() {
                    Ok(seeded) => cursor = seeded,
                    Err(err) => {
                        if session_dead(&err) || self.cancelled() {
                            return;
                        }
                        if self.pause(Duration::from_secs(5)) {
                            return;
                        }
                        continue;
                    }
                }
                fallback_rev = cursor.local;
            }
            if route.is_none() {
                match self.probe_route(&cursor, skip) {
                    Ok(Some((found, first))) => {
                        route = Some(found);
                        skip = None;
                        if let Some(value) = first.as_ref() {
                            if self.consume_poll(&mut cursor, value, &mut emit) {
                                proven = true;
                            }
                            if sync_has_more(value) {
                                continue;
                            }
                        }
                    }
                    Ok(None) => {
                        if self.fallback_scan(&mut fallback_rev, &mut fallback_at, &mut emit) {
                            return;
                        }
                        if self.pause(Duration::from_secs(5)) {
                            return;
                        }
                        continue;
                    }
                    Err(_) => return,
                }
            }
            let Some(current) = route else { continue };
            let started = Instant::now();
            match self.poll_call(current, &cursor, 50, POLL_HOLD) {
                Ok(Some(value)) => {
                    let empty = operation_list(&value).is_empty();
                    if self.consume_poll(&mut cursor, &value, &mut emit) {
                        proven = true;
                    }
                    if sync_has_more(&value) {
                        continue;
                    }
                    if empty && self.cursor_stale(&cursor, proven, &mut last_health) {
                        skip = route;
                        route = None;
                        continue;
                    }
                    quiet_rounds = note_quiet(quiet_rounds, started.elapsed(), empty);
                    if self.pause(next_wait(started.elapsed(), quiet_rounds)) {
                        return;
                    }
                }
                Ok(None) => {
                    route = None;
                }
                Err(CallFail::TimedOut(_)) => {
                    quiet_rounds = 0;
                }
                Err(CallFail::Line(err)) => {
                    if session_dead(&err) || self.cancelled() {
                        return;
                    }
                    route = None;
                    if self.pause(Duration::from_secs(5)) {
                        return;
                    }
                }
            }
        }
    }

    fn seed_cursor(&mut self) -> Result<OpCursor, LineError> {
        let local = self.last_op_revision()?;
        if local <= 0 {
            return Err(line("operation revision was empty"));
        }
        Ok(OpCursor { local, global: 0, individual: 0 })
    }

    fn last_op_revision(&mut self) -> Result<i64, LineError> {
        match self.talk("getLastOpRevision", &[])? {
            Some(Value::Int(value)) => Ok(value),
            Some(value) => {
                let fields = as_struct(&value)?;
                Ok(int_field(fields, 0).unwrap_or(0))
            }
            None => Err(line("operation revision was empty")),
        }
    }

    fn probe_route(&mut self, cursor: &OpCursor, skip: Option<PollRoute>) -> Result<Option<(PollRoute, Option<Value>)>, LineError> {
        let mut deferred = None;
        for route in POLL_ROUTES {
            if skip == Some(route) {
                deferred = Some(route);
                continue;
            }
            if let Some(found) = self.consider_route(route, cursor)? {
                return Ok(Some(found));
            }
        }
        if let Some(route) = deferred {
            return Ok(self.consider_route(route, cursor)?);
        }
        Ok(None)
    }

    fn consider_route(&mut self, route: PollRoute, cursor: &OpCursor) -> Result<Option<(PollRoute, Option<Value>)>, LineError> {
        if self.cancelled() {
            return Ok(None);
        }
        match self.poll_call(route, cursor, 50, POLL_PROBE) {
            Ok(Some(value)) => Ok(Some((route, Some(value)))),
            Ok(None) | Err(CallFail::TimedOut(_)) => Ok(None),
            Err(CallFail::Line(err)) if session_dead(&err) => Err(err),
            Err(CallFail::Line(_)) => Ok(None),
        }
    }

    fn poll_call(&mut self, route: PollRoute, cursor: &OpCursor, count: i32, timeout: Duration) -> Result<Option<Value>, CallFail> {
        let fields;
        let extra;
        match route.method {
            "sync" => {
                fields = sync_fields(cursor, count);
                extra = poll_headers();
            }
            _ => {
                fields = fetch_ops_fields(cursor, count);
                extra = poll_headers();
            }
        }
        self.call(route.method, &fields, route.path, timeout, &extra, None)
    }

    fn consume_poll<F: FnMut(Json)>(&mut self, cursor: &mut OpCursor, value: &Value, emit: &mut F) -> bool {
        let ops = operation_list(value);
        let delivered = !ops.is_empty();
        for op in &ops {
            absorb_op(cursor, op);
        }
        if let Some(payload) = self.ops_to_poll(&ops) {
            emit(payload);
        }
        if needs_full_sync(value) {
            if let Ok(payload) = self.poll_inbox(&HashMap::new()) {
                emit(payload);
            }
        }
        delivered
    }

    fn cursor_stale(&mut self, cursor: &OpCursor, proven: bool, last_health: &mut Instant) -> bool {
        let every = if proven { Duration::from_secs(600) } else { Duration::from_secs(60) };
        if last_health.elapsed() < every {
            return false;
        }
        *last_health = Instant::now();
        match self.last_op_revision() {
            Ok(server) => server > cursor.local,
            Err(_) => false,
        }
    }

    fn fallback_scan<F: FnMut(Json)>(&mut self, fallback_rev: &mut i64, fallback_at: &mut Instant, emit: &mut F) -> bool {
        if fallback_at.elapsed() < FALLBACK_EVERY {
            return false;
        }
        *fallback_at = Instant::now();
        let server = match self.last_op_revision() {
            Ok(server) => server,
            Err(err) => return session_dead(&err),
        };
        if server <= *fallback_rev {
            return false;
        }
        *fallback_rev = server;
        if let Ok(payload) = self.poll_inbox(&HashMap::new()) {
            emit(payload);
        }
        false
    }

    fn ops_to_poll(&mut self, ops: &[BTreeMap<i16, Value>]) -> Option<Json> {
        let me = self.session.mid.clone();
        let mut notices = Vec::new();
        for op in ops {
            match int_field(op, 3).unwrap_or(-1) {
                25 | 26 => {
                    let Some(Value::Struct(message)) = op.get(&20) else { continue };
                    let id = chat_target(message, &me);
                    if id.is_empty() {
                        continue;
                    }
                    let incoming = int_field(op, 3) == Some(26) && text_field(message, 1) != me;
                    notices.push(InboxNotice::Message { id, message: message.clone(), incoming });
                }
                40 => {
                    let id = text_field(op, 10);
                    if looks_like_mid(&id) {
                        notices.push(InboxNotice::Read { id });
                    }
                }
                58 | 59 => {
                    let id = match op.get(&20) {
                        Some(Value::Struct(message)) => chat_target(message, &me),
                        _ => text_field(op, 10),
                    };
                    if looks_like_mid(&id) {
                        notices.push(InboxNotice::Refresh { id });
                    }
                }
                _ => {}
            }
        }
        if notices.is_empty() {
            return None;
        }
        let mut people = Vec::new();
        for notice in &notices {
            if let InboxNotice::Message { id, message, .. } = notice {
                if is_chat_update(message) {
                    continue;
                }
                let sender = text_field(message, 1);
                if !sender.is_empty() && sender != me {
                    people.push(sender);
                }
                if id.starts_with('u') {
                    people.push(id.clone());
                }
            }
        }
        let cards = self.contact_cards(&people);
        let mut failures = Vec::new();
        let mut updates = Vec::new();
        for notice in notices {
            match notice {
                InboxNotice::Read { id } => {
                    updates.push(json!({
                        "id": id,
                        "lastMessage": "",
                        "time": 0,
                        "sealed": false,
                        "read": true,
                    }));
                }
                InboxNotice::Refresh { id } => {
                    let (message, when) = self.latest_message(&id, "inbox", &mut failures);
                    let Some(message) = message else {
                        if when > 0 {
                            updates.push(json!({
                                "id": id,
                                "lastMessage": "",
                                "time": when,
                                "sealed": false,
                                "keepPreview": true,
                            }));
                        }
                        continue;
                    };
                    updates.push(self.message_update(&id, &message, when, false, &cards));
                }
                InboxNotice::Message { id, message, incoming } => {
                    let when = message_time(&message);
                    updates.push(self.message_update(&id, &message, when, incoming, &cards));
                }
            }
        }
        if updates.is_empty() {
            return None;
        }
        Some(json!({
            "type": "poll",
            "unreadTotal": Json::Null,
            "unreads": [],
            "updates": updates,
        }))
    }

    fn message_update(
        &mut self,
        id: &str,
        message: &BTreeMap<i16, Value>,
        when: i64,
        incoming: bool,
        cards: &HashMap<String, (String, String)>,
    ) -> Json {
        let sender_id = text_field(message, 1);
        let mut sender = cards.get(&sender_id).map(|card| card.0.clone()).unwrap_or_default();
        if !sender_id.is_empty() && sender_id == self.session.mid {
            sender = "You".into();
        }
        let chat_update = is_chat_update(message);
        let (preview, sealed, sticker, fallback) = if chat_update {
            (String::new(), false, String::new(), String::new())
        } else {
            let (preview, sealed) = self.visible_preview(message, &sender);
            let (sticker, fallback) = sticker_pair(Some(message));
            (preview, sealed, sticker, fallback)
        };
        let mut item = json!({
            "id": id,
            "lastMessage": preview,
            "time": when,
            "sealed": sealed,
            "sticker": sticker,
            "stickerFallback": fallback,
            "incoming": incoming,
            "keepPreview": chat_update,
        });
        if id.starts_with('u') {
            let card = cards.get(id);
            item["name"] = json!(card.map(|item| item.0.clone()).filter(|name| !name.is_empty()).unwrap_or_else(|| "Direct chat".into()));
            item["picture"] = json!(card.map(|item| item.1.clone()).unwrap_or_default());
            item["kind"] = json!("friend");
        } else if !id.is_empty() {
            item["name"] = json!(match chat_kind(id) {
                "group" => "Group",
                "room" => "Room",
                _ => "Chat",
            });
            item["kind"] = json!(chat_kind(id));
        }
        item
    }

    fn pause(&self, total: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < total {
            if self.cancelled() {
                return true;
            }
            let slice = Duration::from_millis(200).min(total.saturating_sub(started.elapsed()));
            if slice.is_zero() {
                break;
            }
            std::thread::sleep(slice);
        }
        self.cancelled()
    }

    pub fn login_qr<F: FnMut(Json)>(&mut self, mut emit: F) -> Result<(), LineError> {
        loop {
            if self.cancelled() {
                return Ok(());
            }
            match self.login_attempt(&mut emit) {
                Ok(()) => return Ok(()),
                Err(err) if err.message == "QR session expired" => {
                    emit(json!({"type": "status", "message": "That QR expired. Refreshing it."}));
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn login_attempt<F: FnMut(Json)>(&mut self, emit: &mut F) -> Result<(), LineError> {
        let created = self
            .call("createSession", &[(1, Field::Struct(vec![]))], LOGIN_PATH, Duration::from_secs(30), &[], Some(""))
            .map_err(fail_line)?;
        let created = as_struct(created.as_ref().ok_or_else(|| line("login session was empty"))?)?;
        let session_id = text_field(created, 1);
        let issued = self
            .call(
                "createQrCodeForSecure",
                &[(1, Field::Struct(vec![(1, bin(&session_id))]))],
                LOGIN_PATH,
                Duration::from_secs(30),
                &[],
                Some(""),
            )
            .map_err(fail_line)?;
        let issued = as_struct(issued.as_ref().ok_or_else(|| line("QR response was empty"))?)?;
        let mut url = text_field(issued, 1);
        let max_count = int_field(issued, 2).unwrap_or(12).max(1);
        let interval = int_field(issued, 3).unwrap_or(30).max(1);
        let nonce = bytes_field(issued, 4);
        let (private, secret_query) = sqr_secret();
        if url.contains('?') {
            url.push('&');
            url.push_str(secret_query.trim_start_matches('?'));
        } else {
            url.push_str(&secret_query);
        }
        emit(json!({"type": "qr", "url": url}));
        emit(json!({"type": "status", "message": "Scan this QR with the LINE app on your phone."}));
        let mut deadline = Instant::now() + Duration::from_secs((max_count * interval) as u64);
        self.finish_qr(&session_id, &nonce, &private, interval, &mut deadline, emit)
    }

    fn finish_qr<F: FnMut(Json)>(
        &mut self,
        session_id: &str,
        nonce: &[u8],
        private: &[u8],
        interval: i64,
        deadline: &mut Instant,
        emit: &mut F,
    ) -> Result<(), LineError> {
        let mut pin: Option<String> = None;
        while Instant::now() < *deadline {
            if self.cancelled() {
                return Ok(());
            }
            if pin.is_none() {
                match self.poll_once("checkQrCodeVerified", session_id, interval) {
                    Err(CallFail::TimedOut(_)) => continue,
                    Err(CallFail::Line(err)) if same_session_retry(&err) => {}
                    Err(CallFail::Line(err)) => return Err(err),
                    Ok(()) => {}
                }
                if self.verify_certificate(session_id) {
                    self.complete_login(session_id, nonce, private, emit)?;
                    return Ok(());
                }
                match self.issue_pin(session_id, emit) {
                    Ok(code) => {
                        pin = Some(code);
                        let extra = Instant::now() + Duration::from_secs(interval.max(1) as u64);
                        if extra > *deadline {
                            *deadline = extra;
                        }
                    }
                    Err(err) if same_session_retry(&err) => {
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                    Err(err) => return Err(err),
                }
            }
            match self.poll_once("checkPinCodeVerified", session_id, interval) {
                Err(CallFail::TimedOut(_)) => continue,
                Err(CallFail::Line(err)) if same_session_retry(&err) => {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
                Err(CallFail::Line(err)) => return Err(err),
                Ok(()) => {}
            }
            self.complete_login(session_id, nonce, private, emit)?;
            return Ok(());
        }
        Err(line("QR session expired"))
    }

    fn verify_certificate(&mut self, session_id: &str) -> bool {
        self.call(
            "verifyCertificate",
            &[(1, Field::Struct(vec![(1, bin(session_id)), (2, bin(&self.session.certificate))]))],
            LOGIN_PATH,
            Duration::from_secs(30),
            &[],
            Some(""),
        )
        .is_ok()
    }

    fn issue_pin<F: FnMut(Json)>(&mut self, session_id: &str, emit: &mut F) -> Result<String, LineError> {
        let pin = self
            .call(
                "createPinCode",
                &[(1, Field::Struct(vec![(1, bin(session_id))]))],
                LOGIN_PATH,
                Duration::from_secs(30),
                &[],
                Some(""),
            )
            .map_err(fail_line)?;
        let pin = as_struct(pin.as_ref().ok_or_else(|| line("PIN response was empty"))?)?;
        let code = text_field(pin, 1);
        emit(json!({"type": "pin", "pin": code}));
        emit(json!({"type": "status", "message": "Confirm this PIN in the LINE app on your phone."}));
        Ok(code)
    }

    fn poll_once(&mut self, method: &str, session_id: &str, interval: i64) -> Result<(), CallFail> {
        let timeout = Duration::from_secs(interval.max(1) as u64 + 5);
        let wait = format!("{}", interval.max(1) * 1000);
        self.call(
            method,
            &[(1, Field::Struct(vec![(1, bin(session_id))]))],
            LOGIN_POLL_PATH,
            timeout,
            &[("x-lst", wait), ("x-line-access", session_id.to_string())],
            Some(""),
        )?;
        Ok(())
    }

    fn complete_login<F: FnMut(Json)>(&mut self, session_id: &str, nonce: &[u8], private: &[u8], emit: &mut F) -> Result<(), LineError> {
        let host = hostname();
        let logged = self
            .call(
                "qrCodeLoginV2ForSecure",
                &[(
                    1,
                    Field::Struct(vec![
                        (1, bin(session_id)),
                        (2, bin(host)),
                        (3, bin("lineapp")),
                        (4, Field::Bool(true)),
                        (5, bin(nonce)),
                    ]),
                )],
                LOGIN_PATH,
                Duration::from_secs(30),
                &[],
                Some(""),
            )
            .map_err(fail_line)?;
        let logged = as_struct(logged.as_ref().ok_or_else(|| line("login response was empty"))?)?;
        let certificate = text_field(logged, 1);
        let mut access = String::new();
        let mut refresh = String::new();
        let mut expire = None;
        if let Some(Value::Struct(token_info)) = logged.get(&3) {
            access = text_field(token_info, 1);
            refresh = text_field(token_info, 2);
            if let (Some(left), Some(right)) = (int_field(token_info, 3), int_field(token_info, 6)) {
                expire = Some(left + right);
            }
        }
        if access.is_empty() {
            access = text_field(logged, 2);
        }
        if access.is_empty() {
            return Err(line("login response had no access token"));
        }
        self.session.access_token = access;
        self.session.refresh_token = refresh;
        if !certificate.is_empty() {
            self.session.certificate = certificate;
        }
        self.session.mid = text_field(logged, 4);
        self.session.expire = expire;
        self.session.sqr_private_key = STANDARD.encode(private);
        let saved = if let Some(Value::Map(pairs)) = logged.get(&10) {
            self.keep_login_key(pairs, private)
        } else if let Some(meta) = logged.get(&10) {
            let saved = self.keep_chain_value(meta, private);
            if saved > 0 {
                self.keys = None;
                self.listed = None;
            }
            saved
        } else {
            0
        };
        if saved > 0 {
            emit(json!({"type": "status", "message": "Letter-sealing keys from the phone are saved."}));
        } else {
            emit(json!({"type": "status", "message": "This login did not include the phone's letter-sealing key."}));
        }
        let path = save_session(&self.session)?;
        emit(json!({"type": "saved", "path": path.display().to_string()}));
        Ok(())
    }

    fn keep_login_key(&mut self, pairs: &[(Value, Value)], sqr_private: &[u8]) -> usize {
        let mut saved = 0;
        if let (Some(chain), Some(public)) = (meta_bytes(pairs, "encryptedKeyChain"), meta_bytes(pairs, "publicKey")) {
            saved += self.keep_chain(&public, sqr_private, &chain, meta_int(pairs, "keyId"));
        }
        if saved == 0 {
            let value = Value::Map(pairs.to_vec());
            saved += self.keep_chain_value(&value, sqr_private);
        }
        if saved > 0 {
            self.keys = None;
            self.listed = None;
        }
        saved
    }

    fn keep_chain_value(&mut self, value: &Value, sqr_private: &[u8]) -> usize {
        let mut blobs = Vec::new();
        let mut hints = Vec::new();
        collect_material(value, &mut blobs, &mut hints);
        let publics: Vec<Vec<u8>> = blobs.iter().filter(|blob| blob.len() == 32).cloned().collect();
        let chains: Vec<Vec<u8>> = blobs
            .iter()
            .filter(|blob| blob.len() > 32 && blob.len() % 16 == 0)
            .cloned()
            .collect();
        let mut saved = 0;
        for public in &publics {
            for chain in &chains {
                saved += self.keep_chain(public, sqr_private, chain, hints.first().copied());
                if saved > 0 {
                    return saved;
                }
            }
        }
        saved
    }

    fn keep_chain(&mut self, public: &[u8], sqr_private: &[u8], chain: &[u8], hint: Option<i64>) -> usize {
        let Some(keys) = e2ee::decrypt_key_chain(public, sqr_private, chain) else { return 0 };
        let mut saved = 0;
        for key in keys {
            let hinted = key.key_id.or(hint).filter(|id| *id != 0);
            if self.store_secret(&key.private_key, &key.public_key, hinted) {
                saved += 1;
            }
        }
        saved
    }

    fn store_secret(&mut self, private_key: &[u8], public_key: &[u8], hint: Option<i64>) -> bool {
        if private_key.len() != 32 || public_key.len() != 32 {
            return false;
        }
        let secret = x25519_dalek::StaticSecret::from(<[u8; 32]>::try_from(private_key).unwrap_or([0; 32]));
        if x25519_dalek::PublicKey::from(&secret).as_bytes().as_slice() != public_key {
            return false;
        }
        let listed = self.listed_keys();
        let key_id = listed
            .iter()
            .find(|(_, data)| data == public_key)
            .map(|(id, _)| *id)
            .or(hint)
            .unwrap_or(0);
        let private_b64 = STANDARD.encode(private_key);
        let public_b64 = STANDARD.encode(public_key);
        if let Some(existing) = self.session.e2ee_keys.iter_mut().find(|key| key.private_key == private_b64) {
            existing.public_key = public_b64.clone();
            if key_id != 0 {
                existing.key_id = key_id;
            }
        } else {
            self.session.e2ee_keys.push(StoredKey {
                key_id,
                private_key: private_b64.clone(),
                public_key: public_b64.clone(),
            });
        }
        let primary_matches = listed.iter().any(|(_, data)| {
            STANDARD.decode(&self.session.e2ee_public_key).ok().as_ref() == Some(data)
        });
        if !primary_matches {
            self.session.e2ee_private_key = private_b64;
            self.session.e2ee_public_key = public_b64;
            if key_id != 0 {
                self.session.e2ee_key_id = Some(key_id);
            }
        }
        self.keys = None;
        true
    }

    fn talk(&mut self, method: &str, fields: &[(i16, Field)]) -> Result<Option<Value>, LineError> {
        self.call(method, fields, TALK_PATH, Duration::from_secs(30), &[], None)
            .map_err(fail_line)
    }

    fn send_text(&mut self, to: &str, message: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let seq = self.next_seq();
        let success = self.talk("sendMessage", &send_fields(to, message, seq))?.ok_or_else(|| line("empty send response"))?;
        let fields = as_struct(&success)?;
        let when = message_time(fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let body = text_field(fields, 10);
        let text = if body.is_empty() { message.to_string() } else { body };
        Ok(json!({
            "type": "sent",
            "ok": true,
            "id": to,
            "messageId": text_field(fields, 4),
            "text": text,
            "time": if when == 0 { now } else { when },
        }))
    }

    fn send_sealed_text(&mut self, to: &str, message: &str) -> Result<Json, LineError> {
        self.require_token()?;
        if !to.starts_with('u') || self.session.mid.is_empty() {
            return Err(line_code("letter sealing is not available for this chat", Some(82), "E2EE_RETRY_ENCRYPT"));
        }
        let (private, sender_key_id) = self.send_key()?;
        let negotiated = self
            .talk("negotiateE2EEPublicKey", &[(2, bin(to))])?
            .ok_or_else(|| line_code("letter sealing key is missing", Some(82), "E2EE_RETRY_ENCRYPT"))?;
        let negotiated = as_struct(&negotiated)?;
        let spec = int_field(negotiated, 3).unwrap_or(-1);
        if spec == -1 {
            return Err(line_code("letter sealing is not available for this chat", Some(82), "E2EE_RETRY_ENCRYPT"));
        }
        let spec_version = if spec > 0 { spec } else { 2 };
        let public = match negotiated.get(&2) {
            Some(Value::Struct(fields)) => fields,
            _ => return Err(line_code("letter sealing key is missing", Some(82), "E2EE_RETRY_ENCRYPT")),
        };
        let receiver_key_id = int_field(public, 2).ok_or_else(|| line_code("letter sealing key is missing", Some(82), "E2EE_RETRY_ENCRYPT"))?;
        let receiver_public = bytes_field(public, 4);
        if receiver_public.len() != 32 {
            return Err(line_code("letter sealing key is missing", Some(82), "E2EE_RETRY_ENCRYPT"));
        }
        let chunks = seal_text(
            &private,
            sender_key_id,
            &receiver_public,
            receiver_key_id,
            &self.session.mid.clone(),
            to,
            message,
            spec_version,
        )
        .map_err(|err| line_code(err, Some(82), "E2EE_RETRY_ENCRYPT"))?;
        let seq = self.next_seq();
        let success = self
            .talk("sendMessage", &sealed_fields(to, &chunks, spec_version, seq))?
            .ok_or_else(|| line("empty send response"))?;
        let fields = as_struct(&success)?;
        let when = message_time(fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        Ok(json!({
            "type": "sent",
            "ok": true,
            "id": to,
            "messageId": text_field(fields, 4),
            "text": message,
            "time": if when == 0 { now } else { when },
        }))
    }

    fn send_key(&mut self) -> Result<(Vec<u8>, i64), LineError> {
        if let Some(key) = self.secret_keys().into_iter().next() {
            return Ok(key);
        }
        let secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        let public = x25519_dalek::PublicKey::from(&secret);
        let registered = self
            .talk(
                "registerE2EEPublicKey",
                &[(
                    2,
                    Field::Struct(vec![
                        (1, Field::I32(1)),
                        (2, Field::I32(0)),
                        (4, bin(public.as_bytes().as_slice())),
                        (5, Field::I64(0)),
                    ]),
                )],
            )?
            .unwrap_or(Value::Struct(BTreeMap::new()));
        self.listed = None;
        let mut key_id = as_struct(&registered).ok().and_then(|fields| int_field(fields, 2)).filter(|id| *id != 0);
        if key_id.is_none() {
            key_id = self.listed_keys().iter().find(|(_, data)| data == public.as_bytes()).map(|(id, _)| *id);
        }
        let Some(key_id) = key_id else {
            return Err(line_code("letter sealing key was not registered", Some(82), "invalid sender key"));
        };
        self.store_secret(secret.as_bytes(), public.as_bytes(), Some(key_id));
        save_session(&self.session)?;
        Ok((secret.as_bytes().to_vec(), key_id))
    }

    fn secret_keys(&mut self) -> Vec<(Vec<u8>, i64)> {
        if let Some(keys) = &self.keys {
            return keys.clone();
        }
        let mut raw = Vec::new();
        for key in &self.session.e2ee_keys {
            if let Ok(private) = STANDARD.decode(&key.private_key) {
                if private.len() == 32 && !raw.iter().any(|have: &Vec<u8>| have == &private) {
                    raw.push(private);
                }
            }
        }
        if let Ok(private) = STANDARD.decode(&self.session.e2ee_private_key) {
            if private.len() == 32 && !raw.iter().any(|have| have == &private) {
                raw.push(private);
            }
        }
        let listed = self.listed_keys();
        let mut resolved = Vec::new();
        for private in raw {
            let secret = x25519_dalek::StaticSecret::from(<[u8; 32]>::try_from(private.as_slice()).unwrap_or([0; 32]));
            let public = x25519_dalek::PublicKey::from(&secret).as_bytes().to_vec();
            if let Some((id, _)) = listed.iter().find(|(_, data)| data == &public) {
                resolved.push((private, *id));
            }
        }
        self.keys = Some(resolved.clone());
        resolved
    }

    fn listed_keys(&mut self) -> Vec<(i64, Vec<u8>)> {
        if let Some(listed) = &self.listed {
            return listed.clone();
        }
        let mut out = Vec::new();
        if let Ok(Some(Value::List(keys))) = self.talk("getE2EEPublicKeys", &[]) {
            for key in keys {
                let Value::Struct(fields) = key else { continue };
                let Some(id) = int_field(&fields, 2) else { continue };
                let data = bytes_field(&fields, 4);
                if data.len() == 32 {
                    out.push((id, data));
                }
            }
        }
        self.listed = Some(out.clone());
        out
    }

    fn visible_text(&mut self, message: &BTreeMap<i16, Value>, cards: &HashMap<String, (String, String)>) -> (String, bool) {
        let me = self.session.mid.clone();
        if let Some(opened) = self.open_sealed_text(message) {
            return (opened, false);
        }
        if let Some(name) = self.sealed_file_name(message) {
            return (name, false);
        }
        if let Some(line) = event_sentence(message, cards, &me) {
            return (line, false);
        }
        message_preview(message)
    }

    fn visible_preview(&mut self, message: &BTreeMap<i16, Value>, sender_name: &str) -> (String, bool) {
        if is_chat_update(message) {
            return (String::new(), false);
        }
        if let Some(opened) = self.open_sealed_text(message) {
            return (opened, false);
        }
        chat_preview(message, sender_name)
    }

    fn open_sealed_text(&mut self, message: &BTreeMap<i16, Value>) -> Option<String> {
        if int_field(message, 15).unwrap_or(0) != 0 {
            return None;
        }
        let chunks = bytes_list(message.get(&20))?;
        if chunks.len() < 5 || chunks.iter().take(5).any(|part| part.is_empty()) {
            return None;
        }
        let sender = text_field(message, 1);
        let recipient = text_field(message, 2);
        if !sender.starts_with('u') {
            return None;
        }
        let sender_key = u32::from_be_bytes(chunks[3].as_slice().try_into().ok()?) as i64;
        let receiver_key = u32::from_be_bytes(chunks[4].as_slice().try_into().ok()?) as i64;
        let spec = e2ee_spec(message);
        let me = self.session.mid.clone();
        if recipient.starts_with('c') || recipient.starts_with('r') {
            let group_priv = self.group_private(&recipient, receiver_key)?;
            let peer_pub = if sender == me {
                self.public_for_key(sender_key)?
            } else {
                self.peer_public(&sender, sender_key)?
            };
            return open_text(&chunks, &group_priv, &peer_pub, &sender, &recipient, spec).ok();
        }
        if !recipient.starts_with('u') {
            return None;
        }
        for (private, own_id) in self.owned_keys() {
            let (peer, peer_key) = if sender == me && sender_key == own_id {
                (recipient.clone(), receiver_key)
            } else if receiver_key == own_id {
                (sender.clone(), sender_key)
            } else {
                continue;
            };
            let Some(public) = self.peer_public(&peer, peer_key) else { continue };
            if let Ok(text) = open_text(&chunks, &private, &public, &sender, &recipient, spec) {
                return Some(text);
            }
        }
        None
    }

    fn sealed_file_name(&mut self, message: &BTreeMap<i16, Value>) -> Option<String> {
        if int_field(message, 15) != Some(14) {
            return None;
        }
        let name = self.sealed_payload(message)?.get("fileName")?.as_str()?.trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }

    fn peer_public(&mut self, mid: &str, key_id: i64) -> Option<Vec<u8>> {
        let key = (mid.to_string(), key_id);
        if let Some(cached) = self.pubs.get(&key) {
            return cached.clone();
        }
        let found = self
            .talk("getE2EEPublicKey", &[(2, bin(mid)), (3, Field::I32(1)), (4, Field::I32(key_id as i32))])
            .ok()
            .flatten()
            .as_ref()
            .and_then(|value| as_struct(value).ok())
            .map(|fields| bytes_field(fields, 4))
            .filter(|data| data.len() == 32);
        self.pubs.insert(key, found.clone());
        found
    }

    fn latest_message(
        &mut self,
        mid: &str,
        name: &str,
        failures: &mut Vec<Json>,
    ) -> (Option<BTreeMap<i16, Value>>, i64) {
        if mid.is_empty() {
            return (None, 0);
        }
        let recent = match self.talk("getRecentMessagesV2", &[(2, bin(mid)), (3, Field::I32(50))]) {
            Ok(value) => value,
            Err(err) => {
                failures.push(json!({"name": name, "error": err.message}));
                return (None, 0);
            }
        };
        let Some(Value::List(items)) = recent else { return (None, 0) };
        let messages: Vec<BTreeMap<i16, Value>> = items.into_iter().filter_map(|item| match item {
            Value::Struct(fields) => Some(fields),
            _ => None,
        }).collect();
        preview_choice(&messages)
    }

    fn message_page(&mut self, method: &str, fields: &[(i16, Field)]) -> Result<Vec<BTreeMap<i16, Value>>, LineError> {
        let result = self.talk(method, fields)?;
        let Some(Value::List(items)) = result else { return Ok(Vec::new()) };
        Ok(items.into_iter().filter_map(|item| match item { Value::Struct(fields) => Some(fields), _ => None }).collect())
    }

    fn message_box_rows(&mut self) -> Result<Vec<(String, Option<i64>, i64)>, LineError> {
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor = String::new();
        for _ in 0..20 {
            let mut request = vec![(4, Field::I32(100))];
            if !cursor.is_empty() {
                request.push((1, bin(&cursor)));
            }
            let response = self
                .talk("getMessageBoxes", &[(2, Field::Struct(request))])?
                .ok_or_else(|| line("message boxes were empty"))?;
            let response = as_struct(&response)?;
            let Some(Value::List(boxes)) = response.get(&1) else { break };
            if boxes.is_empty() {
                break;
            }
            let mut last = String::new();
            for entry in boxes {
                let Value::Struct(entry) = entry else { continue };
                let mid = text_field(entry, 1);
                if mid.is_empty() || !seen.insert(mid.clone()) {
                    continue;
                }
                let when = match entry.get(&4) {
                    Some(Value::Struct(delivered)) => int_field(delivered, 1).unwrap_or(0),
                    _ => 0,
                };
                let unread = int_field(entry, 6);
                found.push((mid.clone(), unread, when));
                last = mid;
            }
            if response.get(&2) != Some(&Value::Bool(true)) || last.is_empty() || last == cursor {
                break;
            }
            cursor = last;
        }
        Ok(found)
    }

    fn list_profile(&mut self) -> Json {
        let Ok(Some(value)) = self.talk("getProfile", &[]) else {
            return json!({"name": "", "picture": ""});
        };
        let Ok(fields) = as_struct(&value) else {
            return json!({"name": "", "picture": ""});
        };
        json!({
            "name": text_field(fields, 20),
            "picture": picture_url(&text_field(fields, 22)),
        })
    }

    fn contact_cards(&mut self, mids: &[String]) -> HashMap<String, (String, String)> {
        let mut unique = Vec::new();
        for mid in mids {
            if !mid.is_empty() && !unique.contains(mid) {
                unique.push(mid.clone());
            }
        }
        let mut cards = HashMap::new();
        for batch in unique.chunks(50) {
            let items: Vec<Field> = batch.iter().cloned().map(bin).collect();
            let Ok(Some(Value::List(people))) = self.talk("getContacts", &[(2, Field::List(thrift::BINARY, items))]) else {
                continue;
            };
            for person in people {
                let Value::Struct(person) = person else { continue };
                let mid = text_field(&person, 1);
                if mid.is_empty() {
                    continue;
                }
                cards.insert(mid, (text_field(&person, 22).trim().to_string(), picture_url(&text_field(&person, 24))));
            }
        }
        cards
    }
}

fn sort_chat_rows(rows: &mut [Json]) {
    rows.sort_by(|left, right| {
        right.get("time").and_then(|value| value.as_i64()).unwrap_or(0)
            .cmp(&left.get("time").and_then(|value| value.as_i64()).unwrap_or(0))
    });
}

fn patch_time(rows: &mut [Json], id: &str, when: i64) {
    let Some(row) = rows.iter_mut().find(|row| row.get("id").and_then(|value| value.as_str()) == Some(id)) else { return };
    let current = row.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
    if when > current {
        row["time"] = json!(when);
    }
}

fn patch_preview(rows: &mut [Json], id: &str, preview: &str, sealed: bool, when: i64, sticker: &str, fallback: &str) {
    let Some(row) = rows.iter_mut().find(|row| row.get("id").and_then(|value| value.as_str()) == Some(id)) else { return };
    row["lastMessage"] = json!(preview);
    row["sealed"] = json!(sealed);
    row["sticker"] = json!(sticker);
    row["stickerFallback"] = json!(fallback);
    let current = row.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
    if when > current {
        row["time"] = json!(when);
    }
}

fn unread_from_rows(rows: &[(String, Option<i64>, i64)]) -> (HashMap<String, i64>, Json, Vec<String>) {
    let mut counts = HashMap::new();
    let mut ids = Vec::new();
    let mut total = 0i64;
    let mut any = false;
    for (mid, unread, _) in rows {
        if !ids.contains(mid) {
            ids.push(mid.clone());
        }
        if let Some(count) = unread {
            any = true;
            counts.insert(mid.clone(), *count);
            if *count > 0 {
                total += count;
            }
        }
    }
    (counts, if any { json!(total) } else { Json::Null }, ids)
}

fn send_failure(to: &str, err: &LineError) -> Json {
    json!({
        "type": "sent",
        "ok": false,
        "id": to,
        "code": err.code,
        "error": if err.reason.is_empty() { err.message.clone() } else { err.reason.clone() },
    })
}

fn send_fields(to: &str, text_body: &str, seq: i32) -> Vec<(i16, Field)> {
    let mut message = vec![
        (2, bin(to)),
        (5, Field::I64(0)),
        (6, Field::I64(0)),
        (10, bin(text_body)),
        (14, Field::Bool(false)),
        (15, Field::I32(0)),
        (18, Field::MapBin(vec![])),
        (19, Field::Byte(0)),
    ];
    if let Some(kind) = mid_type(to) {
        message.push((3, Field::I32(kind)));
    }
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

fn sealed_fields(to: &str, chunks: &[Vec<u8>], spec_version: i64, seq: i32) -> Vec<(i16, Field)> {
    let meta = vec![
        (b"e2eeVersion".to_vec(), spec_version.to_string().into_bytes()),
        (b"contentType".to_vec(), b"0".to_vec()),
        (b"e2eeMark".to_vec(), b"2".to_vec()),
    ];
    let message = vec![
        (2, bin(to)),
        (3, Field::I32(0)),
        (5, Field::I64(0)),
        (6, Field::I64(0)),
        (14, Field::Bool(false)),
        (15, Field::I32(0)),
        (18, Field::MapBin(meta)),
        (19, Field::Byte(0)),
        (20, Field::List(thrift::BINARY, chunks.iter().cloned().map(Field::Bin).collect())),
    ];
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

fn mid_type(mid: &str) -> Option<i32> {
    match mid.chars().next() {
        Some('u') => Some(0),
        Some('r') => Some(1),
        Some('c') => Some(2),
        _ => None,
    }
}

fn fail_line(err: CallFail) -> LineError {
    match err {
        CallFail::Line(err) => err,
        CallFail::TimedOut(message) => line(message),
    }
}

fn session_dead(err: &LineError) -> bool {
    if err.code == Some(8) || err.code == Some(119) {
        return true;
    }
    let blob = format!("{} {}", err.reason, err.message).to_uppercase();
    blob.contains("LOGGED OUT") || blob.contains("LOGGED_OUT") || blob.contains("V3_TOKEN") || blob.contains("AUTHENTICATION") || blob.contains("ACCESS TOKEN") || blob.contains("INVALID TOKEN")
}

fn sync_fields(cursor: &OpCursor, count: i32) -> Vec<(i16, Field)> {
    vec![(
        1,
        Field::Struct(vec![
            (1, Field::I64(cursor.local)),
            (2, Field::I32(count)),
            (3, Field::I64(cursor.global)),
            (4, Field::I64(cursor.individual)),
        ]),
    )]
}

fn fetch_ops_fields(cursor: &OpCursor, count: i32) -> Vec<(i16, Field)> {
    vec![
        (2, Field::I64(cursor.local)),
        (3, Field::I32(count)),
        (4, Field::I64(cursor.global)),
        (5, Field::I64(cursor.individual)),
    ]
}

fn poll_headers() -> Vec<(&'static str, String)> {
    vec![("x-las", "F".into()), ("x-lam", "w".into())]
}

fn struct_items(items: &[Value]) -> Vec<BTreeMap<i16, Value>> {
    items.iter().filter_map(|item| match item { Value::Struct(fields) => Some(fields.clone()), _ => None }).collect()
}

fn operation_list(value: &Value) -> Vec<BTreeMap<i16, Value>> {
    match value {
        Value::List(items) => struct_items(items),
        Value::Struct(fields) => {
            if let Some(Value::Struct(inner)) = fields.get(&1) {
                if let Some(Value::List(items)) = inner.get(&1) {
                    return struct_items(items);
                }
            }
            if let Some(Value::List(items)) = fields.get(&1) {
                return struct_items(items);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn needs_full_sync(value: &Value) -> bool {
    let Value::Struct(fields) = value else { return false };
    let Some(Value::Struct(full)) = fields.get(&2) else { return false };
    match full.get(&1) {
        Some(Value::List(items)) => !items.is_empty(),
        Some(Value::Map(pairs)) => !pairs.is_empty(),
        _ => false,
    }
}

fn sync_has_more(value: &Value) -> bool {
    let Value::Struct(fields) = value else { return false };
    if let Some(Value::Struct(inner)) = fields.get(&1) {
        if bool_field(inner, 2) {
            return true;
        }
    }
    bool_field(fields, 2)
}

fn absorb_op(cursor: &mut OpCursor, op: &BTreeMap<i16, Value>) {
    let kind = int_field(op, 3).unwrap_or(-1);
    if let Some(rev) = int_field(op, 1) {
        if kind != -1 && rev > cursor.local {
            cursor.local = rev;
        }
    }
    if kind == 0 {
        if let Some(value) = rev_prefix(&text_field(op, 10)) {
            cursor.individual = value;
        }
        if let Some(value) = rev_prefix(&text_field(op, 11)) {
            cursor.global = value;
        }
    }
}

fn rev_prefix(text: &str) -> Option<i64> {
    let (head, _) = text.split_once('\u{1e}')?;
    head.parse().ok()
}

fn chat_target(message: &BTreeMap<i16, Value>, me: &str) -> String {
    let to = text_field(message, 2);
    let from = text_field(message, 1);
    if to.starts_with('c') || to.starts_with('r') || to.starts_with('s') {
        return to;
    }
    if !me.is_empty() && to == me {
        return from;
    }
    if !to.is_empty() { to } else { from }
}

fn looks_like_mid(id: &str) -> bool {
    (10..=64).contains(&id.len()) && id.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn note_quiet(previous: u32, elapsed: Duration, empty: bool) -> u32 {
    if empty && elapsed < Duration::from_secs(1) { previous.saturating_add(1) } else { 0 }
}

fn next_wait(elapsed: Duration, quiet_rounds: u32) -> Duration {
    let pace = if quiet_rounds >= 3 { Duration::from_secs(5) } else { Duration::from_secs(2) };
    pace.saturating_sub(elapsed)
}

fn same_session_retry(err: &LineError) -> bool {
    if err.code == Some(100) {
        return true;
    }
    let blob = format!("{} {}", err.reason, err.message).to_lowercase();
    blob.contains("not confirmed") || blob.contains("timed out")
}

fn sqr_secret() -> (Vec<u8>, String) {
    let secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
    let public = x25519_dalek::PublicKey::from(&secret);
    let encoded = STANDARD.encode(public.as_bytes());
    let quoted = percent_encoding::utf8_percent_encode(&encoded, percent_encoding::NON_ALPHANUMERIC);
    (secret.as_bytes().to_vec(), format!("?secret={quoted}&e2eeVersion=1"))
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "lineapp".into())
}

fn https(
    host: &str,
    path: &str,
    body: &[u8],
    headers: &[(&str, String)],
    timeout: Duration,
) -> Result<(u16, Vec<u8>, HashMap<String, String>), CallFail> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let url = format!("https://{host}{path}");
    let mut request = agent.post(&url);
    for (key, value) in headers {
        request = request.set(key, value);
    }
    let response = match request.send_bytes(body) {
        Ok(response) => response,
        Err(ureq::Error::Status(_code, response)) => response,
        Err(err) => {
            let message = err.to_string();
            if message.to_lowercase().contains("timed") || message.to_lowercase().contains("timeout") {
                return Err(CallFail::TimedOut(format!("timeout talking to {host}{path}")));
            }
            return Err(CallFail::Line(line(format!("{host}{path}: {message}"))));
        }
    };
    let status = response.status();
    let mut header_map = HashMap::new();
    for name in response.headers_names() {
        if let Some(value) = response.header(&name) {
            header_map.insert(name.to_ascii_lowercase(), value.to_string());
        }
    }
    let mut payload = Vec::new();
    let mut reader = response.into_reader();
    reader.read_to_end(&mut payload).map_err(|err| CallFail::Line(line(err.to_string())))?;
    let encoding = header_map.get("content-encoding").cloned().unwrap_or_default();
    if encoding.to_lowercase().contains("gzip") || payload.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(payload.as_slice());
        let mut decoded = Vec::new();
        if decoder.read_to_end(&mut decoded).is_ok() {
            payload = decoded;
        }
    }
    Ok((status, payload, header_map))
}

fn talk_meta(message_id: &str) -> String {
    let id = message_id.as_bytes();
    let mut inner = Vec::new();
    inner.push(11);
    inner.extend_from_slice(&4i16.to_be_bytes());
    inner.extend_from_slice(&(id.len() as i32).to_be_bytes());
    inner.extend_from_slice(id);
    inner.push(15);
    inner.extend_from_slice(&27i16.to_be_bytes());
    inner.push(12);
    inner.extend_from_slice(&0i32.to_be_bytes());
    inner.push(0);
    let message = STANDARD.encode(&inner);
    STANDARD.encode(format!(r#"{{"message": "{message}"}}"#).as_bytes())
}

fn path_segment(value: &str) -> String {
    const SAFE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'/')
        .add(b'+')
        .add(b'=')
        .add(b'?');
    percent_encoding::utf8_percent_encode(value, SAFE).to_string()
}

fn media_file(id: &str, preview: bool, animated: bool) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let suffix = if animated { "anim" } else if preview { "preview" } else { "full" };
    base.join("lineapp").join("media").join(format!("{id}.{suffix}"))
}

fn read_media_cache(id: &str, preview: bool, animated: bool) -> Option<(String, Vec<u8>)> {
    let path = media_file(id, preview, animated);
    let bytes = std::fs::read(&path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let mime = std::fs::read_to_string(mime_file(&path)).ok()?;
    if mime.is_empty() { None } else { Some((mime, bytes)) }
}

fn write_media_cache(id: &str, preview: bool, animated: bool, mime: &str, bytes: &[u8]) {
    let path = media_file(id, preview, animated);
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() || std::fs::write(&path, bytes).is_err() {
        return;
    }
    let _ = std::fs::write(mime_file(&path), mime);
}

fn mime_file(path: &std::path::Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".mime");
    path.with_file_name(name)
}

fn prefer_mime(header: &str, bytes: &[u8]) -> String {
    let sniffed = sniff_media(bytes);
    if sniffed != "application/octet-stream" {
        return sniffed;
    }
    let mime = header.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if mime.starts_with("image/") || mime.starts_with("video/") { mime } else { sniffed }
}

fn sniff_media(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return "image/jpeg".into();
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return "image/png".into();
    }
    if bytes.starts_with(b"GIF8") {
        return "image/gif".into();
    }
    if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        return "image/webp".into();
    }
    if bytes.len() > 12 && &bytes[4..8] == b"ftyp" {
        return "video/mp4".into();
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return "video/webm".into();
    }
    "application/octet-stream".into()
}

fn as_struct(value: &Value) -> Result<&BTreeMap<i16, Value>, LineError> {
    match value {
        Value::Struct(fields) => Ok(fields),
        _ => Err(line("expected a thrift struct")),
    }
}

fn text_field(fields: &BTreeMap<i16, Value>, id: i16) -> String {
    match fields.get(&id) {
        Some(Value::Bytes(raw)) => String::from_utf8_lossy(raw).into_owned(),
        Some(Value::Int(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn int_field(fields: &BTreeMap<i16, Value>, id: i16) -> Option<i64> {
    match fields.get(&id) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

fn bool_field(fields: &BTreeMap<i16, Value>, id: i16) -> bool {
    matches!(fields.get(&id), Some(Value::Bool(true)))
}

fn bytes_field(fields: &BTreeMap<i16, Value>, id: i16) -> Vec<u8> {
    match fields.get(&id) {
        Some(Value::Bytes(raw)) => raw.clone(),
        _ => Vec::new(),
    }
}

fn bytes_list(value: Option<&Value>) -> Option<Vec<Vec<u8>>> {
    let Value::List(items) = value? else { return None };
    Some(items.iter().filter_map(|item| match item { Value::Bytes(raw) => Some(raw.clone()), _ => None }).collect())
}

fn id_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::List(items)) = value else { return Vec::new() };
    items.iter().filter_map(|item| match item {
        Value::Bytes(raw) if !raw.is_empty() => Some(String::from_utf8_lossy(raw).into_owned()),
        _ => None,
    }).collect()
}

fn picture_url(path: &str) -> String {
    let raw = path.trim();
    if raw.is_empty() {
        return String::new();
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return raw.to_string();
    }
    if raw.starts_with('/') {
        format!("https://profile.line-scdn.net{raw}")
    } else {
        format!("https://profile.line-scdn.net/{raw}")
    }
}

fn member_count(chat: &BTreeMap<i16, Value>) -> Option<i64> {
    let Value::Struct(extra) = chat.get(&8)? else { return None };
    let Value::Struct(group) = extra.get(&1)? else { return None };
    let Value::Map(members) = group.get(&4)? else { return None };
    if members.is_empty() { None } else { Some(members.len() as i64) }
}

fn chat_kind(mid: &str) -> &'static str {
    match mid.chars().next() {
        Some('c') => "group",
        Some('u') => "friend",
        Some('r') => "room",
        _ => "other",
    }
}

fn meta_value(message: &BTreeMap<i16, Value>, name: &str) -> String {
    let Some(Value::Map(pairs)) = message.get(&18) else { return String::new() };
    for (key, value) in pairs {
        let label = match key {
            Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
            _ => continue,
        };
        if label != name {
            continue;
        }
        return match value {
            Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
            Value::Int(number) => number.to_string(),
            _ => String::new(),
        };
    }
    String::new()
}

fn message_animated(message: &BTreeMap<i16, Value>) -> bool {
    let info = meta_value(message, "MEDIA_CONTENT_INFO");
    if let Ok(parsed) = serde_json::from_str::<Json>(&info) {
        if parsed.get("animated").and_then(|value| value.as_bool()) == Some(true) {
            return true;
        }
        if parsed.get("extension").and_then(|value| value.as_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("gif")) {
            return true;
        }
    }
    for key in ["FILE_NAME", "FILE_EXT", "EXTENSION"] {
        let value = meta_value(message, key);
        if value.to_ascii_lowercase().ends_with(".gif") || value.eq_ignore_ascii_case("gif") {
            return true;
        }
    }
    false
}

fn sticker_url(message: &BTreeMap<i16, Value>) -> String {
    let stk = meta_value(message, "STKID").trim().to_string();
    if stk.is_empty() || !stk.chars().all(|ch| ch.is_ascii_digit()) {
        return String::new();
    }
    let animated = meta_value(message, "STKOPT").contains('A');
    let asset = if animated { "sticker_animation.png" } else { "sticker.png" };
    format!("https://stickershop.line-scdn.net/stickershop/v1/sticker/{stk}/android/{asset}")
}

fn sticker_fallback(message: &BTreeMap<i16, Value>) -> String {
    let stk = meta_value(message, "STKID").trim().to_string();
    let pkg = meta_value(message, "STKPKGID").trim().to_string();
    if stk.is_empty() || !stk.chars().all(|ch| ch.is_ascii_digit()) {
        return String::new();
    }
    if !pkg.is_empty() && pkg.chars().all(|ch| ch.is_ascii_digit()) && pkg != "0" {
        return format!("https://stickershop.line-scdn.net/stickershop/v1/product/{pkg}/android/stickers/{stk}.png");
    }
    format!("https://stickershop.line-scdn.net/stickershop/v1/sticker/{stk}/android/sticker.png")
}

fn sticker_pair(message: Option<&BTreeMap<i16, Value>>) -> (String, String) {
    let Some(message) = message else { return (String::new(), String::new()) };
    let image = sticker_url(message);
    if image.is_empty() {
        (String::new(), String::new())
    } else {
        (image, sticker_fallback(message))
    }
}

fn needs_fresh_preview(preview: &str, activity: i64, prior_time: i64) -> bool {
    preview.is_empty() || activity > prior_time || preview.eq_ignore_ascii_case("letter-sealed message")
}

fn saved_preview(prior: Option<&crate::store::StoredChat>) -> (String, bool, String, String, i64) {
    let Some(prior) = prior else {
        return (String::new(), false, String::new(), String::new(), 0);
    };
    let preview = if placeholder_preview(&prior.last_message) { String::new() } else { prior.last_message.clone() };
    (preview, prior.sealed, prior.sticker.clone(), prior.sticker_fallback.clone(), prior.time)
}

pub fn placeholder_preview(text: &str) -> bool {
    let text = text.trim();
    text.eq_ignore_ascii_case("chat update") || text.eq_ignore_ascii_case("letter-sealed chat update")
}

fn is_chat_update(message: &BTreeMap<i16, Value>) -> bool {
    int_field(message, 15) == Some(18)
}

fn preview_choice(messages: &[BTreeMap<i16, Value>]) -> (Option<BTreeMap<i16, Value>>, i64) {
    let when = messages.iter().map(message_time).max().unwrap_or(0);
    let preview = messages
        .iter()
        .filter(|message| !is_chat_update(message))
        .max_by_key(|message| message_time(message))
        .cloned();
    (preview, when)
}

fn content_kind(code: Option<i64>) -> &'static str {
    match code {
        Some(0) | None => "message",
        Some(1) => "image",
        Some(2) => "video",
        Some(3) => "audio",
        Some(7) => "sticker",
        Some(13) => "contact",
        Some(14) => "file",
        Some(15) => "location",
        Some(18) => "chat update",
        _ => "message",
    }
}

fn event_mids(message: &BTreeMap<i16, Value>) -> Vec<String> {
    if int_field(message, 15) != Some(18) || !text_field(message, 10).trim().is_empty() {
        return Vec::new();
    }
    if meta_value(message, "LOC_KEY") == "C_PN" {
        return Vec::new();
    }
    let mut mids = Vec::new();
    let sender = text_field(message, 1);
    if looks_like_mid(&sender) {
        mids.push(sender);
    }
    for arg in loc_args(&meta_value(message, "LOC_ARGS")) {
        if looks_like_mid(&arg) && !mids.iter().any(|have| have == &arg) {
            mids.push(arg);
        }
    }
    mids
}

fn loc_args(raw: &str) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    if raw.starts_with('[') && raw.ends_with(']') {
        if let Ok(Json::Array(items)) = serde_json::from_str::<Json>(raw) {
            return items
                .into_iter()
                .filter_map(|item| item.as_str().map(|part| part.trim().to_string()))
                .filter(|part| !part.is_empty())
                .collect();
        }
    }
    raw.split(['\u{1e}', '\u{1f}'])
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn display_name(mid: &str, cards: &HashMap<String, (String, String)>, me: &str) -> String {
    if mid.is_empty() {
        return String::new();
    }
    if !me.is_empty() && mid == me {
        return "You".into();
    }
    if let Some((name, _)) = cards.get(mid) {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    if looks_like_mid(mid) { "Someone".into() } else { mid.to_string() }
}

fn event_sentence(message: &BTreeMap<i16, Value>, cards: &HashMap<String, (String, String)>, me: &str) -> Option<String> {
    if int_field(message, 15) != Some(18) || !text_field(message, 10).trim().is_empty() {
        return None;
    }
    let key = meta_value(message, "LOC_KEY");
    if key.is_empty() {
        return None;
    }
    let raw = loc_args(&meta_value(message, "LOC_ARGS"));
    let sender = display_name(&text_field(message, 1), cards, me);
    let named: Vec<String> = if key == "C_PN" {
        raw
    } else {
        raw.iter().map(|arg| if looks_like_mid(arg) { display_name(arg, cards, me) } else { arg.clone() }).collect()
    };
    chat_event_sentence(&key, &named, &sender)
}

fn join_names(names: &[String]) -> String {
    match names {
        [] => "someone".into(),
        [only] => only.clone(),
        [left, right] => format!("{left} and {right}"),
        _ => {
            let (last, rest) = names.split_last().unwrap();
            format!("{}, and {last}", rest.join(", "))
        }
    }
}

fn chat_event_sentence(key: &str, args: &[String], sender: &str) -> Option<String> {
    let who = |index: usize| -> String {
        args.get(index).cloned().filter(|name| !name.is_empty()).unwrap_or_else(|| {
            if sender.is_empty() { "Someone".into() } else { sender.to_string() }
        })
    };
    let line = match key {
        "C_ML" | "A_ML" => format!("{} left the group.", who(0)),
        "C_MJ" | "A_MJ" => format!("{} joined the group.", who(0)),
        "C_MR" | "A_MR" => {
            if args.len() >= 2 {
                format!("{} removed {} from the group.", who(0), who(1))
            } else {
                format!("{} was removed from the group.", who(0))
            }
        }
        "C_MI" | "A_MI" | "C_GI" => invite_line(args, sender),
        "C_IC" | "A_IC" => {
            if args.len() >= 2 {
                format!("{} canceled the invitation for {}.", who(0), who(1))
            } else {
                format!("{} declined the invitation.", who(0))
            }
        }
        "C_PN" => {
            let name = args.first().map(|item| item.trim()).filter(|item| !item.is_empty());
            match (sender.is_empty(), name) {
                (false, Some(name)) => format!("{sender} changed the group name to {name}."),
                (true, Some(name)) => format!("The group name was changed to {name}."),
                (false, None) => format!("{sender} changed the group name."),
                (true, None) => "The group name was changed.".into(),
            }
        }
        "C_PI" => {
            if sender.is_empty() { "The group picture was changed.".into() } else { format!("{sender} changed the group picture.") }
        }
        "C_PL" => "Group limits were removed.".into(),
        "C_MA" => "A message was announced.".into(),
        "C_OL" => "The group is full, so the invitation did not go through.".into(),
        "C_BG" | "C_SN" => "Joining with an invite link is now allowed.".into(),
        "C_SP" => "Joining with an invite link is now off.".into(),
        _ => return None,
    };
    Some(line)
}

fn invite_line(args: &[String], sender: &str) -> String {
    let names: Vec<&str> = args.iter().map(String::as_str).filter(|name| !name.is_empty()).collect();
    match names.as_slice() {
        [] => {
            if sender.is_empty() { "Someone joined the group.".into() } else { format!("{sender} invited someone to the group.") }
        }
        [only] => {
            if sender.is_empty() || sender == *only {
                format!("{only} joined the group.")
            } else {
                format!("{sender} invited {only} to the group.")
            }
        }
        [first, rest @ ..] => {
            let rest: Vec<String> = rest.iter().map(|name| (*name).to_string()).collect();
            format!("{first} invited {} to the group.", join_names(&rest))
        }
    }
}

fn message_preview(message: &BTreeMap<i16, Value>) -> (String, bool) {
    let mut keys = Vec::new();
    if let Some(Value::Map(pairs)) = message.get(&18) {
        for (key, _) in pairs {
            if let Value::Bytes(raw) = key {
                keys.push(String::from_utf8_lossy(raw).into_owned());
            }
        }
    }
    let chunks = matches!(message.get(&20), Some(Value::List(items)) if !items.is_empty());
    let sealed = keys.iter().any(|key| key == "e2eeVersion") || chunks;
    let body = text_field(message, 10).trim().to_string();
    let kind = content_kind(int_field(message, 15));
    if keys.iter().any(|key| key == "UNSENT") && meta_value(message, "UNSENT").eq_ignore_ascii_case("true") {
        return ("Unsent message".into(), false);
    }
    if kind == "chat update" {
        return (body, false);
    }
    if sealed {
        if kind == "message" {
            return ("Letter-sealed message".into(), true);
        }
        return (format!("Letter-sealed {kind}"), true);
    }
    if !body.is_empty() {
        return (body, false);
    }
    if kind != "message" {
        let mut chars = kind.chars();
        let Some(first) = chars.next() else { return (String::new(), false) };
        return (format!("{}{}", first.to_uppercase(), chars.as_str()), false);
    }
    (String::new(), false)
}

fn chat_preview(message: &BTreeMap<i16, Value>, sender_name: &str) -> (String, bool) {
    let (text, sealed) = message_preview(message);
    let kind = content_kind(int_field(message, 15));
    let sent = match kind {
        "image" => Some("a photo"),
        "video" => Some("a video"),
        "audio" => Some("a voice message"),
        "sticker" => Some("a sticker"),
        "file" => Some("a file"),
        "location" => Some("a location"),
        "contact" => Some("a contact"),
        _ => None,
    };
    if let Some(sent) = sent {
        let who = if sender_name.is_empty() { "Someone" } else { sender_name };
        return (format!("{who} sent {sent}."), sealed);
    }
    (text, sealed)
}

fn e2ee_spec(message: &BTreeMap<i16, Value>) -> i64 {
    let raw = meta_value(message, "e2eeVersion");
    raw.parse::<i64>().ok().filter(|value| *value > 0).unwrap_or(2)
}

fn message_time(message: &BTreeMap<i16, Value>) -> i64 {
    int_field(message, 6).or_else(|| int_field(message, 5)).unwrap_or(0)
}

fn take_messages(collected: &[BTreeMap<i16, Value>], page: &[BTreeMap<i16, Value>]) -> Vec<BTreeMap<i16, Value>> {
    let mut seen: std::collections::HashSet<String> = collected.iter().map(|message| text_field(message, 4)).filter(|id| !id.is_empty()).collect();
    let mut merged = collected.to_vec();
    for message in page {
        let ident = text_field(message, 4);
        if !ident.is_empty() && !seen.insert(ident) {
            continue;
        }
        merged.push(message.clone());
    }
    merged
}

fn exception_parts(value: Option<&Value>) -> (Option<i64>, String) {
    let Some(Value::Struct(fields)) = value else { return (None, String::new()) };
    let reason = text_field(fields, 2);
    (int_field(fields, 1), reason)
}

fn collect_material(value: &Value, blobs: &mut Vec<Vec<u8>>, hints: &mut Vec<i64>) {
    match value {
        Value::Int(number) => {
            if *number > 0 && *number <= i64::from(i32::MAX) {
                hints.push(*number);
            }
        }
        Value::Bytes(raw) => {
            push_blob(blobs, raw);
            if let Ok(text) = std::str::from_utf8(raw) {
                if let Ok(number) = text.trim().parse::<i64>() {
                    if number > 0 && number <= i64::from(i32::MAX) {
                        hints.push(number);
                    }
                }
            }
        }
        Value::List(items) => {
            for item in items {
                collect_material(item, blobs, hints);
            }
        }
        Value::Map(pairs) => {
            for (key, item) in pairs {
                collect_material(key, blobs, hints);
                collect_material(item, blobs, hints);
            }
        }
        Value::Struct(fields) => {
            for item in fields.values() {
                collect_material(item, blobs, hints);
            }
        }
        _ => {}
    }
}

fn push_blob(blobs: &mut Vec<Vec<u8>>, raw: &[u8]) {
    if raw.len() == 32 || (raw.len() > 32 && raw.len() % 16 == 0) {
        blobs.push(raw.to_vec());
    }
    if let Ok(decoded) = STANDARD.decode(raw) {
        if decoded.as_slice() != raw && (decoded.len() == 32 || (decoded.len() > 32 && decoded.len() % 16 == 0)) {
            blobs.push(decoded);
        }
    }
}

fn meta_bytes(pairs: &[(Value, Value)], name: &str) -> Option<Vec<u8>> {
    let raw = pairs.iter().find_map(|(key, value)| {
        let label = match key {
            Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
            _ => return None,
        };
        if label == name { Some(value) } else { None }
    })?;
    match raw {
        Value::Bytes(raw) => {
            if let Ok(decoded) = STANDARD.decode(raw) {
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
            Some(raw.clone())
        }
        _ => None,
    }
}

fn meta_int(pairs: &[(Value, Value)], name: &str) -> Option<i64> {
    let raw = pairs.iter().find_map(|(key, value)| {
        let label = match key {
            Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
            _ => return None,
        };
        if label == name { Some(value) } else { None }
    })?;
    match raw {
        Value::Int(value) => Some(*value),
        Value::Bytes(raw) => String::from_utf8_lossy(raw).parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_sealed_preview_is_fetched_again() {
        assert!(needs_fresh_preview("Letter-sealed message", 10, 10));
        assert!(!needs_fresh_preview("hello", 10, 10));
        assert!(needs_fresh_preview("hello", 11, 10));
    }

    #[test]
    fn sticker_url_uses_the_shop_path() {
        let mut message = BTreeMap::new();
        message.insert(15, Value::Int(7));
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"STKID".to_vec()), Value::Bytes(b"5".to_vec())),
            (Value::Bytes(b"STKOPT".to_vec()), Value::Bytes(b"A".to_vec())),
        ]));
        assert!(sticker_url(&message).ends_with("/5/android/sticker_animation.png"));
        let (text, sealed) = message_preview(&message);
        assert_eq!(text, "Sticker");
        assert!(!sealed);
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"STKID".to_vec()), Value::Bytes(b"5".to_vec())),
            (Value::Bytes(b"e2eeVersion".to_vec()), Value::Bytes(b"2".to_vec())),
        ]));
        let (text, sealed) = message_preview(&message);
        assert_eq!(text, "Letter-sealed sticker");
        assert!(sealed);
    }

    #[test]
    fn chat_update_shows_the_event_sentence() {
        let inviter = "uinviterinviterinviterinviterinvi";
        let invitee = "uinviteeinviteeinviteeinviteeinv";
        let mut message = BTreeMap::new();
        message.insert(1, Value::Bytes(inviter.as_bytes().to_vec()));
        message.insert(15, Value::Int(18));
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"LOC_KEY".to_vec()), Value::Bytes(b"C_MI".to_vec())),
            (Value::Bytes(b"LOC_ARGS".to_vec()), Value::Bytes(format!("{inviter}\u{1e}{invitee}").into_bytes())),
        ]));
        let cards = HashMap::from([
            (inviter.to_string(), ("Ken".into(), String::new())),
            (invitee.to_string(), ("Ada".into(), String::new())),
        ]);
        assert_eq!(event_sentence(&message, &cards, "uotherotherotherotherotherother").as_deref(), Some("Ken invited Ada to the group."));
        message.insert(10, Value::Bytes(b"Ken invited Ada.".to_vec()));
        assert_eq!(event_sentence(&message, &cards, ""), None);
        let (text, sealed) = message_preview(&message);
        assert_eq!(text, "Ken invited Ada.");
        assert!(!sealed);

        message.insert(10, Value::Bytes(Vec::new()));
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"LOC_KEY".to_vec()), Value::Bytes(b"C_PN".to_vec())),
            (Value::Bytes(b"LOC_ARGS".to_vec()), Value::Bytes(b"Weekend".to_vec())),
        ]));
        assert_eq!(event_sentence(&message, &cards, inviter).as_deref(), Some("You changed the group name to Weekend."));
    }

    #[test]
    fn preview_skips_a_chat_update() {
        let mut said = BTreeMap::new();
        said.insert(10, Value::Bytes(b"hello".to_vec()));
        said.insert(15, Value::Int(0));
        said.insert(5, Value::Int(10));
        let mut update = BTreeMap::new();
        update.insert(10, Value::Bytes(b"Ken invited Ada to the group.".to_vec()));
        update.insert(15, Value::Int(18));
        update.insert(5, Value::Int(20));
        let (preview, when) = preview_choice(&[said.clone(), update]);
        assert_eq!(when, 20);
        assert_eq!(preview.as_ref().map(|message| text_field(message, 10)), Some("hello".into()));
        let (preview, when) = preview_choice(&[said]);
        assert_eq!(when, 10);
        assert_eq!(preview.as_ref().map(|message| text_field(message, 10)), Some("hello".into()));
        let mut only = BTreeMap::new();
        only.insert(15, Value::Int(18));
        only.insert(5, Value::Int(30));
        let (preview, when) = preview_choice(&[only]);
        assert!(preview.is_none());
        assert_eq!(when, 30);
        let mut labeled = BTreeMap::new();
        labeled.insert(15, Value::Int(18));
        let (text, sealed) = message_preview(&labeled);
        assert_eq!(text, "");
        assert!(!sealed);
        assert!(placeholder_preview("Chat update"));
        assert!(placeholder_preview("Letter-sealed chat update"));
    }

    #[test]
    fn gif_metadata_counts_as_animated() {
        let mut message = BTreeMap::new();
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"MEDIA_CONTENT_INFO".to_vec()), Value::Bytes(br#"{"animated":true,"extension":"jpg"}"#.to_vec())),
        ]));
        assert!(message_animated(&message));
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"FILE_NAME".to_vec()), Value::Bytes(b"loop.GIF".to_vec())),
        ]));
        assert!(message_animated(&message));
        message.insert(18, Value::Map(vec![
            (Value::Bytes(b"FILE_SIZE".to_vec()), Value::Bytes(b"12".to_vec())),
        ]));
        assert!(!message_animated(&message));
    }

    fn op(kind: i64, revision: i64) -> BTreeMap<i16, Value> {
        let mut fields = BTreeMap::new();
        fields.insert(1, Value::Int(revision));
        fields.insert(3, Value::Int(kind));
        fields
    }

    #[test]
    fn sync_reply_yields_operations_and_advances_the_cursor() {
        let mut message = BTreeMap::new();
        message.insert(1, Value::Bytes(b"ufriendfriendfriendfriendfriendf".to_vec()));
        message.insert(2, Value::Bytes(b"umememememememememememememememe".to_vec()));
        let mut received = op(26, 40);
        received.insert(20, Value::Struct(message));
        let mut marker = op(0, -1);
        marker.insert(10, Value::Bytes(b"1286\x1e58291".to_vec()));
        marker.insert(11, Value::Bytes(b"295818\x1enotice".to_vec()));
        let response = Value::Struct(BTreeMap::from([
            (1, Value::Struct(BTreeMap::from([
                (1, Value::List(vec![Value::Struct(received), Value::Struct(marker)])),
                (2, Value::Bool(true)),
            ]))),
        ]));
        let ops = operation_list(&response);
        assert_eq!(ops.len(), 2);
        assert!(sync_has_more(&response));
        let mut cursor = OpCursor { local: 10, global: 0, individual: 0 };
        for item in &ops {
            absorb_op(&mut cursor, item);
        }
        assert_eq!(cursor.local, 40);
        assert_eq!(cursor.individual, 1286);
        assert_eq!(cursor.global, 295818);
        let me = "umememememememememememememememe";
        assert_eq!(chat_target(as_struct(ops[0].get(&20).unwrap()).unwrap(), me), "ufriendfriendfriendfriendfriendf");
        let request = sync_fields(&cursor, 50);
        assert_eq!(request[0].0, 1);
        match &request[0].1 {
            Field::Struct(fields) => {
                assert!(matches!(fields[0], (1, Field::I64(40))));
                assert!(matches!(fields[1], (2, Field::I32(50))));
                assert!(matches!(fields[2], (3, Field::I64(295818))));
                assert!(matches!(fields[3], (4, Field::I64(1286))));
            }
            _ => panic!("sync arguments are one request struct"),
        }
    }

    #[test]
    fn fetch_ops_list_is_read_flat() {
        let response = Value::List(vec![Value::Struct(op(25, 7))]);
        assert_eq!(operation_list(&response).len(), 1);
        assert!(!sync_has_more(&response));
    }

    #[test]
    fn quiet_rounds_settle_on_the_short_poll_interval() {
        assert_eq!(note_quiet(0, Duration::from_millis(40), true), 1);
        assert_eq!(note_quiet(2, Duration::from_millis(40), true), 3);
        assert_eq!(note_quiet(3, Duration::from_secs(30), true), 0);
        assert_eq!(next_wait(Duration::from_millis(40), 1), Duration::from_millis(1960));
        assert_eq!(next_wait(Duration::from_millis(40), 3), Duration::from_millis(4960));
        assert_eq!(next_wait(Duration::from_secs(30), 0), Duration::ZERO);
        assert!(!session_dead(&line("timeout talking to legy")));
        assert!(session_dead(&line_code("sync /P4 failed: logged out", Some(119), "logged out")));
    }

}
