use serde::{Deserialize, Serialize};
use std::fs;
use std::future::Future;
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
pub struct UsageWindow {
    label: String,
    percent: f64,
    resets_at: Option<String>,
}

impl UsageWindow {
    fn new(label: &str, percent: f64, resets_at: Option<String>) -> Self {
        Self {
            label: label.to_string(),
            percent,
            resets_at,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct UsageData {
    windows: Vec<UsageWindow>,
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
    opencode: ServiceResult,
    grok: ServiceResult,
}

struct CachedResult {
    data: ServiceResult,
    fetched_at: Instant,
}

type Cache = Mutex<Option<CachedResult>>;

struct AppState {
    claude_cache: Cache,
    codex_cache: Cache,
    opencode_cache: Cache,
    grok_cache: Cache,
}

const CACHE_SECS: u64 = 300; // 5 minutes

#[tauri::command]
async fn get_usage(state: tauri::State<'_, AppState>) -> Result<AllUsage, ()> {
    let (claude, codex, opencode, grok) = tokio::join!(
        cached(&state.claude_cache, fetch_claude_usage()),
        cached(&state.codex_cache, fetch_codex_usage()),
        cached(&state.opencode_cache, fetch_opencode_usage()),
        cached(&state.grok_cache, fetch_grok_usage()),
    );
    Ok(AllUsage {
        claude,
        codex,
        opencode,
        grok,
    })
}

async fn cached<F>(cache: &Cache, fetch: F) -> ServiceResult
where
    F: Future<Output = ServiceResult>,
{
    {
        let cache = cache.lock().unwrap();
        if let Some(ref c) = *cache {
            if c.fetched_at.elapsed().as_secs() < CACHE_SECS {
                return c.data.clone();
            }
        }
    }

    let result = fetch.await;
    let mut cache = cache.lock().unwrap();
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
        windows: vec![
            UsageWindow::new(
                "5h limit",
                body["five_hour"]["utilization"].as_f64().unwrap_or(0.0),
                body["five_hour"]["resets_at"].as_str().map(String::from),
            ),
            UsageWindow::new(
                "Weekly limit",
                body["seven_day"]["utilization"].as_f64().unwrap_or(0.0),
                body["seven_day"]["resets_at"].as_str().map(String::from),
            ),
        ],
    }
}

// --- Helpers ---

fn unix_to_iso(ts: u64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// Normalize any RFC3339 timestamp (e.g. `+00:00` offsets) to a canonical UTC string.
fn normalize_iso(s: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
}

fn env_path(var: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

fn position_window_near_tray(window: &tauri::WebviewWindow, tray_rect: &Rect) {
    let window_size = window
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(640, 440));
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

    let primary = &body["rate_limit"]["primary_window"];
    let secondary = &body["rate_limit"]["secondary_window"];

    ServiceResult::Ok(UsageData {
        windows: vec![
            UsageWindow::new(
                "5h limit",
                primary["used_percent"].as_f64().unwrap_or(0.0),
                primary["reset_at"].as_u64().map(unix_to_iso),
            ),
            UsageWindow::new(
                "Weekly limit",
                secondary["used_percent"].as_f64().unwrap_or(0.0),
                secondary["reset_at"].as_u64().map(unix_to_iso),
            ),
        ],
    })
}

// --- OpenCode Go ---

const OPENCODE_GO_USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
const OPENCODE_LOGIN_HINT: &str = "Run: opencode auth login (select OpenCode Go)";

fn opencode_auth_path(home: &std::path::Path) -> std::path::PathBuf {
    let data_dir =
        env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"));
    data_dir.join("opencode").join("auth.json")
}

async fn fetch_opencode_usage() -> ServiceResult {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return ServiceResult::Error {
                message: "Cannot find home directory".into(),
            }
        }
    };

    let auth: serde_json::Value = match fs::read_to_string(opencode_auth_path(&home))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(a) => a,
        None => {
            return ServiceResult::NotLoggedIn {
                login_hint: OPENCODE_LOGIN_HINT.into(),
            }
        }
    };

