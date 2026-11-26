mod e2ee;
mod legy;
mod line;
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

struct AppPrefs {
    minimize_to_tray: AtomicBool,
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
    stop_inbox(state.inner());
    tauri::async_runtime::spawn_blocking(line::clear_session)
        .await
        .map_err(|err| format!("could not log out: {err}"))?;
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
}

#[tauri::command]
fn get_settings(state: State<'_, AppPrefs>) -> Settings {
    Settings { minimize_to_tray: state.minimize_to_tray.load(Ordering::Relaxed) }
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
    save_minimize_to_tray(enabled)?;
    state.minimize_to_tray.store(enabled, Ordering::Relaxed);
    if !enabled {
        reveal(&app);
        drop(app.remove_tray_by_id("line"));
    }
    Ok(())
}

fn settings_path() -> std::path::PathBuf {
    line::config_path().with_file_name("lineapp.settings.json")
}

fn load_minimize_to_tray() -> bool {
    let Ok(text) = std::fs::read_to_string(settings_path()) else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.get("minimizeToTray").and_then(|flag| flag.as_bool()))
        .unwrap_or(true)
}

fn save_minimize_to_tray(enabled: bool) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let body = serde_json::to_vec_pretty(&serde_json::json!({ "minimizeToTray": enabled })).map_err(|err| err.to_string())?;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_taskbar_icon();
    let minimize_to_tray = load_minimize_to_tray();
    tauri::Builder::default()
        .manage(LoginState { cancel: Mutex::new(None), inbox: Mutex::new(None) })
        .manage(AppPrefs { minimize_to_tray: AtomicBool::new(minimize_to_tray), tray_badge: AtomicU32::new(0) })
        .invoke_handler(tauri::generate_handler![start_login, load_chats, load_history, load_media, send_message, logout, quit, get_settings, set_minimize_to_tray, set_tray_badge])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if keep_in_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(move |app| {
            if minimize_to_tray {
                if let Err(err) = ensure_tray(app.handle()) {
                    eprintln!("lineapp: {err}");
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
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
    use super::badge_image;

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
}
