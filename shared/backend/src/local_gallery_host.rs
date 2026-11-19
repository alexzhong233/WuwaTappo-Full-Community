//! Local HTTP bridge shared by the desktop clients.
use serde_json::{json, Value};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, TryLockError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[cfg(windows)]
use std::{os::windows::process::CommandExt, process::Command};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use wuwatappo_assistant_core::{custom_resources, AssistantCore};

// 原生菜单与开机自启只有 Windows 有实现；其他平台保留同名入口，应答明确的错误而不是 404，
// 免得前端把「平台不支持」和「接口不存在」混成一种失败。
#[cfg(windows)]
mod windows_menu;
#[cfg(windows)]
mod windows_startup;

const MAX_BODY: usize = 64 * 1024;
const MAX_CUSTOM_BODY: usize = 16 * 1024;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const ALLOWED_ORIGINS: [&str; 3] = [
    "http://tauri.localhost",
    "https://tauri.localhost",
    "tauri://localhost",
];

struct App {
    root: PathBuf,
    config: Mutex<Value>,
    token: String,
    assistant: Mutex<AssistantCore>,
    offline: Mutex<bool>,
    custom_error: Mutex<Option<String>>,
    sync: Mutex<Value>,
    focus_probe: Mutex<Option<(Instant, Value)>>,
}

#[cfg(windows)]
#[repr(C)]
struct LastInputInfo {
    cb_size: u32,
    dw_time: u32,
}

/// `GUITHREADINFO`：桌面输入法、输入增强工具判「是否正在打字」用的就是它。
/// `hwnd_caret` 非空意味着目标线程当前有插入符，等价于处于文本输入上下文。
/// HWND 统一用 `isize`，与 `windows_menu.rs` 里同一批函数保持一致——
/// 同名 FFI 声明两种签名会让调用约定对不上。
#[cfg(windows)]
#[repr(C)]
struct GuiThreadInfo {
    cb_size: u32,
    flags: u32,
    hwnd_active: isize,
    hwnd_focus: isize,
    hwnd_capture: isize,
    hwnd_menu_owner: isize,
    hwnd_move_size: isize,
    hwnd_caret: isize,
    rc_caret: [i32; 4],
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> isize;
    fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
    fn GetGUIThreadInfo(thread: u32, info: *mut GuiThreadInfo) -> i32;
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetTickCount() -> u32;
}

fn physical_quiet() -> bool {
    #[cfg(windows)]
    unsafe {
        let mut info = LastInputInfo {
            cb_size: std::mem::size_of::<LastInputInfo>() as u32,
            dw_time: 0,
        };
        if GetLastInputInfo(&mut info) == 0 || GetTickCount().wrapping_sub(info.dw_time) < 250 {
            return false;
        }
        return !(1..=254).any(|key| GetAsyncKeyState(key) & (i16::MIN) != 0);
    }
    #[cfg(not(windows))]
    false
}

/// 快判：前台线程当前有没有插入符。
///
/// 输入法、输入增强工具判「是否正在打字」用的就是这个——`GetGUIThreadInfo` 是
/// 微秒级的本机调用，不建进程，覆盖记事本、浏览器地址栏、大多数原生聊天框。
#[cfg(windows)]
fn caret_context() -> Option<Value> {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground == 0 {
            return Some(json!({"editable":false,"source":"no-foreground"}));
        }
        let mut pid = 0u32;
        let thread = GetWindowThreadProcessId(foreground, &mut pid);
        // 自己的窗口（桌宠、控制台里的输入框）不是打字场景，否则会自己触发自己。
        if pid == std::process::id() {
            return Some(json!({"editable":false,"source":"own-window"}));
        }
        let mut info: GuiThreadInfo = std::mem::zeroed();
        info.cb_size = std::mem::size_of::<GuiThreadInfo>() as u32;
        if GetGUIThreadInfo(thread, &mut info) != 0 && info.hwnd_caret != 0 {
            return Some(json!({"editable":true,"source":"win32-caret"}));
        }
    }
    None
}

