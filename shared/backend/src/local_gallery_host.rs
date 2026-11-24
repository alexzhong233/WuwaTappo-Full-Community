//! Local HTTP bridge shared by the Windows desktop client.
//!
//! All business state, catalog/PNG caching, Mod overlays, version checks, and
//! assistant handlers delegate to `wuwatappo_assistant_core::service::FullAtlasService`.

use serde_json::{json, Map, Value};
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use wuwatappo_assistant_core::{
    custom_resources, resource_cache,
    service::{
        FullAtlasService, PersistenceMode, ServiceConfig, COMMUNITY_RELEASES_URL,
    },
};

#[cfg(windows)]
mod windows_menu;
#[cfg(windows)]
mod windows_startup;

const MAX_BODY: usize = 64 * 1024;
const MAX_CUSTOM_BODY: usize = 16 * 1024;
const ALLOWED_ORIGINS: [&str; 3] = [
    "http://tauri.localhost",
    "https://tauri.localhost",
    "tauri://localhost",
];

struct App {
    service: Arc<FullAtlasService>,
    config: Arc<Mutex<Value>>,
    token: String,
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

fn config_value(config: &Mutex<Value>, key: &str) -> Value {
    config
        .lock()
        .unwrap()
        .get(key)
        .cloned()
        .unwrap_or(Value::Null)
}

fn request_origin(request: &tiny_http::Request) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Origin"))
        .map(|header| header.value.as_str().to_owned())
}

