use serde::{Deserialize, Serialize};
use std::fs;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, PhysicalPosition, PhysicalSize, Rect, WindowEvent,
};

#[derive(Serialize, Clone)]
pub struct UsageData {
    five_hour: f64,
    five_hour_resets_at: Option<String>,
    weekly: f64,
    weekly_resets_at: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status")]
pub enum ServiceResult {
    #[serde(rename = "ok")]
    Ok(UsageData),
    #[serde(rename = "not_logged_in")]
    NotLoggedIn { login_hint: String },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Serialize, Clone)]
pub struct AllUsage {
    claude: ServiceResult,
    codex: ServiceResult,
}

struct CachedResult {
    data: ServiceResult,
    fetched_at: Instant,
}

struct AppState {
    claude_cache: Mutex<Option<CachedResult>>,
    codex_cache: Mutex<Option<CachedResult>>,
}

const CACHE_SECS: u64 = 300; // 5 minutes

#[tauri::command]
async fn get_usage(state: tauri::State<'_, AppState>) -> Result<AllUsage, ()> {
    let (claude, codex) = tokio::join!(fetch_claude_cached(&state), fetch_codex_cached(&state));
    Ok(AllUsage { claude, codex })
}

async fn fetch_claude_cached(state: &AppState) -> ServiceResult {
    {
        let cache = state.claude_cache.lock().unwrap();
        if let Some(ref c) = *cache {
            if c.fetched_at.elapsed().as_secs() < CACHE_SECS {
                return c.data.clone();
            }
        }
    }

    let result = fetch_claude_usage().await;
    let mut cache = state.claude_cache.lock().unwrap();
    *cache = Some(CachedResult {
        data: result.clone(),
        fetched_at: Instant::now(),
    });
    result
}

async fn fetch_codex_cached(state: &AppState) -> ServiceResult {
    {
        let cache = state.codex_cache.lock().unwrap();
        if let Some(ref c) = *cache {
            if c.fetched_at.elapsed().as_secs() < CACHE_SECS {
                return c.data.clone();
            }
        }
    }

    let result = fetch_codex_usage().await;
    let mut cache = state.codex_cache.lock().unwrap();
    *cache = Some(CachedResult {
        data: result.clone(),
        fetched_at: Instant::now(),
    });
    result
}

// --- Claude ---

struct ClaudeCreds {
    access_token: String,
    refresh_token: String,
    raw: serde_json::Value,
    storage: ClaudeCredsStorage,
}

#[derive(Deserialize)]
struct ClaudeTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

enum ClaudeCredsStorage {
    File(std::path::PathBuf),
    #[cfg(target_os = "macos")]
    Keychain {
        service: &'static str,
    },
}

async fn refresh_claude_token(refresh_token: &str) -> Result<ClaudeTokenResponse, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://console.anthropic.com/v1/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", "9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;

    resp.json::<ClaudeTokenResponse>()
        .await
        .map_err(|e| e.to_string())
}

fn claude_creds_paths(home: &std::path::Path) -> [std::path::PathBuf; 2] {
    [
        home.join(".claude").join(".credentials.json"),
        home.join(".claude").join("credentials.json"),
    ]
}

fn parse_claude_creds(raw: &str, storage: ClaudeCredsStorage) -> Result<ClaudeCreds, String> {
    let creds = serde_json::from_str::<serde_json::Value>(raw).map_err(|e| e.to_string())?;
    let oauth = creds["claudeAiOauth"]
        .as_object()
        .ok_or_else(|| "Missing claudeAiOauth".to_string())?;
    let access_token = oauth["accessToken"]
        .as_str()
        .ok_or_else(|| "Missing access token".to_string())?
        .to_string();
    let refresh_token = oauth["refreshToken"]
        .as_str()
        .ok_or_else(|| "Missing refresh token".to_string())?
        .to_string();

    Ok(ClaudeCreds {
        access_token,
        refresh_token,
        raw: creds,
        storage,
    })
}

#[cfg(target_os = "macos")]
fn load_claude_creds_from_keychain() -> Option<ClaudeCreds> {
    for service in ["Claude Code-credentials", "Claude Code"] {
        let output = match Command::new("security")
            .args(["find-generic-password", "-s", service, "-w"])
            .output()
        {
            Ok(output) => output,
            Err(_) => continue,
        };

        if !output.status.success() {
            continue;
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            continue;
        }

        if let Ok(creds) = parse_claude_creds(&raw, ClaudeCredsStorage::Keychain { service }) {
            return Some(creds);
        }
    }

    None
}