/// 兜底：WebView2 / Electron 这类自绘控件拿不到 caret，只能问 UIA。
///
/// 每次探测都要起一个 PowerShell，冷启动就要 200ms 以上，所以超时必须给足。
/// 之前写的是 300ms，判定永远超时，表情从不触发。
#[cfg(windows)]
fn uia_typing_context() -> Value {
    // 用 Pattern 判定而不是一堆硬编码的属性 ID：ValuePattern/TextPattern 才是
    // 「这个控件能不能输入文字」的权威答案，属性 ID 会随 UIA 版本变，取错就抛 MethodException。
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop';try{Add-Type -AssemblyName UIAutomationClient;[void][System.Reflection.Assembly]::LoadWithPartialName('UIAutomationTypes');$e=[System.Windows.Automation.AutomationElement]::FocusedElement;if($null -eq $e){@{editable=$false;source='uia';reason='no-focus'}|ConvertTo-Json -Compress;exit};$type=$e.Current.ControlType.ProgrammaticName;$p=$null;if($e.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern,[ref]$p)){$vp=[System.Windows.Automation.ValuePattern]$p;if(-not $vp.Current.IsReadOnly){@{editable=$true;source='uia-value';control=$type}|ConvertTo-Json -Compress;exit}};$p=$null;if($e.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern,[ref]$p)){@{editable=$true;source='uia-text';control=$type}|ConvertTo-Json -Compress;exit};@{editable=$false;source='uia';reason='no-editable-pattern';control=$type}|ConvertTo-Json -Compress}catch{@{editable=$false;source='uia-unavailable';error=($_.Exception.GetType().Name+': '+$_.Exception.Message)}|ConvertTo-Json -Compress}"#;
    let spawned = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        return json!({"editable":false,"source":"uia-unavailable","error":"PowerShellUnavailable"});
    };
    let deadline = Instant::now() + Duration::from_millis(2500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({"editable":false,"source":"uia-timeout"});
            }
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(_) => {
            return json!({"editable":false,"source":"uia-unavailable","error":"ProbeFailed"})
        }
    };
    serde_json::from_slice(&output.stdout).unwrap_or_else(
        |_| json!({"editable":false,"source":"uia-unavailable","error":"InvalidProbeResult"}),
    )
}

fn native_typing_context() -> Value {
    #[cfg(windows)]
    {
        if let Some(value) = caret_context() {
            return value;
        }
        return uia_typing_context();
    }
    #[cfg(not(windows))]
    json!({"editable":false,"source":"native-adapter"})
}

fn typing_context(app: &App) -> Value {
    if app
        .assistant
        .lock()
        .unwrap()
        .saved()
        .pointer("/settings/auto_emote")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return json!({"editable":false,"source":"disabled"});
    }
    let mut cached = match app.focus_probe.try_lock() {
        Ok(value) => value,
        Err(TryLockError::WouldBlock) => return json!({"editable":false,"source":"uia-busy"}),
        Err(TryLockError::Poisoned(_)) => {
            return json!({"editable":false,"source":"uia-unavailable"})
        }
    };
    if let Some((at, value)) = cached.as_ref() {
        // UIA 兜底一次要几百毫秒到两秒多，缓存太短会变成连续打字时反复探测，
        // 把宿主线程占住；三秒内复用同一结果。
        if at.elapsed() < Duration::from_millis(3000) {
            return value.clone();
        }
    }
    let value = native_typing_context();
    *cached = Some((Instant::now(), value.clone()));
    value
}

fn community_update(app: &App) -> Value {
    let configured = config_value(app, "community_version");
    let current = configured
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            config_value(app, "mod_version")
                .as_str()
                .unwrap_or("0.0.0")
                .to_owned()
        });
    let include_prerelease = app
        .assistant
        .lock()
        .unwrap()
        .saved()
        .pointer("/settings/prerelease_updates")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match wuwatappo_assistant_core::release_updates::check_community(&current, include_prerelease) {
        Ok(value) => value,
        Err(error) => json!({
            "current":current,"latest":"","updateAvailable":false,"checked_at":now(),
            "error":error,"message":format!("检查更新失败：{error}"),
            "releaseUrl":"https://github.com/alexzhong233/WuwaTappo-Full-Community/releases",
            "pageUrl":"https://github.com/alexzhong233/WuwaTappo-Full-Community/releases"
        }),
    }
}