fn response_with_cache(
    request: tiny_http::Request,
    status: u16,
    bytes: Vec<u8>,
    content_type: &str,
    cache_immutable: bool,
) {
    let origin = request_origin(&request);
    let mut output = Response::from_data(bytes).with_status_code(StatusCode(status));
    output.add_header(Header::from_bytes("Content-Type", content_type).unwrap());
    let cache_control = if cache_immutable {
        "private, max-age=3600, immutable"
    } else {
        "no-store"
    };
    output.add_header(Header::from_bytes("Cache-Control", cache_control).unwrap());
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

fn response_with_type(
    request: tiny_http::Request,
    status: u16,
    bytes: Vec<u8>,
    content_type: &str,
) {
    response_with_cache(request, status, bytes, content_type, false);
}

fn response(request: tiny_http::Request, status: u16, value: Value) {
    response_with_type(
        request,
        status,
        value.to_string().into_bytes(),
        "application/json; charset=utf-8",
    );
}

fn send_service_result(
    request: tiny_http::Request,
    result: Result<Value, wuwatappo_assistant_core::service::ServiceError>,
) {
    match result {
        Ok(value) => response(request, 200, value),
        Err(err) => response(request, err.status_code(), json!({ "error": err.message })),
    }
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

fn path_of(request: &tiny_http::Request) -> String {
    let raw = request.url().split('?').next().unwrap_or("/");
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

fn mime_type(path: &Path) -> &'static str {
    custom_resources::asset_mime_type(path)
}

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
    windows_startup::run(app.service.root(), action)
}

#[cfg(not(windows))]
fn startup_action(_app: &App, _action: &str) -> Result<Value, String> {
    Err("开机自启托管只能在 Windows 上使用".into())
}

fn send_file_with_cache(request: tiny_http::Request, file: Option<PathBuf>, cache_immutable: bool) {
    let Some(file) = file else {
        return response(request, 404, json!({"error":"Not found"}));
    };
    match fs::read(&file) {
        Ok(bytes) => {
            response_with_cache(request, 200, bytes, mime_type(&file), cache_immutable)
        }
        Err(_) => response(request, 404, json!({"error":"Not found"})),
    }
}

fn send_file(request: tiny_http::Request, file: Option<PathBuf>) {
    send_file_with_cache(request, file, false);
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
            let mut extra = Map::new();
            for key in [
                "region",
                "mod_version",
                "client_version",
                "asset_count",
                "atlas_count",
            ] {
                extra.insert(key.into(), config_value(&app.config, key));
            }
            response(request, 200, app.service.status_snapshot(Some(extra)));
        }
        "/sync/status" => response(request, 200, app.service.sync_status()),
        "/custom/status" => response(request, 200, app.service.custom_status()),
        "/offline-data" => send_service_result(request, app.service.offline_bundle(None)),
        "/asset-base" => {
            let port = config_value(&app.config, "port").as_u64().unwrap_or(0);
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
            if !resource_cache::safe_relative_key(pack)
                || pack.contains('/')
                || !resource_cache::safe_relative_key(asset)
            {
                return response(request, 400, json!({"error":"Invalid path"}));
            }
            let file = custom_resources::resolve_asset_file(
                &app.service.custom_root(),
                path.trim_start_matches('/'),
            );
            send_file(request, file);
        }
        _ if path.starts_with("/assets/") => {
            let name = &path["/assets/".len()..];
            if !resource_cache::safe_relative_key(name) || name.contains('/') {
                return response(request, 400, json!({"error":"Invalid path"}));
            }
            let file = resource_cache::resolve_cached_asset(
                &app.service.cache_dir(),
                path.trim_start_matches('/'),
            );
            if file.is_none() {
                app.service
                    .diagnostic(&json!({"kind":"asset_not_found","path":name}));
            }
            send_file_with_cache(request, file, true);
        }
        _ if [
            "/character/",
            "/accessory/",
            "/emote/",
            "/emote_atlas/",
            "/character_atlas/",
            "/character_skin_atlas/",
            "/accessory_atlas/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix)) =>
        {
            let file = resource_cache::resolve_cached_asset(
                &app.service.cache_dir(),
                path.trim_start_matches('/'),
            );
            send_file(request, file);
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
        return match context_menu(&body) {
            Ok(value) => response(request, 200, value),
            Err(error) => response(
                request,
                400,
                json!({"action":Value::Null,"error":error}),
            ),
        };
    }

    if path == "/assistant/state" {
        return send_service_result(
            request,
            app.service.assistant("state", &body, false),
        );
    }
    let action = path.trim_start_matches('/');
    let quiet = if matches!(action, "assistant/tick" | "assistant/observe") {
        physical_quiet()
    } else {
        false
    };
    if let Some(res) = app.service.handle_shared_action(action, &body, quiet, None) {
        return send_service_result(request, res);
    }

    match path {
        "/community/update-check" => {
            let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);
            response(request, 200, app.service.community_update_check(force, Ok(())))
        }
        "/official/update-check" | "/official/check" => {
            send_service_result(request, app.service.official_update_check(false))
        }
        "/official/download" => {
            let download_dir = env::var_os("USERPROFILE")
                .or_else(|| env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join("Downloads"))
                .unwrap_or_else(|| app.service.root().join("downloads"));
            send_service_result(request, app.service.official_download(&download_dir, false))
        }
        "/official/releases" => response(request, 200, open_url(COMMUNITY_RELEASES_URL)),
        "/official/pet-site" => {
            let url = app.service.pet_site_url();
            response(request, 200, open_url(&url))
        }
        "/update/status" => response(request, 200, app.service.update_status(Ok(()))),
        "/update/check" => {
            let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);
            response(request, 200, app.service.update_check(force, Ok(())))
        }
        "/update/install" => {
            let guard = match app.service.begin_install() {
                Ok(g) => g,
                Err(e) => return response(request, e.status_code(), json!({"error": e.message})),
            };
            wuwatappo_assistant_core::release_installer::write_current_status(
                app.service.root(),
                "download",
                2.0,
                "正在获取更新信息并后台下载安装包…",
            );
            let service = Arc::clone(&app.service);
            std::thread::spawn(move || {
                let _guard = guard;
                let candidate =
                    match wuwatappo_assistant_core::release_updates::community_installer(
                        service.current_community_version(),
                        service.include_prerelease(),
                        service.region_id(),
                    ) {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            wuwatappo_assistant_core::release_installer::write_current_status(
                                service.root(),
                                "failed",
                                0.0,
                                &format!("准备更新失败：{error}"),
                            );
                            return;
                        }
                    };
                if let Err(error) = wuwatappo_assistant_core::release_installer::prepare(
                    service.root(),
                    &candidate,
                ) {
                    wuwatappo_assistant_core::release_installer::write_current_status(
                        service.root(),
                        "failed",
                        0.0,
                        &format!("准备更新失败：{error}"),
                    );
                    return;
                }
                _guard.disarm();
                let _ = service.assistant("quit", &json!({}), false);
                std::thread::sleep(Duration::from_millis(2500));
                std::process::exit(0);
            });
            response(
                request,
                200,
                json!({
                    "started": true,
                    "message": "已开始后台下载并校验安装包…",
                }),
            )
        }
        "/mode" => {
            let Some(enabled) = body.get("enabled").and_then(Value::as_bool) else {
                return response(request, 400, json!({"error":"enabled must be boolean"}));
            };
            send_service_result(request, app.service.set_mode(enabled, true))
        }
        "/startup" => {
            let action = body.get("action").and_then(Value::as_str).unwrap_or("status");
            match startup_action(app, action) {
                Ok(value) => response(request, 200, value),
                Err(error) => response(request, 409, json!({"error":error,"enabled":false})),
            }
        }
        "/diagnostic/open-logs" => {
            let _ = fs::create_dir_all(app.service.root().join("logs"));
            response(request, 200, open_url(&app.service.root().to_string_lossy()))
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
        match resource_cache::sync_with_progress(&out_dir, &region, true, |done, total| {
            println!("{done}/{total}");
        }) {
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
    let initial_config = match wuwatappo_assistant_core::ensure_local_config(&root) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("初始化本机配置失败：{error}");
            std::process::exit(1);
        }
    };
    let port = initial_config.get("port").and_then(Value::as_u64).unwrap_or(0);
    let token = initial_config
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let region_raw = initial_config
        .get("region")
        .and_then(Value::as_str)
        .unwrap_or("cn");
    let region_profile = wuwatappo_assistant_core::regions::profile(region_raw)
        .or_else(|| wuwatappo_assistant_core::regions::profile("cn"))
        .expect("cn region exists");
    let community_version = initial_config
        .get("community_version")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            initial_config
                .get("mod_version")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_owned();
    let client_version = initial_config
        .get("client_version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let config_arc = Arc::new(Mutex::new(initial_config));
    let sync_cfg = Arc::clone(&config_arc);
    let sync_root = root.clone();
    let on_sync_complete = Arc::new(move |summary: &resource_cache::SyncSummary| {
        let mut guard = sync_cfg.lock().unwrap();
        guard["asset_count"] = json!(summary.asset_count);
        guard["atlas_count"] = json!(summary.atlas_count);
        let cfg_path = sync_root.join("local_config.json");
        let temp = cfg_path.with_extension("tmp");
        if fs::write(&temp, serde_json::to_vec_pretty(&*guard).unwrap_or_default()).is_ok() {
            let _ = fs::rename(temp, cfg_path);
        }
    });

    let service = Arc::new(
        FullAtlasService::new(ServiceConfig {
            root,
            persistence: PersistenceMode::SplitFiles,
            region_id: region_profile.id.clone(),
            region_code: region_profile.region.clone(),
            community_version,
            client_version,
            expected_asset_count: None,
            on_sync_complete: Some(on_sync_complete),
        })
        .unwrap_or_else(|error| {
            eprintln!("初始化全图鉴服务失败：{error}");
            std::process::exit(1);
        }),
    );
    service.ensure_initial_sync();

    let server = match Server::http(format!("127.0.0.1:{port}")) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("本地服务端口 {port} 无法监听（可能已被占用）：{error}");
            std::process::exit(2);
        }
    };

    let app = Arc::new(App {
        service,
        config: Arc::clone(&config_arc),
        token,
    });
    // Share live config updates with status queries
    let enabled = app.service.enabled();
    println!(
        "{}",
        json!({"ready":true,"enabled":enabled,"port":server.server_addr().to_ip().map(|address|address.port()).unwrap_or(port.try_into().unwrap_or(0))})
    );
    for request in server.incoming_requests() {
        let app = Arc::clone(&app);
        std::thread::spawn(move || handle(&app, request));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let root = env::temp_dir().join(format!("wuwatappo-host-{label}-{now}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("cache/assets")).unwrap();
        root
    }

    #[test]
    fn cache_readiness_checks_count_errors_names_and_sizes() {
        let root = test_root("cache");
        fs::write(root.join("cache/assets/a.png"), b"png").unwrap();
        fs::write(root.join("cache/offline_catalog.json"), "{}").unwrap();
        fs::write(root.join("cache/atlas.json"), "{\"data\":[]}").unwrap();
        fs::write(
            root.join("cache/asset_manifest.json"),
            json!({
                "records":[{"file":"a.png","size":3}],"errors":[]
            })
            .to_string(),
        )
        .unwrap();
        assert!(resource_cache::is_cache_ready(&root, Some(1)));
        fs::write(
            root.join("cache/asset_manifest.json"),
            json!({
                "records":[{"file":"a.png","size":4}],"errors":[]
            })
            .to_string(),
        )
        .unwrap();
        assert!(!resource_cache::is_cache_ready(&root, Some(1)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_paths_reject_encoded_traversal_and_windows_separators() {
        assert!(resource_cache::safe_relative_key("pack/assets/image.png"));
        for path in [
            "../secret",
            "pack\\secret",
            "%2e%2e/secret",
            "C:/secret",
            "pack//image.png",
        ] {
            assert!(
                !resource_cache::safe_relative_key(path),
                "accepted unsafe path: {path}"
            );
        }
    }
}
