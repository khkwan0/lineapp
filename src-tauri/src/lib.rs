mod agent;
mod e2ee;
mod h2gate;
mod legy;
mod line;
mod mcp;
mod preview;
mod speech;
mod store;
mod thrift;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

struct LoginState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    inbox: Mutex<Option<Arc<AtomicBool>>>,
}

/// One Talk send at a time. Photo upload and the caption share the session,
/// and overlapping sends used to be answered with an older message.
static TALK_SEND: Mutex<()> = Mutex::new(());

struct AppPrefs {
    minimize_to_tray: AtomicBool,
    always_on_top: AtomicBool,
    focus_on_message: AtomicBool,
    tray_badge: AtomicU32,
}

fn stop(state: &LoginState) {
    if let Some(flag) = state.cancel.lock().expect("login lock").take() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn stop_inbox(state: &LoginState) {
    if let Some(flag) = state.inbox.lock().expect("inbox lock").take() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn start_inbox(app: AppHandle, state: &LoginState) {
    stop_inbox(state);
    let cancel = Arc::new(AtomicBool::new(false));
    *state.inbox.lock().expect("inbox lock") = Some(cancel.clone());
    std::thread::spawn(move || {
        let mut client = line::Client::with_cancel(cancel);
        client.watch_inbox(|payload| {
            let _ = store::apply_poll(&payload);
            let Ok(bytes) = serde_json::to_vec(&payload) else { return };
            deliver(&app, "__linePollBuf", "__linePoll", &bytes);
        });
    });
}

fn deliver(app: &AppHandle, slot: &str, callback: &str, payload: &[u8]) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let encoded = base64_encode(payload);
    let _ = window.eval(format!("window.{slot} = ''"));
    for chunk in encoded.as_bytes().chunks(3000) {
        let chunk = std::str::from_utf8(chunk).unwrap_or_default();
        let _ = window.eval(format!("window.{slot} += '{chunk}'"));
    }
    let _ = window.eval(format!(
        "try {{ const encoded = window.{slot} || ''; const bytes = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0)); const detail = JSON.parse(new TextDecoder().decode(bytes)); if (typeof window.{callback} === 'function') window.{callback}(detail); else document.getElementById('status').textContent = 'Chat renderer is missing'; }} catch (error) {{ document.getElementById('status').textContent = String(error).slice(0, 180); }}"
    ));
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn publish(app: &AppHandle, payload: &serde_json::Value) {
    let kind = payload.get("type").and_then(|value| value.as_str()).unwrap_or("chats");
    if kind == "poll" {
        let _ = store::apply_poll(payload);
    } else {
        let _ = store::save_chats(payload);
    }
    let Ok(bytes) = serde_json::to_vec(payload) else { return };
    if kind == "poll" {
        deliver(app, "__linePollBuf", "__linePoll", &bytes);
    } else {
        deliver(app, "__lineChats", "__lineRender", &bytes);
    }
}

#[tauri::command]
async fn load_chats(app: AppHandle, state: State<'_, LoginState>) -> Result<(), String> {
    stop_inbox(state.inner());
    if !line::signed_in() {
        return Err(format!("No LINE session. Write accessToken to {}", line::config_path().display()));
    }
    if let Some(cached) = store::cached_chats() {
        publish(&app, &cached);
    }
    let (tx, mut rx) = tauri::async_runtime::channel(32);
    let app_pump = app.clone();
    let pump = tauri::async_runtime::spawn(async move {
        while let Some(payload) = rx.recv().await {
            publish(&app_pump, &payload);
        }
    });
    let result = tauri::async_runtime::spawn_blocking(move || {
        line::Client::new()
            .list_chats_with(|payload| {
                let _ = tx.blocking_send(payload);
            })
            .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| format!("could not list chats: {err}"))?;
    let _ = pump.await;
    if let Ok(value) = &result {
        let _ = store::save_chats(value);
    }
    if line::signed_in() {
        start_inbox(app, state.inner());
    }
    result?;
    Ok(())
}

#[tauri::command]
async fn load_history(app: AppHandle, id: String) -> Result<(), String> {
    chat_id(&id)?;
    let cached = store::cached_history(&id);
    if let Some(payload) = &cached {
        let bytes = serde_json::to_vec(payload).map_err(|err| format!("could not read saved messages: {err}"))?;
        deliver(&app, "__lineHistoryBuf", "__lineHistory", &bytes);
    }
    let chat = id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || line::Client::new().chat_history(&chat).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load messages: {err}"))?;
    match result {
        Ok(value) => {
            let _ = store::save_history(&value);
            let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read messages: {err}"))?;
            deliver(&app, "__lineHistoryBuf", "__lineHistory", &payload);
            Ok(())
        }
        Err(_) if cached.is_some() => Ok(()),
        Err(err) => Err(err),
    }
}

fn chat_id(id: &str) -> Result<(), String> {
    if id.len() > 64 || id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("that chat id cannot be opened".to_string());
    }
    Ok(())
}

fn user_mid(id: &str) -> Result<(), String> {
    if (2..=64).contains(&id.len()) && id.starts_with('u') && id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Ok(());
    }
    Err("that profile cannot be opened".into())
}

fn group_mid(id: &str) -> Result<(), String> {
    if (2..=64).contains(&id.len()) && id.starts_with('c') && id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Ok(());
    }
    Err("that group cannot be opened".into())
}

fn profile_script(payload: &serde_json::Value) -> String {
    boot_script("__profile", "__applyProfile", payload)
}

fn members_script(payload: &serde_json::Value) -> String {
    boot_script("__members", "__applyMembers", payload)
}

fn boot_script(slot: &str, apply: &str, payload: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let encoded = base64_encode(&bytes);
    format!(
        "window.{slot} = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob('{encoded}'), function (c) {{ return c.charCodeAt(0); }}))); if (typeof window.{apply} === 'function') window.{apply}(window.{slot});"
    )
}

fn show_profile<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, payload: &serde_json::Value) {
    let _ = window.eval(profile_script(payload));
    if let Some(name) = payload.get("name").and_then(|value| value.as_str()).filter(|name| !name.is_empty()) {
        let _ = window.set_title(name);
    }
}

#[tauri::command]
async fn open_profile(app: AppHandle, id: String, name: String, picture: String, mine: bool, friend: bool) -> Result<(), String> {
    user_mid(&id)?;
    let label = format!("p{id}");
    let title = {
        let trimmed = name.trim();
        if trimmed.is_empty() { "LINE".to_string() } else { trimmed.to_string() }
    };
    let initial = serde_json::json!({
        "id": id,
        "name": name.trim(),
        "status": "",
        "picture": picture.trim(),
        "cover": "",
        "mine": mine,
        "friend": friend && !mine,
        "blocked": false,
        "favorite": false,
        "given": name.trim(),
    });
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        let window = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("profile.html".into()))
            .title(&title)
            .inner_size(380.0, 560.0)
            .min_inner_size(320.0, 480.0)
            .resizable(true)
            .always_on_top(app.state::<AppPrefs>().always_on_top.load(Ordering::Relaxed))
            .center()
            .initialization_script(profile_script(&initial))
            .build()
            .map_err(|err| format!("could not open the profile: {err}"))?;
        let _ = window.set_focus();
        enable_capture(&window);
    }
    let app_refresh = app.clone();
    let refresh_label = label.clone();
    tauri::async_runtime::spawn(async move {
        let fetched = tauri::async_runtime::spawn_blocking(move || line::Client::new().contact_profile(&id).map_err(|err| err.to_string())).await;
        let Ok(Ok(card)) = fetched else { return };
        let Some(window) = app_refresh.get_webview_window(&refresh_label) else { return };
        show_profile(&window, &card);
    });
    Ok(())
}