fn load_claude_creds(home: &std::path::Path) -> Option<ClaudeCreds> {
    #[cfg(target_os = "macos")]
    if let Some(creds) = load_claude_creds_from_keychain() {
        return Some(creds);
    }

    for path in claude_creds_paths(home) {
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };

        if let Ok(creds) = parse_claude_creds(&raw, ClaudeCredsStorage::File(path)) {
            return Some(creds);
        }
    }

    None
}

fn save_claude_creds(
    creds: &ClaudeCreds,
    access_token: &str,
    refresh_token: &str,
    expires_in: u64,
) {
    let mut updated = creds.raw.clone();
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + expires_in * 1000;
    updated["claudeAiOauth"]["accessToken"] = serde_json::json!(access_token);
    updated["claudeAiOauth"]["refreshToken"] = serde_json::json!(refresh_token);
    updated["claudeAiOauth"]["expiresAt"] = serde_json::json!(expires_at);

    let serialized = match serde_json::to_string(&updated) {
        Ok(s) => s,
        Err(_) => return,
    };

    match &creds.storage {
        ClaudeCredsStorage::File(path) => {
            let _ = fs::write(path, serialized);
        }
        #[cfg(target_os = "macos")]
        ClaudeCredsStorage::Keychain { service } => {
            let account = std::env::var("USER").unwrap_or_else(|_| "claude".into());
            let _ = Command::new("security")
                .args([
                    "add-generic-password",
                    "-U",
                    "-a",
                    &account,
                    "-s",
                    service,
                    "-w",
                    &serialized,
                ])
                .output();
        }
    }
}

async fn claude_api_call(token: &str) -> Result<serde_json::Value, reqwest::StatusCode> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|_| reqwest::StatusCode::INTERNAL_SERVER_ERROR)?;

    let status = resp.status();
    if !status.is_success() {
        return Err(status);
    }

    resp.json()
        .await
        .map_err(|_| reqwest::StatusCode::INTERNAL_SERVER_ERROR)
}

async fn fetch_claude_usage() -> ServiceResult {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return ServiceResult::Error {
                message: "Cannot find home directory".into(),
            }
        }
    };

    let creds = match load_claude_creds(&home) {
        Some(creds) => creds,
        None => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Run: claude login".into(),
            }
        }
    };

    // Try with current token
    match claude_api_call(&creds.access_token).await {
        Ok(body) => return ServiceResult::Ok(parse_claude_response(&body)),
        Err(status)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
        {
            // Refresh and retry
        }
        Err(e) => {
            return ServiceResult::Error {
                message: format!("API error: {}", e),
            }
        }
    }

    // Refresh token
    let token_resp = match refresh_claude_token(&creds.refresh_token).await {
        Ok(t) => t,
        Err(_) => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Session expired. Run: claude login".into(),
            }
        }
    };
    save_claude_creds(
        &creds,
        &token_resp.access_token,
        &token_resp.refresh_token,
        token_resp.expires_in,
    );

    match claude_api_call(&token_resp.access_token).await {
        Ok(body) => ServiceResult::Ok(parse_claude_response(&body)),
        Err(e) => ServiceResult::Error {
            message: format!("API error: {}", e),
        },
    }
}

fn parse_claude_response(body: &serde_json::Value) -> UsageData {
    UsageData {
        five_hour: body["five_hour"]["utilization"].as_f64().unwrap_or(0.0),
        five_hour_resets_at: body["five_hour"]["resets_at"].as_str().map(String::from),
        weekly: body["seven_day"]["utilization"].as_f64().unwrap_or(0.0),
        weekly_resets_at: body["seven_day"]["resets_at"].as_str().map(String::from),
    }
}

// --- Helpers ---

