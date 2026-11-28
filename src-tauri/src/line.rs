//! Talk, QR login, chats, history, send, and letter sealing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

use crate::e2ee::{self, open_text, seal_raw, seal_text};
use crate::legy::{needs_legy, LegyTransport, LEGY_HOST, LEGY_LAP, LEGY_PATH};
use crate::thrift::{self, bin, Field, Value};

const APP_ID: &str = "dev.lineapp.desktop";
const APP_VERSION: &str = "26.6.0.4004";
const SYSTEM_VERSION: &str = "10.0.26100-11NT";
const DEFAULT_ENDPOINT: &str = "legy-jp.line-apps.com";
const LOGIN_PATH: &str = "/acct/lgn/sq/v1";
const LOGIN_POLL_PATH: &str = "/acct/lp/lgn/sq/v1";
const TALK_PATH: &str = "/S4";
const ATTR_NAME: i32 = 2;
const ATTR_PICTURE: i32 = 8;
const ATTR_STATUS: i32 = 16;
const SHOP_PATH: &str = "/TSHOP4";
const SHOP_ID: &str = "stickershop";
const RELATION_PATH: &str = "/RE4";
const HISTORY_PAGE: i32 = 30;
const HISTORY_TARGET: usize = 40;
const MEDIA_LIMIT: usize = 40_000_000;
const POLL_HOLD: Duration = Duration::from_secs(12);
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
    Message { id: String, message: BTreeMap<i16, Value>, incoming: bool, when: i64 },
    Read { id: String, message_id: String },
    Receipt { id: String },
    Refresh { id: String },
    Reaction { id: String, message_id: String, emoji: String, image: String, mine: bool },
    Unsent { id: String, message_id: String },
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
    #[serde(rename = "e2eeSenderKeyId", default)]
    e2ee_sender_key_id: Option<i64>,
    #[serde(rename = "e2eeLocalKeyIds", default, skip_serializing_if = "Vec::is_empty")]
    e2ee_local_key_ids: Vec<i64>,
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
            e2ee_sender_key_id: None,
            e2ee_local_key_ids: Vec::new(),
            e2ee_keys: Vec::new(),
        }
    }
}

static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn install_config_dir(identifier: &str) {
    let dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(identifier);
    let _ = CONFIG_DIR.set(dir.clone());
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(debug_assertions)]
    import_repo_session(&dir);
}

fn config_dir() -> PathBuf {
    CONFIG_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_ID))
}

pub(crate) fn config_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("LINE_CONFIG") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    config_dir().join("lineapp.local.json")
}

/// A debug build still finds a session written next to the repo. Release
/// packages never see that compile-time path.
#[cfg(debug_assertions)]
fn import_repo_session(dir: &PathBuf) {
    let legacy = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("lineapp.local.json");
    if dir.join("lineapp.local.json").is_file() || !legacy.is_file() {
        return;
    }
    let Some(parent) = legacy.parent() else {
        return;
    };
    for name in [
        "lineapp.local.json",
        "lineapp.settings.json",
        "lineapp.db",
        "lineapp.db-wal",
        "lineapp.db-shm",
    ] {
        let from = parent.join(name);
        if !from.is_file() {
            continue;
        }
        let dest = dir.join(name);
        if dest.exists() || std::fs::copy(&from, &dest).is_err() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600));
        }
    }
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

/// One Talk send at a time. Photo upload and the caption share the session,
/// and overlapping sends used to be answered with an older message.
pub(crate) static TALK_SEND: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn save_session(session: &Session) -> Result<PathBuf, LineError> {
    let mut session = session.clone();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(disk) = serde_json::from_str::<Session>(&text) {
            merge_e2ee(&mut session, &disk);
        }
    }
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| line(err.to_string()))?;
    }
    let payload = serde_json::to_string_pretty(&session).map_err(|err| line(err.to_string()))? + "\n";
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