fn official_update(app: &App) -> Value {
    let region = config_value(app, "region")
        .as_str()
        .unwrap_or("cn")
        .to_owned();
    let current = config_value(app, "client_version")
        .as_str()
        .unwrap_or("")
        .to_owned();
    match wuwatappo_assistant_core::release_updates::check_official(&region, &current) {
        Ok(value) => value,
        Err(error) => json!({
            "currentVersion":current,"latestVersion":"","updateAvailable":false,
            "checked_at":now(),"error":error,"message":format!("检查失败：{error}"),
            "communityReleasesUrl":"https://github.com/alexzhong233/WuwaTappo-Full-Community/releases"
        }),
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn request_origin(request: &tiny_http::Request) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Origin"))
        .map(|header| header.value.as_str().to_owned())
}

fn response_with_type(
    request: tiny_http::Request,
    status: u16,
    bytes: Vec<u8>,
    content_type: &str,
) {
    let origin = request_origin(&request);
    let mut output = Response::from_data(bytes).with_status_code(StatusCode(status));
    output.add_header(Header::from_bytes("Content-Type", content_type).unwrap());
    output.add_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    output.add_header(
        Header::from_bytes(
            "Access-Control-Allow-Headers",
            "Content-Type, X-Local-Token",
        )
        .unwrap(),
    );
    output.add_header(
        Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS").unwrap(),
    );
    output.add_header(Header::from_bytes("Access-Control-Allow-Private-Network", "true").unwrap());
    output.add_header(Header::from_bytes("Cross-Origin-Resource-Policy", "cross-origin").unwrap());
    if let Some(origin) = origin.filter(|value| ALLOWED_ORIGINS.contains(&value.as_str())) {
        output.add_header(Header::from_bytes("Access-Control-Allow-Origin", origin).unwrap());
        output.add_header(Header::from_bytes("Vary", "Origin").unwrap());
    }
    let _ = request.respond(output);
}

fn response(request: tiny_http::Request, status: u16, value: Value) {
    response_with_type(
        request,
        status,
        value.to_string().into_bytes(),
        "application/json; charset=utf-8",
    );
}

fn read_json(request: &mut tiny_http::Request, max: usize) -> Result<Value, String> {
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max {
        return Err("Request too large".into());
    }
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Invalid JSON: {error}"))
}

fn origin_ok(request: &tiny_http::Request) -> bool {
    request_origin(request)
        .map(|origin| ALLOWED_ORIGINS.contains(&origin.as_str()))
        .unwrap_or(true)
}

fn token_ok(app: &App, request: &tiny_http::Request) -> bool {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Local-Token"))
        .is_some_and(|header| header.value.as_str() == app.token)
}

fn save_assistant(app: &App) {
    let state = app.assistant.lock().unwrap().saved();
    let path = app.root.join("assistant_state.json");
    let temp = path.with_extension("tmp");
    if let Ok(bytes) = serde_json::to_vec(&state) {
        if fs::write(&temp, bytes).is_ok() {
            let _ = fs::rename(temp, path);
        }
    }
}

fn path_of(request: &tiny_http::Request) -> String {
    request.url().split('?').next().unwrap_or("/").to_string()
}

fn read_value(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn cache_dir(app: &App) -> PathBuf {
    app.root.join("cache")
}

fn config_value(app: &App, key: &str) -> Value {
    app.config
        .lock()
        .unwrap()
        .get(key)
        .cloned()
        .unwrap_or(Value::Null)
}

fn cache_summary(app: &App) -> Value {
    let manifest = read_value(&cache_dir(app).join("asset_manifest.json")).ok();
    json!({
        "assets": manifest.as_ref().and_then(|value| value["records"].as_array()).map(Vec::len).unwrap_or(0),
        "expected": config_value(app, "asset_count").as_u64().unwrap_or(0),
    })
}

fn cache_ready(app: &App) -> bool {
    let Ok(manifest) = read_value(&cache_dir(app).join("asset_manifest.json")) else {
        return false;
    };
    let Some(records) = manifest["records"].as_array() else {
        return false;
    };
    let expected = config_value(app, "asset_count").as_u64().unwrap_or(0) as usize;
    if manifest["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
        || records.len() != expected
    {
        return false;
    }
    records.iter().all(|record| {
        let Some(name) = record["file"].as_str() else {
            return false;
        };
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            return false;
        }
        let Some(expected_size) = record["size"].as_u64() else {
            return false;
        };
        let path = cache_dir(app).join("assets").join(name);
        fs::metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected_size)
    })
}

fn region(app: &App) -> &'static str {
    let configured = config_value(app, "region");
    wuwatappo_assistant_core::regions::profile(configured.as_str().unwrap_or("cn"))
        .map(|profile| profile.region.as_str())
        .unwrap_or("CN")
}

fn custom_root(app: &App) -> PathBuf {
    app.root.join("custom-atlas")
}