    let key = match auth["opencode-go"]["key"].as_str() {
        Some(k) if !k.is_empty() => k,
        _ => {
            return ServiceResult::NotLoggedIn {
                login_hint: OPENCODE_LOGIN_HINT.into(),
            }
        }
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get(OPENCODE_GO_USAGE_URL)
        .header("Authorization", format!("Bearer {}", key))
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

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return ServiceResult::NotLoggedIn {
            login_hint: format!("Invalid API key. {}", OPENCODE_LOGIN_HINT),
        };
    }
    if !status.is_success() {
        return ServiceResult::Error {
            message: format!("API error: {}", status),
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

    let usage = &body["usage"];
    let windows: Vec<UsageWindow> = [
        ("rolling", "5h limit"),
        ("weekly", "Weekly limit"),
        ("monthly", "Monthly limit"),
    ]
    .iter()
    .filter(|(k, _)| usage[*k].is_object())
    .map(|(k, label)| {
        UsageWindow::new(
            label,
            usage[*k]["percent"].as_f64().unwrap_or(0.0),
            usage[*k]["resetsAt"].as_str().and_then(normalize_iso),
        )
    })
    .collect();

    if windows.is_empty() {
        return ServiceResult::Error {
            message: "No usage data in response".into(),
        };
    }

    ServiceResult::Ok(UsageData { windows })
}

// --- Grok ---

const GROK_BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const GROK_DEFAULT_ISSUER: &str = "https://auth.x.ai";
const GROK_LOGIN_HINT: &str = "Run: grok login";
const GROK_DEFAULT_TOKEN_TTL_SECS: i64 = 6 * 60 * 60;
const GROK_LOCK_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone)]
struct GrokCreds {
    /// Key of this entry inside the top-level `auth.json` object.
    entry_key: String,
    key: String,
    refresh_token: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    issuer: String,
    client_id: Option<String>,
    principal_type: Option<String>,
    principal_id: Option<String>,
}

#[derive(Deserialize)]
struct GrokTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

fn grok_home() -> Option<std::path::PathBuf> {
    env_path("GROK_HOME").or_else(|| dirs::home_dir().map(|h| h.join(".grok")))
}

fn parse_grok_creds(raw: &str) -> Option<GrokCreds> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = value.as_object()?;

    let mut fallback: Option<GrokCreds> = None;
    for (entry_key, entry) in obj {
        let key = match entry["key"].as_str() {
            Some(k) if !k.is_empty() => k.to_string(),
            _ => continue,
        };
        let as_string = |field: &str| entry[field].as_str().map(String::from);
        let creds = GrokCreds {
            entry_key: entry_key.clone(),
            key,
            refresh_token: as_string("refresh_token"),
            expires_at: entry["expires_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc)),
            issuer: as_string("oidc_issuer").unwrap_or_else(|| GROK_DEFAULT_ISSUER.into()),
            client_id: as_string("oidc_client_id"),
            principal_type: as_string("principal_type"),
            principal_id: as_string("principal_id"),
        };
        // Prefer an entry we can actually refresh.
        if creds.refresh_token.is_some() {
            return Some(creds);
        }
        if fallback.is_none() {
            fallback = Some(creds);
        }
    }
    fallback
}

fn grok_token_expired(creds: &GrokCreds) -> bool {
    match creds.expires_at {
        // Treat tokens expiring within the next minute as expired.
        Some(exp) => exp <= chrono::Utc::now() + chrono::Duration::seconds(60),
        None => false,
    }
}

/// Acquire the same advisory lock the Grok CLI uses (`auth.json.lock`) so we never
/// race it on a refresh-token rotation. Polls so a wedged holder can't block forever.
async fn lock_grok_auth(auth_path: &std::path::Path) -> Result<fs::File, String> {
    use fs2::FileExt;

    let lock_path = auth_path.with_file_name("auth.json.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|e| format!("Cannot open lock file: {}", e))?;

    let deadline = Instant::now() + std::time::Duration::from_millis(GROK_LOCK_TIMEOUT_MS);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => return Err(format!("Timed out waiting for auth lock: {}", e)),
        }
    }
}

fn save_grok_creds(
    auth_path: &std::path::Path,
    raw: &str,
    entry_key: &str,
    token: &GrokTokenResponse,
) -> Result<(), String> {
    let mut value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Invalid auth.json: {}", e))?;
    let entry = value
        .get_mut(entry_key)
        .ok_or_else(|| "Auth entry disappeared".to_string())?;

    let now = chrono::Utc::now();
    let expires_at = now
        + chrono::Duration::seconds(token.expires_in.unwrap_or(GROK_DEFAULT_TOKEN_TTL_SECS));
    entry["key"] = serde_json::json!(token.access_token);
    entry["create_time"] =
        serde_json::json!(now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true));
    entry["expires_at"] =
        serde_json::json!(expires_at.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true));
    if let Some(rt) = &token.refresh_token {
        entry["refresh_token"] = serde_json::json!(rt);
    }

    let serialized = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;

    // Atomic replace, mirroring the CLI's own `auth.json.<pid>.tmp` pattern.
    let tmp_path = auth_path.with_file_name(format!("auth.json.{}.tmp", std::process::id()));
    fs::write(&tmp_path, serialized).map_err(|e| format!("Cannot write auth.json: {}", e))?;
    fs::rename(&tmp_path, auth_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Cannot replace auth.json: {}", e)
    })
}