fn merge_e2ee(session: &mut Session, disk: &Session) {
    if session.e2ee_sender_key_id.is_none() {
        session.e2ee_sender_key_id = disk.e2ee_sender_key_id;
    }
    if session.e2ee_private_key.is_empty() {
        session.e2ee_private_key = disk.e2ee_private_key.clone();
        session.e2ee_public_key = disk.e2ee_public_key.clone();
        session.e2ee_key_id = disk.e2ee_key_id;
    }
    for id in &disk.e2ee_local_key_ids {
        if !session.e2ee_local_key_ids.contains(id) {
            session.e2ee_local_key_ids.push(*id);
        }
    }
    for key in &disk.e2ee_keys {
        if !session.e2ee_keys.iter().any(|have| have.private_key == key.private_key) {
            session.e2ee_keys.push(key.clone());
        }
    }
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

    /// Talk treats `seq` as an idempotency key. A fresh client used to send `1`
    /// every time, so a second send — the text next to a photo — came back as
    /// the previous photo. Uploading onto that id failed, and the cleanup
    /// unsend recalled the older photo.
    fn next_seq(&mut self) -> i32 {
        static NEXT: AtomicI32 = AtomicI32::new(0);
        if NEXT.load(Ordering::Relaxed) == 0 {
            let mut bytes = [0u8; 4];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
            let seed = (i32::from_ne_bytes(bytes) & 0x7fff_ffff).max(1);
            let _ = NEXT.compare_exchange(0, seed, Ordering::Relaxed, Ordering::Relaxed);
        }
        let seq = NEXT.fetch_add(1, Ordering::Relaxed).max(1);
        self.seq = seq;
        seq
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
        let me = self.session.mid.clone();
        let marks = self.read_marks(mid);
        let mut messages = Vec::new();
        for message in &collected {
            let (text, sealed) = self.visible_text(message, &cards);
            if text.is_empty() {
                continue;
            }
            let sender = text_field(message, 1);
            let mine = !me.is_empty() && sender == me;
            let ident = text_field(message, 4);
            let (status, readers) = if mine { delivery_state(&ident, &me, &marks) } else { (String::new(), 0) };
            let name = if mine { "" } else { cards.get(&sender).map(|card| card.0.as_str()).unwrap_or("") };
            let unsent = message_unsent(message);
            let (sticker, fallback) = if unsent { (String::new(), String::new()) } else { sticker_pair(Some(message)) };
            let kind = content_kind(int_field(message, 15));
            let heard = message_duration(message);
            let media_id = if !unsent && (kind == "image" || kind == "video" || kind == "audio") { ident.clone() } else { String::new() };
            let (text, kind, media_id) = finish_body(message, mine, name, text, kind, media_id);
            let mut item = json!({
                "id": ident,
                "mine": mine,
                "sender": sender,
                "status": status,
                "readers": readers,
                "name": name,
                "picture": if mine { "" } else { cards.get(&sender).map(|card| card.1.as_str()).unwrap_or("") },
                "text": text,
                "sealed": sealed && !unsent,
                "time": message_time(message),
                "sticker": sticker,
                "stickerFallback": fallback,
                "kind": kind,
                "gid": if media_id.is_empty() || kind != "image" && kind != "video" { String::new() } else { meta_value(message, "GID") },
                "seq": if media_id.is_empty() || kind != "image" && kind != "video" { 0 } else { meta_value(message, "GSEQ").parse::<i64>().unwrap_or(0) },
                "mediaId": media_id,
                "animated": !media_id.is_empty() && message_animated(message),
                "duration": if kind == "audio" { heard } else { 0 },
                "replyId": reply_target(message),
            });
            attach_reaction(&mut item, message, &me);
            messages.push(item);
        }
        Ok(json!({"type": "history", "id": mid, "lastMessageId": latest_message_id(&collected), "messages": messages}))
    }

    /// `TalkService.sendChatChecked`: the desktop call that moves the read cursor.
    ///
    /// The official client sends `seq`, the chat mid, the newest message id the
    /// user has on screen, and `sessionId` 0. LINE then drops that box's unread
    /// count and tells other devices with operation 40.
    pub fn mark_chat_read(&mut self, chat: &str, last_message_id: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !message_id_ok(last_message_id) {
            return Err(line("that message cannot be marked read"));
        }
        let seq = self.next_seq();
        self.talk("sendChatChecked", &chat_checked_fields(seq, chat, last_message_id))?;
        Ok(())
    }

    /// `TalkService.react` / `cancelReaction`. The chosen icon goes out as a paid
    /// sticon. LINE then tells this account's other devices with operation 139
    /// and the other person with operation 140.
    pub fn send_reaction(&mut self, message_id: &str, emoji: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !message_id_ok(message_id) {
            return Err(line("that message cannot take a reaction"));
        }
        let message_id = message_id.parse::<i64>().map_err(|_| line("that message cannot take a reaction"))?;
        let seq = self.next_seq();
        if emoji.is_empty() {
            self.talk(
                "cancelReaction",
                &[(1, Field::Struct(vec![(1, Field::I32(seq)), (2, Field::I64(message_id))]))],
            )?;
            return Ok(());
        }
        let (product, emoji_id) = reaction_sticon(emoji).ok_or_else(|| line("that reaction is not available"))?;
        self.talk(
            "react",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(seq)),
                    (2, Field::I64(message_id)),
                    (
                        3,
                        Field::Struct(vec![(
                            2,
                            Field::Struct(vec![
                                (1, bin(product)),
                                (2, bin(emoji_id)),
                                (3, Field::I32(1)),
                                (4, Field::I64(1)),
                            ]),
                        )]),
                    ),
                ]),
            )],
        )?;
        Ok(())
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

    pub fn send_for_window(&mut self, to: &str, message: &str, related_id: &str, silent: bool) -> Json {
        match self.send_text(to, message, related_id, silent) {
            Ok(sent) => sent,
            Err(err) => {
                if to.starts_with('u') && (err.code == Some(82) || invalid_sender_key(&err)) {
                    match self.send_sealed_text(to, message, related_id, silent) {
                        Ok(sent) => sent,
                        Err(sealed) => send_failure(to, &sealed),
                    }
                } else {
                    send_failure(to, &err)
                }
            }
        }
    }

    /// Send a photo, video, voice clip, or file the way the desktop client does.
    ///
    /// A plain chat gets a content-typed `sendMessage`, then the bytes are posted
    /// to `obs.line-apps.com/r/talk/m/{message id}`. A chat that rejects that with
    /// letter sealing (TalkException 82) encrypts the file and uploads it under
    /// the `emi` / `emv` / `ema` / `emf` object path before the sealed message.
    pub fn send_media_for_window(&mut self, to: &str, name: &str, mime: &str, bytes: &[u8], album: Option<Album>, silent: bool, duration_ms: i64, voice: bool) -> Json {
        if bytes.is_empty() || bytes.len() > MEDIA_LIMIT {
            return send_failure(to, &line("that file is too large to send"));
        }
        let kind = prepare_outgoing(classify_media(name, mime), bytes, duration_ms, voice);
        let album = album.filter(|album| kind.kind == "image" && album_ok(album));
        match self.send_plain_media(to, &kind, bytes, album.as_ref(), silent) {
            Ok(sent) => sent,
            Err(err) if needs_seal(&err) => match self.send_sealed_media(to, &kind, bytes, album.as_ref(), silent) {
                Ok(sent) => sent,
                Err(sealed) => send_failure(to, &sealed),
            },
            Err(err) => send_failure(to, &err),
        }
    }

    /// `contentType` 7. Package and sticker ids are the ones LINE publishes for
    /// sticker messages. `STKVER` is 1 so phones load stickershop, and animated
    /// packs set `STKOPT` to `A`. `hasContent` is set, which a sticker message requires.
    pub fn send_sticker_for_window(&mut self, to: &str, package_id: &str, sticker_id: &str, animated: bool) -> Json {
        if !sticker_id_ok(package_id) || !sticker_id_ok(sticker_id) {
            return send_failure(to, &line("that sticker cannot be sent"));
        }
        let meta = sticker_meta(package_id, sticker_id, animated);
        match self.send_content(to, 7, &meta, &[], true, false) {
            Ok(sent) => self.finish_sticker(sent, package_id, sticker_id, animated),
            Err(err) if needs_seal(&err) => {
                let raw = sticker_body(&meta);
                match self.send_sealed_content(to, 7, &meta, &raw) {
                    Ok(sent) => self.finish_sticker(sent, package_id, sticker_id, animated),
                    Err(sealed) => send_failure(to, &sealed),
                }
            }
            Err(err) => send_failure(to, &err),
        }
    }

    /// Keyword stickers. The dictionary maps a word to tag ids such as `c27`.
    /// `getSuggestResourcesV2` maps each tag to sticker ids inside packs this
    /// account owns. A tag is not itself a sticker id.
    pub fn suggest_stickers(&mut self, query: &str, language: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let key = suggest_key(query);
        if key.is_empty() || key.chars().count() > 80 {
            return Ok(json!({ "stickers": [] }));
        }
        self.ensure_suggest(language)?;
        let cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
        let mut order: Vec<String> = Vec::new();
        let wanted = suggest_language_code(language);
        if cache.dicts.contains_key(&wanted) {
            order.push(wanted);
        }
        for lang in &cache.settings {
            if cache.dicts.contains_key(&lang.language) && !order.contains(&lang.language) {
                order.push(lang.language.clone());
            }
        }
        for lang in &order {
            let Some(dict) = cache.dicts.get(lang) else { continue };
            let tags = suggest_tags(dict, &key);
            if tags.is_empty() {
                continue;
            }
            let resolved = resolve_suggest_tags(&tags, &cache.packs);
            if resolved.is_empty() {
                continue;
            }
            let stickers: Vec<Json> = resolved.into_iter().take(24).map(|item| json!({
                "packageId": item.package_id,
                "stickerId": item.sticker_id.to_string(),
                "animated": item.animated,
            })).collect();
            return Ok(json!({ "stickers": stickers }));
        }
        Ok(json!({ "stickers": [] }))
    }

    fn ensure_suggest(&mut self, language: &str) -> Result<(), LineError> {
        let fresh = {
            let cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.settings_at.is_some_and(|at| at.elapsed() < Duration::from_secs(6 * 3600))
        };
        if !fresh {
            let value = self.shop("getSuggestDictionarySetting", &[(2, Field::Struct(vec![]))])?
                .ok_or_else(|| line("LINE did not return sticker keywords"))?;
            let settings = parse_suggest_langs(&value);
            let mut cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.settings = settings;
            cache.settings_at = Some(Instant::now());
        }
        let wanted = suggest_language_code(language);
        let missing: Vec<SuggestLang> = {
            let cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.settings.iter().filter(|lang| {
                (lang.preload || lang.language == wanted) && !cache.dicts.contains_key(&lang.language)
            }).cloned().collect()
        };
        for lang in missing {
            let text = match load_suggest_file(&lang) {
                Ok(text) => text,
                Err(_) => continue,
            };
            let parsed = parse_suggest_dict(&text);
            let mut cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.dicts.insert(lang.language, parsed);
        }
        let packs_fresh = {
            let cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.packs_at.is_some_and(|at| at.elapsed() < Duration::from_secs(600))
        };
        if !packs_fresh {
            let owned = self.owned_sticker_packs().unwrap_or_else(|_| Vec::new());
            let mut meta = Vec::new();
            for pack in &owned {
                let id = pack.get("id").and_then(|value| value.as_str()).unwrap_or("");
                if !sticker_id_ok(id) {
                    continue;
                }
                let animated = pack.get("animated").and_then(|value| value.as_bool()).unwrap_or(false);
                meta.push((id.to_string(), animated));
            }
            let mut loaded = Vec::new();
            for chunk in meta.chunks(40) {
                let list = Field::List(thrift::BINARY, chunk.iter().map(|(id, _)| bin(id.as_str())).collect());
                let Ok(Some(value)) = self.shop("getSuggestResourcesV2", &[(2, Field::Struct(vec![
                    (1, Field::I32(1)),
                    (2, list),
                ]))]) else {
                    continue;
                };
                let Ok(fields) = as_struct(&value) else { continue };
                let Some(Value::Map(pairs)) = fields.get(&1) else { continue };
                let mut by_id = HashMap::new();
                for (key, item) in pairs {
                    let id = match key {
                        Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
                        Value::Int(n) if *n > 0 => n.to_string(),
                        _ => continue,
                    };
                    let Value::Struct(resource) = item else { continue };
                    let path = text_field(resource, 1);
                    if !suggest_path_ok(&path) {
                        continue;
                    }
                    by_id.insert(id, (path, int_field(resource, 2).unwrap_or(0)));
                }
                for (id, animated) in chunk {
                    let Some((path, version)) = by_id.get(id) else { continue };
                    let name = format!("pack-{id}-{version}.txt");
                    let Ok(text) = load_obs_text(&name, path) else { continue };
                    loaded.push(PackTags {
                        package_id: id.clone(),
                        animated: *animated,
                        tags: parse_pack_tags(&text),
                    });
                }
            }
            let mut cache = suggest_cache().lock().unwrap_or_else(|err| err.into_inner());
            cache.packs = loaded;
            cache.packs_at = Some(Instant::now());
        }
        Ok(())
    }

    /// Owned packs from `ShopService.getOwnedProductSummaries`, plus a shop
    /// page from `getShowcaseV3`. A shop failure still returns the owned list.
    pub fn sticker_shelf(&mut self) -> Result<Json, LineError> {
        self.require_token()?;
        let owned = self.owned_sticker_packs()?;
        let owned_ids: HashSet<String> = owned
            .iter()
            .filter_map(|pack| pack.get("id").and_then(|value| value.as_str()).map(str::to_string))
            .collect();
        let (shop, shop_error) = match self.shop_sticker_packs() {
            Ok(mut packs) => {
                for pack in &mut packs {
                    let id = pack.get("id").and_then(|value| value.as_str()).unwrap_or("");
                    if owned_ids.contains(id) {
                        if let Some(object) = pack.as_object_mut() {
                            object.insert("owned".to_string(), json!(true));
                        }
                    }
                }
                (packs, String::new())
            }
            Err(err) => (Vec::new(), err.to_string()),
        };
        Ok(json!({ "owned": owned, "shop": shop, "shopError": shop_error }))
    }

    /// `ShopService.getProduct` for one sticker set: ids, price, and whether
    /// this account already owns it.
    pub fn sticker_pack(&mut self, product_id: &str) -> Result<Json, LineError> {
        self.require_token()?;
        if !sticker_id_ok(product_id) {
            return Err(line("that sticker set cannot be opened"));
        }
        let detail = self.product_detail(product_id)?;
        Ok(detail_pack(as_struct(&detail)?))
    }

    /// Free sets use `placePurchaseOrderForFreeProduct`. Sets priced in LINE
    /// Coins use `placePurchaseOrderWithLineCoin`. App-store and Premium sets
    /// are refused here. The price comes from `getProduct`, not from the window.
    pub fn buy_sticker(&mut self, product_id: &str) -> Result<Json, LineError> {
        self.require_token()?;
        if !sticker_id_ok(product_id) {
            return Err(line("that sticker set cannot be bought"));
        }
        if !self.session.mid.starts_with('u') {
            return Err(line("this account cannot buy stickers yet"));
        }
        let detail = self.product_detail(product_id)?;
        let fields = as_struct(&detail)?;
        let pack = detail_pack(fields);
        if pack.get("owned") == Some(&Json::Bool(true)) {
            return Ok(json!({ "ok": true, "owned": true, "confirmUrl": "", "pack": pack }));
        }
        let sale = pack.get("sale").and_then(|value| value.as_str()).unwrap_or("none");
        let method = match sale {
            "free" => "placePurchaseOrderForFreeProduct",
            "coins" => "placePurchaseOrderWithLineCoin",
            "store" => return Err(line("that set is sold in the phone app store")),
            "premium" => return Err(line("that set is part of Stickers Premium")),
            _ => return Err(line("that sticker set is not for sale")),
        };
        let (currency, amount, price_string) = order_price(fields, sale);
        let order = Field::Struct(vec![
            (1, bin(SHOP_ID)),
            (2, bin(product_id)),
            (5, bin(&self.session.mid)),
            (11, Field::Struct(vec![
                (1, bin(currency)),
                (2, bin(amount)),
                (3, bin(price_string)),
            ])),
            (12, Field::Bool(false)),
            (21, shop_locale()),
            (31, Field::MapBin(Vec::new())),
        ]);
        let placed = self.shop(method, &[(2, order)])?.ok_or_else(|| line("LINE did not finish that purchase"))?;
        let placed = as_struct(&placed)?;
        let confirm = text_field(placed, 12);
        let order_id = text_field(placed, 1);
        if !confirm.is_empty() {
            return Ok(json!({ "ok": true, "owned": false, "confirmUrl": confirm, "orderId": order_id, "pack": pack }));
        }
        if order_id.is_empty() {
            return Err(line("LINE did not finish that purchase"));
        }
        let again = self.product_detail(product_id).ok();
        let mut pack = again.as_ref().map(|value| detail_pack(as_struct(value).unwrap_or(fields))).unwrap_or(pack);
        if pack.get("owned") != Some(&Json::Bool(true)) {
            if let Some(object) = pack.as_object_mut() {
                object.insert("owned".to_string(), json!(true));
            }
        }
        Ok(json!({ "ok": true, "owned": true, "confirmUrl": "", "orderId": order_id, "pack": pack }))
    }

    fn product_detail(&mut self, product_id: &str) -> Result<Value, LineError> {
        self.shop("getProduct", &[
            (2, bin(SHOP_ID)),
            (3, bin(product_id)),
            (4, shop_locale()),
        ])?
        .ok_or_else(|| line("LINE did not return that sticker set"))
    }

    fn owned_sticker_packs(&mut self) -> Result<Vec<Json>, LineError> {
        let mut offset = 0i32;
        let mut packs = Vec::new();
        loop {
            let page = self.shop("getOwnedProductSummaries", &[
                (2, bin(SHOP_ID)),
                (3, Field::I32(offset)),
                (4, Field::I32(100)),
                (5, shop_locale()),
            ])?
            .ok_or_else(|| line("LINE did not return your stickers"))?;
            let fields = as_struct(&page)?;
            let batch = summary_packs(fields.get(&1));
            let count = batch.len() as i32;
            packs.extend(batch);
            let total = int_field(fields, 3).unwrap_or(0);
            offset += count;
            if count == 0 || packs.len() >= 300 || (total > 0 && i64::from(offset) >= total) {
                break;
            }
        }
        Ok(packs)
    }

    fn shop_sticker_packs(&mut self) -> Result<Vec<Json>, LineError> {
        let mut packs = Vec::new();
        let mut seen = HashSet::new();
        let mut last = line("LINE did not return the sticker shop");
        let mut any = false;
        for kind in [14i32, 1, 0] {
            match self.shop("getShowcaseV3", &[(1, Field::Struct(vec![
                (1, Field::I32(1)),
                (2, Field::I32(kind)),
                (3, Field::I32(0)),
                (5, Field::I32(40)),
            ]))]) {
                Ok(Some(value)) => {
                    any = true;
                    for pack in search_packs(as_struct(&value)?.get(&1)) {
                        let id = pack.get("id").and_then(|item| item.as_str()).unwrap_or("").to_string();
                        if id.is_empty() || !seen.insert(id) {
                            continue;
                        }
                        packs.push(pack);
                    }
                }
                Ok(None) => {}
                Err(err) => last = err,
            }
            if packs.len() >= 60 {
                break;
            }
        }
        if !any {
            return Err(last);
        }
        packs.truncate(60);
        Ok(packs)
    }

    /// `TalkService.unsendMessage`: seq plus the message id. LINE recalls that
    /// message for everyone and follows up with operation 64 or 65.
    pub fn unsend_message(&mut self, message_id: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !message_id_ok(message_id) {
            return Err(line("that message cannot be unsent"));
        }
        let seq = self.next_seq();
        self.talk("unsendMessage", &[(1, Field::I32(seq)), (2, bin(message_id))])?;
        Ok(())
    }

    /// `TalkService.reportSpam` for one message: the chat, the sender, the
    /// message id, and a `SpammerReason` (other, advertising, gender harassment, harassment).
    pub fn report_message(&mut self, chat: &str, sender: &str, message_id: &str, reason: i32, text: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !message_id_ok(message_id) || !looks_like_mid(sender) || !matches!(reason, 0..=3) {
            return Err(line("that message cannot be reported"));
        }
        self.talk("reportSpam", &report_fields(chat, sender, message_id, reason, text))?;
        Ok(())
    }

    pub fn poll_inbox(&mut self, known: &mut HashMap<String, i64>) -> Result<Json, LineError> {
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
            let previous = known.get(mid).copied();
            if previous.is_none() || *when > previous.unwrap_or(0) {
                changed.push((mid.clone(), *unread, *when));
            } else {
                known.insert(mid.clone(), *when);
            }
        }
        let mut pending = Vec::new();
        let mut senders = Vec::new();
        let mut failures = Vec::new();
        for (mid, unread, delivered) in changed.into_iter().take(40) {
            known.insert(mid.clone(), delivered);
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
            let message_id = message.as_ref().map(|item| text_field(item, 4)).unwrap_or_default();
            let mut item = json!({
                "id": mid,
                "lastMessage": preview,
                "time": when,
                "sealed": sealed,
                "sticker": sticker,
                "stickerFallback": fallback,
                "unread": unread,
                "messageId": message_id,
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
        let mut box_times = HashMap::new();
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
                            if self.consume_poll(&mut cursor, value, &mut box_times, &mut emit) {
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
                    if self.consume_poll(&mut cursor, &value, &mut box_times, &mut emit) {
                        proven = true;
                    }
                    if self.emit_box_changes(&mut box_times, &mut emit) {
                        return;
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
                    if self.emit_box_changes(&mut box_times, &mut emit) {
                        return;
                    }
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

    fn consume_poll<F: FnMut(Json)>(&mut self, cursor: &mut OpCursor, value: &Value, box_times: &mut HashMap<String, i64>, emit: &mut F) -> bool {
        let ops = operation_list(value);
        let delivered = !ops.is_empty();
        for op in &ops {
            absorb_op(cursor, op);
        }
        if let Some(payload) = self.ops_to_poll(&ops) {
            emit(payload);
        }
        if needs_full_sync(value) {
            if let Ok(payload) = self.poll_inbox(box_times) {
                emit(payload);
            }
        }
        delivered
    }

    fn emit_box_changes<F: FnMut(Json)>(&mut self, box_times: &mut HashMap<String, i64>, emit: &mut F) -> bool {
        if self.cancelled() {
            return true;
        }
        match self.poll_inbox(box_times) {
            Ok(payload) => {
                let pending = payload.get("updates").and_then(|value| value.as_array()).is_some_and(|rows| !rows.is_empty());
                if pending {
                    emit(payload);
                }
                false
            }
            Err(err) => session_dead(&err),
        }
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
        if let Ok(payload) = self.poll_inbox(&mut HashMap::new()) {
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
                    let when = message_time(message).max(int_field(op, 2).unwrap_or(0));
                    notices.push(InboxNotice::Message { id, message: message.clone(), incoming, when });
                }
                40 => {
                    let id = text_field(op, 10);
                    if looks_like_mid(&id) {
                        let message_id = text_field(op, 11);
                        let message_id = if message_id_ok(&message_id) { message_id } else { String::new() };
                        notices.push(InboxNotice::Read { id, message_id });
                    }
                }
                55 => {
                    if let Some(id) = op_mid(op) {
                        notices.push(InboxNotice::Receipt { id });
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
                64 | 65 => {
                    let Some((id, message_id)) = destroy_target(op, &me) else { continue };
                    notices.push(InboxNotice::Unsent { id, message_id });
                }
                139 | 140 => {
                    let message_id = text_field(op, 10);
                    let payload = text_field(op, 11);
                    let Some((id, emoji, image)) = reaction_notice(&payload) else { continue };
                    if !message_id_ok(&message_id) || !looks_like_mid(&id) {
                        continue;
                    }
                    notices.push(InboxNotice::Reaction {
                        id,
                        message_id,
                        emoji,
                        image,
                        mine: int_field(op, 3) == Some(139),
                    });
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
                InboxNotice::Read { id, message_id } => {
                    updates.push(json!({
                        "id": id,
                        "lastMessage": "",
                        "time": 0,
                        "sealed": false,
                        "read": true,
                        "messageId": message_id,
                    }));
                }
                InboxNotice::Receipt { id } => {
                    let marks = self.read_marks(&id);
                    updates.push(json!({
                        "id": id,
                        "lastMessage": "",
                        "time": 0,
                        "sealed": false,
                        "keepPreview": true,
                        "readMarks": read_mark_json(&marks, &me),
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
                InboxNotice::Message { id, message, incoming, when } => {
                    let message = self.full_message(&id, message, &cards);
                    let when = message_time(&message).max(when);
                    updates.push(self.message_update(&id, &message, when, incoming, &cards));
                }
                InboxNotice::Reaction { id, message_id, emoji, image, mine } => {
                    updates.push(json!({
                        "id": id,
                        "lastMessage": "",
                        "time": 0,
                        "sealed": false,
                        "keepPreview": true,
                        "reaction": {
                            "messageId": message_id,
                            "emoji": emoji,
                            "image": image,
                            "mine": mine,
                        },
                    }));
                }
                InboxNotice::Unsent { id, message_id } => {
                    updates.push(json!({
                        "id": id,
                        "lastMessage": "",
                        "time": 0,
                        "sealed": false,
                        "keepPreview": true,
                        "unsentId": message_id,
                    }));
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

    fn full_message(
        &mut self,
        chat: &str,
        message: BTreeMap<i16, Value>,
        cards: &HashMap<String, (String, String)>,
    ) -> BTreeMap<i16, Value> {
        let (text, _) = self.visible_text(&message, cards);
        if !sealed_label(&text) {
            return message;
        }
        let ident = text_field(&message, 4);
        let Ok(page) = self.message_page("getRecentMessagesV2", &[(2, bin(chat)), (3, Field::I32(20))]) else {
            return message;
        };
        if !ident.is_empty() {
            if let Some(found) = page.iter().find(|item| text_field(item, 4) == ident) {
                let (opened, _) = self.visible_text(found, cards);
                if !sealed_label(&opened) {
                    return found.clone();
                }
            }
        }
        message
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
        let unsent = message_unsent(message);
        let (preview, sealed, sticker, fallback) = if chat_update {
            (String::new(), false, String::new(), String::new())
        } else {
            let (preview, sealed) = self.visible_preview(message, &sender);
            let (sticker, fallback) = if unsent { (String::new(), String::new()) } else { sticker_pair(Some(message)) };
            (preview, sealed && !unsent, sticker, fallback)
        };
        let mut item = json!({
            "id": id,
            "lastMessage": preview,
            "time": when,
            "sealed": sealed,
            "sticker": sticker,
            "stickerFallback": fallback,
            "messageId": text_field(message, 4),
            "incoming": incoming,
            "keepPreview": chat_update,
        });
        if !chat_update {
            let (text, sealed_body) = self.visible_text(message, cards);
                if !text.is_empty() {
                let ident = text_field(message, 4);
                let mine = !self.session.mid.is_empty() && sender_id == self.session.mid;
                let name = if mine { "" } else { cards.get(&sender_id).map(|card| card.0.as_str()).unwrap_or("") };
                let kind = content_kind(int_field(message, 15));
                let heard = message_duration(message);
                let media_id = if !unsent && (kind == "image" || kind == "video" || kind == "audio") { ident.clone() } else { String::new() };
                let (text, kind, media_id) = finish_body(message, mine, name, text, kind, media_id);
                let mut thread = json!({
                    "id": ident,
                    "mine": mine,
                    "sender": sender_id,
                    "name": name,
                    "picture": if mine { "" } else { cards.get(&sender_id).map(|card| card.1.as_str()).unwrap_or("") },
                    "text": text,
                    "sealed": sealed_body && !unsent,
                    "time": when,
                    "sticker": sticker,
                    "stickerFallback": fallback,
                    "kind": kind,
                    "gid": if media_id.is_empty() || kind != "image" && kind != "video" { String::new() } else { meta_value(message, "GID") },
                    "seq": if media_id.is_empty() || kind != "image" && kind != "video" { 0 } else { meta_value(message, "GSEQ").parse::<i64>().unwrap_or(0) },
                    "mediaId": media_id,
                    "animated": !media_id.is_empty() && message_animated(message),
                    "duration": if kind == "audio" { heard } else { 0 },
                    "replyId": reply_target(message),
                });
                attach_reaction(&mut thread, message, &self.session.mid);
                item["thread"] = thread;
            }
        }
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

    fn shop(&mut self, method: &str, fields: &[(i16, Field)]) -> Result<Option<Value>, LineError> {
        self.call(method, fields, SHOP_PATH, Duration::from_secs(30), &[], None)
            .map_err(fail_line)
    }

    fn send_plain_media(&mut self, to: &str, kind: &OutgoingMedia, bytes: &[u8], album: Option<&Album>, silent: bool) -> Result<Json, LineError> {
        let mut meta = plain_media_meta(kind, bytes.len());
        push_album(&mut meta, album);
        let sent = self.send_content(to, kind.code, &meta, &[], false, silent)?;
        let id = sent.get("messageId").and_then(|value| value.as_str()).unwrap_or("").to_string();
        if let Err(err) = self.obs_post(&format!("/r/talk/m/{id}"), bytes, &plain_obs_params(kind, &id)) {
            // A reused request number comes back as an older photo. That photo is
            // already stored, so a failed upload must not recall it.
            if !crate::store::message_known(&id) {
                let _ = self.unsend_message(&id);
            }
            return Err(err);
        }
        cache_sent_media(&id, kind, bytes);
        Ok(media_sent(sent, kind, album))
    }

    fn send_sealed_media(&mut self, to: &str, kind: &OutgoingMedia, bytes: &[u8], album: Option<&Album>, silent: bool) -> Result<Json, LineError> {
        let (private, sender_key, receiver_public, receiver_key, spec) = self.seal_material(to)?;
        let (material, encrypted) = e2ee::encrypt_file(bytes).map_err(line)?;
        let temp = temp_object_id();
        let mut params = json!({"ver": "2.0", "name": kind.name, "type": "file"});
        if kind.animated {
            params["cat"] = json!("original");
        }
        let oid = self.obs_post(&format!("/r/talk/{}/{temp}", kind.sid), &encrypted, &params)?;
        if oid.is_empty() {
            return Err(line("LINE did not accept that file"));
        }
        if kind.code == 1 || kind.code == 2 {
            let preview = self.obs_post(&format!("/r/talk/{}/{oid}__ud-preview", kind.sid), &encrypted, &params);
            if preview.is_err() && kind.code == 1 {
                return Err(line("LINE did not accept that photo"));
            }
        }
        let body = serde_json::json!({"keyMaterial": material, "fileName": kind.name});
        let raw = serde_json::to_vec(&body).map_err(|err| line(err.to_string()))?;
        let chunks = seal_raw(
            &private,
            sender_key,
            &receiver_public,
            receiver_key,
            &self.session.mid,
            to,
            &raw,
            spec,
            i64::from(kind.code),
        )
        .map_err(|err| line_code(err, Some(82), "E2EE_RETRY_ENCRYPT"))?;
        let mut meta = e2ee_meta(spec, kind.code);
        meta.push((b"SID".to_vec(), kind.sid.as_bytes().to_vec()));
        meta.push((b"OID".to_vec(), oid.into_bytes()));
        meta.push((b"FILE_SIZE".to_vec(), encrypted.len().to_string().into_bytes()));
        if kind.code == 14 || (kind.code == 3 && !kind.voice) {
            meta.push((b"FILE_NAME".to_vec(), kind.name.as_bytes().to_vec()));
        }
        if kind.code == 3 && kind.duration_ms > 0 {
            let heard = kind.duration_ms.to_string().into_bytes();
            meta.push((b"AUDLEN".to_vec(), heard));
        }
        if kind.code == 1 || kind.code == 2 {
            let info = serde_json::json!({
                "category": "original",
                "fileSize": encrypted.len(),
                "extension": kind.extension,
                "animated": kind.animated,
            });
            meta.push((b"MEDIA_CONTENT_INFO".to_vec(), info.to_string().into_bytes()));
        }
        push_album(&mut meta, album);
        let sent = self.send_content(to, kind.code, &meta, &chunks, false, silent)?;
        let id = sent.get("messageId").and_then(|value| value.as_str()).unwrap_or("").to_string();
        cache_sent_media(&id, kind, bytes);
        Ok(media_sent(sent, kind, album))
    }

    fn send_sealed_content(&mut self, to: &str, content_type: i32, meta: &[(Vec<u8>, Vec<u8>)], body: &[u8]) -> Result<Json, LineError> {
        let (private, sender_key, receiver_public, receiver_key, spec) = self.seal_material(to)?;
        let chunks = seal_raw(
            &private,
            sender_key,
            &receiver_public,
            receiver_key,
            &self.session.mid,
            to,
            body,
            spec,
            i64::from(content_type),
        )
        .map_err(|err| line_code(err, Some(82), "E2EE_RETRY_ENCRYPT"))?;
        let mut full = e2ee_meta(spec, content_type);
        full.extend(meta.iter().cloned());
        self.send_content(to, content_type, &full, &chunks, true, false)
    }

    fn send_content(&mut self, to: &str, content_type: i32, meta: &[(Vec<u8>, Vec<u8>)], chunks: &[Vec<u8>], has_content: bool, silent: bool) -> Result<Json, LineError> {
        self.require_token()?;
        let seq = self.next_seq();
        let call = if chunks.is_empty() {
            content_fields(to, content_type, meta, seq, has_content, silent)
        } else {
            sealed_content_fields(to, content_type, meta, chunks, seq, has_content, silent)
        };
        let success = self.talk("sendMessage", &call)?.ok_or_else(|| line("empty send response"))?;
        let fields = as_struct(&success)?;
        let when = message_time(fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let id = text_field(fields, 4);
        if !message_id_ok(&id) {
            return Err(line("LINE did not accept that message"));
        }
        Ok(sent_payload(to, &id, "", if when == 0 { now } else { when }, ""))
    }

    fn seal_material(&mut self, to: &str) -> Result<(Vec<u8>, i64, Vec<u8>, i64, i64), LineError> {
        let missing = || line_code("letter sealing key is missing", Some(82), "E2EE_RETRY_ENCRYPT");
        if to.starts_with('u') {
            let negotiated = self.talk("negotiateE2EEPublicKey", &[(2, bin(to))])?.ok_or_else(missing)?;
            let negotiated = as_struct(&negotiated)?;
            let spec = int_field(negotiated, 3).unwrap_or(-1);
            if spec == -1 {
                return Err(line_code("letter sealing is not available for this chat", Some(82), "E2EE_RETRY_ENCRYPT"));
            }
            let spec_version = if spec > 0 { spec } else { 2 };
            let public = match negotiated.get(&2) {
                Some(Value::Struct(fields)) => fields,
                _ => return Err(missing()),
            };
            let receiver_key_id = int_field(public, 2).ok_or_else(missing)?;
            let receiver_public = bytes_field(public, 4);
            if receiver_public.len() != 32 {
                return Err(missing());
            }
            let Some((private, sender_key_id)) = self.sender_key() else {
                return Err(line_code("this window has no letter-sealing key", Some(82), "letter sealing key is missing"));
            };
            return Ok((private, sender_key_id, receiver_public, receiver_key_id, spec_version));
        }
        if to.starts_with('c') || to.starts_with('r') {
            let Some(shared) = self.group_shared_key(to, 0) else { return Err(missing()) };
            let fields = as_struct(&shared)?;
            let group_key = int_field(fields, 2).filter(|id| *id > 0).ok_or_else(missing)?;
            let Some(group_private) = self.group_private(to, group_key) else { return Err(missing()) };
            let Some((_, sender_key_id)) = self.sender_key() else {
                return Err(line_code("this window has no letter-sealing key", Some(82), "letter sealing key is missing"));
            };
            let Some(public) = self.public_for_key(sender_key_id) else { return Err(missing()) };
            return Ok((group_private, sender_key_id, public, group_key, 2));
        }
        Err(line_code("letter sealing is not available for this chat", Some(82), "E2EE_RETRY_ENCRYPT"))
    }

    fn finish_sticker(&self, sent: Json, package_id: &str, sticker_id: &str, animated: bool) -> Json {
        let image = sticker_shop_url(sticker_id, animated);
        let fallback = format!("https://stickershop.line-scdn.net/stickershop/v1/product/{package_id}/android/stickers/{sticker_id}.png");
        let mut sent = sent;
        if let Some(object) = sent.as_object_mut() {
            object.insert("text".to_string(), json!("Sticker"));
            object.insert("kind".to_string(), json!("sticker"));
            object.insert("sticker".to_string(), json!(image));
            object.insert("stickerFallback".to_string(), json!(fallback));
        }
        sent
    }

    fn obs_post(&self, path: &str, body: &[u8], params: &Json) -> Result<String, LineError> {
        let encoded = STANDARD.encode(params.to_string().as_bytes());
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).build();
        let mut request = agent
            .post(&format!("https://obs.line-apps.com{path}"))
            .set("x-line-access", &self.session.access_token)
            .set("x-line-application", &application_header())
            .set("user-agent", &user_agent())
            .set("content-type", "application/octet-stream")
            .set("x-obs-params", &encoded);
        if !self.session.mid.is_empty() {
            request = request.set("x-line-mid", &self.session.mid);
        }
        let response = match request.send_bytes(body) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) if code == 200 || code == 201 => response,
            Err(ureq::Error::Status(code, _)) => return Err(line(format!("upload failed ({code})"))),
            Err(err) => return Err(line(format!("upload failed: {err}"))),
        };
        let oid = response.header("x-obs-oid").unwrap_or("").trim().to_string();
        let mut sink = Vec::new();
        let _ = response.into_reader().take(4096).read_to_end(&mut sink);
        Ok(oid)
    }

    fn send_text(&mut self, to: &str, message: &str, related_id: &str, silent: bool) -> Result<Json, LineError> {
        self.require_token()?;
        let seq = self.next_seq();
        let success = self.talk("sendMessage", &send_fields(to, message, seq, related_id, silent))?.ok_or_else(|| line("empty send response"))?;
        let fields = as_struct(&success)?;
        let when = message_time(fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let body = text_field(fields, 10);
        let text = if body.is_empty() { message.to_string() } else { body };
        Ok(sent_payload(to, &text_field(fields, 4), &text, if when == 0 { now } else { when }, related_id))
    }

    fn send_sealed_text(&mut self, to: &str, message: &str, related_id: &str, silent: bool) -> Result<Json, LineError> {
        self.require_token()?;
        if !to.starts_with('u') || self.session.mid.is_empty() {
            return Err(line_code("letter sealing is not available for this chat", Some(82), "E2EE_RETRY_ENCRYPT"));
        }
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
        let Some((private, sender_key_id)) = self.sender_key() else {
            return Err(line_code(
                "this window has no letter-sealing key",
                Some(82),
                "letter sealing key is missing",
            ));
        };
        let chunks = self.seal_for(to, message, &private, sender_key_id, &receiver_public, receiver_key_id, spec_version)?;
        self.post_sealed(to, message, &chunks, spec_version, related_id, silent)
    }

    fn seal_for(
        &self,
        to: &str,
        message: &str,
        private: &[u8],
        sender_key_id: i64,
        receiver_public: &[u8],
        receiver_key_id: i64,
        spec_version: i64,
    ) -> Result<Vec<Vec<u8>>, LineError> {
        seal_text(private, sender_key_id, receiver_public, receiver_key_id, &self.session.mid, to, message, spec_version)
            .map_err(|err| line_code(err, Some(82), "E2EE_RETRY_ENCRYPT"))
    }

    fn post_sealed(&mut self, to: &str, message: &str, chunks: &[Vec<u8>], spec_version: i64, related_id: &str, silent: bool) -> Result<Json, LineError> {
        let seq = self.next_seq();
        let success = self
            .talk("sendMessage", &sealed_fields(to, chunks, spec_version, seq, related_id, silent))?
            .ok_or_else(|| line("empty send response"))?;
        let fields = as_struct(&success)?;
        let when = message_time(fields);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        Ok(sent_payload(to, &text_field(fields, 4), message, if when == 0 { now } else { when }, related_id))
    }

    // LINE's encryption overview: the registered key id is the latest public key, and older ones are rejected as an invalid sender key.
    fn sender_key(&mut self) -> Option<(Vec<u8>, i64)> {
        self.secret_keys().into_iter().max_by_key(|(_, id)| *id)
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
        if message_unsent(message) {
            return ("Unsent a message.".into(), false);
        }
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
        if message_unsent(message) {
            let who = if sender_name.is_empty() { "Someone" } else { sender_name };
            return (format!("{who} unsent a message."), false);
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
        let kind = int_field(message, 15);
        if kind != Some(14) && kind != Some(3) {
            return None;
        }
        let name = self.sealed_payload(message)?.get("fileName")?.as_str()?.trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }

    fn read_marks(&mut self, chat: &str) -> Vec<(String, i64)> {
        let Ok(Some(value)) = self.talk("getMessageReadRange", &[(2, Field::List(thrift::BINARY, vec![bin(chat)]))]) else {
            return Vec::new();
        };
        let Value::List(rows) = value else { return Vec::new() };
        let mut marks = Vec::new();
        for row in rows {
            let Value::Struct(fields) = row else { continue };
            let Some(Value::Map(pairs)) = fields.get(&2) else { continue };
            for (key, value) in pairs {
                let Value::Bytes(raw) = key else { continue };
                let who = String::from_utf8_lossy(raw).into_owned();
                let Value::List(ranges) = value else { continue };
                let mut end = 0i64;
                for range in ranges {
                    let Value::Struct(item) = range else { continue };
                    end = end.max(int_field(item, 2).unwrap_or(0));
                }
                if end > 0 && !who.is_empty() {
                    marks.push((who, end));
                }
            }
        }
        marks
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
            return json!({"mid": self.session.mid, "name": "", "picture": ""});
        };
        let Ok(fields) = as_struct(&value) else {
            return json!({"mid": self.session.mid, "name": "", "picture": ""});
        };
        let mid = text_field(fields, 1);
        let mut path = text_field(fields, 22);
        if path.trim().is_empty() {
            path = text_field(fields, 33);
        }
        json!({
            "mid": if mid.is_empty() { self.session.mid.clone() } else { mid },
            "name": text_field(fields, 20).trim(),
            "status": text_field(fields, 24).trim(),
            "picture": picture_url(&path),
        })
    }

    pub fn update_account(&mut self, name: &str, status: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let name = checked_profile_text(name, 1, 20, "that name cannot be used")?;
        let status = checked_profile_text(status, 0, 500, "that status cannot be used")?;
        self.update_profile_attributes(&[(ATTR_NAME, name.as_str()), (ATTR_STATUS, status.as_str())])?;
        self.my_profile()
    }

    pub fn update_account_picture(&mut self, bytes: &[u8]) -> Result<Json, LineError> {
        self.require_token()?;
        let mid = self.session.mid.clone();
        if !user_mid_ok(&mid) {
            return Err(line("Sign in to LINE first."));
        }
        let jpeg = profile_jpeg(bytes, false)?;
        let oid = self.obs_post(
            &format!("/r/talk/p/{mid}"),
            &jpeg,
            &json!({"name": "profile.jpg", "ver": "2.0", "type": "image"}),
        )?;
        if oid.is_empty() {
            return Err(line("LINE did not accept that photo"));
        }
        self.my_profile()
    }

    pub fn clear_account_picture(&mut self) -> Result<Json, LineError> {
        self.require_token()?;
        self.update_profile_attributes(&[(ATTR_PICTURE, "")])?;
        self.my_profile()
    }

    pub fn update_account_cover(&mut self, bytes: &[u8]) -> Result<Json, LineError> {
        self.require_token()?;
        let mid = self.session.mid.clone();
        if !user_mid_ok(&mid) {
            return Err(line("Sign in to LINE first."));
        }
        let jpeg = profile_jpeg(bytes, true)?;
        let oid = self.upload_home_image(&jpeg)?;
        self.set_home_cover(&oid)?;
        let mut card = self.my_profile()?;
        if card.get("cover").and_then(|value| value.as_str()).unwrap_or("").is_empty() {
            card["cover"] = json!(cover_object_url(&oid));
        }
        Ok(card)
    }

    fn update_profile_attributes(&mut self, attrs: &[(i32, &str)]) -> Result<(), LineError> {
        if attrs.is_empty() {
            return Ok(());
        }
        let seq = self.next_seq();
        self.talk("updateProfileAttributes", &profile_attribute_fields(seq, attrs))?;
        Ok(())
    }

    fn upload_home_image(&self, jpeg: &[u8]) -> Result<String, LineError> {
        let params = json!({"name": "cover.jpg", "ver": "2.0", "type": "image"});
        if let Ok(oid) = self.obs_post("/r/myhome/h", jpeg, &params) {
            if !oid.is_empty() {
                return Ok(oid);
            }
        }
        let id = fresh_object_id();
        let params = json!({"name": "cover.jpg", "ver": "2.0", "type": "image", "oid": id});
        let oid = self.obs_post(&format!("/r/myhome/h/{id}"), jpeg, &params)?;
        if oid.is_empty() { Ok(id) } else { Ok(oid) }
    }

    fn set_home_cover(&mut self, object_id: &str) -> Result<(), LineError> {
        let object_id = object_id.trim();
        if object_id.is_empty() || object_id.len() > 180 || object_id.chars().any(|ch| ch.is_control() || ch == '/' || ch == '\\') {
            return Err(line("LINE did not accept that cover"));
        }
        let Some(token) = self.channel_token() else {
            return Err(line("LINE did not accept that cover"));
        };
        let body = json!({
            "homeId": self.session.mid,
            "coverObjectId": object_id,
            "objectId": object_id,
            "storyShare": false,
            "meta": {}
        });
        let urls = [
            "https://gd2.line.naver.jp/mh/api/v1/home/cover.json",
            "https://gd2.line.naver.jp/mh/api/v1/home/updateCover.json",
        ];
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
        let mut last = line("LINE did not accept that cover");
        for url in urls {
            let response = agent
                .post(url)
                .set("user-agent", &user_agent())
                .set("x-line-application", &application_header())
                .set("x-line-access", &self.session.access_token)
                .set("x-line-channeltoken", &token)
                .set("content-type", "application/json")
                .set("accept", "application/json")
                .send_string(&body.to_string());
            match response {
                Ok(response) if response.status() < 400 => return Ok(()),
                Ok(response) => last = line(format!("LINE did not accept that cover ({})", response.status())),
                Err(ureq::Error::Status(code, _)) if code < 400 => return Ok(()),
                Err(ureq::Error::Status(code, _)) => last = line(format!("LINE did not accept that cover ({code})")),
                Err(err) => last = line(format!("LINE did not accept that cover: {err}")),
            }
        }
        Err(last)
    }

    pub fn my_profile(&mut self) -> Result<Json, LineError> {
        self.require_token()?;
        let mid = self.session.mid.clone();
        if mid.is_empty() {
            return Err(line("Sign in to LINE first."));
        }
        self.contact_profile(&mid)
    }

    pub fn contact_profile(&mut self, mid: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let own = !self.session.mid.is_empty() && mid == self.session.mid;
        let fields = if own {
            let value = self.talk("getProfile", &[])?.ok_or_else(|| line("that profile could not be loaded"))?;
            as_struct(&value)?.clone()
        } else {
            self.one_contact(mid)?
        };
        let mut card = person_card(&fields, own);
        card["id"] = json!(mid);
        card["mine"] = json!(own);
        card["cover"] = json!(self.home_cover(mid));
        Ok(card)
    }

    pub fn set_blocked(&mut self, mid: &str, blocked: bool) -> Result<(), LineError> {
        self.require_token()?;
        if !self.session.mid.is_empty() && mid == self.session.mid {
            return Err(line("you cannot block yourself"));
        }
        let seq = self.next_seq();
        let method = if blocked { "blockContact" } else { "unblockContact" };
        self.talk(method, &[(1, Field::I32(seq)), (2, bin(mid))])?;
        Ok(())
    }

    pub fn update_contact_setting(&mut self, mid: &str, flag: i32, value: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !self.session.mid.is_empty() && mid == self.session.mid {
            return Err(line("that profile is you"));
        }
        let seq = self.next_seq();
        self.talk(
            "updateContactSetting",
            &[(1, Field::I32(seq)), (2, bin(mid)), (3, Field::I32(flag)), (4, bin(value))],
        )?;
        Ok(())
    }

    pub fn add_contact(&mut self, mid: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !self.session.mid.is_empty() && mid == self.session.mid {
            return Err(line("that profile is you"));
        }
        let seq = self.next_seq();
        self.talk(
            "findAndAddContactsByMid",
            &[(1, Field::I32(seq)), (2, bin(mid)), (3, Field::I32(0)), (4, bin(""))],
        )?;
        Ok(())
    }

    fn one_contact(&mut self, mid: &str) -> Result<BTreeMap<i16, Value>, LineError> {
        let Some(value) = self.talk("getContacts", &[(2, Field::List(thrift::BINARY, vec![bin(mid)]))])? else {
            return Err(line("that profile could not be loaded"));
        };
        let Value::List(people) = value else {
            return Err(line("that profile could not be loaded"));
        };
        let mut fallback = None;
        for person in people {
            let Value::Struct(person) = person else { continue };
            if text_field(&person, 1) == mid {
                return Ok(person);
            }
            if fallback.is_none() {
                fallback = Some(person);
            }
        }
        fallback.ok_or_else(|| line("that profile could not be loaded"))
    }

    fn channel_token(&mut self) -> Option<String> {
        let mid = self.session.mid.clone();
        if let Some(token) = cached_channel_token(&mid) {
            return Some(token);
        }
        for path in ["/CH4", "/S4"] {
            let Ok(Some(value)) = self.call(
                "issueChannelToken",
                &[(1, bin("1341209850"))],
                path,
                Duration::from_secs(6),
                &[],
                None,
            ) else {
                continue;
            };
            let Ok(fields) = as_struct(&value) else { continue };
            let primary = text_field(fields, 5);
            let token = if primary.is_empty() { text_field(fields, 1) } else { primary };
            if token.is_empty() {
                continue;
            }
            if let Ok(mut slot) = CHANNEL_TOKEN.lock() {
                *slot = Some(CachedChannel { mid: mid.clone(), token: token.clone(), at: Instant::now() });
            }
            return Some(token);
        }
        None
    }

    fn home_cover(&mut self, mid: &str) -> String {
        let Some(token) = self.channel_token() else { return String::new() };
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(8)).build();
        let url = format!("https://gd2.line.naver.jp/mh/api/v1/home/profile.json?homeId={mid}");
        let response = agent
            .get(&url)
            .set("user-agent", &user_agent())
            .set("x-line-application", &application_header())
            .set("x-line-access", &self.session.access_token)
            .set("x-line-channeltoken", &token)
            .set("accept", "application/json")
            .call();
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return String::new(),
        };
        if response.status() >= 400 {
            return String::new();
        }
        let Ok(body) = response.into_string() else { return String::new() };
        let Ok(parsed) = serde_json::from_str::<Json>(&body) else { return String::new() };
        cover_from_home(&parsed)
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

    pub fn group_roster(&mut self, mid: &str) -> Result<Json, LineError> {
        self.require_token()?;
        if !group_mid_ok(mid) {
            return Err(line("that group cannot be opened"));
        }
        let chat = self.one_chat(mid, true, false)?;
        let facts = roster_facts(&chat)?;
        let ids: Vec<String> = facts.members.iter().map(|(id, _)| id.clone()).collect();
        let cards = self.people_cards(&ids);
        let saved = crate::store::cached_rows();
        let mine = self.session.mid.clone();
        // LINE group chats have no admin role. A personal member may remove
        // anyone else. Official Accounts cannot, and you cannot remove yourself.
        let can_remove = !mine.is_empty() && facts.members.iter().any(|(id, _)| id == &mine);
        let members: Vec<Json> = facts
            .members
            .iter()
            .map(|(id, _)| {
                let card = cards.get(id);
                let fallback = saved.get(id).map(|row| row.name.as_str()).unwrap_or("");
                json!({
                    "id": id,
                    "name": person_name(card, fallback, "Member"),
                    "picture": person_picture(card, saved.get(id).map(|row| row.picture.as_str()).unwrap_or("")),
                    "friend": card.and_then(|item| item.get("friend")).and_then(|value| value.as_bool()).unwrap_or(false),
                    "mine": *id == mine,
                })
            })
            .collect();
        Ok(json!({
            "id": mid,
            "name": facts.name,
            "picture": facts.picture,
            "autoJoin": facts.auto_join,
            "canRemove": can_remove,
            "members": members,
        }))
    }

    pub fn remove_group_member(&mut self, group: &str, member: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !group_mid_ok(group) || !user_mid_ok(member) {
            return Err(line("that member cannot be removed"));
        }
        if !self.session.mid.is_empty() && member == self.session.mid {
            return Err(line("you cannot remove yourself"));
        }
        let chat = self.one_chat(group, true, false)?;
        let facts = roster_facts(&chat)?;
        let mine = self.session.mid.clone();
        if mine.is_empty() || !facts.members.iter().any(|(id, _)| id == &mine) {
            return Err(line("you cannot remove members from this group"));
        }
        if !facts.members.iter().any(|(id, _)| id == member) {
            return Err(line("that person is not in the group"));
        }
        let seq = self.next_seq();
        self.talk(
            "deleteOtherFromChat",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(seq)),
                    (2, bin(group)),
                    (3, Field::Set(thrift::BINARY, vec![bin(member)])),
                ]),
            )],
        )?;
        Ok(())
    }

    pub fn rename_group(&mut self, mid: &str, name: &str) -> Result<(), LineError> {
        self.require_token()?;
        if !group_mid_ok(mid) {
            return Err(line("that group cannot be opened"));
        }
        let name = name.trim();
        if name.is_empty() || name.chars().any(|ch| ch.is_control()) {
            return Err(line("that name cannot be used"));
        }
        if name.chars().count() > 50 {
            return Err(line("that name is too long"));
        }
        let seq = self.next_seq();
        self.talk(
            "updateChat",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(seq)),
                    (
                        2,
                        Field::Struct(vec![(1, Field::I32(0)), (2, bin(mid)), (6, bin(name))]),
                    ),
                    (3, Field::I32(1)),
                ]),
            )],
        )?;
        Ok(())
    }

    pub fn set_group_auto_join(&mut self, mid: &str, enabled: bool) -> Result<(), LineError> {
        self.require_token()?;
        if !group_mid_ok(mid) {
            return Err(line("that group cannot be opened"));
        }
        let seq = self.next_seq();
        self.talk(
            "updateChat",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(seq)),
                    (
                        2,
                        Field::Struct(vec![
                            (1, Field::I32(0)),
                            (2, bin(mid)),
                            (
                                8,
                                Field::Struct(vec![(1, Field::Struct(vec![(2, Field::Bool(!enabled))]))]),
                            ),
                        ]),
                    ),
                    (3, Field::I32(4)),
                ]),
            )],
        )?;
        Ok(())
    }

    pub fn invite_into_group(&mut self, mid: &str, targets: &[String]) -> Result<(), LineError> {
        self.require_token()?;
        if !group_mid_ok(mid) {
            return Err(line("that group cannot be opened"));
        }
        if targets.is_empty() {
            return Err(line("choose someone to invite"));
        }
        if targets.len() > 30 {
            return Err(line("invite fewer people at once"));
        }
        if targets.iter().any(|target| !user_mid_ok(target)) {
            return Err(line("that person cannot be invited"));
        }
        let seq = self.next_seq();
        let items: Vec<Field> = targets.iter().map(bin).collect();
        self.talk(
            "inviteIntoChat",
            &[(
                1,
                Field::Struct(vec![
                    (1, Field::I32(seq)),
                    (2, bin(mid)),
                    (3, Field::Set(thrift::BINARY, items)),
                ]),
            )],
        )?;
        Ok(())
    }

    pub fn group_invite_candidates(&mut self, mid: &str) -> Result<Json, LineError> {
        self.require_token()?;
        if !group_mid_ok(mid) {
            return Err(line("that group cannot be opened"));
        }
        let chat = self.one_chat(mid, true, false)?;
        let facts = roster_facts(&chat)?;
        let mut skip: HashSet<String> = facts.members.iter().map(|(id, _)| id.clone()).collect();
        if !self.session.mid.is_empty() {
            skip.insert(self.session.mid.clone());
        }
        let mut ids = self.friend_ids();
        ids.retain(|id| user_mid_ok(id) && !skip.contains(id));
        ids.truncate(400);
        let cards = self.people_cards(&ids);
        let saved = crate::store::cached_rows();
        let mut people: Vec<Json> = ids
            .iter()
            .filter_map(|id| {
                let card = cards.get(id);
                let fallback = saved.get(id).map(|row| row.name.as_str()).unwrap_or("");
                let name = person_name(card, fallback, "");
                if name.is_empty() {
                    return None;
                }
                Some(json!({
                    "id": id,
                    "name": name,
                    "picture": person_picture(card, saved.get(id).map(|row| row.picture.as_str()).unwrap_or("")),
                }))
            })
            .collect();
        people.sort_by(|left, right| {
            let left_name = left.get("name").and_then(|value| value.as_str()).unwrap_or("");
            let right_name = right.get("name").and_then(|value| value.as_str()).unwrap_or("");
            left_name.to_lowercase().cmp(&right_name.to_lowercase())
        });
        Ok(Json::Array(people))
    }

    fn one_chat(&mut self, mid: &str, with_members: bool, with_invitees: bool) -> Result<BTreeMap<i16, Value>, LineError> {
        let value = self
            .talk(
                "getChats",
                &[(
                    1,
                    Field::Struct(vec![
                        (1, Field::List(thrift::BINARY, vec![bin(mid)])),
                        (2, Field::Bool(with_members)),
                        (3, Field::Bool(with_invitees)),
                    ]),
                )],
            )?
            .ok_or_else(|| line("that group could not be loaded"))?;
        let response = as_struct(&value)?;
        let Some(Value::List(chats)) = response.get(&1) else {
            return Err(line("that group could not be loaded"));
        };
        let mut only = None;
        for chat in chats {
            let Value::Struct(fields) = chat else { continue };
            if text_field(fields, 2) == mid {
                return Ok(fields.clone());
            }
            if only.is_none() {
                only = Some(fields.clone());
            }
        }
        only.ok_or_else(|| line("that group could not be loaded"))
    }

    fn people_cards(&mut self, mids: &[String]) -> HashMap<String, Json> {
        let mut unique = Vec::new();
        for mid in mids {
            if user_mid_ok(mid) && !unique.contains(mid) {
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
                // getContacts returns a Contact. Field 20 is the display name only on getProfile.
                let mut card = person_card(&person, false);
                card["id"] = json!(mid);
                cards.insert(mid, card);
            }
        }
        let mine = self.session.mid.clone();
        let unnamed = cards.get(&mine).and_then(|card| card.get("name")).and_then(|value| value.as_str()).is_none_or(|name| name.trim().is_empty());
        if user_mid_ok(&mine) && unique.iter().any(|id| id == &mine) && unnamed {
            if let Ok(Some(value)) = self.talk("getProfile", &[]) {
                if let Ok(fields) = as_struct(&value) {
                    let mut card = person_card(fields, true);
                    card["id"] = json!(mine.clone());
                    cards.insert(mine, card);
                }
            }
        }
        cards
    }

    fn friend_ids(&mut self) -> Vec<String> {
        let looked = self
            .call("getAllContactIds", &[], TALK_PATH, Duration::from_secs(8), &[], None)
            .ok()
            .flatten();
        let ids = collect_mids(looked.as_ref());
        if !ids.is_empty() {
            return ids;
        }
        crate::store::cached_rows()
            .into_iter()
            .filter(|(id, chat)| chat.kind == "friend" && user_mid_ok(id))
            .map(|(id, _)| id)
            .collect()
    }

    pub fn list_friends(&mut self) -> Result<Json, LineError> {
        self.require_token()?;
        let mine = self.session.mid.clone();
        let mut ids = self.friend_ids();
        ids.retain(|id| user_mid_ok(id) && id != &mine);
        ids.truncate(2000);
        Ok(Json::Array(friend_rows(&ids, &self.people_cards(&ids))))
    }

    pub fn friend_recommendations(&mut self) -> Result<Json, LineError> {
        self.require_token()?;
        let ids = self.recommendation_ids()?;
        let friends: HashSet<String> = self.friend_ids().into_iter().collect();
        let mine = self.session.mid.clone();
        let ids: Vec<String> = ids.into_iter().filter(|id| user_mid_ok(id) && id != &mine && !friends.contains(id)).take(200).collect();
        let reasons = self.recommendation_reasons(&ids);
        let cards = self.people_cards(&ids);
        let saved = crate::store::cached_rows();
        let mut people = Vec::new();
        for id in &ids {
            let card = cards.get(id);
            let fallback = saved.get(id).map(|row| row.name.as_str()).unwrap_or("");
            let name = person_name(card, fallback, "");
            if name.is_empty() {
                continue;
            }
            people.push(json!({
                "id": id,
                "name": name,
                "picture": person_picture(card, saved.get(id).map(|row| row.picture.as_str()).unwrap_or("")),
                "reason": reasons.get(id).cloned().unwrap_or_default(),
            }));
        }
        Ok(Json::Array(people))
    }

    pub fn find_person(&mut self, query: &str) -> Result<Json, LineError> {
        self.require_token()?;
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 40 {
            return Err(line("Enter a LINE ID or phone number."));
        }
        let letters = query.chars().any(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '.' | '@' | '-'));
        let digits = query.chars().filter(|ch| ch.is_ascii_digit()).count();
        let phone = !letters && (query.starts_with('+') || digits >= 8);
        let found = if phone {
            self.find_by_phone(query)
        } else if query.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '-')) {
            self.find_by_id(query.trim_start_matches('@'))
        } else {
            Err(line("Enter a LINE ID or phone number."))
        };
        found.map_err(friendly_search)
    }

    pub fn create_group(&mut self, name: &str, targets: &[String]) -> Result<Json, LineError> {
        self.require_token()?;
        let name = name.trim();
        if name.chars().count() > 50 || name.chars().any(|ch| ch.is_control()) {
            return Err(line("That name cannot be used."));
        }
        if targets.is_empty() {
            return Err(line("Choose at least one friend."));
        }
        if targets.len() > 100 {
            return Err(line("Choose fewer friends."));
        }
        let mine = self.session.mid.clone();
        if targets.iter().any(|id| !user_mid_ok(id) || id == &mine) {
            return Err(line("That person cannot be added to the group."));
        }
        let seq = self.next_seq();
        let members: Vec<Field> = targets.iter().map(bin).collect();
        let created = self
            .talk(
                "createChat",
                &[(
                    1,
                    Field::Struct(vec![
                        (1, Field::I32(seq)),
                        (2, Field::I32(0)),
                        (3, bin(name)),
                        (4, Field::Set(thrift::BINARY, members)),
                    ]),
                )],
            )
            .map_err(|_| line("LINE could not create that group."))?;
        let id = created.as_ref().map(first_group_mid).unwrap_or_default();
        if !group_mid_ok(&id) {
            return Err(line("LINE created the group, but did not return the chat."));
        }
        Ok(json!({
            "id": id,
            "name": name,
            "members": targets.len() as i64 + 1,
        }))
    }

    fn recommendation_ids(&mut self) -> Result<Vec<String>, LineError> {
        let mut last = None;
        for fields in [vec![(1, Field::I32(4))], Vec::new()] {
            match self.talk("getRecommendationIds", &fields) {
                Ok(Some(value)) => return Ok(collect_mids(Some(&value))),
                Ok(None) => return Ok(Vec::new()),
                Err(err) => last = Some(err),
            }
        }
        if let Ok(Some(value)) = self.talk("getRecommendationList", &[]) {
            return Ok(collect_mids(Some(&value)));
        }
        Err(last.unwrap_or_else(|| line("Could not load friend recommendations.")))
    }

    fn recommendation_reasons(&mut self, ids: &[String]) -> HashMap<String, String> {
        let mut reasons = HashMap::new();
        for batch in ids.chunks(50) {
            let targets: Vec<Field> = batch.iter().map(|id| Field::Struct(vec![(1, bin(id.as_str()))])).collect();
            let request = Field::Struct(vec![
                (1, Field::List(thrift::STRUCT, targets)),
                (2, Field::I32(4)),
            ]);
            let Ok(Some(value)) = self
                .call("getRecommendationDetails", &[(1, request)], RELATION_PATH, Duration::from_secs(20), &[], None)
                .map_err(fail_line)
            else {
                continue;
            };
            for (id, reason) in recommendation_reason_map(&value) {
                reasons.entry(id).or_insert(reason);
            }
        }
        reasons
    }

    fn find_by_id(&mut self, id: &str) -> Result<Json, LineError> {
        if let Some(card) = self.contact_lookup("findContactByUserid", &[(2, bin(id))], TALK_PATH) {
            return Ok(card);
        }
        self.contact_lookup(
            "findContactBySearchIdOrTicketV3",
            &[(1, Field::Struct(vec![(1, Field::Struct(vec![(1, bin(id))]))]))],
            RELATION_PATH,
        )
        .ok_or_else(|| line("No one was found."))
    }

    fn find_by_phone(&mut self, query: &str) -> Result<Json, LineError> {
        let digits: String = query.chars().filter(|ch| ch.is_ascii_digit()).collect();
        let mut phones = Vec::new();
        if query.starts_with('+') {
            phones.push(query.to_string());
        }
        if !digits.is_empty() {
            let plus = format!("+{digits}");
            if !phones.contains(&plus) {
                phones.push(plus);
            }
        }
        for phone in &phones {
            if let Some(card) = self.contact_lookup(
                "findContactsByPhone",
                &[(2, Field::Set(thrift::BINARY, vec![bin(phone.as_str())]))],
                TALK_PATH,
            ) {
                return Ok(card);
            }
            if let Some(card) = self.contact_lookup(
                "findContactByPhoneV3",
                &[(1, Field::Struct(vec![(1, bin(phone.as_str()))]))],
                RELATION_PATH,
            ) {
                return Ok(card);
            }
        }
        Err(line("No one was found."))
    }

    fn contact_lookup(&mut self, method: &str, fields: &[(i16, Field)], path: &str) -> Option<Json> {
        let value = self.call(method, fields, path, Duration::from_secs(20), &[], None).ok().flatten()?;
        let fields = first_contact(&value)?;
        let mid = text_field(&fields, 1);
        if !user_mid_ok(&mid) {
            return None;
        }
        let mut card = person_card(&fields, mid == self.session.mid);
        card["id"] = json!(mid);
        Some(card)
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

fn chat_checked_fields(seq: i32, chat_mid: &str, last_message_id: &str) -> Vec<(i16, Field)> {
    vec![
        (1, Field::I32(seq)),
        (2, bin(chat_mid)),
        (3, bin(last_message_id)),
        (4, Field::Byte(0)),
    ]
}

fn message_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 24 && id.chars().all(|ch| ch.is_ascii_digit())
}

