//! Local chats and messages. The window paints these before Talk answers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as Json};

use crate::line;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct StoredChat {
    pub name: String,
    pub last_message: String,
    pub sealed: bool,
    pub time: i64,
    pub members: Option<i64>,
    pub picture: String,
    pub muted: bool,
    pub kind: String,
    pub unread: Option<i64>,
    pub sticker: String,
    pub sticker_fallback: String,
}

pub fn db_path() -> PathBuf {
    line::config_path().with_file_name("lineapp.db")
}

pub(crate) fn with_db<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let conn = connect(&db_path())?;
    f(&conn)
}

pub fn clear() {
    let path = db_path();
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    remove_files(&path);
}

pub fn cached_chats() -> Option<Json> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    load_locked(&db_path(), &account_mid()).ok().flatten()
}

pub fn cached_rows() -> HashMap<String, StoredChat> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    rows_locked(&db_path(), &account_mid()).unwrap_or_default()
}

pub fn save_chats(payload: &Json) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    save_locked(&db_path(), &account_mid(), payload)
}

pub fn apply_poll(payload: &Json) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    apply_locked(&db_path(), &account_mid(), payload)
}

pub fn cached_history(chat: &str) -> Option<Json> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    history_locked(&db_path(), &account_mid(), chat).ok().flatten()
}

pub fn save_history(payload: &Json) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    save_history_locked(&db_path(), &account_mid(), payload)
}

pub fn shown_history(chat: &str, fetched: &Json) -> Result<Json, String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    shown_history_locked(&db_path(), &account_mid(), chat, fetched)
}

pub fn older_page(chat: &str, before_id: &str) -> Result<Json, String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    older_page_locked(&db_path(), &account_mid(), chat, before_id)
}

pub fn merge_messages(chat: &str, payload: &Json) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    merge_messages_locked(&db_path(), &account_mid(), chat, payload)
}

pub fn oldest_server_cursor(chat: &str) -> Result<Option<(i64, String)>, String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let path = db_path();
    if !path.is_file() {
        return Ok(None);
    }
    let conn = connect(&path)?;
    if !claim(&conn, &account_mid())? {
        return Ok(None);
    }
    oldest_server_cursor_locked(&conn, chat)
}

pub fn novel_count(chat: &str, payload: &Json) -> Result<usize, String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let path = db_path();
    if !path.is_file() {
        let messages = payload.get("messages").and_then(|value| value.as_array()).map(|items| items.len()).unwrap_or(0);
        return Ok(messages);
    }
    let conn = connect(&path)?;
    if !claim(&conn, &account_mid())? {
        return Ok(0);
    }
    novel_count_locked(&conn, chat, payload)
}

pub fn save_outgoing(chat: &str, sent: &Json) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    save_outgoing_locked(&db_path(), &account_mid(), chat, sent)
}

