//! Local chat list. The window paints this before Talk answers.

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
         );",
    )
    .map_err(|err| err.to_string())?;
    let _ = fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    Ok(conn)
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
    conn.execute_batch("DELETE FROM chats; DELETE FROM profile; DELETE FROM meta;").map_err(|err| err.to_string())
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
            let Some(id) = update.get("id").and_then(|value| value.as_str()) else { continue };
            if id.is_empty() {
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
                    } else if (refresh || time > chat.time) && (refresh || !text.is_empty() || time > chat.time) {
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
        }
    }
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    tx.execute("DELETE FROM chats", []).map_err(|err| err.to_string())?;
    for (id, chat) in &rows {
        insert_chat(&tx, id, chat)?;
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
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