const REACTION_PACK: &str = "670e0cce840a8236ddd4ee4c";
const REACTION_TRIAL: &str = "5ac1bfd5040ab15980c9b435";

fn reaction_sticon(emoji: &str) -> Option<(&'static str, &'static str)> {
    REACTION_ICONS.iter().find(|item| item.0 == emoji).map(|item| (item.1, item.2))
}

fn sticon_emoji(product: &str, emoji_id: &str) -> Option<&'static str> {
    REACTION_ICONS.iter().find(|item| item.1 == product && item.2 == emoji_id).map(|item| item.0)
}

const REACTION_ICONS: &[(&str, &str, &str)] = &[
    ("👍", REACTION_PACK, "143"),
    ("😎", REACTION_PACK, "086"),
    ("😍", REACTION_TRIAL, "001"),
    ("😠", REACTION_PACK, "094"),
    ("🥺", REACTION_PACK, "087"),
    ("😭", REACTION_PACK, "093"),
    ("😮", REACTION_PACK, "089"),
];

fn predefined_emoji(kind: i64) -> Option<&'static str> {
    match kind {
        2 => Some("👍"),
        3 => Some("❤️"),
        4 => Some("😆"),
        5 => Some("😮"),
        6 => Some("😢"),
        7 => Some("😡"),
        _ => None,
    }
}