fn custom_status(app: &App) -> Value {
    let enabled = *app.offline.lock().unwrap();
    let error = app.custom_error.lock().unwrap().clone();
    custom_resources::status(&custom_root(app), region(app), enabled, error).unwrap_or_else(|error| {
        json!({
            "installed": false,
            "enabled": enabled,
            "name": "",
            "atlas_count": 0,
            "asset_count": 0,
            "packs": [],
            "region": region(app),
            "error": error,
        })
    })
}
fn offline_bundle(app: &App) -> Result<Value, String> {
    let cache = cache_dir(app);
    let catalog = read_value(&cache.join("offline_catalog.json"))?;
    let atlas_doc = read_value(&cache.join("atlas.json"))?;
    let atlas = atlas_doc
        .get("data")
        .and_then(Value::as_array)
        .ok_or("图鉴缓存结构无效")?;
    let official = json!({
        "catalog": catalog,
        "atlas": atlas,
        "atlas_count": atlas.len(),
        "asset_count": config_value(app, "asset_count").as_u64().unwrap_or(0),
    });
    let (merged, error) = custom_resources::merge_or_official(&official, &custom_root(app), region(app));
    *app.custom_error.lock().unwrap() = error;
    Ok(merged)
}

fn safe_relative(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains(['\\', ':', '?', '#', '%'])
        && !raw.chars().any(char::is_control)
        && raw
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "json" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn contained_file(root: &Path, relative: &str) -> Option<PathBuf> {
    if !safe_relative(relative) {
        return None;
    }
    let base = root.canonicalize().ok()?;
    let file = root.join(relative).canonicalize().ok()?;
    (file.starts_with(base) && file.is_file()).then_some(file)
}

fn manifest_asset(app: &App, requested: &str) -> Option<PathBuf> {
    let manifest = read_value(&cache_dir(app).join("asset_manifest.json")).ok()?;
    let record = manifest["records"].as_array()?.iter().find(|record| {
        let source_name = record["source"]
            .as_str()
            .and_then(|source| source.rsplit('/').next());
        source_name == Some(requested)
    })?;
    let name = record["file"].as_str()?;
    contained_file(&cache_dir(app).join("assets"), name)
}

fn start_sync(app: &Arc<App>, force: bool) -> Value {
    let mut state = app.sync.lock().unwrap();
    if state["status"] == "running" {
        return state.clone();
    }
    if !force && now() - state["checked_at"].as_f64().unwrap_or(0.0) < 900.0 {
        return state.clone();
    }
    *state = json!({"status":"running","done":0,"total":0,"checked_at":state["checked_at"]});
    let snapshot = state.clone();
    drop(state);
    let app = Arc::clone(app);
    std::thread::spawn(move || {
        let selected_region = config_value(&app, "region")
            .as_str()
            .unwrap_or("cn")
            .to_owned();
        let progress_app = Arc::clone(&app);
        let result = wuwatappo_assistant_core::resource_cache::sync_with_progress(
            &app.root,
            &selected_region,
            force,
            move |done, total| {
                let mut guard = progress_app.sync.lock().unwrap();
                *guard = json!({"status":"running","done":done,"total":total,"checked_at":now()});
            },
        );
        let mut state = app.sync.lock().unwrap();
        *state = match result {
            Ok(summary) => {
                let mut config = app.config.lock().unwrap();
                config["asset_count"] = json!(summary.asset_count);
                config["atlas_count"] = json!(summary.atlas_count);
                let cfg_path = app.root.join("local_config.json");
                let temp = cfg_path.with_extension("tmp");
                if fs::write(&temp, serde_json::to_vec_pretty(&*config).unwrap_or_default()).is_ok()
                {
                    let _ = fs::rename(temp, cfg_path);
                }
                json!({"status":"complete","done":summary.asset_count,"total":summary.asset_count,"checked_at":now(),"result":{"changed":summary.changed,"atlas_count":summary.atlas_count,"asset_count":summary.asset_count}})
            }
            Err(error) => {
                json!({"status":"failed","done":0,"total":0,"checked_at":now(),"error":error})
            }
        };
    });
    snapshot
}

const COMMUNITY_RELEASES_URL: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

/// 官方桌宠下载页：国服 / 国际服各一个入口，跟着构建时锁定的区域走。
fn pet_site_url(app: &App) -> String {
    let configured = config_value(app, "region");
    wuwatappo_assistant_core::regions::profile(configured.as_str().unwrap_or("cn"))
        .map(|profile| profile.pet_site.clone())
        .unwrap_or_else(|| "https://download-mc-wuwatappo.kurogames-global.com/download/".into())
}

/// 地址一律由服务端决定，不接受前端传参：能传地址就等于让页面能拉起任意程序。
#[cfg(windows)]
fn open_url(url: &str) -> Value {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect()
    };
    let verb = wide("open");
    let target = wide(url);
    let code = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    // ShellExecuteW 大于 32 才算成功，小于等于 32 是 SE_ERR_* 错误码
    if code > 32 {
        json!({"ok":true,"url":url})
    } else {
        json!({"ok":false,"url":url,"error":format!("系统没有打开浏览器（错误 {code}）")})
    }
}