#[tauri::command]
async fn block_profile(id: String, blocked: bool) -> Result<(), String> {
    user_mid(&id)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().set_blocked(&id, blocked).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not update this profile: {err}"))?
}

#[tauri::command]
async fn add_profile(id: String) -> Result<(), String> {
    user_mid(&id)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().add_contact(&id).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not add this friend: {err}"))?
}

#[tauri::command]
async fn rename_profile(app: AppHandle, id: String, name: String) -> Result<(), String> {
    user_mid(&id)?;
    let trimmed = name.trim();
    if trimmed.chars().count() > 20 {
        return Err("that name is too long".into());
    }
    let chat = id.clone();
    let label = trimmed.to_string();
    tauri::async_runtime::spawn_blocking(move || line::Client::new().update_contact_setting(&chat, 2, &label).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not rename this friend: {err}"))??;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(profile_script_call("__renameChat", &serde_json::json!({"id": id, "name": trimmed})));
    }
    Ok(())
}

#[tauri::command]
async fn favorite_profile(id: String, favorite: bool) -> Result<(), String> {
    user_mid(&id)?;
    let value = if favorite { "true" } else { "false" };
    tauri::async_runtime::spawn_blocking(move || line::Client::new().update_contact_setting(&id, 8, value).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not update this friend: {err}"))?
}

fn sync_group(app: &AppHandle, id: &str, name: &str, members: Option<i64>) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.eval(profile_script_call(
        "__patchChat",
        &serde_json::json!({ "id": id, "name": name, "members": members }),
    ));
}

#[tauri::command]
async fn open_members(app: AppHandle, id: String, name: String, picture: String) -> Result<(), String> {
    group_mid(&id)?;
    let initial = serde_json::json!({
        "id": id,
        "name": name.trim(),
        "picture": picture.trim(),
        "autoJoin": true,
        "members": [],
    });
    if let Some(window) = app.get_webview_window("members") {
        let _ = window.eval(members_script(&initial));
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let window = tauri::WebviewWindowBuilder::new(&app, "members", tauri::WebviewUrl::App("members.html".into()))
        .title("Members")
        .inner_size(380.0, 680.0)
        .min_inner_size(320.0, 520.0)
        .resizable(true)
        .always_on_top(app.state::<AppPrefs>().always_on_top.load(Ordering::Relaxed))
        .center()
        .initialization_script(members_script(&initial))
        .build()
        .map_err(|err| format!("could not open the members: {err}"))?;
    let _ = window.set_focus();
    enable_capture(&window);
    Ok(())
}

#[tauri::command]
async fn group_roster(app: AppHandle, id: String) -> Result<serde_json::Value, String> {
    group_mid(&id)?;
    let chat = id.clone();
    let roster = tauri::async_runtime::spawn_blocking(move || line::Client::new().group_roster(&chat).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load the members: {err}"))??;
    let name = roster.get("name").and_then(|value| value.as_str()).unwrap_or("");
    let members = roster.get("members").and_then(|value| value.as_array()).map(|items| items.len() as i64);
    sync_group(&app, &id, name, members);
    Ok(roster)
}

#[tauri::command]
async fn rename_group(app: AppHandle, id: String, name: String) -> Result<(), String> {
    group_mid(&id)?;
    let trimmed = name.trim().to_string();
    let chat = id.clone();
    let label = trimmed.clone();
    tauri::async_runtime::spawn_blocking(move || line::Client::new().rename_group(&chat, &label).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not rename this group: {err}"))??;
    sync_group(&app, &id, &trimmed, None);
    Ok(())
}

#[tauri::command]
async fn set_group_auto_join(id: String, enabled: bool) -> Result<(), String> {
    group_mid(&id)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().set_group_auto_join(&id, enabled).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not update this group: {err}"))?
}

#[tauri::command]
async fn invite_into_group(id: String, targets: Vec<String>) -> Result<(), String> {
    group_mid(&id)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().invite_into_group(&id, &targets).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not invite those friends: {err}"))?
}

#[tauri::command]
async fn remove_group_member(id: String, member: String) -> Result<(), String> {
    group_mid(&id)?;
    user_mid(&member)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().remove_group_member(&id, &member).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not remove that member: {err}"))?
}

#[tauri::command]
async fn list_friends() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| line::Client::new().list_friends().map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load friends: {err}"))?
}

#[tauri::command]
async fn friend_recommendations() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| line::Client::new().friend_recommendations().map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load recommendations: {err}"))?
}

#[tauri::command]
async fn find_person(query: String) -> Result<serde_json::Value, String> {
    let query = query.trim().to_string();
    if query.is_empty() || query.chars().count() > 40 {
        return Err("Enter a LINE ID or phone number.".into());
    }
    tauri::async_runtime::spawn_blocking(move || line::Client::new().find_person(&query).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not search: {err}"))?
}

#[tauri::command]
async fn create_group(name: String, targets: Vec<String>) -> Result<serde_json::Value, String> {
    if targets.len() > 100 {
        return Err("Choose fewer friends.".into());
    }
    for id in &targets {
        user_mid(id)?;
    }
    let label = name.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || line::Client::new().create_group(&label, &targets).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not create the group: {err}"))?
}

#[tauri::command]
async fn group_invite_candidates(id: String) -> Result<serde_json::Value, String> {
    group_mid(&id)?;
    tauri::async_runtime::spawn_blocking(move || line::Client::new().group_invite_candidates(&id).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load friends: {err}"))?
}

#[tauri::command]
async fn open_friend_chat(app: AppHandle, id: String, name: Option<String>, picture: Option<String>) -> Result<(), String> {
    user_mid(&id)?;
    let Some(window) = app.get_webview_window("main") else {
        return Err("the chat window is not open".into());
    };
    let _ = window.eval(profile_script_call(
        "__openChat",
        &serde_json::json!({
            "id": id,
            "name": name.unwrap_or_default().trim(),
            "picture": picture.unwrap_or_default().trim(),
        }),
    ));
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

fn profile_script_call(name: &str, payload: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let encoded = base64_encode(&bytes);
    format!(
        "(() => {{ const detail = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob('{encoded}'), function (c) {{ return c.charCodeAt(0); }}))); if (typeof window.{name} === 'function') window.{name}(detail); }})();"
    )
}

#[tauri::command]
fn open_fraud_help() -> Result<(), String> {
    preview::open_in_browser("https://help.line.me/line/desktop/?contentId=20011496")
}

#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    preview::open_in_browser(&url)
}

#[tauri::command]
async fn link_preview(url: String) -> Result<preview::Card, String> {
    Ok(tauri::async_runtime::spawn_blocking(move || preview::load(&url))
        .await
        .map_err(|_| "could not read that link".to_string())??)
}

#[tauri::command]
async fn mark_chat_read(id: String, message_id: String) -> Result<(), String> {
    chat_id(&id)?;
    if message_id.len() > 24 || message_id.is_empty() || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("that message cannot be marked read".to_string());
    }
    let chat = id.clone();
    let checked = message_id.clone();
    tauri::async_runtime::spawn_blocking(move || line::Client::new().mark_chat_read(&chat, &checked).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not mark the chat read: {err}"))??;
    let _ = store::apply_poll(&serde_json::json!({
        "unreads": [{"id": id, "unread": 0}],
        "updates": [{
            "id": id,
            "read": true,
            "messageId": message_id,
            "lastMessage": "",
            "time": 0,
            "sealed": false
        }]
    }));
    Ok(())
}

#[tauri::command]
async fn load_media(id: String, message_ids: Vec<String>, preview: bool, animated_ids: Option<Vec<String>>) -> Result<serde_json::Value, String> {
    chat_id(&id)?;
    if message_ids.len() > 24 {
        return Err("too many photos".to_string());
    }
    let animated_ids = animated_ids.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || line::Client::new().load_media(&id, &message_ids, preview, &animated_ids).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load the photo: {err}"))?
}

#[tauri::command]
async fn send_reaction(id: String, message_id: String, emoji: String) -> Result<(), String> {
    chat_id(&id)?;
    if message_id.len() > 24 || message_id.is_empty() || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("that message cannot take a reaction".to_string());
    }
    let chosen = emoji.trim().to_string();
    if chosen.len() > 16 {
        return Err("that reaction is not available".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || line::Client::new().send_reaction(&message_id, &chosen).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not send the reaction: {err}"))?
}

fn dropped_mime(name: &str) -> String {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "m4v" => "video/x-m4v",
        "webm" => "video/webm",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn safe_download_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let cleaned: String = base
        .chars()
        .filter(|ch| !ch.is_control() && !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .take(120)
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "download".to_string() } else { cleaned }
}

fn decode_download(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| "could not save that file".to_string())?;
    if bytes.is_empty() || bytes.len() > 40_000_000 {
        return Err("that file is too large to save".to_string());
    }
    Ok(bytes)
}

/// Ask where to save a chat attachment, then write the chosen name.
#[tauri::command]
async fn save_download(app: AppHandle, name: String, data: String, title: Option<String>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let bytes = decode_download(&data)?;
    let name = safe_download_name(&name);
    let title = title.unwrap_or_default();
    let title: String = title.chars().filter(|ch| !ch.is_control()).take(80).collect();
    let title = if title.trim().is_empty() { "Save".to_string() } else { title };
    let mut dialog = app.dialog().file().set_title(&title).set_file_name(&name);
    if let Some(dir) = dirs::download_dir().or_else(dirs::home_dir) {
        dialog = dialog.set_directory(dir);
    }
    let Some(picked) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|err| err.to_string())?;
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    let saved = path.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&path, bytes).map_err(|_| "could not save that file".to_string()))
        .await
        .map_err(|_| "could not save that file".to_string())??;
    Ok(Some(saved.display().to_string()))
}