fn sticon_url(product: &str, emoji_id: &str) -> String {
    format!("https://stickershop.line-scdn.net/sticonshop/v1/sticon/{product}/android/{emoji_id}.png")
}

fn reaction_notice(payload: &str) -> Option<(String, String, String)> {
    let parsed = serde_json::from_str::<Json>(payload).ok()?;
    let chat = parsed.get("chatMid").and_then(|value| value.as_str()).unwrap_or("").to_string();
    if chat.is_empty() {
        return None;
    }
    let Some(curr) = parsed.get("curr").filter(|value| !value.is_null()) else {
        return Some((chat, String::new(), String::new()));
    };
    let (emoji, image) = reaction_from_detail(curr);
    if emoji.is_empty() && image.is_empty() {
        return None;
    }
    Some((chat, emoji, image))
}

fn reaction_from_detail(curr: &Json) -> (String, String) {
    if let Some(paid) = curr.get("paidReactionType").filter(|value| !value.is_null()) {
        let product = json_text(paid, "productId");
        let emoji_id = emoji_token(paid.get("emojiId"));
        if let Some(emoji) = sticon_emoji(&product, &emoji_id) {
            return (emoji.to_string(), String::new());
        }
        if !product.is_empty() && !emoji_id.is_empty() {
            return (String::new(), sticon_url(&product, &emoji_id));
        }
    }
    let kind = curr.get("predefinedReactionType").and_then(json_int).unwrap_or(0);
    (predefined_emoji(kind).unwrap_or("").to_string(), String::new())
}