#[cfg(not(windows))]
fn open_url(url: &str) -> Value {
    let opened = std::process::Command::new("open")
        .arg(url)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    json!({"ok":opened,"url":url})
}

#[cfg(windows)]
fn context_menu(body: &Value) -> Result<Value, String> {
    windows_menu::popup(body.get("items").unwrap_or(&Value::Null))
}

#[cfg(not(windows))]
fn context_menu(_body: &Value) -> Result<Value, String> {
    Err("原生右键菜单只能在 Windows 上使用".into())
}

#[cfg(windows)]
fn startup_action(app: &App, action: &str) -> Result<Value, String> {
    windows_startup::run(&app.root, action)
}

#[cfg(not(windows))]
fn startup_action(_app: &App, _action: &str) -> Result<Value, String> {
    Err("开机自启托管只能在 Windows 上使用".into())
}

fn send_file_as(request: tiny_http::Request, file: Option<PathBuf>, kind: fn(&Path) -> &'static str) {
    let Some(file) = file else {
        return response(request, 404, json!({"error":"Not found"}));
    };
    match fs::read(&file) {
        Ok(bytes) => response_with_type(request, 200, bytes, kind(&file)),
        Err(_) => response(request, 404, json!({"error":"Not found"})),
    }
}

fn send_file(request: tiny_http::Request, file: Option<PathBuf>) {
    send_file_as(request, file, mime_type);
}

