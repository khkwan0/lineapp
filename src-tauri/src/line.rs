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
        }
    }
}

fn config_path() -> PathBuf {
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
    owned: Option<(Vec<u8>, i64)>,
    pubs: HashMap<(String, i64), Option<Vec<u8>>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Client {
    pub fn new() -> Self {
        Self {
            session: load_session(),
            seq: 0,
            legy: None,
            owned: None,
            pubs: HashMap::new(),
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
        let use_legy = !token.is_empty() && path == TALK_PATH && needs_legy(&token);
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

    pub fn list_chats(&mut self) -> Result<Json, LineError> {
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
        let (unread, unread_total, box_ids) = match self.message_box_rows() {
            Ok(rows) => unread_from_rows(&rows),
            Err(err) => {
                failures.push(json!({"name": "unread", "error": err.message}));
                (HashMap::new(), Json::Null, Vec::new())
            }
        };
        let known: std::collections::HashSet<String> = raw_chats.iter().map(|chat| text_field(chat, 2)).collect();
        let direct_mids: Vec<String> = box_ids
            .into_iter()
            .filter(|mid| mid.starts_with('u') && !known.contains(mid))
            .collect();
        let mut pending = Vec::new();
        let mut senders = Vec::new();
        for chat in &raw_chats {
            let name = {
                let raw = text_field(chat, 6);
                if raw.trim().is_empty() { "Unnamed chat".into() } else { raw.trim().to_string() }
            };
            let mid = text_field(chat, 2);
            let (message, when) = self.latest_message(&mid, &name, &mut failures);
            if let Some(message) = &message {
                let sender = text_field(message, 1);
                if !sender.is_empty() && sender != self.session.mid {
                    senders.push(sender);
                }
            }
            pending.push((name, mid, message, when, member_count(chat), picture_url(&text_field(chat, 7)), bool_field(chat, 4), false));
        }
        for mid in &direct_mids {
            let (message, when) = self.latest_message(mid, "direct chat", &mut failures);
            if let Some(message) = &message {
                let sender = text_field(message, 1);
                if !sender.is_empty() && sender != self.session.mid {
                    senders.push(sender);
                }
            }
            pending.push((String::new(), mid.clone(), message, when, None, String::new(), false, true));
        }
        let mut people = senders.clone();
        people.extend(direct_mids.iter().cloned());
        let cards = self.contact_cards(&people);
        let mut rows = Vec::new();
        for (mut name, mid, message, when, members, mut picture, muted, direct) in pending {
            let (preview, sealed) = if let Some(message) = &message {
                let sender_id = text_field(message, 1);
                let mut sender = cards.get(&sender_id).map(|card| card.0.clone()).unwrap_or_default();
                if !sender_id.is_empty() && sender_id == self.session.mid {
                    sender = "You".into();
                }
                self.visible_preview(message, &sender)
            } else {
                (String::new(), false)
            };
            if direct {
                let card = cards.get(&mid);
                name = card.map(|item| item.0.clone()).filter(|item| !item.is_empty()).unwrap_or_else(|| "Direct chat".into());
                picture = card.map(|item| item.1.clone()).unwrap_or_default();
            }
            let (sticker, fallback) = sticker_pair(message.as_ref());
            let unread = unread.get(&mid).copied();
            rows.push(json!({
                "id": mid,
                "name": name,
                "lastMessage": preview,
                "sealed": sealed,
                "time": when,
                "members": members,
                "picture": picture,
                "muted": muted,
                "kind": chat_kind(&mid),
                "unread": unread,
                "sticker": sticker,
                "stickerFallback": fallback,
            }));
        }
        rows.sort_by(|left, right| {
            right.get("time").and_then(|value| value.as_i64()).unwrap_or(0)
                .cmp(&left.get("time").and_then(|value| value.as_i64()).unwrap_or(0))
        });
        let profile = self.list_profile();
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
        let senders: Vec<String> = collected
            .iter()
            .map(|message| text_field(message, 1))
            .filter(|sender| !sender.is_empty() && sender != &self.session.mid)
            .collect();
        let cards = self.contact_cards(&senders);
        let mut messages = Vec::new();
        for message in &collected {
            let (text, sealed) = self.visible_text(message);
            if text.is_empty() {
                continue;
            }
            let sender = text_field(message, 1);
            let mine = !self.session.mid.is_empty() && sender == self.session.mid;
            let (sticker, fallback) = sticker_pair(Some(message));
            messages.push(json!({
                "mine": mine,
                "name": if mine { "" } else { cards.get(&sender).map(|card| card.0.as_str()).unwrap_or("") },
                "text": text,
                "sealed": sealed,
                "time": message_time(message),
                "sticker": sticker,
                "stickerFallback": fallback,
            }));
        }
        Ok(json!({"type": "history", "id": mid, "messages": messages}))
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
            let (preview, sealed, sticker, fallback) = if let Some(message) = &message {
                let sender_id = text_field(&message.clone(), 1);
                let mut sender = cards.get(&sender_id).map(|card| card.0.clone()).unwrap_or_default();
                if !sender_id.is_empty() && sender_id == self.session.mid {
                    sender = "You".into();
                }
                let (preview, sealed) = self.visible_preview(message, &sender);
                let (sticker, fallback) = sticker_pair(Some(message));
                (preview, sealed, sticker, fallback)
            } else {
                (String::new(), false, String::new(), String::new())
            };
            let mut item = json!({
                "id": mid,
                "lastMessage": preview,
                "time": when,
                "sealed": sealed,
                "sticker": sticker,
                "stickerFallback": fallback,
                "unread": unread,
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
        if let Some(Value::Map(pairs)) = logged.get(&10) {
            self.keep_login_key(pairs, private);
        } else if let Some(Value::Struct(fields)) = logged.get(&10) {
            let pairs: Vec<(Value, Value)> = fields
                .iter()
                .map(|(id, value)| (Value::Bytes(id.to_string().into_bytes()), value.clone()))
                .collect();
            self.keep_login_key(&pairs, private);
        }
        let path = save_session(&self.session)?;
        emit(json!({"type": "saved", "path": path.display().to_string()}));
        Ok(())
    }

    fn keep_login_key(&mut self, pairs: &[(Value, Value)], sqr_private: &[u8]) {
        let chain = meta_bytes(pairs, "encryptedKeyChain");
        let public = meta_bytes(pairs, "publicKey");
        let (Some(chain), Some(public)) = (chain, public) else { return };
        let Some((secret, public_key)) = e2ee::decrypt_key_chain(&public, sqr_private, &chain) else { return };
        let Some(key_id) = meta_int(pairs, "keyId") else { return };
        if key_id == 0 {
            return;
        }
        self.session.e2ee_private_key = STANDARD.encode(&secret);
        self.session.e2ee_public_key = STANDARD.encode(&public_key);
        self.session.e2ee_key_id = Some(key_id);
        self.owned = None;
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
        let (private, sender_key_id) = self.self_e2ee_key()?;
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

    fn self_e2ee_key(&mut self) -> Result<(Vec<u8>, i64), LineError> {
        if let Some(owned) = self.owned_private()? {
            return Ok(owned);
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
        let mut key_id = as_struct(&registered).ok().and_then(|fields| int_field(fields, 2)).filter(|id| *id != 0);
        if key_id.is_none() {
            key_id = self.key_id_for_public(public.as_bytes());
        }
        let Some(key_id) = key_id else {
            return Err(line_code("letter sealing key was not registered", Some(82), "invalid sender key"));
        };
        self.session.e2ee_private_key = STANDARD.encode(secret.as_bytes());
        self.session.e2ee_public_key = STANDARD.encode(public.as_bytes());
        self.session.e2ee_key_id = Some(key_id);
        self.owned = Some((secret.as_bytes().to_vec(), key_id));
        save_session(&self.session)?;
        Ok((secret.as_bytes().to_vec(), key_id))
    }

    fn owned_private(&mut self) -> Result<Option<(Vec<u8>, i64)>, LineError> {
        if let Some(owned) = &self.owned {
            return Ok(Some(owned.clone()));
        }
        if self.session.e2ee_private_key.is_empty() || self.session.e2ee_key_id.is_none() {
            return Ok(None);
        }
        let private = STANDARD.decode(&self.session.e2ee_private_key).map_err(|_| line("saved letter-sealing key is unreadable"))?;
        let mut key_id = self.session.e2ee_key_id.unwrap_or(0);
        if !self.session.e2ee_public_key.is_empty() {
            if let Ok(public) = STANDARD.decode(&self.session.e2ee_public_key) {
                if let Some(matched) = self.key_id_for_public(&public) {
                    if matched != 0 {
                        key_id = matched;
                        if Some(key_id) != self.session.e2ee_key_id {
                            self.session.e2ee_key_id = Some(key_id);
                            let _ = save_session(&self.session);
                        }
                    }
                }
            }
        }
        self.owned = Some((private.clone(), key_id));
        Ok(Some((private, key_id)))
    }

    fn key_id_for_public(&mut self, public: &[u8]) -> Option<i64> {
        let listed = self.talk("getE2EEPublicKeys", &[]).ok().flatten()?;
        let Value::List(keys) = listed else { return None };
        for key in keys {
            let Value::Struct(fields) = key else { continue };
            if bytes_field(&fields, 4) == public {
                return int_field(&fields, 2);
            }
        }
        None
    }

    fn visible_text(&mut self, message: &BTreeMap<i16, Value>) -> (String, bool) {
        if let Some(opened) = self.open_sealed_text(message) {
            return (opened, false);
        }
        message_preview(message)
    }

    fn visible_preview(&mut self, message: &BTreeMap<i16, Value>, sender_name: &str) -> (String, bool) {
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
        if !sender.starts_with('u') || !recipient.starts_with('u') {
            return None;
        }
        let (private, own_id) = self.owned_private().ok().flatten()?;
        let sender_key = u32::from_be_bytes(chunks[3].as_slice().try_into().ok()?) as i64;
        let receiver_key = u32::from_be_bytes(chunks[4].as_slice().try_into().ok()?) as i64;
        let (peer, peer_key) = if sender == self.session.mid && sender_key == own_id {
            (recipient.clone(), receiver_key)
        } else if receiver_key == own_id {
            (sender.clone(), sender_key)
        } else {
            return None;
        };
        let public = self.peer_public(&peer, peer_key)?;
        let spec = e2ee_spec(message);
        open_text(&chunks, &private, &public, &sender, &recipient, spec).ok()
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
        let recent = match self.talk("getRecentMessagesV2", &[(2, bin(mid)), (3, Field::I32(1))]) {
            Ok(value) => value,
            Err(err) => {
                failures.push(json!({"name": name, "error": err.message}));
                return (None, 0);
            }
        };
        let Some(Value::List(items)) = recent else { return (None, 0) };
        let Some(Value::Struct(message)) = items.last() else { return (None, 0) };
        let when = message_time(message);
        (Some(message.clone()), when)
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
}