fn json_text(value: &Json, key: &str) -> String {
    match value.get(key) {
        Some(Json::String(text)) => text.clone(),
        Some(Json::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn json_int(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn emoji_token(value: Option<&Json>) -> String {
    let raw = match value {
        Some(Json::String(text)) => text.trim().to_string(),
        Some(Json::Number(number)) => number.to_string(),
        _ => return String::new(),
    };
    if raw.chars().all(|ch| ch.is_ascii_digit()) && (1..3).contains(&raw.len()) {
        return format!("{raw:0>3}");
    }
    raw
}

fn attach_reaction(item: &mut Json, message: &BTreeMap<i16, Value>, me: &str) {
    let Some(reaction) = message_reaction(message, me) else { return };
    item["reaction"] = json!(reaction.0);
    if !reaction.1.is_empty() {
        item["reactionImage"] = json!(reaction.1);
    }
    item["reactionMine"] = json!(reaction.2);
}

fn message_reaction(message: &BTreeMap<i16, Value>, me: &str) -> Option<(String, String, bool)> {
    let Value::List(rows) = message.get(&27)? else { return None };
    let mut mine = None;
    let mut latest: Option<(i64, (String, String, bool))> = None;
    for row in rows {
        let Value::Struct(fields) = row else { continue };
        let who = text_field(fields, 1);
        let when = int_field(fields, 2).unwrap_or(0);
        let Some(Value::Struct(kind)) = fields.get(&3) else { continue };
        let (emoji, image) = reaction_from_type(kind);
        if emoji.is_empty() && image.is_empty() {
            continue;
        }
        let own = !me.is_empty() && who == me;
        let shown = (emoji, image, own);
        if own {
            mine = Some(shown.clone());
        }
        let replace = match &latest {
            None => true,
            Some((at, _)) => when >= *at,
        };
        if replace {
            latest = Some((when, shown));
        }
    }
    mine.or_else(|| latest.map(|item| item.1))
}

fn reaction_from_type(kind: &BTreeMap<i16, Value>) -> (String, String) {
    if let Some(Value::Struct(paid)) = kind.get(&2) {
        let product = text_field(paid, 1);
        let emoji_id = emoji_token(Some(&Json::String(text_field(paid, 2))));
        if let Some(emoji) = sticon_emoji(&product, &emoji_id) {
            return (emoji.to_string(), String::new());
        }
        if !product.is_empty() && !emoji_id.is_empty() {
            return (String::new(), sticon_url(&product, &emoji_id));
        }
    }
    let code = int_field(kind, 1).unwrap_or(0);
    (predefined_emoji(code).unwrap_or("").to_string(), String::new())
}

fn newer_message_id(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return left.len() > right.len();
    }
    left > right
}

fn latest_message_id(messages: &[BTreeMap<i16, Value>]) -> String {
    let mut best = String::new();
    for message in messages {
        let ident = text_field(message, 4);
        if message_id_ok(&ident) && (best.is_empty() || newer_message_id(&ident, &best)) {
            best = ident;
        }
    }
    best
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

fn sent_payload(to: &str, message_id: &str, text: &str, when: i64, related_id: &str) -> Json {
    json!({
        "type": "sent",
        "ok": true,
        "id": to,
        "messageId": message_id,
        "text": text,
        "time": when,
        "replyId": if message_id_ok(related_id) { related_id } else { "" },
    })
}

struct OutgoingMedia {
    code: i32,
    obs: &'static str,
    sid: &'static str,
    kind: &'static str,
    label: String,
    name: String,
    extension: String,
    mime: String,
    animated: bool,
    duration_ms: i64,
    voice: bool,
}

fn classify_media(name: &str, mime: &str) -> OutgoingMedia {
    let mime_l = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let raw_ext = name.rsplit('.').next().unwrap_or("").trim().to_ascii_lowercase();
    let ext = if !raw_ext.is_empty() && raw_ext.len() <= 8 && raw_ext.chars().all(|ch| ch.is_ascii_alphanumeric()) && !name.eq_ignore_ascii_case(&raw_ext) {
        raw_ext
    } else {
        String::new()
    };
    let animated = mime_l == "image/gif" || ext == "gif";
    let image_ext = matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic" | "bmp");
    let video_ext = matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "webm");
    let audio_ext = matches!(ext.as_str(), "m4a" | "aac" | "mp3" | "wav");
    let (code, obs, sid, kind, fallback_ext, fallback_mime) = if animated || mime_l.starts_with("image/") || (mime_l.is_empty() && image_ext) || (!mime_l.starts_with("video/") && !mime_l.starts_with("audio/") && image_ext) {
        (1, "image", "emi", "image", "jpg", "image/jpeg")
    } else if mime_l.starts_with("video/") || (mime_l.is_empty() && video_ext) {
        (2, "video", "emv", "video", "mp4", "video/mp4")
    } else if mime_l.starts_with("audio/") || audio_ext {
        (3, "audio", "ema", "audio", "m4a", "audio/mp4")
    } else if video_ext {
        (2, "video", "emv", "video", "mp4", "video/mp4")
    } else {
        (14, "file", "emf", "file", "bin", "application/octet-stream")
    };
    let extension = if animated { "gif".to_string() } else if ext.is_empty() { fallback_ext.to_string() } else { ext };
    let file_name = media_name(name, &extension);
    let label = match kind {
        "image" => "Image".to_string(),
        "video" => "Video".to_string(),
        _ => file_name.clone(),
    };
    let mime = if mime_l.is_empty() { fallback_mime.to_string() } else { mime_l };
    OutgoingMedia { code, obs, sid, kind, label, name: file_name, extension, mime, animated, duration_ms: 0, voice: false }
}

/// A held voice note is content type 3 with `AUDLEN` in milliseconds and no file
/// name, which is how the phone draws a player instead of an attachment.
fn prepare_outgoing(mut kind: OutgoingMedia, bytes: &[u8], duration_ms: i64, voice: bool) -> OutgoingMedia {
    if kind.code != 3 {
        return kind;
    }
    if voice {
        kind.voice = true;
        kind.label = "Voice message".into();
        kind.name = "voice.m4a".into();
        kind.extension = "m4a".into();
        kind.mime = "audio/mp4".into();
    }
    let measured = if duration_ms > 0 { duration_ms } else { audio_duration_ms(bytes) };
    kind.duration_ms = measured.clamp(0, 3_600_000);
    kind
}

fn audio_duration_ms(bytes: &[u8]) -> i64 {
    wav_duration_ms(bytes).or_else(|| mp4_duration_ms(bytes)).or_else(|| mp3_duration_ms(bytes)).unwrap_or(0)
}

fn mp3_duration_ms(bytes: &[u8]) -> Option<i64> {
    let mut index = 0usize;
    if bytes.len() > 10 && &bytes[..3] == b"ID3" {
        let size = ((bytes[6] as usize & 0x7f) << 21)
            | ((bytes[7] as usize & 0x7f) << 14)
            | ((bytes[8] as usize & 0x7f) << 7)
            | (bytes[9] as usize & 0x7f);
        index = 10 + size;
    }
    if index + 4 > bytes.len() {
        return None;
    }
    let header = u32::from_be_bytes(bytes[index..index + 4].try_into().ok()?);
    if header >> 21 != 0x7ff {
        return None;
    }
    let version = (header >> 19) & 0b11;
    let layer = (header >> 17) & 0b11;
    let bitrate_index = ((header >> 12) & 0b1111) as usize;
    let sample_index = (header >> 10) & 0b11;
    if version == 1 || layer == 0 || bitrate_index == 0 || bitrate_index == 15 || sample_index == 3 {
        return None;
    }
    const MPEG1_L3: [u64; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    const MPEG2_L3: [u64; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
    let kbps = if version == 3 && layer == 1 { MPEG1_L3[bitrate_index] } else { MPEG2_L3[bitrate_index] };
    if kbps == 0 {
        return None;
    }
    let payload = bytes.len().saturating_sub(index) as u64;
    i64::try_from(payload.saturating_mul(8) / kbps).ok().filter(|ms| *ms > 0)
}

fn wav_duration_ms(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut index = 12usize;
    let mut rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data_len = 0usize;
    while index + 8 <= bytes.len() {
        let id = &bytes[index..index + 4];
        let declared = u32::from_le_bytes([bytes[index + 4], bytes[index + 5], bytes[index + 6], bytes[index + 7]]) as usize;
        let start = index + 8;
        if id == b"fmt " && declared >= 16 && start + 16 <= bytes.len() {
            channels = u16::from_le_bytes([bytes[start + 2], bytes[start + 3]]);
            rate = u32::from_le_bytes([bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7]]);
            bits = u16::from_le_bytes([bytes[start + 14], bytes[start + 15]]);
        }
        if id == b"data" {
            let available = bytes.len().saturating_sub(start);
            data_len = if declared == 0 || declared > available { available } else { declared };
            break;
        }
        let step = 8 + declared + (declared % 2);
        if step == 0 {
            break;
        }
        index = index.saturating_add(step);
    }
    let frame = usize::from(channels) * usize::from(bits / 8);
    if rate == 0 || frame == 0 || data_len < frame {
        return None;
    }
    Some((data_len / frame) as i64 * 1000 / i64::from(rate))
}

fn mp4_duration_ms(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    mp4_box_duration(bytes, 0)
}

fn mp4_box_duration(bytes: &[u8], depth: u8) -> Option<i64> {
    if depth > 6 {
        return None;
    }
    let mut index = 0usize;
    while index + 8 <= bytes.len() {
        let mut size = u32::from_be_bytes(bytes[index..index + 4].try_into().ok()?) as u64;
        let kind = &bytes[index + 4..index + 8];
        let mut header = 8usize;
        if size == 1 {
            if index + 16 > bytes.len() {
                return None;
            }
            size = u64::from_be_bytes(bytes[index + 8..index + 16].try_into().ok()?);
            header = 16;
        } else if size == 0 {
            size = (bytes.len() - index) as u64;
        }
        if size < header as u64 || index as u64 + size > bytes.len() as u64 {
            return None;
        }
        let start = index + header;
        let end = index + size as usize;
        let body = &bytes[start..end];
        if kind == b"mvhd" {
            return mvhd_duration_ms(body);
        }
        if matches!(kind, b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl") {
            if let Some(ms) = mp4_box_duration(body, depth + 1) {
                return Some(ms);
            }
        }
        index = end;
    }
    None
}

fn mvhd_duration_ms(body: &[u8]) -> Option<i64> {
    let version = *body.first()?;
    let (timescale, duration) = if version == 0 {
        if body.len() < 20 {
            return None;
        }
        let timescale = u32::from_be_bytes(body[12..16].try_into().ok()?);
        let duration = u32::from_be_bytes(body[16..20].try_into().ok()?);
        (u64::from(timescale), u64::from(duration))
    } else if version == 1 {
        if body.len() < 32 {
            return None;
        }
        let timescale = u32::from_be_bytes(body[20..24].try_into().ok()?);
        let duration = u64::from_be_bytes(body[24..32].try_into().ok()?);
        (u64::from(timescale), duration)
    } else {
        return None;
    };
    if timescale == 0 {
        return None;
    }
    i64::try_from(duration.saturating_mul(1000) / timescale).ok()
}

fn media_name(raw: &str, ext: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let name: String = base.chars().filter(|ch| !ch.is_control()).take(120).collect();
    if name.is_empty() || name == "." || name == ".." {
        format!("line.{ext}")
    } else {
        name
    }
}

fn plain_media_meta(kind: &OutgoingMedia, size: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut meta = vec![(b"FILE_SIZE".to_vec(), size.to_string().into_bytes())];
    if kind.code == 14 || (kind.code == 3 && !kind.voice) {
        meta.push((b"FILE_NAME".to_vec(), kind.name.as_bytes().to_vec()));
    }
    if kind.code == 2 {
        meta.push((b"VIDLEN".to_vec(), b"1000".to_vec()));
        meta.push((b"DURATION".to_vec(), b"1000".to_vec()));
    }
    if kind.code == 3 && kind.duration_ms > 0 {
        meta.push((b"AUDLEN".to_vec(), kind.duration_ms.to_string().into_bytes()));
    }
    if kind.code == 1 || kind.code == 2 {
        let info = serde_json::json!({
            "category": "original",
            "fileSize": size,
            "extension": kind.extension,
            "animated": kind.animated,
        });
        meta.push((b"MEDIA_CONTENT_INFO".to_vec(), info.to_string().into_bytes()));
    }
    meta
}

fn plain_obs_params(kind: &OutgoingMedia, oid: &str) -> Json {
    let mut params = json!({
        "ver": "2.0",
        "name": kind.name,
        "type": if kind.animated { "image" } else { kind.obs },
        "oid": oid,
    });
    if kind.code == 1 || kind.animated {
        params["cat"] = json!("original");
    }
    if kind.code == 2 {
        params["duration"] = json!("1000");
    }
    if kind.code == 3 && kind.duration_ms > 0 {
        params["duration"] = json!(kind.duration_ms.to_string());
    }
    params
}

fn e2ee_meta(spec: i64, content_type: i32) -> Vec<(Vec<u8>, Vec<u8>)> {
    vec![
        (b"e2eeVersion".to_vec(), spec.to_string().into_bytes()),
        (b"contentType".to_vec(), content_type.to_string().into_bytes()),
        (b"e2eeMark".to_vec(), b"2".to_vec()),
    ]
}

pub struct Album {
    pub id: String,
    pub seq: i32,
    pub count: i32,
}

fn album_ok(album: &Album) -> bool {
    (2..=10).contains(&album.count)
        && (1..=album.count).contains(&album.seq)
        && (1..=20).contains(&album.id.len())
        && album.id.chars().all(|ch| ch.is_ascii_digit())
}

fn push_album(meta: &mut Vec<(Vec<u8>, Vec<u8>)>, album: Option<&Album>) {
    let Some(album) = album.filter(|album| album_ok(album)) else { return };
    meta.push((b"GID".to_vec(), album.id.as_bytes().to_vec()));
    meta.push((b"GSEQ".to_vec(), album.seq.to_string().into_bytes()));
    meta.push((b"GTOTAL".to_vec(), album.count.to_string().into_bytes()));
}

fn media_sent(mut sent: Json, kind: &OutgoingMedia, album: Option<&Album>) -> Json {
    if let Some(object) = sent.as_object_mut() {
        let id = object.get("messageId").cloned().unwrap_or(json!(""));
        object.insert("text".to_string(), json!(kind.label));
        object.insert("kind".to_string(), json!(kind.kind));
        object.insert("mediaId".to_string(), id);
        object.insert("animated".to_string(), json!(kind.animated));
        object.insert("fileName".to_string(), json!(kind.name));
        object.insert("duration".to_string(), json!(kind.duration_ms));
        if let Some(album) = album.filter(|album| album_ok(album)) {
            object.insert("gid".to_string(), json!(album.id));
            object.insert("seq".to_string(), json!(album.seq));
        }
    }
    sent
}

fn cache_sent_media(id: &str, kind: &OutgoingMedia, bytes: &[u8]) {
    if !message_id_ok(id) {
        return;
    }
    if kind.kind == "file" || kind.kind == "audio" {
        write_media_cache(id, false, false, &kind.mime, bytes);
        return;
    }
    if kind.kind != "image" && kind.kind != "video" {
        return;
    }
    if kind.animated {
        write_media_cache(id, false, true, &kind.mime, bytes);
        return;
    }
    write_media_cache(id, true, false, &kind.mime, bytes);
    write_media_cache(id, false, false, &kind.mime, bytes);
}

fn temp_object_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("reqid-{hex}")
}

fn sticker_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 16 && id.chars().all(|ch| ch.is_ascii_digit())
}

fn sticker_meta(package_id: &str, sticker_id: &str, animated: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    // STKVER 1 is the current stickershop. Version 100 is the retired
    // dl.stickershop.line.naver.jp layout, and a phone that still honors it
    // cannot find these packs.
    let mut meta = vec![
        (b"STKVER".to_vec(), b"1".to_vec()),
        (b"STKPKGID".to_vec(), package_id.as_bytes().to_vec()),
        (b"STKID".to_vec(), sticker_id.as_bytes().to_vec()),
    ];
    if animated {
        meta.push((b"STKOPT".to_vec(), b"A".to_vec()));
    }
    meta
}

fn sticker_body(meta: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    for (key, value) in meta {
        let Ok(label) = std::str::from_utf8(key) else { continue };
        let Ok(text) = std::str::from_utf8(value) else { continue };
        map.insert(label.to_string(), Json::String(text.to_string()));
    }
    serde_json::to_vec(&Json::Object(map)).unwrap_or_default()
}

/// `notificationDisabled` is the Messaging API name for a send that arrives
fn meta_with_quiet(meta: &[(Vec<u8>, Vec<u8>)], _silent: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    // TalkService ignores a notificationDisabled metadata entry and still pushes.
    meta.to_vec()
}

fn sticker_shop_url(sticker_id: &str, animated: bool) -> String {
    let asset = if animated { "sticker_animation.png" } else { "sticker.png" };
    format!("https://stickershop.line-scdn.net/stickershop/v1/sticker/{sticker_id}/android/{asset}")
}

#[derive(Clone)]
struct SuggestLang {
    language: String,
    preload: bool,
    path: String,
    version: i64,
}

struct PackTags {
    package_id: String,
    animated: bool,
    tags: HashMap<String, Vec<(u32, i32)>>,
}

struct ResolvedSticker {
    package_id: String,
    sticker_id: u32,
    animated: bool,
    weight: i32,
}

struct SuggestCache {
    settings: Vec<SuggestLang>,
    settings_at: Option<Instant>,
    dicts: HashMap<String, BTreeMap<String, Vec<String>>>,
    packs: Vec<PackTags>,
    packs_at: Option<Instant>,
}

fn suggest_cache() -> &'static Mutex<SuggestCache> {
    static CACHE: OnceLock<Mutex<SuggestCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SuggestCache {
        settings: Vec::new(),
        settings_at: None,
        dicts: HashMap::new(),
        packs: Vec::new(),
        packs_at: None,
    }))
}

fn suggest_language_code(language: &str) -> String {
    match language.trim() {
        "zh-Hans" | "zh-CN" | "zh_CN" => "zh-CN".to_string(),
        "zh-Hant" | "zh-TW" | "zh_TW" => "zh-Hant".to_string(),
        "pt-BR" => "pt".to_string(),
        "es-419" => "es-419".to_string(),
        other => other.to_string(),
    }
}