fn unix_to_iso(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn position_window_near_tray(window: &tauri::WebviewWindow, tray_rect: &Rect) {
    let window_size = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(400, 380));
    let tray_position = tray_rect.position.to_physical::<f64>(1.0);
    let tray_size = tray_rect.size.to_physical::<u32>(1.0);

    let monitor = window
        .monitor_from_point(tray_position.x, tray_position.y)
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        let x = (tray_position.x - (window_size.width as f64 / 2.0)).round() as i32;
        let y = tray_position.y.round() as i32 + tray_size.height as i32 + 8;
        let _ = window.set_position(PhysicalPosition::new(x, y));
        return;
    };

    let work_area = monitor.work_area();
    let work_x = work_area.position.x;
    let work_y = work_area.position.y;
    let work_width = work_area.size.width as i32;
    let work_height = work_area.size.height as i32;
    let window_width = window_size.width as i32;
    let window_height = window_size.height as i32;
    let padding = 8;

    let icon_center_x = tray_position.x + (tray_size.width as f64 / 2.0);
    let icon_center_y = tray_position.y + (tray_size.height as f64 / 2.0);
    let work_center_y = work_y as f64 + (work_height as f64 / 2.0);

    let mut x = (icon_center_x - (window_width as f64 / 2.0)).round() as i32;
    let mut y = if icon_center_y <= work_center_y {
        (tray_position.y + tray_size.height as f64).round() as i32 + padding
    } else {
        tray_position.y.round() as i32 - window_height - padding
    };

    let min_x = work_x + padding;
    let max_x = work_x + work_width - window_width - padding;
    let min_y = work_y + padding;
    let max_y = work_y + work_height - window_height - padding;

    x = if max_x < min_x {
        work_x
    } else {
        x.clamp(min_x, max_x)
    };

    y = if max_y < min_y {
        work_y
    } else {
        y.clamp(min_y, max_y)
    };

    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[cfg(target_os = "macos")]
fn set_popup_space_visibility_webview(window: &tauri::WebviewWindow, visible: bool) {
    let _ = window.set_visible_on_all_workspaces(visible);
}

#[cfg(not(target_os = "macos"))]
fn set_popup_space_visibility_webview(_window: &tauri::WebviewWindow, _visible: bool) {}

#[cfg(target_os = "macos")]
fn set_popup_space_visibility_window(window: &tauri::Window, visible: bool) {
    let _ = window.set_visible_on_all_workspaces(visible);
}

#[cfg(not(target_os = "macos"))]
fn set_popup_space_visibility_window(_window: &tauri::Window, _visible: bool) {}

// --- Codex ---

async fn fetch_codex_usage() -> ServiceResult {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return ServiceResult::Error {
                message: "Cannot find home directory".into(),
            }
        }
    };

    let auth_path = home.join(".codex").join("auth.json");
    let auth_str = match fs::read_to_string(&auth_path) {
        Ok(s) => s,
        Err(_) => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Run: codex --login".into(),
            }
        }
    };

    let auth: serde_json::Value = match serde_json::from_str(&auth_str) {
        Ok(a) => a,
        Err(_) => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Run: codex --login".into(),
            }
        }
    };

    let token = match auth["tokens"]["access_token"].as_str() {
        Some(t) => t,
        None => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Run: codex --login".into(),
            }
        }
    };
    let account_id = match auth["tokens"]["account_id"].as_str() {
        Some(id) => id,
        None => {
            return ServiceResult::NotLoggedIn {
                login_hint: "Run: codex --login".into(),
            }
        }
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("ChatGPT-Account-Id", account_id)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return ServiceResult::Error {
                message: format!("Request failed: {}", e),
            }
        }
    };

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        return ServiceResult::NotLoggedIn {
            login_hint: "Session expired. Run: codex --login".into(),
        };
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            return ServiceResult::Error {
                message: format!("Invalid response: {}", e),
            }
        }
    };

    let five_hour_reset = body["rate_limit"]["primary_window"]["reset_at"]
        .as_u64()
        .map(unix_to_iso);
    let weekly_reset = body["rate_limit"]["secondary_window"]["reset_at"]
        .as_u64()
        .map(unix_to_iso);

    ServiceResult::Ok(UsageData {
        five_hour: body["rate_limit"]["primary_window"]["used_percent"]
            .as_f64()
            .unwrap_or(0.0),
        five_hour_resets_at: five_hour_reset,
        weekly: body["rate_limit"]["secondary_window"]["used_percent"]
            .as_f64()
            .unwrap_or(0.0),
        weekly_resets_at: weekly_reset,
    })
}

// --- App ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            claude_cache: Mutex::new(None),
            codex_cache: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![get_usage])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Hide the main window on startup
            let window = app.get_webview_window("main").unwrap();
            window.hide()?;

            // Create system tray icon (embedded at compile time)
            let tray_icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;

            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            let win = window.clone();
            TrayIconBuilder::new()
                .icon(tray_icon)
                .tooltip("codemeter")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(move |_tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        rect,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                            set_popup_space_visibility_webview(&win, false);
                        } else {
                            set_popup_space_visibility_webview(&win, true);
                            position_window_near_tray(&win, &rect);
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                set_popup_space_visibility_window(window, false);
            } else if let WindowEvent::Focused(false) = event {
                let _ = window.hide();
                set_popup_space_visibility_window(window, false);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