fn handle_get(app: &Arc<App>, request: tiny_http::Request, path: &str) {
    match path {
        "/bootstrap" => {
            if !request_origin(&request)
                .is_some_and(|origin| ALLOWED_ORIGINS.contains(&origin.as_str()))
            {
                return response(request, 403, json!({"error":"App origin required"}));
            }
            response(request, 200, json!({"token":app.token}));
        }
        "/status" => {
            let offline = *app.offline.lock().unwrap();
            let ready = cache_ready(app);
            response(
                request,
                200,
                json!({
                    "ready":ready,"enabled":offline,"mode_verified":true,"network_policy":"rooms-only",
                    "sync":{"status":"idle"},"cache":cache_summary(app),
                    "mod_version":config_value(app, "mod_version"),"client_version":config_value(app, "client_version"),
                    "asset_count":config_value(app, "asset_count"),"atlas_count":config_value(app, "atlas_count"),
                }),
            );
        }
        "/sync/status" => response(request, 200, app.sync.lock().unwrap().clone()),
        "/custom/status" => response(request, 200, custom_status(app)),
        "/offline-data" => {
            if !cache_ready(app) {
                return response(
                    request,
                    409,
                    json!({"error":"本地素材缓存不完整，请重新同步后再开启本地图鉴"}),
                );
            }
            match offline_bundle(app) {
                Ok(value) => response(request, 200, value),
                Err(error) => response(request, 409, json!({"error":error})),
            }
        }
        "/asset-base" => {
            let port = config_value(app, "port").as_u64().unwrap_or(0);
            response(
                request,
                200,
                json!({"base":format!("http://127.0.0.1:{port}/")}),
            );
        }
        _ if path.starts_with("/custom-assets/") => {
            let relative = &path["/custom-assets/".len()..];
            let mut parts = relative.splitn(2, '/');
            let pack = parts.next().unwrap_or_default();
            let asset = parts.next().unwrap_or_default();
            if !safe_relative(pack) || pack.contains('/') || !safe_relative(asset) {
                return response(request, 400, json!({"error":"Invalid path"}));
            }
            let file = custom_resources::resolve_asset_file(
                &custom_root(app),
                path.trim_start_matches('/'),
            );
            send_file_as(request, file, custom_resources::asset_mime_type);
        }
        _ if path.starts_with("/assets/") => {
            let name = &path["/assets/".len()..];
            if !safe_relative(name) || name.contains('/') {
                return response(request, 400, json!({"error":"Invalid path"}));
            }
            let file = contained_file(&cache_dir(app).join("assets"), name);
            if file.is_none() {
                // 缺图是「缓存没同步全」的主要症状，逐行记下来才能定位是哪一张。
                wuwatappo_assistant_core::append_diagnostic(
                    &app.root,
                    &json!({"kind":"asset_not_found","path":name}),
                );
            }
            send_file(request, file);
        }
        _ if [
            "/character/",
            "/accessory/",
            "/emote/",
            "/emote_atlas/",
            "/character_skin_atlas/",
            "/accessory_atlas/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix)) =>
        {
            let name = path.rsplit('/').next().unwrap_or_default();
            send_file(request, manifest_asset(app, name));
        }
        _ => response(request, 404, json!({"error":"Not found"})),
    }
}

fn handle_post(app: &Arc<App>, mut request: tiny_http::Request, path: &str) {
    if !token_ok(app, &request) {
        return response(request, 403, json!({"error":"Invalid local control token"}));
    }
    let max_body = if path.starts_with("/custom/") {
        MAX_CUSTOM_BODY
    } else {
        MAX_BODY
    };
    let body = match read_json(&mut request, max_body) {
        Ok(value) => value,
        Err(error) => return response(request, 400, json!({"error":error})),
    };
    if !body.is_object() {
        return response(request, 400, json!({"error":"Object required"}));
    }
    if path == "/assistant/context-menu" {
        // 原生菜单是阻塞调用（要等用户点完或收起），必须单开一条路由：
        // 它不该占着 Assistant 的锁，否则菜单开着时看板读不到状态。
        return match context_menu(&body) {
            Ok(value) => response(request, 200, value),
            Err(error) => response(
                request,
                400,
                json!({"action":Value::Null,"error":error}),
            ),
        };
    }
    if path.starts_with("/assistant/") {
        if path == "/assistant/typing-context" {
            return response(request, 200, typing_context(app));
        }
        let action = path.trim_start_matches("/assistant/");
        let offline = *app.offline.lock().unwrap();
        let mut core = app.assistant.lock().unwrap();
        let quiet = if action == "tick" {
            physical_quiet()
        } else {
            false
        };
        let result = core.handle(action, &body, now(), quiet, offline, false);
        drop(core);
        save_assistant(app);
        return match result {
            Ok(value) => response(request, 200, value),
            Err(error) => response(request, 400, json!({"error":error})),
        };
    }
    match path {
        "/community/version" => response(
            request,
            200,
            json!({"current":config_value(app, "community_version").as_str().unwrap_or("")}),
        ),
        "/community/update-check" => response(request, 200, community_update(app)),
        "/official/update-check" | "/official/check" => {
            response(request, 200, official_update(app))
        }
        "/official/download" => {
            let selected_region = config_value(app, "region")
                .as_str()
                .unwrap_or("cn")
                .to_owned();
            let current = config_value(app, "client_version")
                .as_str()
                .unwrap_or("")
                .to_owned();
            let download_dir = env::var_os("USERPROFILE")
                .or_else(|| env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join("Downloads"))
                .unwrap_or_else(|| app.root.join("downloads"));
            match wuwatappo_assistant_core::release_updates::download_official_installer(
                &selected_region,
                &current,
                &download_dir,
                false,
            ) {
                Ok(value) => response(request, 200, value),
                Err(error) => response(request, 400, json!({"error":error})),
            }
        }
        // 面板上的两个「打开官方页面」按钮：地址写死在服务端，页面拿不到入口就无法诱导跳转。
        "/official/releases" => response(request, 200, open_url(COMMUNITY_RELEASES_URL)),
        "/official/pet-site" => {
            let url = pet_site_url(app);
            response(request, 200, open_url(&url))
        }
        "/update/status" | "/update/check" => {
            let check = community_update(app);
            let latest = check["latest"].as_str().unwrap_or("").to_owned();
            let available = check["updateAvailable"].as_bool().unwrap_or(false);
            let message = if available {
                format!("发现新版本 {latest}，请到 GitHub Releases 下载 Windows 安装包。")
            } else {
                check["message"]
                    .as_str()
                    .unwrap_or("更新检查失败")
                    .to_owned()
            };
            response(
                request,
                200,
                json!({
                    "required":false,"current":check["current"],"repository":"alexzhong233/WuwaTappo-Full-Community",
                    "checked_at":check["checked_at"],"error":check["error"],
                    "github_version":if latest.is_empty(){check["current"].clone()}else{json!(latest)},
                    "latest":latest,"available":available,"installable":available,
                    "release_notes":check["releaseNotes"],"release_url":check["releaseUrl"],"message":message
                }),
            );
        }
        "/update/healthy" => {
            let version = config_value(app, "community_version");
            wuwatappo_assistant_core::release_installer::ack_healthy(
                &app.root,
                version.as_str().unwrap_or(""),
            );
            response(request, 200, json!({"healthy":true}))
        }
        "/update/install" => {
            let configured = config_value(app, "community_version");
            let current = configured
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    config_value(app, "mod_version")
                        .as_str()
                        .unwrap_or("")
                        .to_owned()
                });
            let include_prerelease = app
                .assistant
                .lock()
                .unwrap()
                .saved()
                .pointer("/settings/prerelease_updates")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let region_id = config_value(app, "region")
                .as_str()
                .unwrap_or("cn")
                .to_owned();
            match wuwatappo_assistant_core::release_updates::community_installer(
                &current,
                include_prerelease,
                &region_id,
            ) {
                Ok(candidate) => match wuwatappo_assistant_core::release_installer::prepare(
                    &app.root,
                    &candidate,
                ) {
                    Ok(value) => {
                        std::thread::spawn(|| {
                            std::thread::sleep(Duration::from_millis(1200));
                            std::process::exit(0);
                        });
                        response(request, 200, value)
                    }
                    Err(error) => response(request, 409, json!({"error":error})),
                },
                Err(error) => response(request, 409, json!({"error":error})),
            }
        }
        "/custom/import" => {
            let Some(archive) = body.get("path").and_then(Value::as_str) else {
                return response(request, 400, json!({"error":"请选择有效的 ZIP 资源包"}));
            };
            let result = custom_resources::install_archive(
                Path::new(archive),
                &custom_root(app),
                region(app),
            );
            match result {
                Ok(pack) => response(
                    request,
                    200,
                    json!({"id":pack.id,"name":pack.name,"item_count":pack.item_count,"asset_count":pack.asset_count,"installed":true,"enabled":*app.offline.lock().unwrap()}),
                ),
                Err(error) => response(request, 409, json!({"error":error})),
            }
        }
        "/custom/remove" => {
            let Some(id) = body.get("id").and_then(Value::as_str) else {
                return response(request, 400, json!({"error":"Mod ID 无效"}));
            };
            match custom_resources::remove_pack(&custom_root(app), id) {
                Ok(()) => response(
                    request,
                    200,
                    json!({"removed":id,"any_installed":!custom_resources::list_summaries(&custom_root(app), region(app)).unwrap_or_default().is_empty(),"enabled":*app.offline.lock().unwrap()}),
                ),
                Err(error) => response(request, 409, json!({"error":error})),
            }
        }
        "/mode" => {
            let Some(enabled) = body.get("enabled").and_then(Value::as_bool) else {
                return response(request, 400, json!({"error":"enabled must be boolean"}));
            };
            if enabled && !cache_ready(app) {
                return response(
                    request,
                    409,
                    json!({"error":"离线素材缓存不完整，未开启。"}),
                );
            }
            let state_path = app.root.join("offline_state.json");
            let temp = state_path.with_extension("tmp");
            if fs::write(&temp, json!({"enabled":enabled}).to_string()).is_err()
                || fs::rename(&temp, state_path).is_err()
            {
                return response(request, 500, json!({"error":"无法保存图鉴模式"}));
            }
            *app.offline.lock().unwrap() = enabled;
            response(
                request,
                200,
                json!({"enabled":enabled,"firewall_verified":false,"mode_verified":true,"network_policy":"rooms-only","ready":cache_ready(app)}),
            );
        }
        "/diagnostic" => {
            // 页面通过 d.report() 上报的就是排障用的现场；丢掉了就只能靠猜。
            wuwatappo_assistant_core::append_diagnostic(&app.root, &body);
            response(request, 200, json!({"ok":true}))
        }
        "/startup" => {
            // 开机自启改由本机服务托管：计划任务优先运行 WuwaTappoLauncher.exe（旧安装退回 launch.ps1），先起本地服务再起桌宠。
            // 官方原来的自启项直接拉桌宠 exe，开机后图鉴和控制台连不上服务。
            let action = body.get("action").and_then(Value::as_str).unwrap_or("status");
            match startup_action(app, action) {
                Ok(value) => response(request, 200, value),
                Err(error) => response(request, 409, json!({"error":error,"enabled":false})),
            }
        }
        "/sync" => {
            let force = body.get("force") == Some(&Value::Bool(true));
            let state = start_sync(app, force);
            response(request, 200, state);
        }
        _ => response(request, 404, json!({"error":"Not found"})),
    }
}