fn suggest_key(raw: &str) -> String {
    raw.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn suggest_tag(token: &str) -> Option<&str> {
    let token = token.trim();
    let body = token.strip_prefix('c').unwrap_or(token);
    if body.is_empty() || body.len() > 16 || body.bytes().any(|byte| !byte.is_ascii_digit()) || body.bytes().all(|byte| byte == b'0') {
        return None;
    }
    Some(token)
}

fn parse_suggest_dict(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut dict = BTreeMap::new();
    for line in text.lines() {
        let Some((word, ids)) = line.split_once('\t') else { continue };
        let word = suggest_key(word);
        if word.is_empty() || word.chars().count() > 80 {
            continue;
        }
        let mut parsed = Vec::new();
        for token in ids.split(',') {
            let Some(tag) = suggest_tag(token) else { continue };
            if !parsed.iter().any(|item: &String| item == tag) {
                parsed.push(tag.to_string());
            }
            if parsed.len() >= 48 {
                break;
            }
        }
        if parsed.is_empty() {
            continue;
        }
        dict.entry(word).or_insert(parsed);
    }
    dict
}

/// `tag<tab>stickerId-weight,stickerId-weight` for one owned pack.
fn parse_pack_tags(text: &str) -> HashMap<String, Vec<(u32, i32)>> {
    let mut tags = HashMap::new();
    for line in text.lines() {
        let Some((tag, rest)) = line.split_once('\t') else { continue };
        let Some(tag) = suggest_tag(tag) else { continue };
        let mut stickers = Vec::new();
        for part in rest.split(',') {
            let Some((id, weight)) = part.trim().split_once('-') else { continue };
            let Ok(id) = id.parse::<u32>() else { continue };
            if id == 0 {
                continue;
            }
            let weight = (weight.trim().parse::<f32>().unwrap_or(0.0) * 10_000.0).round() as i32;
            stickers.push((id, weight));
            if stickers.len() >= 40 {
                break;
            }
        }
        if !stickers.is_empty() {
            tags.insert(tag.to_string(), stickers);
        }
    }
    tags
}

fn push_tag(hits: &mut Vec<String>, tag: &str) {
    if !hits.iter().any(|item| item == tag) && hits.len() < 48 {
        hits.push(tag.to_string());
    }
}

/// Exact keyword first, then the last word, then a short prefix of the typed text.
fn suggest_tags(dict: &BTreeMap<String, Vec<String>>, key: &str) -> Vec<String> {
    let mut hits = Vec::new();
    if let Some(tags) = dict.get(key) {
        for tag in tags {
            push_tag(&mut hits, tag);
        }
        return hits;
    }
    if let Some(word) = key.rsplit(' ').next() {
        if word != key && word.chars().count() >= 2 {
            if let Some(tags) = dict.get(word) {
                for tag in tags {
                    push_tag(&mut hits, tag);
                }
                return hits;
            }
        }
    }
    if key.chars().count() < 2 {
        return hits;
    }
    let mut words = 0usize;
    for (word, tags) in dict.range(key.to_string()..) {
        if !word.starts_with(key) {
            break;
        }
        words += 1;
        if words > 4 {
            break;
        }
        for tag in tags {
            push_tag(&mut hits, tag);
        }
    }
    hits
}

fn resolve_suggest_tags(tags: &[String], packs: &[PackTags]) -> Vec<ResolvedSticker> {
    let mut best: HashMap<(usize, u32), i32> = HashMap::new();
    for tag in tags {
        for (index, pack) in packs.iter().enumerate() {
            let Some(stickers) = pack.tags.get(tag) else { continue };
            for (sticker, weight) in stickers {
                let slot = best.entry((index, *sticker)).or_insert(*weight);
                if *weight > *slot {
                    *slot = *weight;
                }
            }
        }
    }
    let mut resolved: Vec<ResolvedSticker> = best.into_iter().map(|((index, sticker_id), weight)| {
        let pack = &packs[index];
        ResolvedSticker {
            package_id: pack.package_id.clone(),
            sticker_id,
            animated: pack.animated,
            weight,
        }
    }).collect();
    resolved.sort_by(|left, right| right.weight.cmp(&left.weight).then(left.sticker_id.cmp(&right.sticker_id)));
    resolved.truncate(24);
    resolved
}

fn parse_suggest_langs(value: &Value) -> Vec<SuggestLang> {
    let Ok(fields) = as_struct(value) else { return Vec::new() };
    let mut langs = Vec::new();
    for item in struct_list(fields.get(&1)) {
        let language = text_field(item, 1);
        let Some(resource) = struct_at(item, 4) else { continue };
        let path = text_field(resource, 1);
        if !suggest_path_ok(&path) || !suggest_language_ok(&language) {
            continue;
        }
        langs.push(SuggestLang {
            language,
            preload: bool_field(item, 3),
            path,
            version: int_field(resource, 2).unwrap_or(0),
        });
    }
    langs
}

fn suggest_language_ok(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 16
        && language.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn suggest_path_ok(path: &str) -> bool {
    path.starts_with("/r/shop/suggest/")
        && !path.contains("..")
        && path.len() <= 180
        && path.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.'))
}

fn load_suggest_file(lang: &SuggestLang) -> Result<String, LineError> {
    load_obs_text(&format!("{}-{}.txt", lang.language, lang.version), &lang.path)
}

fn load_obs_text(name: &str, obs_path: &str) -> Result<String, LineError> {
    if !suggest_path_ok(obs_path) || name.contains('/') || name.contains("..") {
        return Err(line("LINE did not return that sticker keyword list"));
    }
    let path = config_dir().join("suggest").join(name);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if !text.is_empty() {
            return Ok(text);
        }
    }
    let url = format!("https://obs.line-scdn.net{obs_path}");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    let response = agent.get(&url).set("user-agent", &user_agent()).call().map_err(|err| line(err.to_string()))?;
    let mut body = Vec::new();
    response.into_reader().take(8_000_000).read_to_end(&mut body).map_err(|err| line(err.to_string()))?;
    if body.is_empty() || body.len() >= 8_000_000 {
        return Err(line("LINE did not return that sticker keyword list"));
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &text);
    Ok(text)
}

fn shop_locale() -> Field {
    let raw = std::env::var("LINE_LAL").unwrap_or_else(|_| "ja_JP".into());
    let raw = raw.trim();
    let (language, country) = match raw.split_once(['_', '-']) {
        Some((language, country)) if !language.is_empty() && country.len() >= 2 => (raw.to_string(), country[..2].to_ascii_uppercase()),
        _ => ("ja_JP".to_string(), "JP".to_string()),
    };
    Field::Struct(vec![(1, bin(language)), (2, bin(country))])
}

fn sticker_animated(resource: i64) -> bool {
    matches!(resource, 2 | 4 | 5 | 6)
}

fn coin_count(raw: &str) -> Option<u64> {
    let digits: String = raw.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn amount_is_zero(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.is_empty() || trimmed.chars().all(|ch| ch == '0' || ch == '.' || ch == ',')
}

/// `free` can be ordered with no charge. `coins` spends LINE Coins.
/// `store` is an app-store price. `premium` is Stickers Premium.
/// `none` is not for sale to this account.
fn sale_kind(on_sale: bool, availability: i64, tier: i64, coins: &str, amount: &str) -> &'static str {
    if availability == 2 {
        return "premium";
    }
    if !on_sale {
        return "none";
    }
    let coins = coin_count(coins);
    if tier <= 0 || coins == Some(0) || (coins.is_none() && amount_is_zero(amount)) {
        return "free";
    }
    if coins.unwrap_or(0) > 0 {
        return "coins";
    }
    "store"
}

fn summary_packs(list: Option<&Value>) -> Vec<Json> {
    struct_list(list).into_iter().filter_map(summary_pack).collect()
}

fn summary_pack(fields: &BTreeMap<i16, Value>) -> Option<Json> {
    let id = text_field(fields, 1);
    if !sticker_id_ok(&id) {
        return None;
    }
    let sticker = struct_at(fields, 93).and_then(|summary| struct_at(summary, 1));
    let resource = sticker.and_then(|summary| int_field(summary, 5)).unwrap_or(1);
    let ids = sticker.map(|summary| ids_from_ranges(summary.get(&1))).unwrap_or_default();
    let name = text_field(fields, 11);
    Some(pack_json(&id, &name, sticker_animated(resource), &ids, true, "", "", "free"))
}

fn search_packs(list: Option<&Value>) -> Vec<Json> {
    struct_list(list).into_iter().filter_map(search_pack).collect()
}

fn search_pack(fields: &BTreeMap<i16, Value>) -> Option<Json> {
    if int_field(fields, 2).unwrap_or(1) != 1 {
        return None;
    }
    let id = text_field(fields, 1);
    if !sticker_id_ok(&id) {
        return None;
    }
    let property = struct_at(fields, 10).and_then(|property| struct_at(property, 1));
    let animated = property.map(|sticker| bool_field(sticker, 1)).unwrap_or(false)
        || property.and_then(|sticker| int_field(sticker, 4)).is_some_and(sticker_animated);
    let ids = property.map(|sticker| sticker_id_list(sticker.get(&9))).unwrap_or_default();
    let coins = text_field(fields, 9);
    let tier = int_field(fields, 8).unwrap_or(0);
    let on_sale = bool_or(fields, 12, true);
    let availability = int_field(fields, 19).unwrap_or(0);
    let sale = sale_kind(on_sale, availability, tier, &coins, "");
    let price = if sale == "coins" { coins.clone() } else { String::new() };
    Some(pack_json(&id, &text_field(fields, 3), animated, &ids, false, &price, &coins, sale))
}

fn detail_pack(fields: &BTreeMap<i16, Value>) -> Json {
    let id = text_field(fields, 1);
    let property = struct_at(fields, 95).and_then(|property| struct_at(property, 1));
    let animated = property.map(|sticker| bool_field(sticker, 1)).unwrap_or(false)
        || property.and_then(|sticker| int_field(sticker, 4)).is_some_and(sticker_animated);
    let ids = property.map(|sticker| sticker_id_list(sticker.get(&9))).unwrap_or_default();
    let owned = bool_field(fields, 31) || bool_field(fields, 32);
    let coins = text_field(fields, 63);
    let price_fields = struct_at(fields, 62);
    let amount = price_fields.map(|price| text_field(price, 2)).unwrap_or_default();
    let price_string = price_fields.map(|price| text_field(price, 3)).unwrap_or_default();
    let tier = int_field(fields, 61).unwrap_or(0);
    let sale = if owned {
        "free"
    } else {
        sale_kind(bool_or(fields, 51, true) && bool_or(fields, 54, true), 0, tier, &coins, &amount)
    };
    let price = if sale == "coins" && !price_string.is_empty() { price_string } else if sale == "coins" { coins.clone() } else { String::new() };
    pack_json(&id, &text_field(fields, 11), animated, &ids, owned, &price, &coins, sale)
}

fn order_price(fields: &BTreeMap<i16, Value>, sale: &str) -> (String, String, String) {
    let price = struct_at(fields, 62);
    let currency = price.map(|item| text_field(item, 1)).unwrap_or_default();
    let amount = price.map(|item| text_field(item, 2)).unwrap_or_default();
    let price_string = price.map(|item| text_field(item, 3)).unwrap_or_default();
    if sale == "coins" {
        let coins = text_field(fields, 63);
        let shown = if price_string.is_empty() { coins.clone() } else { price_string };
        let currency = if currency.is_empty() { "LINE".to_string() } else { currency };
        return (currency, coins, shown);
    }
    let currency = if currency.is_empty() { "JPY".to_string() } else { currency };
    let amount = if amount.is_empty() { "0".to_string() } else { amount };
    let price_string = if price_string.is_empty() { "0".to_string() } else { price_string };
    (currency, amount, price_string)
}

fn pack_json(id: &str, name: &str, animated: bool, sticker_ids: &[String], owned: bool, price: &str, coins: &str, sale: &str) -> Json {
    let name = if name.trim().is_empty() { id.to_string() } else { name.trim().to_string() };
    json!({
        "id": id,
        "name": name,
        "animated": animated,
        "stickerIds": sticker_ids,
        "owned": owned,
        "price": price,
        "coins": coins,
        "sale": sale,
    })
}

fn ids_from_ranges(list: Option<&Value>) -> Vec<String> {
    let mut ids = Vec::new();
    for range in struct_list(list) {
        let Some(start) = int_field(range, 1).filter(|value| *value > 0) else { continue };
        let size = int_field(range, 2).unwrap_or(0).clamp(0, 80);
        for offset in 0..size {
            ids.push((start + offset).to_string());
            if ids.len() >= 80 {
                return ids;
            }
        }
    }
    ids
}

fn struct_at(fields: &BTreeMap<i16, Value>, id: i16) -> Option<&BTreeMap<i16, Value>> {
    match fields.get(&id) {
        Some(Value::Struct(inner)) => Some(inner),
        _ => None,
    }
}

fn struct_list(value: Option<&Value>) -> Vec<&BTreeMap<i16, Value>> {
    let Some(Value::List(items)) = value else { return Vec::new() };
    items.iter().filter_map(|item| match item {
        Value::Struct(fields) => Some(fields),
        _ => None,
    }).collect()
}

fn sticker_id_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::List(items)) = value else { return Vec::new() };
    items.iter().filter_map(|item| match item {
        Value::Bytes(raw) if !raw.is_empty() => {
            let text = String::from_utf8_lossy(raw).into_owned();
            sticker_id_ok(&text).then_some(text)
        }
        Value::Int(value) if *value > 0 => Some(value.to_string()),
        _ => None,
    }).take(80).collect()
}

fn bool_or(fields: &BTreeMap<i16, Value>, id: i16, fallback: bool) -> bool {
    match fields.get(&id) {
        Some(Value::Bool(value)) => *value,
        _ => fallback,
    }
}