/// A file-manager drop on Linux arrives as a `file://` path, not a browser File.
#[tauri::command]
async fn read_dropped_file(path: String) -> Result<serde_json::Value, String> {
    let path = path.trim().to_string();
    if path.contains('\0') || path.len() > 4096 {
        return Err("that file cannot be attached".to_string());
    }
    let path = std::path::PathBuf::from(&path);
    if !path.is_absolute() {
        return Err("that file cannot be attached".to_string());
    }
    let file = tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path).map_err(|_| "that file cannot be attached".to_string())?;
        if !meta.is_file() {
            return Err("that file cannot be attached".to_string());
        }
        if meta.len() == 0 {
            return Err("that file cannot be attached".to_string());
        }
        if meta.len() > 40_000_000 {
            return Err("that file is too large to send".to_string());
        }
        let bytes = std::fs::read(&path).map_err(|_| "that file cannot be attached".to_string())?;
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("file").to_string();
        if name.len() > 240 {
            return Err("that file cannot be attached".to_string());
        }
        use base64::Engine;
        Ok(serde_json::json!({
            "name": name,
            "mime": dropped_mime(&name),
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
        }))
    })
    .await
    .map_err(|_| "that file cannot be attached".to_string())?;
    file
}

#[tauri::command]
async fn send_media(app: AppHandle, id: String, name: String, mime: String, data: String, local_id: Option<String>, group_id: Option<String>, group_seq: Option<i32>, group_count: Option<i32>, silent: Option<bool>, duration_ms: Option<i64>, voice: Option<bool>) -> Result<(), String> {
    chat_id(&id)?;
    let name = name.trim().to_string();
    let mime = mime.trim().to_string();
    if name.len() > 240 || mime.len() > 160 {
        return Err("that file cannot be sent".to_string());
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| "that file cannot be sent".to_string())?;
    if bytes.is_empty() || bytes.len() > 40_000_000 {
        return Err("that file is too large to send".to_string());
    }
    let local_id = local_send_id(local_id.unwrap_or_default().trim());
    let chat = id.clone();
    let silent = silent.unwrap_or(false);
    let duration_ms = duration_ms.unwrap_or(0).clamp(0, 3_600_000);
    let voice = voice.unwrap_or(false);
    let album = group_id.and_then(|group| {
        let seq = group_seq.unwrap_or(0);
        let count = group_count.unwrap_or(0);
        if group.is_empty() || seq <= 0 || count <= 0 {
            None
        } else {
            Some(line::Album { id: group, seq, count })
        }
    });
    let mut value = tauri::async_runtime::spawn_blocking(move || {
        let _send = TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        line::Client::new().send_media_for_window(&id, &name, &mime, &bytes, album, silent, duration_ms, voice)
    })
        .await
        .map_err(|err| format!("could not send: {err}"))?;
    if !local_id.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert("localId".to_string(), serde_json::Value::String(local_id));
        }
    }
    let _ = store::save_outgoing(&chat, &value);
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the send result: {err}"))?;
    deliver(&app, "__lineSentBuf", "__lineSent", &payload);
    Ok(())
}

#[tauri::command]
async fn send_sticker(app: AppHandle, id: String, package_id: String, sticker_id: String, animated: bool, local_id: Option<String>) -> Result<(), String> {
    chat_id(&id)?;
    let package_id = package_id.trim().to_string();
    let sticker_id = sticker_id.trim().to_string();
    if package_id.len() > 16 || sticker_id.len() > 16 || !package_id.chars().all(|ch| ch.is_ascii_digit()) || !sticker_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("that sticker cannot be sent".to_string());
    }
    let local_id = local_send_id(local_id.unwrap_or_default().trim());
    let chat = id.clone();
    let mut value = tauri::async_runtime::spawn_blocking(move || {
        let _send = TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        line::Client::new().send_sticker_for_window(&id, &package_id, &sticker_id, animated)
    })
        .await
        .map_err(|err| format!("could not send: {err}"))?;
    if !local_id.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert("localId".to_string(), serde_json::Value::String(local_id));
        }
    }
    let _ = store::save_outgoing(&chat, &value);
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the send result: {err}"))?;
    deliver(&app, "__lineSentBuf", "__lineSent", &payload);
    Ok(())
}

