use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use std::time::Instant;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
    WindowEvent,
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
    let (claude, codex) = tokio::join!(
        fetch_claude_cached(&state),
        fetch_codex_cached(&state)
    );
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

#[derive(Deserialize)]
struct ClaudeCreds {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: ClaudeOauth,
}

#[derive(Deserialize)]
struct ClaudeOauth {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
}

#[derive(Deserialize)]
struct ClaudeTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
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

fn save_claude_creds(home: &std::path::Path, access_token: &str, refresh_token: &str, expires_in: u64) {
    let creds_path = home.join(".claude").join(".credentials.json");
    if let Ok(creds_str) = fs::read_to_string(&creds_path) {
        if let Ok(mut creds) = serde_json::from_str::<serde_json::Value>(&creds_str) {
            let expires_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + expires_in * 1000;
            creds["claudeAiOauth"]["accessToken"] = serde_json::json!(access_token);
            creds["claudeAiOauth"]["refreshToken"] = serde_json::json!(refresh_token);
            creds["claudeAiOauth"]["expiresAt"] = serde_json::json!(expires_at);
            let _ = fs::write(&creds_path, serde_json::to_string(&creds).unwrap());
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

    resp.json().await.map_err(|_| reqwest::StatusCode::INTERNAL_SERVER_ERROR)
}

async fn fetch_claude_usage() -> ServiceResult {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return ServiceResult::Error { message: "Cannot find home directory".into() },
    };

    let creds_path = home.join(".claude").join(".credentials.json");
    let creds_str = match fs::read_to_string(&creds_path) {
        Ok(s) => s,
        Err(_) => return ServiceResult::NotLoggedIn { login_hint: "Run: claude login".into() },
    };

    let creds: ClaudeCreds = match serde_json::from_str(&creds_str) {
        Ok(c) => c,
        Err(_) => return ServiceResult::NotLoggedIn { login_hint: "Run: claude login".into() },
    };

    // Try with current token
    match claude_api_call(&creds.claude_ai_oauth.access_token).await {
        Ok(body) => return ServiceResult::Ok(parse_claude_response(&body)),
        Err(status) if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            // Refresh and retry
        }
        Err(e) => return ServiceResult::Error { message: format!("API error: {}", e) },
    }

    // Refresh token
    let token_resp = match refresh_claude_token(&creds.claude_ai_oauth.refresh_token).await {
        Ok(t) => t,
        Err(_) => return ServiceResult::NotLoggedIn { login_hint: "Session expired. Run: claude login".into() },
    };
    save_claude_creds(&home, &token_resp.access_token, &token_resp.refresh_token, token_resp.expires_in);

    match claude_api_call(&token_resp.access_token).await {
        Ok(body) => ServiceResult::Ok(parse_claude_response(&body)),
        Err(e) => ServiceResult::Error { message: format!("API error: {}", e) },
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

// --- Codex ---

async fn fetch_codex_usage() -> ServiceResult {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return ServiceResult::Error { message: "Cannot find home directory".into() },
    };

    let auth_path = home.join(".codex").join("auth.json");
    let auth_str = match fs::read_to_string(&auth_path) {
        Ok(s) => s,
        Err(_) => return ServiceResult::NotLoggedIn { login_hint: "Run: codex --login".into() },
    };

    let auth: serde_json::Value = match serde_json::from_str(&auth_str) {
        Ok(a) => a,
        Err(_) => return ServiceResult::NotLoggedIn { login_hint: "Run: codex --login".into() },
    };

    let token = match auth["tokens"]["access_token"].as_str() {
        Some(t) => t,
        None => return ServiceResult::NotLoggedIn { login_hint: "Run: codex --login".into() },
    };
    let account_id = match auth["tokens"]["account_id"].as_str() {
        Some(id) => id,
        None => return ServiceResult::NotLoggedIn { login_hint: "Run: codex --login".into() },
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
        Err(e) => return ServiceResult::Error { message: format!("Request failed: {}", e) },
    };

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED || resp.status() == reqwest::StatusCode::FORBIDDEN {
        return ServiceResult::NotLoggedIn { login_hint: "Session expired. Run: codex --login".into() };
    }

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return ServiceResult::Error { message: format!("Invalid response: {}", e) },
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

            // Create system tray icon
            let tray_icon = Image::from_path("icons/icon.png")
                .or_else(|_| Image::from_path("public/icon.png"))?;

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
                    if let tauri::tray::TrayIconEvent::Click { button_state: tauri::tray::MouseButtonState::Up, .. } = event {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
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
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