fn content_fields(to: &str, content_type: i32, meta: &[(Vec<u8>, Vec<u8>)], seq: i32, has_content: bool, silent: bool) -> Vec<(i16, Field)> {
    let mut message = vec![
        (2, bin(to)),
        (5, Field::I64(0)),
        (6, Field::I64(0)),
        (10, bin("")),
        (14, Field::Bool(has_content)),
        (15, Field::I32(content_type)),
        (18, Field::MapBin(meta_with_quiet(meta, silent))),
        (19, Field::Byte(0)),
    ];
    if let Some(kind) = mid_type(to) {
        message.push((3, Field::I32(kind)));
    }
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

fn sealed_content_fields(to: &str, content_type: i32, meta: &[(Vec<u8>, Vec<u8>)], chunks: &[Vec<u8>], seq: i32, has_content: bool, silent: bool) -> Vec<(i16, Field)> {
    let mut message = vec![
        (2, bin(to)),
        (5, Field::I64(0)),
        (6, Field::I64(0)),
        (10, bin("")),
        (14, Field::Bool(has_content)),
        (15, Field::I32(content_type)),
        (18, Field::MapBin(meta_with_quiet(meta, silent))),
        (19, Field::Byte(0)),
        (20, Field::List(thrift::BINARY, chunks.iter().cloned().map(Field::Bin).collect())),
    ];
    if let Some(kind) = mid_type(to) {
        message.push((3, Field::I32(kind)));
    }
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

fn send_fields(to: &str, text_body: &str, seq: i32, related_id: &str, silent: bool) -> Vec<(i16, Field)> {
    let mut message = vec![
        (2, bin(to)),
        (5, Field::I64(0)),
        (6, Field::I64(0)),
        (10, bin(text_body)),
        (14, Field::Bool(false)),
        (15, Field::I32(0)),
        (18, Field::MapBin(meta_with_quiet(&[], silent))),
        (19, Field::Byte(0)),
    ];
    if let Some(kind) = mid_type(to) {
        message.push((3, Field::I32(kind)));
    }
    attach_reply(&mut message, related_id);
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

fn sealed_fields(to: &str, chunks: &[Vec<u8>], spec_version: i64, seq: i32, related_id: &str, silent: bool) -> Vec<(i16, Field)> {
    let meta = meta_with_quiet(
        &[
            (b"e2eeVersion".to_vec(), spec_version.to_string().into_bytes()),
            (b"contentType".to_vec(), b"0".to_vec()),
            (b"e2eeMark".to_vec(), b"2".to_vec()),
        ],
        silent,
    );
    let mut message = vec![
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
    attach_reply(&mut message, related_id);
    vec![(1, Field::I32(seq)), (2, Field::Struct(message))]
}

/// A reply points at the original message with relation type REPLY and service TALK.
fn string_list(items: &[&str]) -> Field {
    Field::List(thrift::BINARY, items.iter().filter(|item| !item.is_empty()).map(|item| bin(*item)).collect())
}

fn report_fields(chat: &str, sender: &str, message_id: &str, reason: i32, text: &str) -> Vec<(i16, Field)> {
    let clipped: String = text.chars().take(1000).collect();
    vec![
        (2, bin(chat)),
        (3, string_list(&[])),
        (4, Field::List(thrift::I32, vec![Field::I32(reason)])),
        (5, string_list(&[sender])),
        (6, string_list(&[message_id])),
        (7, string_list(&[clipped.as_str()])),
    ]
}

fn attach_reply(message: &mut Vec<(i16, Field)>, related_id: &str) {
    if !message_id_ok(related_id) {
        return;
    }
    message.push((21, bin(related_id)));
    message.push((22, Field::I32(3)));
    message.push((24, Field::I32(1)));
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
    #[cfg(target_os = "linux")]
    if let Some(name) = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    {
        return name;
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "lineapp".into()
}

fn https(
    host: &str,
    path: &str,
    body: &[u8],
    headers: &[(&str, String)],
    timeout: Duration,
) -> Result<(u16, Vec<u8>, HashMap<String, String>), CallFail> {
    let headers = headers.iter().map(|(key, value)| ((*key).to_string(), value.clone())).collect::<Vec<_>>();
    let (status, mut payload, header_map) = match crate::h2gate::post(host, path, body, &headers, timeout) {
        Ok(response) => response,
        Err(message) => {
            let lower = message.to_lowercase();
            if lower.contains("timed") || lower.contains("timeout") {
                return Err(CallFail::TimedOut(message));
            }
            return Err(CallFail::Line(line(message)));
        }
    };
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
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
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
    if mime.starts_with("image/") || mime.starts_with("video/") || mime.starts_with("audio/") { mime } else { sniffed }
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
        let brand = &bytes[8..12];
        if matches!(brand, b"M4A " | b"M4B " | b"M4P ") {
            return "audio/mp4".into();
        }
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

struct CachedChannel {
    mid: String,
    token: String,
    at: Instant,
}

static CHANNEL_TOKEN: Mutex<Option<CachedChannel>> = Mutex::new(None);

fn cached_channel_token(mid: &str) -> Option<String> {
    let guard = CHANNEL_TOKEN.lock().ok()?;
    let hit = guard.as_ref()?;
    if hit.mid != mid || hit.at.elapsed() > Duration::from_secs(20 * 60) {
        return None;
    }
    Some(hit.token.clone())
}

fn person_card(fields: &BTreeMap<i16, Value>, own: bool) -> Json {
    if own {
        let path = text_field(fields, 22);
        let path = if path.trim().is_empty() { text_field(fields, 33) } else { path };
        let name = text_field(fields, 20).trim().to_string();
        return json!({
            "name": name,
            "given": name,
            "status": text_field(fields, 24).trim(),
            "picture": picture_url(&path),
            "friend": true,
            "blocked": false,
            "favorite": false,
        });
    }
    let given = text_field(fields, 22).trim().to_string();
    let over = text_field(fields, 27).trim().to_string();
    let name = if over.is_empty() { given.clone() } else { over };
    let path = text_field(fields, 24);
    let path = if path.trim().is_empty() { text_field(fields, 37) } else { path };
    let status_code = int_field(fields, 11).unwrap_or(0);
    let relation = int_field(fields, 21).unwrap_or(-1);
    let settings = int_field(fields, 36).unwrap_or(0);
    json!({
        "name": name,
        "given": given,
        "status": text_field(fields, 26).trim(),
        "picture": picture_url(&path),
        "friend": status_code == 1 || relation == 1,
        "blocked": matches!(status_code, 2 | 4 | 6),
        "favorite": settings & 8 != 0,
    })
}

fn cover_object_url(id: &str) -> String {
    let id = id.trim();
    if id.is_empty() || id.contains("..") || id.contains('\\') || id.len() > 180 {
        return String::new();
    }
    if let Some(rest) = id.strip_prefix("https://") {
        let host_ok = rest.starts_with("obs.line-scdn.net/")
            || rest.starts_with("profile.line-scdn.net/")
            || rest.starts_with("obs.line-apps.com/");
        return if host_ok { id.to_string() } else { String::new() };
    }
    let id = id.trim_start_matches('/');
    let user_mid = matches!(id.chars().next(), Some('u' | 'c' | 'r'))
        && (30..=40).contains(&id.len())
        && id.chars().all(|ch| ch.is_ascii_alphanumeric());
    if id.is_empty() || id.contains('/') || user_mid {
        return String::new();
    }
    format!("https://obs.line-scdn.net/myhome/h/{id}")
}

fn cover_from_home(value: &Json) -> String {
    for key in ["coverUrl", "backgroundUrl", "coverImageUrl", "profileCoverUrl"] {
        for pointer in [format!("/result/{key}"), format!("/result/cover/{key}")] {
            if let Some(url) = value.pointer(&pointer).and_then(|item| item.as_str()) {
                let url = cover_object_url(url);
                if !url.is_empty() {
                    return url;
                }
            }
        }
    }
    for pointer in ["/result/cover/objectId", "/result/objectId"] {
        if let Some(id) = value.pointer(pointer).and_then(|item| item.as_str()) {
            let url = cover_object_url(id);
            if !url.is_empty() {
                return url;
            }
        }
    }
    String::new()
}

fn checked_profile_text(value: &str, min: usize, max: usize, message: &str) -> Result<String, LineError> {
    let text = value.trim();
    let count = text.chars().count();
    if count < min || count > max || text.chars().any(|ch| ch.is_control()) {
        return Err(line(message));
    }
    Ok(text.to_string())
}

fn fresh_object_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn profile_attribute_fields(seq: i32, attrs: &[(i32, &str)]) -> Vec<(i16, Field)> {
    let pairs = attrs
        .iter()
        .map(|(id, value)| {
            (
                Field::I32(*id),
                Field::Struct(vec![
                    (1, bin(*value)),
                    (2, Field::Map(thrift::BINARY, thrift::BINARY, Vec::new())),
                ]),
            )
        })
        .collect();
    vec![
        (1, Field::I32(seq)),
        (2, Field::Struct(vec![(1, Field::Map(thrift::I32, thrift::STRUCT, pairs))])),
    ]
}

fn fitted(width: u32, height: u32, edge: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (1, 1);
    }
    if width <= edge && height <= edge {
        return (width, height);
    }
    let scale = edge as f32 / width.max(height) as f32;
    let next_width = ((width as f32) * scale).round().max(1.0) as u32;
    let next_height = ((height as f32) * scale).round().max(1.0) as u32;
    (next_width, next_height)
}

fn profile_jpeg(bytes: &[u8], cover: bool) -> Result<Vec<u8>, LineError> {
    if bytes.is_empty() || bytes.len() > 20_000_000 {
        return Err(line("that photo is too large"));
    }
    let image = image::load_from_memory(bytes).map_err(|_| line("that photo could not be read"))?;
    let image = if cover {
        let (width, height) = fitted(image.width(), image.height(), 1920);
        if width == image.width() && height == image.height() {
            image
        } else {
            image.resize(width, height, image::imageops::FilterType::Triangle)
        }
    } else {
        let side = image.width().min(image.height()).max(1);
        let x = image.width().saturating_sub(side) / 2;
        let y = image.height().saturating_sub(side) / 2;
        let square = image.crop_imm(x, y, side, side);
        if side > 640 {
            square.resize(640, 640, image::imageops::FilterType::Triangle)
        } else {
            square
        }
    };
    let rgba = image.to_rgba8();
    let mut canvas = image::RgbaImage::from_pixel(rgba.width(), rgba.height(), image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut canvas, &rgba, 0, 0);
    let rgb = image::DynamicImage::ImageRgba8(canvas).into_rgb8();
    let mut jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85);
    encoder
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|_| line("that photo could not be prepared"))?;
    Ok(jpeg)
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
    let members = group_people(&group_extra(chat).unwrap_or_default(), 4);
    if members.is_empty() { None } else { Some(members.len() as i64) }
}

fn group_mid_ok(mid: &str) -> bool {
    (2..=64).contains(&mid.len()) && mid.starts_with('c') && mid.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn user_mid_ok(mid: &str) -> bool {
    (2..=64).contains(&mid.len()) && mid.starts_with('u') && mid.chars().all(|ch| ch.is_ascii_alphanumeric())
}

struct RosterFacts {
    name: String,
    picture: String,
    auto_join: bool,
    members: Vec<(String, i64)>,
}

fn group_extra(chat: &BTreeMap<i16, Value>) -> Option<BTreeMap<i16, Value>> {
    let Value::Struct(extra) = chat.get(&8)? else { return None };
    let Value::Struct(group) = extra.get(&1)? else { return None };
    Some(group.clone())
}

fn group_people(group: &BTreeMap<i16, Value>, field: i16) -> Vec<(String, i64)> {
    let Some(Value::Map(pairs)) = group.get(&field) else { return Vec::new() };
    let mut people = Vec::new();
    for (key, value) in pairs {
        let Value::Bytes(raw) = key else { continue };
        let id = String::from_utf8_lossy(raw).trim().to_string();
        if id.is_empty() || id.len() > 64 || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            continue;
        }
        let joined = match value {
            Value::Int(number) => *number,
            _ => 0,
        };
        people.push((id, joined));
    }
    people.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    people
}

fn roster_facts(chat: &BTreeMap<i16, Value>) -> Result<RosterFacts, LineError> {
    let raw = text_field(chat, 6);
    let name = if raw.trim().is_empty() { "Unnamed group".into() } else { raw.trim().to_string() };
    let group = group_extra(chat).unwrap_or_default();
    Ok(RosterFacts {
        name,
        picture: picture_url(&text_field(chat, 7)),
        auto_join: !bool_field(&group, 2),
        members: group_people(&group, 4),
    })
}

fn person_name(card: Option<&Json>, fallback: &str, empty: &str) -> String {
    let named = card
        .and_then(|item| item.get("name"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(name) = named {
        return name.to_string();
    }
    let fallback = fallback.trim();
    if fallback.is_empty() { empty.to_string() } else { fallback.to_string() }
}

fn person_picture(card: Option<&Json>, fallback: &str) -> String {
    let picture = card
        .and_then(|item| item.get("picture"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    picture.unwrap_or(fallback).to_string()
}

fn friend_rows(ids: &[String], cards: &HashMap<String, Json>) -> Vec<Json> {
    let saved = crate::store::cached_rows();
    let mut people: Vec<Json> = ids
        .iter()
        .filter_map(|id| {
            let card = cards.get(id);
            let fallback = saved.get(id).map(|row| row.name.as_str()).unwrap_or("");
            let name = person_name(card, fallback, "");
            if name.is_empty() {
                return None;
            }
            Some(json!({
                "id": id,
                "name": name,
                "picture": person_picture(card, saved.get(id).map(|row| row.picture.as_str()).unwrap_or("")),
                "favorite": card.and_then(|item| item.get("favorite")).and_then(|value| value.as_bool()).unwrap_or(false),
            }))
        })
        .collect();
    people.sort_by(|left, right| {
        let left_favorite = left.get("favorite").and_then(|value| value.as_bool()).unwrap_or(false);
        let right_favorite = right.get("favorite").and_then(|value| value.as_bool()).unwrap_or(false);
        right_favorite.cmp(&left_favorite).then_with(|| {
            let left_name = left.get("name").and_then(|value| value.as_str()).unwrap_or("");
            let right_name = right.get("name").and_then(|value| value.as_str()).unwrap_or("");
            left_name.to_lowercase().cmp(&right_name.to_lowercase())
        })
    });
    people
}

fn friendly_search(err: LineError) -> LineError {
    let blob = format!("{} {}", err.reason, err.message).to_uppercase();
    if blob.contains("NOT_FOUND") || blob.contains("NOT FOUND") || err.code == Some(5) || err.message == "No one was found." {
        return line("No one was found.");
    }
    if err.message == "Enter a LINE ID or phone number." {
        return err;
    }
    line("Could not search for that person.")
}

fn first_group_mid(value: &Value) -> String {
    match value {
        Value::Struct(fields) => {
            let mid = text_field(fields, 2);
            if group_mid_ok(&mid) {
                return mid;
            }
            let mid = text_field(fields, 1);
            if group_mid_ok(&mid) {
                return mid;
            }
            for child in fields.values() {
                let found = first_group_mid(child);
                if !found.is_empty() {
                    return found;
                }
            }
        }
        Value::List(items) => {
            for item in items {
                let found = first_group_mid(item);
                if !found.is_empty() {
                    return found;
                }
            }
        }
        _ => {}
    }
    String::new()
}

fn first_contact(value: &Value) -> Option<BTreeMap<i16, Value>> {
    match value {
        Value::Struct(fields) => {
            if user_mid_ok(&text_field(fields, 1)) {
                return Some(fields.clone());
            }
            for child in fields.values() {
                if let Some(found) = first_contact(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::List(items) => items.iter().find_map(first_contact),
        Value::Map(pairs) => pairs.iter().find_map(|(_, child)| first_contact(child)),
        _ => None,
    }
}

fn recommendation_reason_map(value: &Value) -> HashMap<String, String> {
    let mut reasons = HashMap::new();
    harvest_recommendation(value, "", &mut reasons);
    reasons
}

fn harvest_recommendation(value: &Value, mid: &str, reasons: &mut HashMap<String, String>) {
    match value {
        Value::Struct(fields) => {
            let here = fields
                .values()
                .find_map(|child| match child {
                    Value::Bytes(raw) => {
                        let id = String::from_utf8_lossy(raw).into_owned();
                        user_mid_ok(&id).then_some(id)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| mid.to_string());
            if !here.is_empty() {
                if let Some(reason) = recommendation_union(fields) {
                    reasons.entry(here.clone()).or_insert(reason);
                }
            }
            for child in fields.values() {
                harvest_recommendation(child, &here, reasons);
            }
        }
        Value::List(items) => {
            for item in items {
                harvest_recommendation(item, mid, reasons);
            }
        }
        Value::Map(pairs) => {
            for (_, child) in pairs {
                harvest_recommendation(child, mid, reasons);
            }
        }
        _ => {}
    }
}

fn recommendation_union(fields: &BTreeMap<i16, Value>) -> Option<String> {
    if let Some(reason) = union_reason(fields) {
        return Some(reason);
    }
    for child in fields.values() {
        if let Value::Struct(inner) = child {
            if let Some(reason) = union_reason(inner) {
                return Some(reason);
            }
        }
    }
    None
}

fn union_reason(fields: &BTreeMap<i16, Value>) -> Option<String> {
    if fields.is_empty() || fields.keys().any(|id| !matches!(id, 1 | 2 | 3 | 4 | 5 | 12 | 21)) {
        return None;
    }
    let hit = [2, 3, 4, 5, 12, 21].into_iter().filter(|id| matches!(fields.get(id), Some(Value::Struct(_)))).collect::<Vec<_>>();
    if hit.len() != 1 {
        return None;
    }
    Some(match hit[0] {
        2 => "phone",
        3 => "id",
        4 => "qr",
        5 => "group",
        12 => "share",
        21 => "added",
        _ => return None,
    }
    .to_string())
}

fn collect_mids(value: Option<&Value>) -> Vec<String> {
    let mut ids = Vec::new();
    match value {
        Some(Value::List(_)) => ids.extend(id_list(value)),
        Some(Value::Struct(fields)) => {
            for field in fields.values() {
                ids.extend(id_list(Some(field)));
            }
        }
        Some(Value::Map(pairs)) => {
            for (key, _) in pairs {
                if let Value::Bytes(raw) = key {
                    ids.push(String::from_utf8_lossy(raw).into_owned());
                }
            }
        }
        _ => {}
    }
    ids.retain(|id| user_mid_ok(id));
    ids.sort();
    ids.dedup();
    ids
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

fn delivery_state(message_id: &str, me: &str, marks: &[(String, i64)]) -> (String, i64) {
    let Ok(id) = message_id.parse::<i64>() else { return (String::new(), 0) };
    if id <= 0 {
        return (String::new(), 0);
    }
    let readers = marks.iter().filter(|(who, end)| who != me && *end >= id).count() as i64;
    if readers > 0 { ("read".into(), readers) } else { ("sent".into(), 0) }
}

fn read_mark_json(marks: &[(String, i64)], me: &str) -> Json {
    Json::Array(
        marks
            .iter()
            .filter(|(who, _)| who != me)
            .map(|(_, end)| json!({ "end": end.to_string() }))
            .collect(),
    )
}

fn op_mid(op: &BTreeMap<i16, Value>) -> Option<String> {
    [10, 11, 12].into_iter().map(|id| text_field(op, id)).find(|value| looks_like_mid(value))
}

fn invalid_sender_key(err: &LineError) -> bool {
    format!("{} {}", err.reason, err.message).to_ascii_lowercase().contains("invalid sender key")
}

fn needs_seal(err: &LineError) -> bool {
    err.code == Some(82) || invalid_sender_key(err)
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

fn message_duration(message: &BTreeMap<i16, Value>) -> i64 {
    let raw = {
        let heard = meta_value(message, "AUDLEN");
        if heard.trim().is_empty() { meta_value(message, "DURATION") } else { heard }
    };
    raw.trim().parse::<i64>().ok().filter(|ms| *ms > 0).map(|ms| ms.clamp(1, 3_600_000)).unwrap_or(0)
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

fn sealed_label(text: &str) -> bool {
    text.is_empty() || text.to_ascii_lowercase().starts_with("letter-sealed")
}

fn message_unsent(message: &BTreeMap<i16, Value>) -> bool {
    meta_value(message, "UNSENT").eq_ignore_ascii_case("true")
}

fn unsent_line(mine: bool, name: &str) -> String {
    if mine {
        "You unsent a message.".into()
    } else if name.is_empty() {
        "Unsent a message.".into()
    } else {
        format!("{name} unsent a message.")
    }
}

fn finish_body(message: &BTreeMap<i16, Value>, mine: bool, name: &str, text: String, kind: &str, media_id: String) -> (String, String, String) {
    if message_unsent(message) {
        return (unsent_line(mine, name), "unsent".into(), String::new());
    }
    (text, kind.to_string(), media_id)
}

fn reply_target(message: &BTreeMap<i16, Value>) -> String {
    if int_field(message, 22) != Some(3) {
        return String::new();
    }
    let id = text_field(message, 21);
    if message_id_ok(&id) { id } else { String::new() }
}

/// Operation 64/65 carries the chat mid and the recalled message id, in either
/// param order, and sometimes on the nested message.
fn destroy_target(op: &BTreeMap<i16, Value>, me: &str) -> Option<(String, String)> {
    let mut chat = String::new();
    let mut message_id = String::new();
    if let Some(Value::Struct(message)) = op.get(&20) {
        let id = text_field(message, 4);
        if message_id_ok(&id) {
            message_id = id;
        }
        let target = chat_target(message, me);
        if target.starts_with(['u', 'c', 'r', 's']) && looks_like_mid(&target) {
            chat = target;
        }
    }
    for field in [10_i16, 11, 12] {
        let value = text_field(op, field);
        if message_id.is_empty() && message_id_ok(&value) {
            message_id = value;
            continue;
        }
        if chat.is_empty() && value.starts_with(['u', 'c', 'r', 's']) && looks_like_mid(&value) {
            chat = value;
        }
    }
    if message_id.is_empty() {
        return None;
    }
    Some((chat, message_id))
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
    if message_unsent(message) {
        return ("Unsent a message.".into(), false);
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
    if kind == "file" {
        let name = meta_value(message, "FILE_NAME").trim().to_string();
        if !name.is_empty() {
            return (name, false);
        }
    }
    if kind == "audio" {
        let name = meta_value(message, "FILE_NAME").trim().to_string();
        if !name.is_empty() && !name.eq_ignore_ascii_case("voice.m4a") {
            return (name, false);
        }
        return ("Voice message".into(), false);
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
    fn a_phone_recommendation_keeps_its_reason() {
        let mid = "u0123456789abcdef0123456789abcdef";
        let phone = Value::Struct(BTreeMap::from([(1, Value::Bytes(b"+8190".to_vec()))]));
        let union = Value::Struct(BTreeMap::from([(2, phone)]));
        let person = Value::Struct(BTreeMap::from([
            (1, Value::Bytes(mid.as_bytes().to_vec())),
            (2, union),
        ]));
        let value = Value::Struct(BTreeMap::from([(1, Value::List(vec![person]))]));
        let reasons = recommendation_reason_map(&value);
        assert_eq!(reasons.get(mid).map(String::as_str), Some("phone"));
    }

    #[test]
    fn session_file_lives_in_the_app_config_directory() {
        if std::env::var_os("LINE_CONFIG").is_some_and(|value| !value.is_empty()) {
            return;
        }
        let path = config_path();
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("lineapp.local.json"));
        assert_eq!(
            path.parent().and_then(|parent| parent.file_name()).and_then(|name| name.to_str()),
            Some(APP_ID)
        );
        assert!(!path.starts_with(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn a_saved_sender_key_survives_an_older_session_write() {
        let mut current = Session::default();
        current.access_token = "new".into();
        let mut disk = Session::default();
        disk.access_token = "old".into();
        disk.e2ee_sender_key_id = Some(9);
        disk.e2ee_private_key = "phone".into();
        disk.e2ee_keys.push(StoredKey { key_id: 9, private_key: "desk".into(), public_key: "pub".into() });
        merge_e2ee(&mut current, &disk);
        assert_eq!(current.access_token, "new");
        assert_eq!(current.e2ee_sender_key_id, Some(9));
        assert_eq!(current.e2ee_private_key, "phone");
        assert_eq!(current.e2ee_keys.len(), 1);
        current.e2ee_sender_key_id = Some(10);
        merge_e2ee(&mut current, &disk);
        assert_eq!(current.e2ee_sender_key_id, Some(10));
    }

    #[test]
    fn a_reaction_notice_keeps_the_chosen_icon() {
        let (chat, emoji, image) = reaction_notice(
            r#"{"chatMid":"uabc","curr":{"paidReactionType":{"productId":"670e0cce840a8236ddd4ee4c","emojiId":"086","resourceType":1,"version":1}}}"#,
        )
        .unwrap();
        assert_eq!(chat, "uabc");
        assert_eq!(emoji, "😎");
        assert!(image.is_empty());
        let (chat, emoji, image) = reaction_notice(r#"{"chatMid":"uabc","curr":null}"#).unwrap();
        assert_eq!(chat, "uabc");
        assert!(emoji.is_empty());
        assert!(image.is_empty());
        let (_, emoji, _) = reaction_notice(r#"{"chatMid":"uabc","curr":{"predefinedReactionType":"3"}}"#).unwrap();
        assert_eq!(emoji, "❤️");
        let mut message = BTreeMap::new();
        message.insert(
            27,
            Value::List(vec![Value::Struct(BTreeMap::from([
                (1, Value::Bytes(b"uother".to_vec())),
                (2, Value::Int(20)),
                (3, Value::Struct(BTreeMap::from([(1, Value::Int(7))]))),
            ]))]),
        );
        assert_eq!(message_reaction(&message, "ume"), Some(("😡".into(), String::new(), false)));
    }

    #[test]
    fn reply_and_unsend_match_the_desktop_calls() {
        let frame = thrift::write_call("sendMessage", &send_fields("uabc", "Xo", 4, "1500", false));
        let parsed = thrift::read_message(&frame).unwrap();
        let Value::Struct(message) = parsed.fields.get(&2).unwrap() else { panic!("message") };
        assert_eq!(text_field(message, 10), "Xo");
        assert_eq!(text_field(message, 21), "1500");
        assert_eq!(int_field(message, 22), Some(3));
        assert_eq!(int_field(message, 24), Some(1));
        let plain = thrift::write_call("sendMessage", &send_fields("uabc", "Xo", 4, "", false));
        let plain = thrift::read_message(&plain).unwrap();
        let Value::Struct(message) = plain.fields.get(&2).unwrap() else { panic!("message") };
        assert!(message.get(&21).is_none());
        let unsend = thrift::write_call("unsendMessage", &[(1, Field::I32(9)), (2, bin("1500"))]);
        let unsend = thrift::read_message(&unsend).unwrap();
        assert_eq!(unsend.name, "unsendMessage");
        assert_eq!(unsend.fields.get(&1), Some(&Value::Int(9)));
        let photo = classify_media("cat.PNG", "image/png");
        assert_eq!(photo.code, 1);
        assert_eq!(photo.extension, "png");
        assert!(!photo.animated);
        let gif = classify_media("loop.gif", "image/gif");
        assert!(gif.animated);
        assert_eq!(gif.sid, "emi");
        assert_eq!(classify_media("voice.m4a", "audio/mp4").code, 3);
        assert_eq!(classify_media("notes.pdf", "application/pdf").code, 14);
        let mut voice = classify_media("voice.m4a", "audio/mp4");
        voice.voice = true;
        voice.duration_ms = 3200;
        let voice_meta = plain_media_meta(&voice, 80);
        assert_eq!(voice_meta.iter().find(|(key, _)| key == b"AUDLEN").map(|(_, value)| value.as_slice()), Some(b"3200".as_slice()));
        assert!(voice_meta.iter().all(|(key, _)| key != b"FILE_NAME"));
        assert_eq!(plain_obs_params(&voice, "9")["duration"], "3200");
        let song = prepare_outgoing(classify_media("song.mp3", "audio/mpeg"), &[], 0, false);
        assert!(!song.voice);
        assert_eq!(song.duration_ms, 0);
        let note = prepare_outgoing(classify_media("voice.m4a", "audio/mp4"), &[], 1800, true);
        assert!(note.voice);
        assert_eq!(note.label, "Voice message");
        assert_eq!(note.duration_ms, 1800);
        let wav = {
            let mut bytes = b"RIFF".to_vec();
            bytes.extend_from_slice(&36u32.to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16u32.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&22_050u32.to_le_bytes());
            bytes.extend_from_slice(&44_100u32.to_le_bytes());
            bytes.extend_from_slice(&2u16.to_le_bytes());
            bytes.extend_from_slice(&16u16.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&(22_050u32 * 2).to_le_bytes());
            bytes.extend(std::iter::repeat(0u8).take(22_050 * 2));
            let size = (bytes.len() - 8) as u32;
            bytes[4..8].copy_from_slice(&size.to_le_bytes());
            bytes
        };
        assert_eq!(audio_duration_ms(&wav), 1000);
        let mut moov = Vec::new();
        let mut mvhd = vec![0u8, 0, 0, 0];
        mvhd.extend_from_slice(&[0; 8]);
        mvhd.extend_from_slice(&1000u32.to_be_bytes());
        mvhd.extend_from_slice(&2500u32.to_be_bytes());
        let mut mvhd_box = (8 + mvhd.len() as u32).to_be_bytes().to_vec();
        mvhd_box.extend_from_slice(b"mvhd");
        mvhd_box.extend(mvhd);
        let mut moov_box = (8 + mvhd_box.len() as u32).to_be_bytes().to_vec();
        moov_box.extend_from_slice(b"moov");
        moov_box.extend(mvhd_box);
        moov.extend_from_slice(&32u32.to_be_bytes());
        moov.extend_from_slice(b"ftyp");
        moov.extend_from_slice(b"M4A ");
        moov.extend(std::iter::repeat(0u8).take(20));
        moov.extend(moov_box);
        assert_eq!(audio_duration_ms(&moov), 2500);
        assert_eq!(sniff_media(&moov), "audio/mp4");
        let mut heard = BTreeMap::new();
        heard.insert(18, Value::Map(vec![(Value::Bytes(b"AUDLEN".to_vec()), Value::Bytes(b"1800".to_vec()))]));
        assert_eq!(message_duration(&heard), 1800);
        let frame = thrift::write_call("sendMessage", &content_fields("uabc", photo.code, &plain_media_meta(&photo, 12), 4, false, false));
        let quiet = thrift::write_call("sendMessage", &send_fields("uabc", "Xo", 4, "", true));
        let quiet = thrift::read_message(&quiet).unwrap();
        let Value::Struct(message) = quiet.fields.get(&2).unwrap() else { panic!("message") };
        assert_eq!(meta_value(message, "notificationDisabled"), "");
        let mut grouped = plain_media_meta(&photo, 12);
        push_album(&mut grouped, Some(&Album { id: "1700000000000".into(), seq: 2, count: 3 }));
        assert_eq!(grouped.iter().find(|(key, _)| key == b"GID").map(|(_, value)| value.as_slice()), Some(b"1700000000000".as_slice()));
        assert_eq!(grouped.iter().find(|(key, _)| key == b"GSEQ").map(|(_, value)| value.as_slice()), Some(b"2".as_slice()));
        assert_eq!(grouped.iter().find(|(key, _)| key == b"GTOTAL").map(|(_, value)| value.as_slice()), Some(b"3".as_slice()));
        assert!(!album_ok(&Album { id: "1700000000000".into(), seq: 4, count: 3 }));
        let sticker = sticker_meta("11537", "52002734", true);
        assert_eq!(String::from_utf8(sticker.iter().find(|(key, _)| key == b"STKVER").unwrap().1.clone()).unwrap(), "1");
        let sealed = sticker_body(&sticker);
        let parsed_body: Json = serde_json::from_slice(&sealed).unwrap();
        assert_eq!(parsed_body["STKOPT"], "A");
        assert!(parsed_body.get("STKTXT").is_none());
        let parsed = thrift::read_message(&frame).unwrap();
        let Value::Struct(message) = parsed.fields.get(&2).unwrap() else { panic!("message") };
        assert_eq!(int_field(message, 15), Some(1));
        assert_eq!(meta_value(message, "FILE_SIZE"), "12");
        assert_eq!(unsend.fields.get(&2), Some(&Value::Bytes(b"1500".to_vec())));
        let mut op = BTreeMap::new();
        op.insert(10, Value::Bytes(b"uabcdefghij".to_vec()));
        op.insert(11, Value::Bytes(b"1500".to_vec()));
        assert_eq!(destroy_target(&op, "ume"), Some(("uabcdefghij".into(), "1500".into())));
        let mut swapped = BTreeMap::new();
        swapped.insert(10, Value::Bytes(b"1500".to_vec()));
        swapped.insert(11, Value::Bytes(b"cgroupgroup".to_vec()));
        assert_eq!(destroy_target(&swapped, "ume"), Some(("cgroupgroup".into(), "1500".into())));
        let report = thrift::write_call("reportSpam", &report_fields("uabcdefghij", "uotherothero", "1500", 1, "Hi"));
        let report = thrift::read_message(&report).unwrap();
        assert_eq!(report.name, "reportSpam");
        assert_eq!(report.fields.get(&2), Some(&Value::Bytes(b"uabcdefghij".to_vec())));
        assert_eq!(report.fields.get(&4), Some(&Value::List(vec![Value::Int(1)])));
        assert_eq!(report.fields.get(&5), Some(&Value::List(vec![Value::Bytes(b"uotherothero".to_vec())])));
        assert_eq!(report.fields.get(&6), Some(&Value::List(vec![Value::Bytes(b"1500".to_vec())])));
        assert_eq!(report.fields.get(&7), Some(&Value::List(vec![Value::Bytes(b"Hi".to_vec())])));
        assert_eq!(unsent_line(true, ""), "You unsent a message.");
        assert_eq!(unsent_line(false, "Ada"), "Ada unsent a message.");
        assert_eq!(reply_target(&BTreeMap::from([
            (21, Value::Bytes(b"1500".to_vec())),
            (22, Value::Int(3)),
        ])), "1500");
        assert!(reply_target(&BTreeMap::from([(21, Value::Bytes(b"1500".to_vec())), (22, Value::Int(0))])).is_empty());
    }

    #[test]
    fn keyword_dictionary_picks_an_exact_sticker_then_a_prefix() {
        let dict = parse_suggest_dict("thanks\t3325,c27,c27\nthank you\t1452\nok\t2289\n");
        assert_eq!(suggest_key("  OK "), "ok");
        assert_eq!(suggest_tags(&dict, "thanks"), vec!["3325".to_string(), "c27".to_string()]);
        assert_eq!(suggest_tags(&dict, &suggest_key("  OK ")), vec!["2289".to_string()]);
        assert_eq!(suggest_tags(&dict, "hey thanks"), vec!["3325".to_string(), "c27".to_string()]);
        assert_eq!(suggest_tags(&dict, "thank"), vec!["1452".to_string(), "3325".to_string(), "c27".to_string()]);
        assert!(suggest_tags(&dict, "z").is_empty());
        let pack = PackTags {
            package_id: "11537".to_string(),
            animated: true,
            tags: parse_pack_tags("c27\t52002739-0.2199\n3325\t52002739-0.1000\nc43\t52002734-0.0766\n"),
        };
        let found = resolve_suggest_tags(&["3325".into(), "c27".into(), "c43".into()], &[pack]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].sticker_id, 52002739);
        assert_eq!(found[0].package_id, "11537");
        assert_eq!(found[0].weight, 2199);
        assert!(found[0].animated);
        assert_eq!(found[1].sticker_id, 52002734);
    }

    #[test]
    fn chat_checked_matches_the_desktop_call() {
        let frame = thrift::write_call("sendChatChecked", &chat_checked_fields(7, "uabc", "1400"));
        let parsed = thrift::read_message(&frame).unwrap();
        assert_eq!(parsed.name, "sendChatChecked");
        assert_eq!(parsed.fields.get(&1), Some(&Value::Int(7)));
        assert_eq!(parsed.fields.get(&2), Some(&Value::Bytes(b"uabc".to_vec())));
        assert_eq!(parsed.fields.get(&3), Some(&Value::Bytes(b"1400".to_vec())));
        assert_eq!(parsed.fields.get(&4), Some(&Value::Byte(0)));
        assert!(newer_message_id("100", "99"));
        assert!(!newer_message_id("99", "100"));
        assert_eq!(latest_message_id(&[
            BTreeMap::from([(4, Value::Bytes(b"99".to_vec()))]),
            BTreeMap::from([(4, Value::Bytes(b"100".to_vec()))]),
        ]), "100");
    }

    #[test]
    fn profile_card_reads_the_name_status_and_photo() {
        let fields = BTreeMap::from([
            (11, Value::Int(1)),
            (21, Value::Int(1)),
            (22, Value::Bytes(b"Methini Mint Minto".to_vec())),
            (24, Value::Bytes(b"0habc".to_vec())),
            (26, Value::Bytes(b"Let it be.".to_vec())),
        ]);
        let card = person_card(&fields, false);
        assert_eq!(card["name"], "Methini Mint Minto");
        assert_eq!(card["status"], "Let it be.");
        assert_eq!(card["picture"], "https://profile.line-scdn.net/0habc");
        assert_eq!(card["friend"], true);
        assert_eq!(card["blocked"], false);
        assert_eq!(card["favorite"], false);
        let named = BTreeMap::from([
            (11, Value::Int(1)),
            (22, Value::Bytes(b"Supansa".to_vec())),
            (27, Value::Bytes(b"Sun".to_vec())),
            (36, Value::Int(8)),
        ]);
        let named = person_card(&named, false);
        assert_eq!(named["name"], "Sun");
        assert_eq!(named["given"], "Supansa");
        assert_eq!(named["favorite"], true);
        let blocked = BTreeMap::from([(11, Value::Int(2)), (22, Value::Bytes(b"A".to_vec()))]);
        assert_eq!(person_card(&blocked, false)["blocked"], true);
        let cover = cover_from_home(&json!({"result": {"objectId": "cover1", "homeId": "u0123456789abcdef0123456789abcdef"}}));
        assert_eq!(cover, "https://obs.line-scdn.net/myhome/h/cover1");
        let linked = cover_from_home(&json!({"result": {"coverUrl": "https://obs.line-scdn.net/myhome/h/direct"}}));
        assert_eq!(linked, "https://obs.line-scdn.net/myhome/h/direct");
        assert_eq!(cover_from_home(&json!({"result": {"objectId": "../x"}})), "");
        assert_eq!(cover_from_home(&json!({"result": {"coverUrl": "https://evil.example/a"}})), "");
    }

    #[test]
    fn profile_photo_becomes_a_square_jpeg() {
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 2, image::Rgb([10, 20, 30])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let jpeg = profile_jpeg(&png, false).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]));
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(decoded.width(), decoded.height());
        assert!(checked_profile_text("  Ada  ", 1, 20, "name").is_ok());
        assert!(checked_profile_text("", 1, 20, "name").is_err());
        assert!(checked_profile_text(&"a".repeat(21), 1, 20, "name").is_err());
        let fields = profile_attribute_fields(3, &[(ATTR_NAME, "Ada"), (ATTR_STATUS, "here")]);
        let frame = thrift::write_call("updateProfileAttributes", &fields);
        let message = thrift::read_message(&frame).unwrap();
        assert_eq!(message.fields.get(&1), Some(&Value::Int(3)));
    }

    #[test]
    fn delivery_state_follows_the_other_reader() {
        let marks = vec![("ume".to_string(), 10), ("ufriend".to_string(), 20)];
        assert_eq!(delivery_state("15", "ume", &marks), ("read".into(), 1));
        assert_eq!(delivery_state("21", "ume", &marks), ("sent".into(), 0));
        assert_eq!(delivery_state("5", "ume", &marks), ("read".into(), 1));
    }

    #[test]
    fn invalid_sender_key_is_recognized() {
        let err = line_code("sendMessage failed: invalid sender key", Some(5), "invalid sender key");
        assert!(invalid_sender_key(&err));
        assert!(!invalid_sender_key(&line("empty send response")));
    }

    #[test]
    fn letter_sealed_preview_is_fetched_again() {
        assert!(needs_fresh_preview("Letter-sealed message", 10, 10));
        assert!(!needs_fresh_preview("hello", 10, 10));
        assert!(needs_fresh_preview("hello", 11, 10));
    }

    #[test]
    fn owned_summary_lists_sticker_ids_and_the_shop_row_keeps_its_price() {
        let summary = Value::Struct(BTreeMap::from([
            (1, Value::Bytes(b"11537".to_vec())),
            (11, Value::Bytes(b"Moon".to_vec())),
            (93, Value::Struct(BTreeMap::from([
                (1, Value::Struct(BTreeMap::from([
                    (1, Value::List(vec![Value::Struct(BTreeMap::from([
                        (1, Value::Int(52002734)),
                        (2, Value::Int(2)),
                    ]))])),
                    (5, Value::Int(2)),
                ]))),
            ]))),
        ]));
        let pack = summary_pack(as_struct(&summary).unwrap()).unwrap();
        assert_eq!(pack["owned"], true);
        assert_eq!(pack["animated"], true);
        assert_eq!(pack["stickerIds"], json!(["52002734", "52002735"]));

        let search = Value::Struct(BTreeMap::from([
            (1, Value::Bytes(b"200".to_vec())),
            (2, Value::Int(1)),
            (3, Value::Bytes(b"Shop".to_vec())),
            (8, Value::Int(2)),
            (9, Value::Bytes(b"50".to_vec())),
            (10, Value::Struct(BTreeMap::from([
                (1, Value::Struct(BTreeMap::from([
                    (1, Value::Bool(false)),
                    (9, Value::List(vec![Value::Bytes(b"9".to_vec())])),
                ]))),
            ]))),
            (12, Value::Bool(true)),
            (19, Value::Int(0)),
        ]));
        let shop = search_pack(as_struct(&search).unwrap()).unwrap();
        assert_eq!(shop["sale"], "coins");
        assert_eq!(shop["price"], "50");
        assert_eq!(shop["owned"], false);

        let premium = BTreeMap::from([
            (1, Value::Bytes(b"9".to_vec())),
            (11, Value::Bytes(b"Premium".to_vec())),
            (51, Value::Bool(true)),
            (54, Value::Bool(true)),
            (61, Value::Int(0)),
            (102, Value::Struct(BTreeMap::from([(2, Value::Int(2))]))),
        ]);
        assert_eq!(detail_pack(&premium)["sale"], "free");
        assert_eq!(sale_kind(true, 2, 0, "0", "0"), "premium");
        assert_eq!(sale_kind(true, 0, 3, "", "120"), "store");
        assert_eq!(sale_kind(false, 0, 0, "0", "0"), "none");
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

    #[test]
    fn group_roster_reads_members_in_join_order_and_auto_join() {
        let mut group = BTreeMap::new();
        group.insert(2, Value::Bool(true));
        group.insert(
            4,
            Value::Map(vec![
                (Value::Bytes(b"uaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec()), Value::Int(20)),
                (Value::Bytes(b"ubbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_vec()), Value::Int(10)),
            ]),
        );
        let mut extra = BTreeMap::new();
        extra.insert(1, Value::Struct(group));
        let mut chat = BTreeMap::new();
        chat.insert(6, Value::Bytes(b"Weekend".to_vec()));
        chat.insert(7, Value::Bytes(b"/p".to_vec()));
        chat.insert(8, Value::Struct(extra));
        let facts = roster_facts(&chat).unwrap();
        assert_eq!(facts.name, "Weekend");
        assert_eq!(facts.picture, "https://profile.line-scdn.net/p");
        assert!(!facts.auto_join);
        assert_eq!(
            facts.members,
            vec![
                ("ubbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(), 10),
                ("uaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(), 20),
            ]
        );
        assert_eq!(member_count(&chat), Some(2));

        let mut bare = BTreeMap::new();
        bare.insert(6, Value::Bytes(b"  Plain  ".to_vec()));
        let plain = roster_facts(&bare).unwrap();
        assert_eq!(plain.name, "Plain");
        assert!(plain.auto_join);
        assert!(plain.members.is_empty());
        assert_eq!(member_count(&bare), None);
    }

    #[test]
    fn your_contact_uses_the_contact_display_name() {
        let mut fields = BTreeMap::new();
        fields.insert(22, Value::Bytes(b"Kenneth".to_vec()));
        fields.insert(24, Value::Bytes(b"/picture".to_vec()));
        let card = person_card(&fields, false);
        assert_eq!(card.get("name").and_then(|value| value.as_str()), Some("Kenneth"));
        assert!(card.get("picture").and_then(|value| value.as_str()).unwrap_or("").contains("/picture"));
    }

}