#[tauri::command]
async fn send_message(app: AppHandle, id: String, text: String, related_id: Option<String>, local_id: Option<String>, silent: Option<bool>) -> Result<(), String> {
    chat_id(&id)?;
    let body = text.trim().to_string();
    if body.is_empty() || body.len() > 5000 {
        return Err("enter a message first".to_string());
    }
    let related = related_id.unwrap_or_default();
    if !related.is_empty() && (related.len() > 24 || !related.chars().all(|ch| ch.is_ascii_digit())) {
        return Err("that message cannot be replied to".to_string());
    }
    let local_id = local_send_id(local_id.unwrap_or_default().trim());
    let chat = id.clone();
    let mut value = tauri::async_runtime::spawn_blocking(move || {
        let _send = TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        line::Client::new().send_for_window(&id, &body, &related, silent.unwrap_or(false))
    })
        .await
        .map_err(|err| format!("could not send: {err}"))?;
    if !local_id.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert("localId".to_string(), serde_json::Value::String(local_id));
        }
    }
    let _ = store::save_outgoing(&chat, &value);
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the send result: {err}"))?;
    deliver(&app, "__lineSentBuf", "__lineSent", &payload);
    Ok(())
}

#[tauri::command]
async fn unsend_message(id: String, message_id: String) -> Result<(), String> {
    chat_id(&id)?;
    if message_id.len() > 24 || message_id.is_empty() || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("that message cannot be unsent".to_string());
    }
    let checked = message_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _send = TALK_SEND.lock().unwrap_or_else(|err| err.into_inner());
        line::Client::new().unsend_message(&checked).map_err(|err| unsend_reason(&err))
    })
        .await
        .map_err(|err| format!("could not unsend: {err}"))??;
    let _ = store::mark_unsent(&message_id);
    Ok(())
}

#[tauri::command]
async fn report_message(id: String, message_id: String, sender: String, reason: i32, text: String) -> Result<(), String> {
    chat_id(&id)?;
    if message_id.len() > 24 || message_id.is_empty() || !message_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("that message cannot be reported".to_string());
    }
    if !matches!(reason, 0..=3) || sender.len() > 64 || !sender.starts_with('u') || !sender.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("that message cannot be reported".to_string());
    }
    let body = text.chars().take(1000).collect::<String>();
    tauri::async_runtime::spawn_blocking(move || {
        line::Client::new().report_message(&id, &sender, &message_id, reason, &body).map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| format!("could not send the report: {err}"))?
}

fn local_send_id(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("local-") else {
        return String::new();
    };
    if rest.is_empty() || rest.len() > 32 || !rest.chars().all(|ch| ch.is_ascii_digit() || ch == '-') {
        return String::new();
    }
    raw.to_string()
}

fn unsend_reason(err: &line::LineError) -> String {
    let reason = err.reason.to_ascii_uppercase();
    if err.code == Some(71) || reason.contains("NOT_DESTRUCTIBLE") {
        return "You can't unsend this message as too much time has passed.".into();
    }
    if reason.contains("NOT_FOUND") {
        return "That message is already gone.".into();
    }
    if err.reason.is_empty() { err.message.clone() } else { err.reason.clone() }
}

#[tauri::command]
async fn logout(app: AppHandle, state: State<'_, LoginState>) -> Result<(), String> {
    stop(state.inner());
    stop_inbox(state.inner());
    tauri::async_runtime::spawn_blocking(line::clear_session)
        .await
        .map_err(|err| format!("could not log out: {err}"))?;
    if let Some(window) = app.get_webview_window("members") {
        let _ = window.close();
    }
    show_tray_badge(&app, 0);
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    minimize_to_tray: bool,
    always_on_top: bool,
    focus_on_message: bool,
}

#[derive(Debug, serde::Serialize)]
struct ListedDevice {
    id: String,
    name: String,
    detail: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaDevices {
    outputs: Vec<ListedDevice>,
    inputs: Vec<ListedDevice>,
    cameras: Vec<ListedDevice>,
    audio_error: String,
}

struct PulseNode {
    name: String,
    description: String,
    state: String,
}

fn parse_pulse(text: &str, header: &str) -> Vec<PulseNode> {
    let mut nodes = Vec::new();
    let mut current: Option<PulseNode> = None;
    for line in text.lines() {
        if line.starts_with(header) {
            if let Some(node) = current.take() {
                nodes.push(node);
            }
            current = Some(PulseNode { name: String::new(), description: String::new(), state: String::new() });
            continue;
        }
        let Some(node) = current.as_mut() else {
            continue;
        };
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Name: ") {
            node.name = value.to_string();
        } else if let Some(value) = line.strip_prefix("Description: ") {
            node.description = value.to_string();
        } else if let Some(value) = line.strip_prefix("State: ") {
            node.state = value.to_string();
        }
    }
    if let Some(node) = current {
        nodes.push(node);
    }
    nodes
}