fn handle(app: &Arc<App>, request: tiny_http::Request) {
    let path = path_of(&request);
    if request.method() == &Method::Options {
        return response(request, 200, json!({}));
    }
    if !origin_ok(&request) {
        return response(request, 403, json!({"error":"App origin required"}));
    }
    if request.method() == &Method::Get {
        return handle_get(app, request, &path);
    }
    if request.method() == &Method::Post {
        return handle_post(app, request, &path);
    }
    response(request, 405, json!({"error":"Method not allowed"}));
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("--apply-update") {
        let root = PathBuf::from(args.get(2).cloned().unwrap_or_default());
        let stage = PathBuf::from(args.get(3).cloned().unwrap_or_default());
        if let Err(error) = wuwatappo_assistant_core::release_installer::apply_update(&root, &stage) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--prepare-cache") {
        let out_dir = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| ".".into()));
        let region = args
            .get(3)
            .cloned()
            .or_else(|| env::var("WUWA_REGION").ok())
            .unwrap_or_else(|| "cn".into());
        match wuwatappo_assistant_core::resource_cache::sync_with_progress(
            &out_dir,
            &region,
            true,
            |done, total| {
                println!("{done}/{total}");
            },
        ) {
            Ok(summary) => {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "changed": summary.changed,
                        "atlas_count": summary.atlas_count,
                        "asset_count": summary.asset_count
                    })
                );
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if args.get(1).map(String::as_str) == Some("--make-update") {
        let Some(payload) = args.get(2) else {
            eprintln!("用法：LocalGalleryHost.exe --make-update <payload_dir> <out_dir> [region]");
            std::process::exit(1);
        };
        let Some(out_dir) = args.get(3) else {
            eprintln!("用法：LocalGalleryHost.exe --make-update <payload_dir> <out_dir> [region]");
            std::process::exit(1);
        };
        let region = args.get(4).map(String::as_str);
        match wuwatappo_assistant_core::release_installer::build_update_archive(
            Path::new(payload),
            Path::new(out_dir),
            region,
        ) {
            Ok(summary) => {
                println!("{summary}");
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    let root = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let config = match wuwatappo_assistant_core::ensure_local_config(&root) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("初始化本机配置失败：{error}");
            std::process::exit(1);
        }
    };
    let port = config.get("port").and_then(Value::as_u64).unwrap_or(0);
    let token = config
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let saved = fs::read(root.join("assistant_state.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let offline = fs::read_to_string(root.join("offline_state.json"))
        .ok()
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .and_then(|value| value.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false);
    // 端口被别的进程占住时不要 panic：panic 只会在 stderr 留一行英文，
    // 而用户看到的是启动器的「本地服务启动超时」，根本对不上号。
    let server = match Server::http(format!("127.0.0.1:{port}")) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("本地服务端口 {port} 无法监听（可能已被占用）：{error}");
            std::process::exit(2);
        }
    };
    let app = Arc::new(App {
        root,
        config: Mutex::new(config),
        token,
        assistant: Mutex::new(AssistantCore::new(saved)),
        offline: Mutex::new(offline),
        custom_error: Mutex::new(None),
        sync: Mutex::new(json!({"status":"idle","done":0,"total":0,"checked_at":0})),
        focus_probe: Mutex::new(None),
    });
    println!(
        "{}",
        json!({"ready":true,"enabled":offline,"port":server.server_addr().to_ip().map(|address|address.port()).unwrap_or(port.try_into().unwrap_or(0))})
    );
    for request in server.incoming_requests() {
        let app = Arc::clone(&app);
        std::thread::spawn(move || handle(&app, request));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("wuwatappo-host-{label}-{}", now() as u64));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("cache/assets")).unwrap();
        root
    }

    #[test]
    fn cache_readiness_checks_count_errors_names_and_sizes() {
        let root = test_root("cache");
        let config = json!({"asset_count":1});
        let app = App {
            root: root.clone(),
            config: Mutex::new(config),
            token: String::new(),
            assistant: Mutex::new(AssistantCore::new(None)),
            offline: Mutex::new(false),
            custom_error: Mutex::new(None),
            sync: Mutex::new(json!({"status":"idle","checked_at":0})),
            focus_probe: Mutex::new(None),
        };
        fs::write(root.join("cache/assets/a.png"), b"png").unwrap();
        fs::write(
            root.join("cache/asset_manifest.json"),
            json!({
                "records":[{"file":"a.png","size":3}],"errors":[]
            })
            .to_string(),
        )
        .unwrap();
        assert!(cache_ready(&app));
        fs::write(
            root.join("cache/asset_manifest.json"),
            json!({
                "records":[{"file":"a.png","size":4}],"errors":[]
            })
            .to_string(),
        )
        .unwrap();
        assert!(!cache_ready(&app));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_paths_reject_encoded_traversal_and_windows_separators() {
        assert!(safe_relative("pack/assets/image.png"));
        for path in [
            "../secret",
            "pack\\secret",
            "%2e%2e/secret",
            "C:/secret",
            "pack//image.png",
        ] {
            assert!(!safe_relative(path), "accepted unsafe path: {path}");
        }
    }
}