/// Refresh the Grok OIDC session and persist it. Returns the new access token.
async fn refresh_grok_token(
    auth_path: &std::path::Path,
    creds: &GrokCreds,
) -> Result<String, String> {
    let refresh_token = creds
        .refresh_token
        .clone()
        .ok_or_else(|| "No refresh token".to_string())?;

    let lock = lock_grok_auth(auth_path).await?;
    let result = refresh_grok_token_locked(auth_path, creds, &refresh_token).await;
    let _ = fs2::FileExt::unlock(&lock);
    result
}

async fn refresh_grok_token_locked(
    auth_path: &std::path::Path,
    creds: &GrokCreds,
    refresh_token: &str,
) -> Result<String, String> {
    // Re-read under the lock: the CLI may have rotated the token in the meantime.
    let raw =
        fs::read_to_string(auth_path).map_err(|e| format!("Cannot read auth.json: {}", e))?;
    if let Some(fresh) = parse_grok_creds(&raw) {
        if fresh.key != creds.key && !grok_token_expired(&fresh) {
            return Ok(fresh.key);
        }
    }

    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if let Some(cid) = &creds.client_id {
        params.push(("client_id", cid));
    }
    if let Some(pt) = &creds.principal_type {
        params.push(("principal_type", pt));
    }
    if let Some(pid) = &creds.principal_id {
        params.push(("principal_id", pid));
    }

    let token_url = format!("{}/oauth2/token", creds.issuer.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .post(&token_url)
        .form(&params)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Refresh failed: {}", resp.status()));
    }

    let token: GrokTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Invalid refresh response: {}", e))?;

    save_grok_creds(auth_path, &raw, &creds.entry_key, &token)?;
    Ok(token.access_token)
}

async fn grok_api_call(token: &str) -> Result<serde_json::Value, reqwest::StatusCode> {
    let client = reqwest::Client::new();
    let resp = client
        .get(GROK_BILLING_URL)
        .header("Authorization", format!("Bearer {}", token))
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

fn parse_grok_response(body: &serde_json::Value) -> UsageData {
    let cfg = &body["config"];

    // Mirrors the CLI: prefer `creditUsagePercent`, fall back to legacy used/monthlyLimit.
    let percent = cfg["creditUsagePercent"]
        .as_f64()
        .or_else(|| {
            let limit = cfg["monthlyLimit"]["val"].as_f64()?;
            let used = cfg["used"]["val"].as_f64()?;
            (limit > 0.0).then(|| used / limit * 100.0)
        })
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);

    let resets_at = cfg["currentPeriod"]["end"]
        .as_str()
        .or_else(|| cfg["billingPeriodEnd"].as_str())
        .and_then(normalize_iso);

    let label = match cfg["currentPeriod"]["type"].as_str() {
        Some(t) if t.contains("WEEKLY") => "Weekly limit",
        Some(t) if t.contains("MONTHLY") => "Monthly limit",
        _ => "Usage",
    };

    UsageData {
        windows: vec![UsageWindow::new(label, percent, resets_at)],
    }
}

async fn fetch_grok_usage() -> ServiceResult {
    let auth_path = match grok_home() {
        Some(h) => h.join("auth.json"),
        None => {
            return ServiceResult::Error {
                message: "Cannot find home directory".into(),
            }
        }
    };

    let creds = match fs::read_to_string(&auth_path)
        .ok()
        .and_then(|raw| parse_grok_creds(&raw))
    {
        Some(c) => c,
        None => {
            return ServiceResult::NotLoggedIn {
                login_hint: GROK_LOGIN_HINT.into(),
            }
        }
    };

    let expired_hint = || ServiceResult::NotLoggedIn {
        login_hint: format!("Session expired. {}", GROK_LOGIN_HINT),
    };

    // Refresh up front if we already know the token is stale.
    let mut token = creds.key.clone();
    if grok_token_expired(&creds) {
        token = match refresh_grok_token(&auth_path, &creds).await {
            Ok(t) => t,
            Err(_) => return expired_hint(),
        };
    }

    match grok_api_call(&token).await {
        Ok(body) => return ServiceResult::Ok(parse_grok_response(&body)),
        Err(status)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN =>
        {
            // Refresh and retry
        }
        Err(e) => {
            return ServiceResult::Error {
                message: format!("API error: {}", e),
            }
        }
    }

    let token = match refresh_grok_token(&auth_path, &creds).await {
        Ok(t) => t,
        Err(_) => return expired_hint(),
    };

    match grok_api_call(&token).await {
        Ok(body) => ServiceResult::Ok(parse_grok_response(&body)),
        Err(status)
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN =>
        {
            expired_hint()
        }
        Err(e) => ServiceResult::Error {
            message: format!("API error: {}", e),
        },
    }
}

// --- App ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            claude_cache: Mutex::new(None),
            codex_cache: Mutex::new(None),
            opencode_cache: Mutex::new(None),
            grok_cache: Mutex::new(None),
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
                        button: tauri::tray::MouseButton::Left,
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
            } else if cfg!(target_os = "macos") {
                if let WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                    set_popup_space_visibility_window(window, false);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