#[cfg(target_os = "linux")]
fn pulse_text(kind: &str) -> Result<String, String> {
    let output = std::process::Command::new("pactl")
        .args(["list", kind])
        .output()
        .map_err(|err| format!("could not list audio devices: {err}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        return Err(if err.is_empty() { "could not list audio devices".into() } else { err.to_string() });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn pactl_line(args: &[&str]) -> String {
    let Ok(output) = std::process::Command::new("pactl").args(args).output() else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn device_detail(state: &str, default: bool) -> String {
    let mut parts = Vec::new();
    if default {
        parts.push("Default");
    }
    if state == "RUNNING" {
        parts.push("In use");
    }
    parts.join(" · ")
}

fn listed_from_pulse(nodes: Vec<PulseNode>, default_name: &str, skip_monitors: bool) -> Vec<ListedDevice> {
    let mut devices: Vec<(bool, bool, ListedDevice)> = nodes
        .into_iter()
        .filter(|node| !node.description.is_empty() && !(skip_monitors && node.name.ends_with(".monitor")))
        .map(|node| {
            let default = !default_name.is_empty() && node.name == default_name;
            let running = node.state == "RUNNING";
            let device = ListedDevice { id: node.name, name: node.description, detail: device_detail(&node.state, default) };
            (default, running, device)
        })
        .collect();
    devices.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.cmp(&left.1)).then(left.2.name.cmp(&right.2.name)));
    devices.into_iter().map(|(_, _, device)| device).collect()
}

#[cfg(any(test, target_os = "linux"))]
fn capture_flags(capabilities: u32, device_caps: u32) -> bool {
    const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
    const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x0000_1000;
    const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
    let flags = if capabilities & CAP_DEVICE_CAPS != 0 && device_caps != 0 { device_caps } else { capabilities };
    flags & (CAP_VIDEO_CAPTURE | CAP_VIDEO_CAPTURE_MPLANE) != 0
}

#[cfg(any(test, target_os = "linux"))]
fn list_cameras() -> Vec<ListedDevice> {
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    let mut cameras = Vec::new();
    for entry in entries.flatten() {
        let node = entry.file_name();
        let node = node.to_string_lossy();
        if !node.starts_with("video") {
            continue;
        }
        let path = format!("/dev/{node}");
        match probe_camera(&path) {
            Some(false) => continue,
            Some(true) | None => {}
        }
        let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        let name = name.trim();
        cameras.push(ListedDevice {
            id: path.clone(),
            name: if name.is_empty() { path.clone() } else { name.to_string() },
            detail: path,
        });
    }
    cameras.sort_by(|left, right| left.name.cmp(&right.name).then(left.detail.cmp(&right.detail)));
    cameras
}

#[cfg(any(test, target_os = "linux"))]
fn probe_camera(path: &str) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        const VIDIOC_QUERYCAP: std::os::raw::c_ulong = 0x8068_5600;
        #[repr(C)]
        struct V4l2Capability {
            driver: [u8; 16],
            card: [u8; 32],
            bus_info: [u8; 32],
            version: u32,
            capabilities: u32,
            device_caps: u32,
            reserved: [u32; 3],
        }
        let file = std::fs::File::open(path).ok()?;
        let mut caps = V4l2Capability {
            driver: [0; 16],
            card: [0; 32],
            bus_info: [0; 32],
            version: 0,
            capabilities: 0,
            device_caps: 0,
            reserved: [0; 3],
        };
        let rc = unsafe { ioctl(file.as_raw_fd(), VIDIOC_QUERYCAP, &mut caps) };
        if rc < 0 {
            return None;
        }
        return Some(capture_flags(caps.capabilities, caps.device_caps));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

#[cfg(target_os = "linux")]
extern "C" {
    fn ioctl(fd: i32, request: std::os::raw::c_ulong, ...) -> i32;
}

#[tauri::command]
fn list_media_devices() -> MediaDevices {
    #[cfg(target_os = "linux")]
    {
        return linux_media_devices();
    }
    #[cfg(target_os = "macos")]
    {
        return macos_media_devices();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    MediaDevices { outputs: Vec::new(), inputs: Vec::new(), cameras: Vec::new(), audio_error: "could not list audio devices".into() }
}

#[cfg(target_os = "linux")]
fn linux_media_devices() -> MediaDevices {
    let audio = (|| -> Result<(Vec<ListedDevice>, Vec<ListedDevice>), String> {
        let sinks = parse_pulse(&pulse_text("sinks")?, "Sink #");
        let sources = parse_pulse(&pulse_text("sources")?, "Source #");
        let default_sink = pactl_line(&["get-default-sink"]);
        let default_source = pactl_line(&["get-default-source"]);
        Ok((listed_from_pulse(sinks, &default_sink, false), listed_from_pulse(sources, &default_source, true)))
    })();
    match audio {
        Ok((outputs, inputs)) => MediaDevices { outputs, inputs, cameras: list_cameras(), audio_error: String::new() },
        Err(audio_error) => MediaDevices { outputs: Vec::new(), inputs: Vec::new(), cameras: list_cameras(), audio_error },
    }
}

#[cfg(target_os = "macos")]
fn macos_media_devices() -> MediaDevices {
    let cameras = macos_cameras();
    match macos_audio() {
        Ok((outputs, inputs)) => MediaDevices { outputs, inputs, cameras, audio_error: String::new() },
        Err(audio_error) => MediaDevices { outputs: Vec::new(), inputs: Vec::new(), cameras, audio_error },
    }
}

#[cfg(target_os = "macos")]
fn macos_audio() -> Result<(Vec<ListedDevice>, Vec<ListedDevice>), String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let default_out = host.default_output_device().and_then(|device| device.name().ok()).unwrap_or_default();
    let default_in = host.default_input_device().and_then(|device| device.name().ok()).unwrap_or_default();
    let outputs = host.output_devices().map_err(|err| format!("could not list audio devices: {err}"))?;
    let inputs = host.input_devices().map_err(|err| format!("could not list audio devices: {err}"))?;
    Ok((listed_from_coreaudio(outputs, &default_out), listed_from_coreaudio(inputs, &default_in)))
}

#[cfg(target_os = "macos")]
fn listed_from_coreaudio(devices: impl Iterator<Item = cpal::Device>, default_name: &str) -> Vec<ListedDevice> {
    use cpal::traits::DeviceTrait;
    let mut listed = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        if name.is_empty() || listed.iter().any(|item: &ListedDevice| item.id == name) {
            continue;
        }
        let default = !default_name.is_empty() && name == default_name;
        listed.push(ListedDevice { id: name.clone(), name, detail: if default { "Default".into() } else { String::new() } });
    }
    listed.sort_by(|left, right| right.detail.starts_with("Default").cmp(&left.detail.starts_with("Default")).then(left.name.cmp(&right.name)));
    listed
}

#[cfg(target_os = "macos")]
fn macos_cameras() -> Vec<ListedDevice> {
    let Ok(output) = std::process::Command::new("system_profiler").args(["SPCameraDataType", "-json"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    cameras_from_profiler(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(any(test, target_os = "macos"))]
fn cameras_from_profiler(text: &str) -> Vec<ListedDevice> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut cameras = Vec::new();
    collect_cameras(&value, &mut cameras);
    cameras.sort_by(|left, right| left.name.cmp(&right.name).then(left.detail.cmp(&right.detail)));
    cameras
}

#[cfg(any(test, target_os = "macos"))]
fn collect_cameras(value: &serde_json::Value, cameras: &mut Vec<ListedDevice>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_cameras(item, cameras);
            }
        }
        serde_json::Value::Object(map) => {
            let name = map.get("_name").and_then(|item| item.as_str()).unwrap_or("").trim();
            let unique = map.get("spcamera_unique-id").and_then(|item| item.as_str()).map(str::trim);
            let model = map.get("spcamera_model-id").and_then(|item| item.as_str()).map(str::trim);
            if !name.is_empty() && (unique.is_some() || model.is_some()) {
                let id = unique.filter(|item| !item.is_empty()).unwrap_or(name).to_string();
                if !cameras.iter().any(|camera| camera.id == id) {
                    let detail = model.filter(|item| !item.is_empty()).unwrap_or(id.as_str()).to_string();
                    cameras.push(ListedDevice { id, name: name.to_string(), detail });
                }
            }
            for child in map.values() {
                collect_cameras(child, cameras);
            }
        }
        _ => {}
    }
}