pub fn message_known(message_id: &str) -> bool {
    if message_id.is_empty() || message_id.len() > 24 || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let Ok(conn) = connect(&db_path()) else {
        return false;
    };
    conn.query_row("SELECT 1 FROM messages WHERE msg_id = ?1", params![message_id], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

pub fn mark_unsent(message_id: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let conn = connect(&db_path())?;
    if !claim(&conn, &account_mid())? {
        return Ok(());
    }
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    mark_unsent_tx(&tx, message_id)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn account_mid() -> String {
    let Ok(text) = fs::read_to_string(line::config_path()) else {
        return String::new();
    };
    serde_json::from_str::<Json>(&text)
        .ok()
        .and_then(|value| value.get("mid").and_then(|mid| mid.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn remove_files(path: &Path) {
    let _ = fs::remove_file(path);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("lineapp.db");
    if let Some(parent) = path.parent() {
        let _ = fs::remove_file(parent.join(format!("{name}-wal")));
        let _ = fs::remove_file(parent.join(format!("{name}-shm")));
    }
}

fn connect(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let conn = Connection::open(path).map_err(|err| err.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(3)).map_err(|err| err.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS profile (id INTEGER PRIMARY KEY CHECK (id = 1), name TEXT NOT NULL, picture TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS chats (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            last_message TEXT NOT NULL,
            sealed INTEGER NOT NULL,
            time INTEGER NOT NULL,
            members INTEGER,
            picture TEXT NOT NULL,
            muted INTEGER NOT NULL,
            kind TEXT NOT NULL,
            unread INTEGER,
            sticker TEXT NOT NULL,
            sticker_fallback TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS messages (
            chat_id TEXT NOT NULL,
            msg_id TEXT NOT NULL,
            mine INTEGER NOT NULL,
            name TEXT NOT NULL,
            picture TEXT NOT NULL,
            body TEXT NOT NULL,
            sealed INTEGER NOT NULL,
            time INTEGER NOT NULL,
            status TEXT NOT NULL,
            readers INTEGER NOT NULL,
            sticker TEXT NOT NULL,
            sticker_fallback TEXT NOT NULL,
            kind TEXT NOT NULL,
            gid TEXT NOT NULL,
            seq INTEGER NOT NULL,
            media_id TEXT NOT NULL,
            animated INTEGER NOT NULL,
            sender TEXT NOT NULL DEFAULT '',
            reply_id TEXT NOT NULL DEFAULT '',
            duration INTEGER NOT NULL DEFAULT 0,
            reaction TEXT NOT NULL DEFAULT '',
            reaction_image TEXT NOT NULL DEFAULT '',
            reaction_mine INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (chat_id, msg_id)
         );
         CREATE INDEX IF NOT EXISTS messages_chat_time ON messages (chat_id, time, msg_id);
         CREATE TABLE IF NOT EXISTS llm_settings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            saved_at INTEGER NOT NULL,
            chat_id TEXT NOT NULL,
            chat_name TEXT NOT NULL,
            chat_kind TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            model TEXT NOT NULL,
            api_key TEXT NOT NULL,
            protocol TEXT NOT NULL,
            temperature TEXT NOT NULL,
            top_p TEXT NOT NULL,
            max_tokens TEXT NOT NULL,
            thinking TEXT NOT NULL,
            extra TEXT NOT NULL,
            context TEXT NOT NULL,
            language TEXT NOT NULL,
            mark_ai INTEGER NOT NULL,
            interrupt INTEGER NOT NULL,
            history INTEGER NOT NULL DEFAULT 20
         );",
    )
    .map_err(|err| err.to_string())?;
    ensure_sender_column(&conn)?;
    ensure_reply_column(&conn)?;
    ensure_message_columns(&conn)?;
    ensure_llm_history_column(&conn)?;
    let _ = fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    Ok(conn)
}

fn ensure_sender_column(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA table_info(messages)").map_err(|err| err.to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    if names.iter().any(|name| name == "sender") {
        return Ok(());
    }
    conn.execute("ALTER TABLE messages ADD COLUMN sender TEXT NOT NULL DEFAULT ''", [])
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn ensure_llm_history_column(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA table_info(llm_settings)").map_err(|err| err.to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    if names.iter().any(|name| name == "history") {
        return Ok(());
    }
    conn.execute("ALTER TABLE llm_settings ADD COLUMN history INTEGER NOT NULL DEFAULT 20", [])
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn ensure_message_columns(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA table_info(messages)").map_err(|err| err.to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    drop(stmt);
    let missing = [
        ("duration", "ALTER TABLE messages ADD COLUMN duration INTEGER NOT NULL DEFAULT 0"),
        ("reaction", "ALTER TABLE messages ADD COLUMN reaction TEXT NOT NULL DEFAULT ''"),
        ("reaction_image", "ALTER TABLE messages ADD COLUMN reaction_image TEXT NOT NULL DEFAULT ''"),
        ("reaction_mine", "ALTER TABLE messages ADD COLUMN reaction_mine INTEGER NOT NULL DEFAULT 0"),
    ];
    for (name, sql) in missing {
        if names.iter().any(|item| item == name) {
            continue;
        }
        conn.execute(sql, []).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn ensure_reply_column(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn.prepare("PRAGMA table_info(messages)").map_err(|err| err.to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    if names.iter().any(|name| name == "reply_id") {
        return Ok(());
    }
    conn.execute("ALTER TABLE messages ADD COLUMN reply_id TEXT NOT NULL DEFAULT ''", [])
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn stored_mid(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = 'mid'", [], |row| row.get(0))
        .optional()
        .map_err(|err| err.to_string())
}

fn set_mid(conn: &Connection, mid: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('mid', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![mid],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn wipe(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("DELETE FROM messages; DELETE FROM chats; DELETE FROM profile; DELETE FROM meta; DELETE FROM llm_settings;")
        .map_err(|err| err.to_string())
}

pub(crate) struct LlmSettingsSave {
    pub chat_id: String,
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub protocol: String,
    pub temperature: String,
    pub top_p: String,
    pub max_tokens: String,
    pub thinking: String,
    pub extra: String,
    pub context: String,
    pub language: String,
    pub mark_ai: bool,
    pub interrupt: bool,
    pub history: u32,
}

pub(crate) fn append_llm_settings(saved: &LlmSettingsSave) -> Result<(), String> {
    with_db(|conn| {
        let (name, raw_kind) = conn
            .query_row("SELECT name, kind FROM chats WHERE id = ?1", params![saved.chat_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .optional()
            .map_err(|err| err.to_string())?
            .unwrap_or_default();
        let kind = match raw_kind.as_str() {
            "friend" => "person",
            "group" | "room" => "group",
            other => other,
        };
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO llm_settings (
                saved_at, chat_id, chat_name, chat_kind,
                endpoint, model, api_key, protocol,
                temperature, top_p, max_tokens, thinking,
                extra, context, language, mark_ai, interrupt, history
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                saved_at,
                saved.chat_id,
                name,
                kind,
                saved.endpoint,
                saved.model,
                saved.api_key,
                saved.protocol,
                saved.temperature,
                saved.top_p,
                saved.max_tokens,
                saved.thinking,
                saved.extra,
                saved.context,
                saved.language,
                saved.mark_ai,
                saved.interrupt,
                saved.history,
            ],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    })
}

fn claim(conn: &Connection, mid: &str) -> Result<bool, String> {
    match stored_mid(conn)? {
        Some(stored) if stored != mid => {
            wipe(conn)?;
            set_mid(conn, mid)?;
            Ok(false)
        }
        _ => {
            set_mid(conn, mid)?;
            Ok(true)
        }
    }
}

fn load_locked(path: &Path, mid: &str) -> Result<Option<Json>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(None);
    }
    let rows = read_chats(&conn)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut chats = Vec::new();
    for (id, chat) in rows {
        chats.push(chat_json(&id, &chat));
    }
    chats.sort_by(|left, right| {
        right.get("time").and_then(|value| value.as_i64()).unwrap_or(0).cmp(&left.get("time").and_then(|value| value.as_i64()).unwrap_or(0))
    });
    let profile = conn
        .query_row("SELECT name, picture FROM profile WHERE id = 1", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .optional()
        .map_err(|err| err.to_string())?
        .map(|(name, picture)| json!({"name": name, "picture": picture}))
        .unwrap_or_else(|| json!({"name": "", "picture": ""}));
    let mut total = 0i64;
    let mut any = false;
    for chat in &chats {
        if let Some(count) = chat.get("unread").and_then(|value| value.as_i64()) {
            any = true;
            if count > 0 {
                total += count;
            }
        }
    }
    Ok(Some(json!({
        "type": "chats",
        "profile": profile,
        "chats": chats,
        "failures": [],
        "unreadTotal": if any { json!(total) } else { Json::Null },
    })))
}

fn rows_locked(path: &Path, mid: &str) -> Result<HashMap<String, StoredChat>, String> {
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(HashMap::new());
    }
    read_chats(&conn)
}

fn read_chats(conn: &Connection) -> Result<HashMap<String, StoredChat>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, last_message, sealed, time, members, picture, muted, kind, unread, sticker, sticker_fallback FROM chats",
        )
        .map_err(|err| err.to_string())?;
    let mapped = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                StoredChat {
                    name: row.get(1)?,
                    last_message: {
                        let text: String = row.get(2)?;
                        if line::placeholder_preview(&text) { String::new() } else { text }
                    },
                    sealed: row.get::<_, i64>(3)? != 0,
                    time: row.get(4)?,
                    members: row.get(5)?,
                    picture: row.get(6)?,
                    muted: row.get::<_, i64>(7)? != 0,
                    kind: row.get(8)?,
                    unread: row.get(9)?,
                    sticker: row.get(10)?,
                    sticker_fallback: row.get(11)?,
                },
            ))
        })
        .map_err(|err| err.to_string())?;
    let mut rows = HashMap::new();
    for item in mapped {
        let (id, chat) = item.map_err(|err| err.to_string())?;
        rows.insert(id, chat);
    }
    Ok(rows)
}

fn save_locked(path: &Path, mid: &str, payload: &Json) -> Result<(), String> {
    let Some(chats) = payload.get("chats").and_then(|value| value.as_array()) else {
        return Ok(());
    };
    let conn = connect(path)?;
    let _ = claim(&conn, mid)?;
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    tx.execute("DELETE FROM chats", []).map_err(|err| err.to_string())?;
    for chat in chats {
        let Some(id) = chat.get("id").and_then(|value| value.as_str()) else { continue };
        if id.is_empty() {
            continue;
        }
        insert_chat(&tx, id, &stored_from_json(chat))?;
    }
    let name = payload.get("profile").and_then(|value| value.get("name")).and_then(|value| value.as_str()).unwrap_or("");
    let picture = payload.get("profile").and_then(|value| value.get("picture")).and_then(|value| value.as_str()).unwrap_or("");
    tx.execute("DELETE FROM profile", []).map_err(|err| err.to_string())?;
    tx.execute("INSERT INTO profile (id, name, picture) VALUES (1, ?1, ?2)", params![name, picture]).map_err(|err| err.to_string())?;
    set_mid(&tx, mid)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn apply_locked(path: &Path, mid: &str, payload: &Json) -> Result<(), String> {
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(());
    }
    let mut rows = read_chats(&conn)?;
    let mut threads = Vec::new();
    let mut unsents = Vec::new();
    if let Some(unreads) = payload.get("unreads").and_then(|value| value.as_array()) {
        for item in unreads {
            let Some(id) = item.get("id").and_then(|value| value.as_str()) else { continue };
            let Some(count) = item.get("unread").and_then(|value| value.as_i64()) else { continue };
            if let Some(chat) = rows.get_mut(id) {
                chat.unread = Some(count);
            }
        }
    }
    if let Some(updates) = payload.get("updates").and_then(|value| value.as_array()) {
        for update in updates {
            if let Some(message_id) = update.get("unsentId").and_then(|value| value.as_str()).filter(|id| !id.is_empty()) {
                unsents.push(message_id.to_string());
                continue;
            }
            let Some(id) = update.get("id").and_then(|value| value.as_str()) else { continue };
            if id.is_empty() {
                continue;
            }
            if update.get("reaction").is_some() {
                continue;
            }
            let read = update.get("read").and_then(|value| value.as_bool()).unwrap_or(false);
            if read {
                if let Some(chat) = rows.get_mut(id) {
                    chat.unread = Some(0);
                }
                continue;
            }
            let time = update.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
            let refresh = update.get("refresh").and_then(|value| value.as_bool()).unwrap_or(false);
            let raw = update.get("lastMessage").and_then(|value| value.as_str()).unwrap_or("");
            let dropped = line::placeholder_preview(raw);
            let text = if dropped { String::new() } else { raw.to_string() };
            let keep_preview = update.get("keepPreview").and_then(|value| value.as_bool()).unwrap_or(false);
            match rows.get_mut(id) {
                Some(chat) => {
                    if dropped || keep_preview {
                        if line::placeholder_preview(&chat.last_message) {
                            chat.last_message.clear();
                            chat.sticker.clear();
                            chat.sticker_fallback.clear();
                        }
                    } else if refresh || time > chat.time || (!text.is_empty() && text != chat.last_message) {
                        chat.last_message = text;
                        chat.sealed = update.get("sealed").and_then(|value| value.as_bool()).unwrap_or(false);
                        chat.sticker = update.get("sticker").and_then(|value| value.as_str()).unwrap_or("").to_string();
                        chat.sticker_fallback = update.get("stickerFallback").and_then(|value| value.as_str()).unwrap_or("").to_string();
                    }
                    if time > chat.time {
                        chat.time = time;
                    }
                    if let Some(unread) = update.get("unread").and_then(|value| value.as_i64()) {
                        chat.unread = Some(unread);
                    }
                    if let Some(name) = update.get("name").and_then(|value| value.as_str()) {
                        if !name.is_empty() && (chat.name.is_empty() || chat.name == "Direct chat") {
                            chat.name = name.to_string();
                        }
                    }
                    if let Some(picture) = update.get("picture").and_then(|value| value.as_str()) {
                        if chat.picture.is_empty() {
                            chat.picture = picture.to_string();
                        }
                    }
                }
                None => {
                    let mut chat = stored_from_json(update);
                    chat.last_message = text;
                    chat.time = time;
                    if chat.name.is_empty() {
                        chat.name = "Direct chat".into();
                    }
                    if chat.kind.is_empty() {
                        chat.kind = kind_for(id).into();
                    }
                    rows.insert(id.to_string(), chat);
                }
            }
            if let Some(message) = update.get("thread") {
                if message.get("text").and_then(|value| value.as_str()).is_some_and(|text| !text.is_empty()) {
                    threads.push((id.to_string(), message.clone()));
                }
            }
        }
    }
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    tx.execute("DELETE FROM chats", []).map_err(|err| err.to_string())?;
    for (id, chat) in &rows {
        insert_chat(&tx, id, chat)?;
    }
    for (chat, message) in &threads {
        let key = message_key(message, 0);
        if !key.is_empty() {
            upsert_message(&tx, chat, &key, message)?;
        }
    }
    for message_id in &unsents {
        mark_unsent_tx(&tx, message_id)?;
    }
    set_mid(&tx, mid)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn insert_chat(conn: &Connection, id: &str, chat: &StoredChat) -> Result<(), String> {
    conn.execute(
        "INSERT INTO chats (id, name, last_message, sealed, time, members, picture, muted, kind, unread, sticker, sticker_fallback)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            chat.name,
            chat.last_message,
            if chat.sealed { 1 } else { 0 },
            chat.time,
            chat.members,
            chat.picture,
            if chat.muted { 1 } else { 0 },
            chat.kind,
            chat.unread,
            chat.sticker,
            chat.sticker_fallback,
        ],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn stored_from_json(chat: &Json) -> StoredChat {
    StoredChat {
        name: chat.get("name").and_then(|value| value.as_str()).unwrap_or("").to_string(),
        last_message: chat.get("lastMessage").and_then(|value| value.as_str()).unwrap_or("").to_string(),
        sealed: chat.get("sealed").and_then(|value| value.as_bool()).unwrap_or(false),
        time: chat.get("time").and_then(|value| value.as_i64()).unwrap_or(0),
        members: chat.get("members").and_then(|value| value.as_i64()),
        picture: chat.get("picture").and_then(|value| value.as_str()).unwrap_or("").to_string(),
        muted: chat.get("muted").and_then(|value| value.as_bool()).unwrap_or(false),
        kind: chat.get("kind").and_then(|value| value.as_str()).unwrap_or("").to_string(),
        unread: chat.get("unread").and_then(|value| value.as_i64()),
        sticker: chat.get("sticker").and_then(|value| value.as_str()).unwrap_or("").to_string(),
        sticker_fallback: chat.get("stickerFallback").and_then(|value| value.as_str()).unwrap_or("").to_string(),
    }
}

fn chat_json(id: &str, chat: &StoredChat) -> Json {
    json!({
        "id": id,
        "name": chat.name,
        "lastMessage": chat.last_message,
        "sealed": chat.sealed,
        "time": chat.time,
        "members": chat.members,
        "picture": chat.picture,
        "muted": chat.muted,
        "kind": chat.kind,
        "unread": chat.unread,
        "sticker": chat.sticker,
        "stickerFallback": chat.sticker_fallback,
    })
}

const HISTORY_SHOW: usize = 40;
const MESSAGE_COLUMNS: &str = "msg_id, mine, name, picture, body, sealed, time, status, readers, sticker, sticker_fallback, kind, gid, seq, media_id, animated, sender, reply_id, duration, reaction, reaction_image, reaction_mine";

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Json> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "mine": row.get::<_, i64>(1)? != 0,
        "name": row.get::<_, String>(2)?,
        "picture": row.get::<_, String>(3)?,
        "text": row.get::<_, String>(4)?,
        "sealed": row.get::<_, i64>(5)? != 0,
        "time": row.get::<_, i64>(6)?,
        "status": row.get::<_, String>(7)?,
        "readers": row.get::<_, i64>(8)?,
        "sticker": row.get::<_, String>(9)?,
        "stickerFallback": row.get::<_, String>(10)?,
        "kind": row.get::<_, String>(11)?,
        "gid": row.get::<_, String>(12)?,
        "seq": row.get::<_, i64>(13)?,
        "mediaId": row.get::<_, String>(14)?,
        "animated": row.get::<_, i64>(15)? != 0,
        "sender": row.get::<_, String>(16)?,
        "replyId": row.get::<_, String>(17)?,
        "duration": row.get::<_, i64>(18)?,
        "reaction": row.get::<_, String>(19)?,
        "reactionImage": row.get::<_, String>(20)?,
        "reactionMine": row.get::<_, i64>(21)? != 0,
    }))
}

fn history_locked(path: &Path, mid: &str, chat: &str) -> Result<Option<Json>, String> {
    if chat.is_empty() || !path.is_file() {
        return Ok(None);
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(None);
    }
    let (messages, older) = messages_latest(&conn, chat, HISTORY_SHOW)?;
    if messages.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({"type": "history", "id": chat, "messages": messages, "older": older})))
}

fn shown_history_locked(path: &Path, mid: &str, chat: &str, fetched: &Json) -> Result<Json, String> {
    let server_older = fetched.get("older").and_then(|value| value.as_bool()).unwrap_or(false);
    let mut messages = fetched.get("messages").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    if messages.len() > HISTORY_SHOW {
        messages = messages.split_off(messages.len() - HISTORY_SHOW);
    }
    let mut older = server_older;
    if !older {
        if let Some(cursor) = messages.first() {
            let time = cursor.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
            let id = cursor.get("id").and_then(|value| value.as_str()).unwrap_or("");
            if !id.is_empty() && path.is_file() {
                let conn = connect(path)?;
                if claim(&conn, mid)? {
                    older = exists_before(&conn, chat, time, id)?;
                }
            }
        }
    }
    let mut page = fetched.clone();
    page["messages"] = Json::Array(messages);
    page["older"] = json!(older);
    Ok(page)
}

fn older_page_locked(path: &Path, mid: &str, chat: &str, before_id: &str) -> Result<Json, String> {
    if chat.is_empty() || before_id.is_empty() || !path.is_file() {
        return Ok(json!({"messages": [], "more": false}));
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(json!({"messages": [], "more": false}));
    }
    let cursor: Option<(i64, String)> = conn
        .query_row(
            "SELECT time, msg_id FROM messages WHERE chat_id = ?1 AND msg_id = ?2",
            params![chat, before_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let Some((time, id)) = cursor else {
        return Err("that message is not saved".to_string());
    };
    let (messages, more) = messages_before(&conn, chat, time, &id, HISTORY_SHOW)?;
    Ok(json!({"messages": messages, "more": more}))
}

fn merge_messages_locked(path: &Path, mid: &str, chat: &str, payload: &Json) -> Result<(), String> {
    let Some(messages) = payload.get("messages").and_then(|value| value.as_array()) else {
        return Ok(());
    };
    if chat.is_empty() || messages.is_empty() {
        return Ok(());
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(());
    }
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    for (index, message) in messages.iter().enumerate() {
        let key = message_key(message, index);
        upsert_message(&tx, chat, &key, message)?;
    }
    set_mid(&tx, mid)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn oldest_server_cursor_locked(conn: &Connection, chat: &str) -> Result<Option<(i64, String)>, String> {
    conn.query_row(
        "SELECT time, msg_id FROM messages
         WHERE chat_id = ?1 AND length(msg_id) BETWEEN 1 AND 24 AND msg_id GLOB '[0-9]*' AND msg_id NOT GLOB '*[^0-9]*'
         ORDER BY time ASC, msg_id ASC LIMIT 1",
        params![chat],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|err| err.to_string())
}

fn novel_count_locked(conn: &Connection, chat: &str, payload: &Json) -> Result<usize, String> {
    let Some(messages) = payload.get("messages").and_then(|value| value.as_array()) else {
        return Ok(0);
    };
    let mut count = 0;
    for message in messages {
        let id = message.get("id").and_then(|value| value.as_str()).unwrap_or("").trim();
        if id.is_empty() {
            continue;
        }
        let known = conn
            .query_row("SELECT 1 FROM messages WHERE chat_id = ?1 AND msg_id = ?2", params![chat, id], |_| Ok(()))
            .optional()
            .map_err(|err| err.to_string())?;
        if known.is_none() {
            count += 1;
        }
    }
    Ok(count)
}

fn messages_latest(conn: &Connection, chat: &str, limit: usize) -> Result<(Vec<Json>, bool), String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE chat_id = ?1 ORDER BY time DESC, msg_id DESC LIMIT ?2"))
        .map_err(|err| err.to_string())?;
    let mut messages = read_messages(&mut stmt, params![chat, (limit + 1) as i64])?;
    let older = messages.len() > limit;
    if older {
        messages.pop();
    }
    messages.reverse();
    Ok((messages, older))
}

fn messages_before(conn: &Connection, chat: &str, time: i64, id: &str, limit: usize) -> Result<(Vec<Json>, bool), String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE chat_id = ?1 AND (time < ?2 OR (time = ?2 AND msg_id < ?3))
             ORDER BY time DESC, msg_id DESC LIMIT ?4"
        ))
        .map_err(|err| err.to_string())?;
    let mut messages = read_messages(&mut stmt, params![chat, time, id, (limit + 1) as i64])?;
    let more = messages.len() > limit;
    if more {
        messages.pop();
    }
    messages.reverse();
    Ok((messages, more))
}

fn exists_before(conn: &Connection, chat: &str, time: i64, id: &str) -> Result<bool, String> {
    let found = conn
        .query_row(
            "SELECT 1 FROM messages WHERE chat_id = ?1 AND (time < ?2 OR (time = ?2 AND msg_id < ?3)) LIMIT 1",
            params![chat, time, id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    Ok(found.is_some())
}

fn read_messages(stmt: &mut rusqlite::Statement<'_>, params: impl rusqlite::Params) -> Result<Vec<Json>, String> {
    let mapped = stmt.query_map(params, map_message).map_err(|err| err.to_string())?;
    let mut messages = Vec::new();
    for item in mapped {
        messages.push(item.map_err(|err| err.to_string())?);
    }
    Ok(messages)
}

fn save_history_locked(path: &Path, mid: &str, payload: &Json) -> Result<(), String> {
    let chat = payload.get("id").and_then(|value| value.as_str()).unwrap_or("");
    let Some(messages) = payload.get("messages").and_then(|value| value.as_array()) else {
        return Ok(());
    };
    if chat.is_empty() {
        return Ok(());
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(());
    }
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    let mut keys = Vec::new();
    let mut oldest = i64::MAX;
    for (index, message) in messages.iter().enumerate() {
        let time = message.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
        oldest = oldest.min(time);
        let key = message_key(message, index);
        upsert_message(&tx, chat, &key, message)?;
        keys.push(key);
    }
    if !keys.is_empty() {
        let keep: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
        let mut stmt = tx
            .prepare("SELECT msg_id, time FROM messages WHERE chat_id = ?1 AND time >= ?2")
            .map_err(|err| err.to_string())?;
        let stored: Vec<(String, i64)> = stmt
            .query_map(params![chat, oldest], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|err| err.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        drop(stmt);
        for (key, time) in stored {
            if time >= oldest && !keep.contains(key.as_str()) {
                tx.execute("DELETE FROM messages WHERE chat_id = ?1 AND msg_id = ?2", params![chat, key])
                    .map_err(|err| err.to_string())?;
            }
        }
    }
    set_mid(&tx, mid)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn save_outgoing_locked(path: &Path, mid: &str, chat: &str, sent: &Json) -> Result<(), String> {
    let key = sent.get("messageId").and_then(|value| value.as_str()).unwrap_or("").trim();
    let text = sent.get("text").and_then(|value| value.as_str()).unwrap_or("");
    if chat.is_empty() || key.is_empty() || text.is_empty() || sent.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        return Ok(());
    }
    let conn = connect(path)?;
    if !claim(&conn, mid)? {
        return Ok(());
    }
    let message = json!({
        "id": key,
        "mine": true,
        "name": "",
        "picture": "",
        "text": text,
        "sealed": false,
        "time": sent.get("time").and_then(|value| value.as_i64()).unwrap_or(0),
        "status": "sent",
        "readers": 0,
        "sticker": sent.get("sticker").and_then(|value| value.as_str()).unwrap_or(""),
        "stickerFallback": sent.get("stickerFallback").and_then(|value| value.as_str()).unwrap_or(""),
        "kind": sent.get("kind").and_then(|value| value.as_str()).unwrap_or(""),
        "gid": sent.get("gid").and_then(|value| value.as_str()).unwrap_or(""),
        "seq": sent.get("seq").and_then(|value| value.as_i64()).unwrap_or(0),
        "mediaId": sent.get("mediaId").and_then(|value| value.as_str()).unwrap_or(""),
        "animated": sent.get("animated").and_then(|value| value.as_bool()).unwrap_or(false),
        "replyId": sent.get("replyId").and_then(|value| value.as_str()).unwrap_or(""),
    });
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    upsert_message(&tx, chat, key, &message)?;
    set_mid(&tx, mid)?;
    tx.commit().map_err(|err| err.to_string())?;
    Ok(())
}

fn message_key(message: &Json, index: usize) -> String {
    let id = message.get("id").and_then(|value| value.as_str()).unwrap_or("").trim();
    if !id.is_empty() {
        return id.to_string();
    }
    let time = message.get("time").and_then(|value| value.as_i64()).unwrap_or(0);
    format!("local-{time}-{index}")
}

fn upsert_message(conn: &Connection, chat: &str, key: &str, message: &Json) -> Result<(), String> {
    let text = message.get("text").and_then(|value| value.as_str()).unwrap_or("");
    conn.execute(
        "INSERT INTO messages (
            chat_id, msg_id, mine, name, picture, body, sealed, time, status, readers, sticker, sticker_fallback, kind, gid, seq, media_id, animated, sender, reply_id, duration, reaction, reaction_image, reaction_mine
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
         ON CONFLICT(chat_id, msg_id) DO UPDATE SET
            mine = excluded.mine,
            sender = CASE WHEN excluded.sender != '' THEN excluded.sender ELSE messages.sender END,
            name = CASE WHEN excluded.name != '' THEN excluded.name ELSE messages.name END,
            picture = CASE WHEN excluded.picture != '' THEN excluded.picture ELSE messages.picture END,
            body = excluded.body,
            sealed = excluded.sealed,
            time = excluded.time,
            status = CASE WHEN excluded.status != '' THEN excluded.status ELSE messages.status END,
            readers = CASE WHEN excluded.status != '' THEN excluded.readers ELSE messages.readers END,
            sticker = excluded.sticker,
            sticker_fallback = excluded.sticker_fallback,
            kind = CASE WHEN excluded.kind != '' THEN excluded.kind ELSE messages.kind END,
            gid = CASE WHEN excluded.gid != '' THEN excluded.gid ELSE messages.gid END,
            seq = CASE WHEN excluded.seq != 0 THEN excluded.seq ELSE messages.seq END,
            media_id = CASE WHEN excluded.media_id != '' THEN excluded.media_id ELSE messages.media_id END,
            animated = excluded.animated,
            reply_id = CASE WHEN excluded.reply_id != '' THEN excluded.reply_id ELSE messages.reply_id END,
            duration = excluded.duration,
            reaction = excluded.reaction,
            reaction_image = excluded.reaction_image,
            reaction_mine = excluded.reaction_mine",
        params![
            chat,
            key,
            if message.get("mine").and_then(|value| value.as_bool()).unwrap_or(false) { 1 } else { 0 },
            message.get("name").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("picture").and_then(|value| value.as_str()).unwrap_or(""),
            text,
            if message.get("sealed").and_then(|value| value.as_bool()).unwrap_or(false) { 1 } else { 0 },
            message.get("time").and_then(|value| value.as_i64()).unwrap_or(0),
            message.get("status").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("readers").and_then(|value| value.as_i64()).unwrap_or(0),
            message.get("sticker").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("stickerFallback").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("kind").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("gid").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("seq").and_then(|value| value.as_i64()).unwrap_or(0),
            message.get("mediaId").and_then(|value| value.as_str()).unwrap_or(""),
            if message.get("animated").and_then(|value| value.as_bool()).unwrap_or(false) { 1 } else { 0 },
            message.get("sender").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("replyId").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("duration").and_then(|value| value.as_i64()).unwrap_or(0),
            message.get("reaction").and_then(|value| value.as_str()).unwrap_or(""),
            message.get("reactionImage").and_then(|value| value.as_str()).unwrap_or(""),
            if message.get("reactionMine").and_then(|value| value.as_bool()).unwrap_or(false) { 1 } else { 0 },
        ],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

fn mark_unsent_tx(conn: &Connection, message_id: &str) -> Result<(), String> {
    if message_id.is_empty() || message_id.len() > 24 || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(());
    }
    let found = conn
        .query_row(
            "SELECT chat_id, mine, name, body FROM messages WHERE msg_id = ?1",
            params![message_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let Some((chat, mine, name, body)) = found else {
        return Ok(());
    };
    let sentence = if mine != 0 {
        "You unsent a message.".to_string()
    } else if name.is_empty() {
        "Unsent a message.".to_string()
    } else {
        format!("{name} unsent a message.")
    };
    conn.execute(
        "UPDATE messages SET body = ?1, kind = 'unsent', sticker = '', sticker_fallback = '', media_id = '', gid = '', seq = 0, animated = 0 WHERE msg_id = ?2",
        params![sentence, message_id],
    )
    .map_err(|err| err.to_string())?;
    let latest: Option<String> = conn
        .query_row("SELECT msg_id FROM messages WHERE chat_id = ?1 ORDER BY time DESC LIMIT 1", params![chat], |row| row.get(0))
        .optional()
        .map_err(|err| err.to_string())?;
    let preview: Option<String> = conn
        .query_row("SELECT last_message FROM chats WHERE id = ?1", params![chat], |row| row.get(0))
        .optional()
        .map_err(|err| err.to_string())?;
    if latest.as_deref() == Some(message_id) || preview.as_deref() == Some(body.as_str()) {
        conn.execute(
            "UPDATE chats SET last_message = ?1, sticker = '', sticker_fallback = '', sealed = 0 WHERE id = ?2",
            params![sentence, chat],
        )
        .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn kind_for(id: &str) -> &'static str {
    if id.starts_with('u') {
        "friend"
    } else if id.starts_with('c') {
        "group"
    } else if id.starts_with('r') {
        "room"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!("lineapp-store-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path.join("lineapp.db")
    }

    #[test]
    fn round_trip_keeps_the_list() {
        let path = scratch();
        let payload = json!({
            "type": "chats",
            "profile": {"name": "Ken", "picture": "https://example/p"},
            "chats": [
                {"id": "c1", "name": "Group", "lastMessage": "hi", "sealed": false, "time": 20, "members": 3, "picture": "", "muted": true, "kind": "group", "unread": 2, "sticker": "", "stickerFallback": ""},
                {"id": "u1", "name": "Ada", "lastMessage": "", "sealed": true, "time": 50, "members": null, "picture": "https://example/a", "muted": false, "kind": "friend", "unread": null, "sticker": "https://example/s", "stickerFallback": "https://example/f"}
            ]
        });
        save_locked(&path, "uMe", &payload).unwrap();
        let loaded = load_locked(&path, "uMe").unwrap().unwrap();
        assert_eq!(loaded["profile"]["name"], "Ken");
        assert_eq!(loaded["chats"][0]["id"], "u1");
        assert_eq!(loaded["chats"][0]["sticker"], "https://example/s");
        assert!(loaded["chats"][0]["unread"].is_null());
        assert_eq!(loaded["chats"][1]["unread"], 2);
        assert_eq!(loaded["chats"][1]["members"], 3);
        assert_eq!(loaded["unreadTotal"], 2);
        assert!(load_locked(&path, "uOther").unwrap().is_none());
        assert!(load_locked(&path, "uMe").unwrap().is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn poll_updates_a_saved_preview() {
        let path = scratch();
        save_locked(&path, "uMe", &json!({
            "profile": {"name": "", "picture": ""},
            "chats": [{"id": "u1", "name": "Ada", "lastMessage": "old", "sealed": false, "time": 10, "members": null, "picture": "", "muted": false, "kind": "friend", "unread": 0, "sticker": "", "stickerFallback": ""}]
        })).unwrap();
        apply_locked(&path, "uMe", &json!({
            "unreads": [{"id": "u1", "unread": 1}],
            "updates": [{"id": "u1", "lastMessage": "new", "time": 10, "sealed": false, "refresh": true, "sticker": "", "stickerFallback": ""}]
        })).unwrap();
        let rows = rows_locked(&path, "uMe").unwrap();
        let chat = rows.get("u1").unwrap();
        assert_eq!(chat.last_message, "new");
        assert_eq!(chat.time, 10);
        assert_eq!(chat.unread, Some(1));
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u2", "name": "Bea", "lastMessage": "hey", "time": 30, "sealed": false, "kind": "friend"}]
        })).unwrap();
        let rows = rows_locked(&path, "uMe").unwrap();
        assert_eq!(rows.get("u2").unwrap().name, "Bea");
        assert_eq!(rows.get("u2").unwrap().last_message, "hey");
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u1", "lastMessage": "Ken invited Ada to the group.", "time": 40, "sealed": false, "keepPreview": true}]
        })).unwrap();
        let rows = rows_locked(&path, "uMe").unwrap();
        assert_eq!(rows.get("u1").unwrap().last_message, "new");
        assert_eq!(rows.get("u1").unwrap().time, 40);
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u1", "lastMessage": "Chat update", "time": 10, "sealed": false, "refresh": true}]
        })).unwrap();
        let rows = rows_locked(&path, "uMe").unwrap();
        assert_eq!(rows.get("u1").unwrap().last_message, "new");
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u9", "lastMessage": "", "time": 0, "sealed": false, "keepPreview": true, "reaction": {"messageId": "12", "emoji": "👍", "mine": false}}]
        })).unwrap();
        let rows = rows_locked(&path, "uMe").unwrap();
        assert!(rows.get("u9").is_none());
        assert_eq!(rows.get("u1").unwrap().last_message, "new");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn history_paints_from_disk_and_keeps_a_new_message() {
        let path = scratch();
        save_history_locked(&path, "uMe", &json!({
            "id": "u1",
            "messages": [
                {"id": "1", "mine": false, "sender": "uAda", "name": "Ada", "picture": "", "text": "one", "sealed": false, "time": 10, "status": "", "readers": 0, "sticker": "", "stickerFallback": "", "kind": "", "gid": "", "seq": 0, "mediaId": "", "animated": false},
                {"id": "2", "mine": true, "name": "", "picture": "", "text": "two", "sealed": false, "time": 20, "status": "read", "readers": 1, "sticker": "", "stickerFallback": "", "kind": "", "gid": "", "seq": 0, "mediaId": "", "animated": false}
            ]
        })).unwrap();
        let loaded = history_locked(&path, "uMe", "u1").unwrap().unwrap();
        assert_eq!(loaded["messages"][0]["text"], "one");
        assert_eq!(loaded["messages"][0]["sender"], "uAda");
        assert_eq!(loaded["messages"][1]["status"], "read");
        apply_locked(&path, "uMe", &json!({
            "updates": [{
                "id": "u1",
                "lastMessage": "three",
                "time": 30,
                "thread": {"id": "3", "mine": false, "name": "Ada", "text": "three", "sealed": false, "time": 30, "kind": ""}
            }]
        })).unwrap();
        let loaded = history_locked(&path, "uMe", "u1").unwrap().unwrap();
        assert_eq!(loaded["messages"].as_array().unwrap().len(), 3);
        assert_eq!(loaded["messages"][2]["text"], "three");
        save_history_locked(&path, "uMe", &json!({
            "id": "u1",
            "messages": [
                {"id": "1", "mine": false, "name": "Ada", "text": "one", "time": 10, "status": ""},
                {"id": "3", "mine": false, "name": "Ada", "text": "three", "time": 30, "status": ""}
            ]
        })).unwrap();
        let loaded = history_locked(&path, "uMe", "u1").unwrap().unwrap();
        let texts: Vec<&str> = loaded["messages"].as_array().unwrap().iter().filter_map(|item| item.get("text").and_then(|value| value.as_str())).collect();
        assert_eq!(texts, ["one", "three"]);
        assert_eq!(loaded["messages"][0]["sender"], "uAda");
        save_history_locked(&path, "uMe", &json!({
            "id": "u1",
            "messages": [
                {"id": "1", "mine": false, "name": "Ada", "text": "one", "time": 10, "replyId": "9"},
                {"id": "2", "mine": true, "name": "", "text": "two", "time": 20}
            ]
        })).unwrap();
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u1", "lastMessage": "two", "time": 20, "sealed": false}]
        })).unwrap();
        apply_locked(&path, "uMe", &json!({
            "updates": [{"id": "u1", "unsentId": "2", "lastMessage": "", "time": 0, "sealed": false, "keepPreview": true}]
        })).unwrap();
        let loaded = history_locked(&path, "uMe", "u1").unwrap().unwrap();
        assert_eq!(loaded["messages"][0]["replyId"], "9");
        assert_eq!(loaded["messages"][1]["kind"], "unsent");
        assert_eq!(loaded["messages"][1]["text"], "You unsent a message.");
        let rows = rows_locked(&path, "uMe").unwrap();
        assert_eq!(rows.get("u1").unwrap().last_message, "You unsent a message.");
        assert!(history_locked(&path, "uOther", "u1").unwrap().is_none());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn history_keeps_every_message_and_pages_backward() {
        let path = scratch();
        let messages: Vec<Json> = (1..=100)
            .map(|index| json!({"id": index.to_string(), "mine": false, "name": "Ada", "text": index.to_string(), "time": index, "duration": if index == 100 { 1500 } else { 0 }, "reaction": if index == 100 { "👍" } else { "" }, "reactionMine": index == 100}))
            .collect();
        save_history_locked(&path, "uMe", &json!({"id": "u1", "messages": messages})).unwrap();
        let loaded = history_locked(&path, "uMe", "u1").unwrap().unwrap();
        let shown = loaded["messages"].as_array().unwrap();
        assert_eq!(shown.len(), 40);
        assert_eq!(shown[0]["text"], "61");
        assert_eq!(shown[39]["text"], "100");
        assert_eq!(shown[39]["duration"], 1500);
        assert_eq!(shown[39]["reaction"], "👍");
        assert_eq!(shown[39]["reactionMine"], true);
        assert_eq!(loaded["older"], true);
        let page = older_page_locked(&path, "uMe", "u1", "61").unwrap();
        let older = page["messages"].as_array().unwrap();
        assert_eq!(older.len(), 40);
        assert_eq!(older[0]["text"], "21");
        assert_eq!(older[39]["text"], "60");
        assert_eq!(page["more"], true);
        let rest = older_page_locked(&path, "uMe", "u1", "21").unwrap();
        let earliest = rest["messages"].as_array().unwrap();
        assert_eq!(earliest[0]["text"], "1");
        assert_eq!(earliest.len(), 20);
        assert_eq!(rest["more"], false);
        save_history_locked(&path, "uMe", &json!({
            "id": "u1",
            "messages": (61..=100).map(|index| json!({"id": index.to_string(), "mine": false, "name": "Ada", "text": index.to_string(), "time": index})).collect::<Vec<_>>()
        })).unwrap();
        let kept = older_page_locked(&path, "uMe", "u1", "61").unwrap();
        assert_eq!(kept["messages"].as_array().unwrap()[0]["text"], "21");
        let conn = connect(&path).unwrap();
        let cursor = oldest_server_cursor_locked(&conn, "u1").unwrap().unwrap();
        assert_eq!(cursor, (1, "1".to_string()));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
