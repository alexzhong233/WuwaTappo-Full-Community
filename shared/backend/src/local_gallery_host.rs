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

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
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

fn native_typing_context() -> Value {
    #[cfg(windows)]
    {
        const SCRIPT: &str = r#"$ErrorActionPreference='Stop';try{Add-Type -AssemblyName UIAutomationClient;$e=[System.Windows.Automation.AutomationElement]::FocusedElement;if($null -eq $e){@{editable=$false;source='uia'}|ConvertTo-Json -Compress;exit};$t=$e.Current.ControlType.Id;$r=$e.GetCurrentPropertyValue(30046);$v=$e.GetCurrentPropertyValue(30043);$x=$e.GetCurrentPropertyValue(30149);$ok=($r -ne $true -and (($t -eq 50004)-or($t -eq 50003 -and $v -eq $true)-or($t -eq 50030 -and $x -eq $true)));@{editable=[bool]$ok;source='uia';control_type=$t}|ConvertTo-Json -Compress}catch{@{editable=$false;source='uia-unavailable';error=$_.Exception.GetType().Name}|ConvertTo-Json -Compress}"#;
        let spawned = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else {
            return json!({"editable":false,"source":"uia-unavailable","error":"PowerShellUnavailable"});
        };
        let deadline = Instant::now() + Duration::from_millis(300);
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
        return serde_json::from_slice(&output.stdout).unwrap_or_else(
            |_| json!({"editable":false,"source":"uia-unavailable","error":"InvalidProbeResult"}),
        );
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
        if at.elapsed() < Duration::from_millis(200) {
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
    if config_value(app, "region").as_str() == Some("global") {
        "GLOBAL"
    } else {
        "CN"
    }
}

fn custom_root(app: &App) -> PathBuf {
    app.root.join("custom-atlas")
}

fn custom_status(app: &App) -> Value {
    let packs = custom_resources::list_details(&custom_root(app), region(app)).unwrap_or_default();
    let item_count: usize = packs
        .iter()
        .map(|pack| pack["item_count"].as_u64().unwrap_or(0) as usize)
        .sum();
    let asset_count: usize = packs
        .iter()
        .map(|pack| pack["asset_count"].as_u64().unwrap_or(0) as usize)
        .sum();
    json!({
        "installed": !packs.is_empty(),
        "enabled": *app.offline.lock().unwrap(),
        "name": packs.iter().filter_map(|pack| pack["name"].as_str()).collect::<Vec<_>>().join("、"),
        "atlas_count": item_count,
        "asset_count": asset_count,
        "packs": packs,
        "region": region(app),
        "error": app.custom_error.lock().unwrap().clone(),
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
    match custom_resources::merge_official(&official, &custom_root(app), region(app)) {
        Ok(merged) => {
            *app.custom_error.lock().unwrap() = None;
            Ok(merged)
        }
        Err(error) => {
            *app.custom_error.lock().unwrap() = Some(error);
            Ok(official)
        }
    }
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
        let result = wuwatappo_assistant_core::resource_cache::sync(&app.root, &selected_region);
        let mut state = app.sync.lock().unwrap();
        *state = match result {
            Ok(summary) => {
                let mut config = app.config.lock().unwrap();
                config["asset_count"] = json!(summary.asset_count);
                config["atlas_count"] = json!(summary.atlas_count);
                let _ = fs::write(
                    app.root.join("local_config.json"),
                    serde_json::to_vec_pretty(&*config).unwrap_or_default(),
                );
                json!({"status":"complete","done":summary.asset_count,"total":summary.asset_count,"checked_at":now(),"result":{"changed":summary.changed,"atlas_count":summary.atlas_count,"asset_count":summary.asset_count}})
            }
            Err(error) => {
                json!({"status":"failed","done":0,"total":0,"checked_at":now(),"error":error})
            }
        };
    });
    snapshot
}

fn send_file(request: tiny_http::Request, file: Option<PathBuf>) {
    let Some(file) = file else {
        return response(request, 404, json!({"error":"Not found"}));
    };
    match fs::read(&file) {
        Ok(bytes) => response_with_type(request, 200, bytes, mime_type(&file)),
        Err(_) => response(request, 404, json!({"error":"Not found"})),
    }
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
            let file = contained_file(&custom_root(app), &format!("{pack}/assets/{asset}"));
            send_file(request, file);
        }
        _ if path.starts_with("/assets/") => {
            let name = &path["/assets/".len()..];
            if !safe_relative(name) || name.contains('/') {
                return response(request, 400, json!({"error":"Invalid path"}));
            }
            send_file(
                request,
                contained_file(&cache_dir(app).join("assets"), name),
            );
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
                    "latest":latest,"available":available,"installable":false,
                    "release_notes":check["releaseNotes"],"release_url":check["releaseUrl"],"message":message
                }),
            );
        }
        "/update/healthy" => response(request, 200, json!({"healthy":true})),
        "/update/install" => response(
            request,
            409,
            json!({"error":"请从 Releases 下载 Windows 安装包"}),
        ),
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
        "/diagnostic" => response(request, 200, json!({"ok":true})),
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
    let root = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let config = fs::read_to_string(root.join("local_config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({"port":0,"token":""}));
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
    let server = Server::http(format!("127.0.0.1:{port}")).expect("bind local bridge");
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