fn enable_capture(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    {
        let _ = window.with_webview(|webview| {
            use webkit2gtk::{SettingsExt, WebViewExt};
            if let Some(settings) = webview.inner().settings() {
                settings.set_enable_media(true);
                settings.set_enable_media_stream(true);
                settings.set_enable_mediasource(true);
                settings.set_enable_webaudio(true);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = window;
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppPrefs>) -> Settings {
    Settings {
        minimize_to_tray: state.minimize_to_tray.load(Ordering::Relaxed),
        always_on_top: state.always_on_top.load(Ordering::Relaxed),
        focus_on_message: state.focus_on_message.load(Ordering::Relaxed),
    }
}

#[tauri::command]
fn set_tray_badge(app: AppHandle, count: u32) {
    show_tray_badge(&app, count);
}

#[tauri::command]
fn set_minimize_to_tray(app: AppHandle, state: State<'_, AppPrefs>, enabled: bool) -> Result<(), String> {
    if enabled {
        ensure_tray(&app)?;
    }
    save_setting("minimizeToTray", enabled)?;
    state.minimize_to_tray.store(enabled, Ordering::Relaxed);
    if !enabled {
        reveal(&app);
        drop(app.remove_tray_by_id("line"));
    }
    Ok(())
}

#[tauri::command]
fn set_always_on_top(app: AppHandle, state: State<'_, AppPrefs>, enabled: bool) -> Result<(), String> {
    save_setting("alwaysOnTop", enabled)?;
    if let Err(err) = apply_always_on_top(&app, enabled) {
        let _ = save_setting("alwaysOnTop", !enabled);
        return Err(err);
    }
    state.always_on_top.store(enabled, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn set_focus_on_message(state: State<'_, AppPrefs>, enabled: bool) -> Result<(), String> {
    save_setting("focusOnMessage", enabled)?;
    state.focus_on_message.store(enabled, Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if !enabled {
        let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-focus"]);
    }
    Ok(())
}

#[tauri::command]
fn focus_for_message(app: AppHandle, state: State<'_, AppPrefs>) {
    if state.focus_on_message.load(Ordering::Relaxed) {
        reveal(&app);
        #[cfg(target_os = "linux")]
        if wayland_session() {
            let _ = kwin_activate_main();
        }
    }
}

#[tauri::command]
fn attention_for_message(app: AppHandle) {
    #[cfg(target_os = "linux")]
    {
        let _ = kwin_attention(true);
        let _ = app;
    }
    #[cfg(not(target_os = "linux"))]
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

#[tauri::command]
fn clear_attention(app: AppHandle) {
    #[cfg(target_os = "linux")]
    {
        let _ = kwin_attention(false);
        let _ = app;
    }
    #[cfg(not(target_os = "linux"))]
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(None);
    }
}

#[cfg(target_os = "linux")]
static FOCUS_GEN: AtomicU32 = AtomicU32::new(0);

#[cfg(target_os = "linux")]
fn kwin_activate_main() -> Result<(), String> {
    let path = settings_path().with_file_name("lineapp-focus.js");
    let body = "function lineappActivate(client) {\n    if (!client) return;\n    if (client.resourceName !== \"lineapp\" && client.resourceClass !== \"lineapp\") return;\n    if (client.caption !== \"LINE\") return;\n    client.minimized = false;\n    try { workspace.raiseWindow(client); } catch (error) {}\n    workspace.activeWindow = client;\n}\nworkspace.windowList().forEach(lineappActivate);\nworkspace.windowAdded.connect(lineappActivate);\n";
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-focus"]);
    kwin_script(&["org.kde.kwin.Scripting.loadScript", &path.display().to_string(), "lineapp-focus"])?;
    kwin_script(&["org.kde.kwin.Scripting.start"])?;
    let gen = FOCUS_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if FOCUS_GEN.load(Ordering::Relaxed) == gen {
            let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-focus"]);
        }
    });
    Ok(())
}

fn apply_always_on_top(app: &AppHandle, enabled: bool) -> Result<(), String> {
    for (_, window) in app.webview_windows() {
        window.set_always_on_top(enabled).map_err(|err| err.to_string())?;
    }
    // GTK's keep-above flag does nothing on Wayland. KDE still honors keepAbove.
    #[cfg(target_os = "linux")]
    if wayland_session() {
        kwin_keep_above(enabled)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
static ATTENTION_GEN: AtomicU32 = AtomicU32::new(0);

#[cfg(target_os = "linux")]
fn kde_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|value| value.to_ascii_lowercase().contains("kde"))
        .unwrap_or(false)
        || std::env::var_os("KDE_FULL_SESSION").is_some()
}

#[cfg(target_os = "linux")]
fn kwin_attention(enabled: bool) -> Result<(), String> {
    if !kde_session() {
        return Ok(());
    }
    let plugin = if enabled { "lineapp-attention" } else { "lineapp-clear-attention" };
    let path = settings_path().with_file_name(format!("{plugin}.js"));
    let body = if enabled {
        "function lineappAttention(client) {\n    if (!client) return;\n    if (client.resourceName !== \"lineapp\" && client.resourceClass !== \"lineapp\") return;\n    if (client.caption !== \"LINE\") return;\n    client.demandsAttention = true;\n}\nworkspace.windowList().forEach(lineappAttention);\n"
    } else {
        "function lineappClearAttention(client) {\n    if (!client) return;\n    if (client.resourceName !== \"lineapp\" && client.resourceClass !== \"lineapp\") return;\n    if (client.caption !== \"LINE\") return;\n    client.demandsAttention = false;\n}\nworkspace.windowList().forEach(lineappClearAttention);\n"
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-attention"]);
    let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-clear-attention"]);
    kwin_script(&["org.kde.kwin.Scripting.loadScript", &path.display().to_string(), plugin])?;
    kwin_script(&["org.kde.kwin.Scripting.start"])?;
    let gen = ATTENTION_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if ATTENTION_GEN.load(Ordering::Relaxed) == gen {
            let _ = kwin_script(&["org.kde.kwin.Scripting.unloadScript", plugin]);
        }
    });
    Ok(())
}

#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return true;
    }
    std::env::var_os("XDG_SESSION_TYPE").is_some_and(|kind| kind == "wayland")
}

#[cfg(target_os = "linux")]
fn kwin_keep_above(enabled: bool) -> Result<(), String> {
    let plugin = if enabled { "lineapp-keep-above" } else { "lineapp-clear-above" };
    let path = settings_path().with_file_name(format!("{plugin}.js"));
    let body = if enabled {
        "function lineappKeepAbove(client) {\n    if (client.resourceClass === \"lineapp\" || client.resourceName === \"lineapp\") {\n        client.keepAbove = true;\n    }\n}\nworkspace.windowList().forEach(lineappKeepAbove);\nworkspace.windowAdded.connect(lineappKeepAbove);\n"
    } else {
        "workspace.windowList().forEach(function (client) {\n    if (client.resourceClass === \"lineapp\" || client.resourceName === \"lineapp\") {\n        client.keepAbove = false;\n    }\n});\n"
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-keep-above"])?;
    kwin_script(&["org.kde.kwin.Scripting.unloadScript", "lineapp-clear-above"])?;
    kwin_script(&["org.kde.kwin.Scripting.loadScript", &path.display().to_string(), plugin])?;
    kwin_script(&["org.kde.kwin.Scripting.start"])?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn kwin_script(args: &[&str]) -> Result<(), String> {
    let mut last_error = String::from("this desktop cannot keep a window above others");
    for bin in ["qdbus6", "qdbus"] {
        match std::process::Command::new(bin)
            .args(["org.kde.KWin", "/Scripting"])
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                let err = err.trim();
                if !err.is_empty() {
                    last_error = err.to_string();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => last_error = err.to_string(),
        }
    }
    Err(last_error)
}

fn settings_path() -> std::path::PathBuf {
    line::config_path().with_file_name("lineapp.settings.json")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrefFlags {
    minimize_to_tray: bool,
    always_on_top: bool,
    focus_on_message: bool,
}

fn load_settings() -> PrefFlags {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return PrefFlags { minimize_to_tray: true, always_on_top: false, focus_on_message: false };
    };
    settings_flags(&text)
}

fn settings_flags(text: &str) -> PrefFlags {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return PrefFlags { minimize_to_tray: true, always_on_top: false, focus_on_message: false };
    };
    PrefFlags {
        minimize_to_tray: value.get("minimizeToTray").and_then(|flag| flag.as_bool()).unwrap_or(true),
        always_on_top: value.get("alwaysOnTop").and_then(|flag| flag.as_bool()).unwrap_or(false),
        focus_on_message: value.get("focusOnMessage").and_then(|flag| flag.as_bool()).unwrap_or(false),
    }
}

fn save_setting(key: &str, enabled: bool) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let mut value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    value[key] = serde_json::Value::Bool(enabled);
    let body = serde_json::to_vec_pretty(&value).map_err(|err| err.to_string())?;
    std::fs::write(&path, body).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn keep_in_tray(app: &AppHandle) -> bool {
    app.state::<AppPrefs>().minimize_to_tray.load(Ordering::Relaxed) && app.tray_by_id("line").is_some()
}

fn reveal(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn ensure_tray(app: &AppHandle) -> Result<(), String> {
    if app.tray_by_id("line").is_some() {
        return Ok(());
    }
    let show = MenuItem::with_id(app, "tray-show", "Open LINE", true, None::<&str>).map_err(|err| err.to_string())?;
    let quit = MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>).map_err(|err| err.to_string())?;
    let menu = Menu::with_items(app, &[&show, &quit]).map_err(|err| err.to_string())?;
    let count = app.state::<AppPrefs>().tray_badge.load(Ordering::Relaxed);
    let icon = tray_image(count);
    // Linux does not emit tray clicks, so the menu is how the window comes back.
    TrayIconBuilder::with_id("line")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id == "tray-show" {
                reveal(app);
            } else if event.id == "tray-quit" {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                reveal(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|err| format!("could not add the tray icon: {err}"))?;
    let latest = app.state::<AppPrefs>().tray_badge.load(Ordering::Relaxed);
    if latest != count {
        if let Some(tray) = app.tray_by_id("line") {
            let _ = tray.set_icon(Some(tray_image(latest)));
        }
    }
    Ok(())
}

fn show_tray_badge(app: &AppHandle, count: u32) {
    let shown = u32::from(count > 0);
    let prefs = app.state::<AppPrefs>();
    if prefs.tray_badge.swap(shown, Ordering::Relaxed) == shown {
        return;
    }
    let Some(tray) = app.tray_by_id("line") else {
        return;
    };
    let _ = tray.set_icon(Some(tray_image(shown)));
}

fn tray_image(count: u32) -> tauri::image::Image<'static> {
    let icon = badge_image(count);
    let (width, height) = icon.dimensions();
    tauri::image::Image::new_owned(icon.into_raw(), width, height)
}

fn badge_image(count: u32) -> image::RgbaImage {
    let mut icon = image::load_from_memory(include_bytes!("../icons/64x64.png")).expect("line icon").into_rgba8();
    if count == 0 {
        return icon;
    }
    let (width, _) = icon.dimensions();
    let dot = 22;
    let halo = 2;
    let outer = dot + halo * 2;
    let x = width.saturating_sub(outer);
    fill_pill(&mut icon, x, 0, outer, outer, [255, 255, 255, 255]);
    fill_pill(&mut icon, x + halo, halo, dot, dot, [220, 38, 38, 255]);
    icon
}

fn fill_pill(icon: &mut image::RgbaImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) {
    if w == 0 || h == 0 {
        return;
    }
    let radius = h as f32 / 2.0;
    let (width, height) = icon.dimensions();
    for py in 0..h {
        for px in 0..w {
            let px_i = x as i32 + px as i32;
            let py_i = y as i32 + py as i32;
            if px_i < 0 || py_i < 0 || px_i >= width as i32 || py_i >= height as i32 {
                continue;
            }
            let local_x = px as f32 + 0.5;
            let local_y = py as f32 + 0.5;
            let nearest_x = local_x.clamp(radius, w as f32 - radius);
            let dx = local_x - nearest_x;
            let dy = local_y - h as f32 / 2.0;
            let dist = (dx * dx + dy * dy).sqrt();
            let coverage = (radius + 0.6 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut src = color;
            src[3] = (color[3] as f32 * coverage) as u8;
            blend(icon.get_pixel_mut(px_i as u32, py_i as u32), src);
        }
    }
}

fn blend(pixel: &mut image::Rgba<u8>, src: [u8; 4]) {
    let alpha = src[3] as f32 / 255.0;
    if alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        pixel.0[channel] = (src[channel] as f32 * alpha + pixel.0[channel] as f32 * (1.0 - alpha)).round() as u8;
    }
    pixel.0[3] = (src[3] as f32 + pixel.0[3] as f32 * (1.0 - alpha)).clamp(0.0, 255.0).round() as u8;
}

#[tauri::command]
fn start_login(app: AppHandle, state: State<LoginState>) -> Result<(), String> {
    stop(state.inner());
    stop_inbox(state.inner());
    show_tray_badge(&app, 0);
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().expect("login lock") = Some(cancel.clone());
    std::thread::spawn(move || {
        let mut client = line::Client::with_cancel(cancel);
        if let Err(err) = client.login_qr(|event| {
            let _ = app.emit("login-event", event);
        }) {
            let _ = app.emit("login-event", serde_json::json!({"type": "error", "message": err.to_string()}));
        }
    });
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_taskbar_icon() {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share")));
    let Some(data) = data else {
        return;
    };
    // KWin names this window "lineapp" and Plasma loads that desktop file.
    // The icon has to be a theme name; a file path falls back to the Wayland logo.
    let sizes = [
        ("16", include_bytes!("../icons/16x16.png").as_slice()),
        ("22", include_bytes!("../icons/22x22.png").as_slice()),
        ("32", include_bytes!("../icons/32x32.png").as_slice()),
        ("48", include_bytes!("../icons/48x48.png").as_slice()),
        ("64", include_bytes!("../icons/64x64.png").as_slice()),
        ("128", include_bytes!("../icons/128x128.png").as_slice()),
        ("256", include_bytes!("../icons/256x256.png").as_slice()),
        ("512", include_bytes!("../icons/icon.png").as_slice()),
    ];
    for (size, bytes) in sizes {
        let dir = data.join(format!("icons/hicolor/{size}x{size}/apps"));
        if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join("lineapp.png"), bytes).is_err() {
            return;
        }
    }
    let apps = data.join("applications");
    if std::fs::create_dir_all(&apps).is_err() {
        return;
    }
    let program = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("lineapp"));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=LINE\nExec={}\nIcon=lineapp\nStartupWMClass=lineapp\n",
        program.display()
    );
    if std::fs::write(apps.join("lineapp.desktop"), entry).is_err() {
        return;
    }
    let _ = std::fs::remove_file(apps.join("dev.lineapp.desktop"));
    std::thread::spawn(|| {
        let _ = std::process::Command::new("kbuildsycoca6")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

#[cfg(target_os = "macos")]
const MAC_INFO_PLIST_LEN: usize = include_bytes!("../Info.plist").len();

/// Mach-O tools read this section, so a `tauri dev` binary can ask for the microphone.
#[cfg(target_os = "macos")]
#[used]
#[link_section = "__TEXT,__info_plist"]
static MAC_INFO_PLIST: [u8; MAC_INFO_PLIST_LEN] = *include_bytes!("../Info.plist");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    install_taskbar_icon();
    let context = tauri::generate_context!();
    line::install_config_dir(&context.config().identifier);
    let prefs = load_settings();
    let minimize_to_tray = prefs.minimize_to_tray;
    let always_on_top = prefs.always_on_top;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("keep-page")
                .on_navigation(|_webview, url| url.scheme() != "file")
                .build(),
        )
        .manage(LoginState { cancel: Mutex::new(None), inbox: Mutex::new(None) })
        .manage(AppPrefs {
            minimize_to_tray: AtomicBool::new(minimize_to_tray),
            always_on_top: AtomicBool::new(always_on_top),
            focus_on_message: AtomicBool::new(prefs.focus_on_message),
            tray_badge: AtomicU32::new(0),
        })
        .on_permission_request(|_window, kind| match kind {
            tauri::webview::PermissionKind::Microphone | tauri::webview::PermissionKind::Camera => tauri::webview::PermissionResponse::Allow,
            _ => tauri::webview::PermissionResponse::Default,
        })
        .invoke_handler(tauri::generate_handler![start_login, load_chats, load_history, mark_chat_read, load_media, save_download, read_dropped_file, send_message, send_media, send_sticker, unsend_message, report_message, send_reaction, logout, quit, get_settings, set_minimize_to_tray, set_always_on_top, set_focus_on_message, focus_for_message, attention_for_message, clear_attention, set_tray_badge, list_media_devices, speech::dictate_start, speech::dictate_stop, speech::dictate_cancel, speech::dictate_level, speech::mic_test_stop, speech::voice_start, speech::voice_stop, speech::voice_cancel, speech::voice_level, agent::get_agent_settings, agent::set_agent_settings, agent::set_reply_settings, agent::get_chat_llm, agent::set_chat_llm, agent::clear_chat_llm, agent::agent_ask, agent::agent_probe, agent::agent_compose, mcp::get_mcp_servers, mcp::set_mcp_servers, open_profile, block_profile, add_profile, open_fraud_help, open_link, link_preview, rename_profile, favorite_profile, open_friend_chat, open_members, group_roster, rename_group, set_group_auto_join, invite_into_group, remove_group_member, group_invite_candidates, list_friends, friend_recommendations, find_person, create_group])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if keep_in_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                enable_capture(&window);
            }
            if always_on_top {
                if let Err(err) = apply_always_on_top(app.handle(), true) {
                    eprintln!("lineapp: {err}");
                }
            }
            if minimize_to_tray {
                if let Err(err) = ensure_tray(app.handle()) {
                    eprintln!("lineapp: {err}");
                }
            }
            Ok(())
        })
        .build(context)
        .expect("failed to build the LINE window")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if code.is_none() && keep_in_tray(app) {
                    api.prevent_exit();
                }
            }
            tauri::RunEvent::Exit => {
                let state = app.state::<LoginState>();
                stop(state.inner());
                stop_inbox(state.inner());
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::{badge_image, capture_flags, local_send_id, parse_pulse, settings_flags, PrefFlags};

    #[test]
    fn settings_keep_both_flags() {
        let absent = PrefFlags { minimize_to_tray: true, always_on_top: false, focus_on_message: false };
        assert_eq!(settings_flags("{"), absent);
        assert_eq!(settings_flags(r#"{"minimizeToTray": false}"#), PrefFlags { minimize_to_tray: false, ..absent });
        assert_eq!(
            settings_flags(r#"{"minimizeToTray": true, "alwaysOnTop": true, "focusOnMessage": true}"#),
            PrefFlags { minimize_to_tray: true, always_on_top: true, focus_on_message: true }
        );
    }

    #[test]
    fn unread_badge_sits_on_the_icon() {
        let plain = badge_image(0);
        let one = badge_image(1);
        let dozen = badge_image(12);
        assert_eq!(plain.dimensions(), (64, 64));
        assert!(plain.pixels().zip(one.pixels()).any(|(left, right)| left != right));
        assert!(one.pixels().zip(dozen.pixels()).all(|(left, right)| left == right));
        assert_eq!(badge_image(120).get_pixel(8, 56), plain.get_pixel(8, 56));
        let red = one.pixels().any(|pixel| pixel[0] > 180 && pixel[1] < 80 && pixel[2] < 80);
        assert!(red);
    }

    #[test]
    fn pulse_listing_keeps_speakers_and_skips_monitors() {
        let sinks = "\
Sink #61
	State: SUSPENDED
	Name: alsa_output.speakers
	Description: USB Audio Speakers
Sink #465
	State: RUNNING
	Name: bluez_output.headphones
	Description: h.ear go (SRS-HG1)
";
        let sources = "\
Source #65
	State: SUSPENDED
	Name: alsa_input.mic
	Description: USB Audio Microphone
Source #61
	State: SUSPENDED
	Name: alsa_output.speakers.monitor
	Description: Monitor of USB Audio Speakers
";
        let outputs = super::listed_from_pulse(parse_pulse(sinks, "Sink #"), "bluez_output.headphones", false);
        let inputs = super::listed_from_pulse(parse_pulse(sources, "Source #"), "alsa_input.mic", true);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "h.ear go (SRS-HG1)");
        assert_eq!(outputs[0].detail, "Default · In use");
        assert_eq!(outputs[1].name, "USB Audio Speakers");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].id, "alsa_input.mic");
        assert_eq!(inputs[0].name, "USB Audio Microphone");
        assert_eq!(inputs[0].detail, "Default");
    }

    #[test]
    fn metadata_video_nodes_are_not_cameras() {
        assert!(capture_flags(0x8000_0000 | 0x0000_0001, 0x0000_0001));
        assert!(!capture_flags(0x8000_0000 | 0x0000_0001, 0x0000_0200));
        assert!(capture_flags(0x0000_1000, 0));
    }

    #[test]
    fn connected_webcam_is_listed_once() {
        if !std::path::Path::new("/dev/video0").exists() {
            return;
        }
        let cameras = super::list_cameras();
        assert!(cameras.iter().any(|camera| camera.detail == "/dev/video0"), "{cameras:?}");
        assert!(cameras.iter().all(|camera| camera.detail != "/dev/video1"), "{cameras:?}");
    }

    #[test]
    fn mac_camera_report_lists_each_camera_once() {
        let text = r#"{
            "SPCameraDataType": [
                {"_name": "FaceTime HD Camera", "spcamera_model-id": "FaceTime HD Camera", "spcamera_unique-id": "0xabc"},
                {"_name": "FaceTime HD Camera", "spcamera_model-id": "FaceTime HD Camera", "spcamera_unique-id": "0xabc"},
                {"_name": "USB Capture", "spcamera_model-id": "USB Capture"}
            ]
        }"#;
        let cameras = super::cameras_from_profiler(text);
        assert_eq!(cameras.len(), 2, "{cameras:?}");
        assert_eq!(cameras[0].id, "0xabc");
        assert_eq!(cameras[0].detail, "FaceTime HD Camera");
        assert_eq!(cameras[1].name, "USB Capture");
        assert!(super::cameras_from_profiler(r#"{"SPCameraDataType":[{"_name":"Display"}]}"#).is_empty());
    }

    #[test]
    fn local_send_ids_round_trip_only_for_pending_bubbles() {
        assert_eq!(local_send_id("local-1710000000000-2"), "local-1710000000000-2");
        assert_eq!(local_send_id("local-"), "");
        assert_eq!(local_send_id("5993848201234567890"), "");
        assert_eq!(local_send_id("local-1-2 extra"), "");
    }
}
