mod e2ee;
mod legy;
mod line;
mod thrift;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

struct LoginState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

fn stop(state: &LoginState) {
    if let Some(flag) = state.cancel.lock().expect("login lock").take() {
        flag.store(true, Ordering::Relaxed);
    }
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

#[tauri::command]
async fn load_chats(app: AppHandle) -> Result<(), String> {
    let value = tauri::async_runtime::spawn_blocking(|| line::Client::new().list_chats().map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not list chats: {err}"))??;
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the chat list: {err}"))?;
    deliver(&app, "__lineChats", "__lineRender", &payload);
    Ok(())
}

#[tauri::command]
async fn load_history(app: AppHandle, id: String) -> Result<(), String> {
    chat_id(&id)?;
    let value = tauri::async_runtime::spawn_blocking(move || line::Client::new().chat_history(&id).map_err(|err| err.to_string()))
        .await
        .map_err(|err| format!("could not load messages: {err}"))??;
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read messages: {err}"))?;
    deliver(&app, "__lineHistoryBuf", "__lineHistory", &payload);
    Ok(())
}

fn chat_id(id: &str) -> Result<(), String> {
    if id.len() > 64 || id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("that chat id cannot be opened".to_string());
    }
    Ok(())
}

#[tauri::command]
async fn send_message(app: AppHandle, id: String, text: String) -> Result<(), String> {
    chat_id(&id)?;
    let body = text.trim().to_string();
    if body.is_empty() || body.len() > 5000 {
        return Err("enter a message first".to_string());
    }
    let value = tauri::async_runtime::spawn_blocking(move || line::Client::new().send_for_window(&id, &body))
        .await
        .map_err(|err| format!("could not send: {err}"))?;
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the send result: {err}"))?;
    deliver(&app, "__lineSentBuf", "__lineSent", &payload);
    Ok(())
}

#[tauri::command]
async fn logout(app: AppHandle, state: State<'_, LoginState>) -> Result<(), String> {
    stop(state.inner());
    tauri::async_runtime::spawn_blocking(line::clear_session)
        .await
        .map_err(|err| format!("could not log out: {err}"))?;
    let _ = app;
    Ok(())
}

#[tauri::command]
async fn refresh_inbox(app: AppHandle, known: String) -> Result<(), String> {
    if known.len() > 200_000 {
        return Err("inbox clock is too large".to_string());
    }
    let value = tauri::async_runtime::spawn_blocking(move || {
        let parsed: serde_json::Value = serde_json::from_str(&known).map_err(|err| format!("could not read the inbox clock: {err}"))?;
        let mut times = HashMap::new();
        if let Some(map) = parsed.get("times").and_then(|value| value.as_object()) {
            for (key, value) in map {
                if let Some(time) = value.as_i64() {
                    times.insert(key.clone(), time);
                }
            }
        }
        line::Client::new().poll_inbox(&times).map_err(|err| err.to_string())
    })
        .await
        .map_err(|err| format!("could not refresh chats: {err}"))??;
    let payload = serde_json::to_vec(&value).map_err(|err| format!("could not read the inbox: {err}"))?;
    deliver(&app, "__linePollBuf", "__linePoll", &payload);
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn start_login(app: AppHandle, state: State<LoginState>) -> Result<(), String> {
    stop(state.inner());
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(LoginState { cancel: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![start_login, load_chats, load_history, send_message, refresh_inbox, logout, quit])
        .build(tauri::generate_context!())
        .expect("failed to build the LINE window")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop(app.state::<LoginState>().inner());
            }
        });
}
