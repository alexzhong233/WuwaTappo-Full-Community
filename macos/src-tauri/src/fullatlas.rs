//! Native transport and state for the MIT FullAtlas controllers. No OS input
//! injection, Windows updater, or simulated rewards. The only local HTTP
//! listener is the read-only Mod image route in `mod_assets`.
use chrono::{Duration as Days, Local};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const UPDATE_MESSAGE: &str =
    "这是 Mac 整合版；Windows FullAtlas 更新包不能安装到 Mac。请使用本项目的 Mac 安装包更新。";
const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_OFFICIAL_INSTALLER: u64 = 512 * 1024 * 1024;
const COMMUNITY_RELEASES_URL: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    enabled: bool,
    speed: u32,
    auto_chest: bool,
    threshold: u64,
    remember_monitor: bool,
    /// 更新检查是否把预发布（alpha/beta）算进候选。默认关闭：只提醒正式版。
    prerelease_updates: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            speed: 5,
            auto_chest: false,
            threshold: 1000,
            remember_monitor: true,
            prerelease_updates: false,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    settings: Settings,
    enabled: bool,
    session: u64,
    confirmed: u64,
    session_chests: u64,
    rewards: Vec<Value>,
    rooms: Vec<Value>,
    deleted_rooms: Vec<String>,
    monitor: Option<Value>,
    baselines: BTreeMap<String, u64>,
    days: BTreeMap<String, u64>,
}
#[derive(Default)]
struct Runtime {
    account: String,
    values: Value,
    next_tick: Option<Instant>,
    flush_generation: u64,
    flush_done: u64,
    last_save: Option<Instant>,
}
struct Inner {
    saved: Saved,
    runtime: Runtime,
}
pub struct FullAtlasState {
    dir: PathBuf,
    inner: Mutex<Inner>,
    catalog: Mutex<Option<Value>>,
    official_catalog: Mutex<Option<Value>>,
    custom_error: Mutex<Option<String>>,
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    fs::create_dir_all(path.parent().ok_or("Missing state directory")?)
        .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())
}
fn local_window(label: &str, url: &Url) -> bool {
    matches!(label, "main" | "panel")
        && url.port().is_none()
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost")))
}
fn trusted_asset_url(region: crate::profile::Region, url: &str) -> Result<Url, String> {
    let url = Url::parse(url).map_err(|_| "图鉴地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some(region.asset_host())
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("图鉴目录地址不是当前地区官方素材源".into());
    }
    Ok(url)
}
fn get_json(client: &reqwest::blocking::Client, url: Url) -> Result<Value, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("读取图鉴失败：{e}"))?
        .error_for_status()
        .map_err(|e| format!("图鉴响应失败：{e}"))?;
    if response.content_length().is_some_and(|n| n > MAX_JSON) {
        return Err("图鉴目录超出大小限制".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_JSON {
        return Err("图鉴目录超出大小限制".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("图鉴 JSON 无效：{e}"))
}

fn version_numbers(value: &str) -> Vec<u64> {
    value
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| {
            part.chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>()
        })
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn official_is_newer(latest: &str, current: &str) -> bool {
    let mut left = version_numbers(latest);
    let mut right = version_numbers(current);
    left.resize(3, 0);
    right.resize(3, 0);
    left > right
}

fn official_installer(
    region: crate::profile::Region,
    manifest: &Value,
    version: &str,
) -> Result<(Url, String), String> {
    let platform = manifest
        .pointer("/platforms/windows-x86_64")
        .ok_or("官方版本清单缺少 Windows 原包")?;
    let value = platform
        .get("url")
        .and_then(Value::as_str)
        .ok_or("官方版本清单缺少 Windows 原包地址")?;
    let url = trusted_asset_url(region, value)?;
    let filename = url
        .path_segments()
        .and_then(|segments| segments.last())
        .filter(|name| !name.is_empty())
        .ok_or("官方原包文件名无效")?
        .to_string();
    let region_id = match region {
        crate::profile::Region::Cn => "cn",
        crate::profile::Region::Global => "global",
    };
    let expected = format!("WuwaTappo-{region_id}_{version}_x64-setup.exe");
    if filename != expected {
        return Err("官方原包文件名与版本或服务器区域不匹配".into());
    }
    Ok((url, filename))
}

const COMMUNITY_REPO: &str = "alexzhong233/WuwaTappo-Full-Community";
const COMMUNITY_RELEASES_PAGE: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

/// 全图鉴整合包的发布版本；与 `package.json` 同源（bump-version 同步 Cargo.toml）。
fn community_current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 拆成 (数字段, 预发布段)。`1.3.0-alpha.2` → ([1,3,0], Some("alpha.2"))。
fn split_semver(value: &str) -> (Vec<u64>, Option<String>) {
    let value = value.trim().trim_start_matches('v');
    let (core, pre) = match value.split_once('-') {
        Some((core, pre)) => (core, Some(pre.to_string())),
        None => (value, None),
    };
    let mut numbers = core
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect::<Vec<_>>();
    numbers.resize(3, 0);
    (numbers, pre)
}

/// 预发布段按点分段比较，数字段按数值、其余按字典序，符合 semver 对 alpha.10 > alpha.9 的约定。
fn compare_prerelease(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => {
                let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => a.cmp(b),
                };
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

fn compare_semver(left: &str, right: &str) -> std::cmp::Ordering {
    let (left_numbers, left_pre) = split_semver(left);
    let (right_numbers, right_pre) = split_semver(right);
    match left_numbers.cmp(&right_numbers) {
        std::cmp::Ordering::Equal => {}
        order => return order,
    }
    match (left_pre.is_some(), right_pre.is_some()) {
        (false, false) => std::cmp::Ordering::Equal,
        // 正式版高于同号的预发布。
        (false, true) => std::cmp::Ordering::Greater,
        (true, false) => std::cmp::Ordering::Less,
        (true, true) => compare_prerelease(
            left_pre.as_deref().unwrap_or_default(),
            right_pre.as_deref().unwrap_or_default(),
        ),
    }
}

/// 从 GitHub releases 列表里挑出最高版本，返回它和被跳过的预发布数量。
///
/// 单独抽出来是为了能直接测「开关关闭时不会把 alpha 当更新」这条行为。
fn pick_latest_release(
    releases: &[Value],
    include_prerelease: bool,
) -> (Option<(String, String, String)>, usize) {
    let mut best: Option<(String, String, String)> = None;
    let mut skipped = 0usize;
    for release in releases {
        if release.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if !include_prerelease && release.get("prerelease").and_then(Value::as_bool) == Some(true) {
            skipped += 1;
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let version = tag.trim().trim_start_matches('v').to_string();
        let url = release
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or(COMMUNITY_RELEASES_PAGE)
            .to_string();
        let published = release
            .get("published_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let newer = match &best {
            Some((seen, _, _)) => compare_semver(&version, seen) == std::cmp::Ordering::Greater,
            None => true,
        };
        if newer {
            best = Some((version, url, published));
        }
    }
    (best, skipped)
}

/// 对比本仓库 GitHub 发布，排除 draft。
///
/// 不用 `/releases/latest`：GitHub 会跳过 prerelease，而本项目的 alpha 正是以 prerelease
/// 发布，用它会永远查不到新版本。所以这里自己扫列表，并由 `include_prerelease`
/// 决定要不要把 alpha/beta 算进候选——默认不算，用户可在设置里打开。
fn community_update_status(include_prerelease: bool) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("WuwaTappo-Full-Community-Mac")
        .build()
        .map_err(|e| e.to_string())?;
    let list = get_json(
        &client,
        Url::parse(&format!(
            "https://api.github.com/repos/{COMMUNITY_REPO}/releases?per_page=20"
        ))
        .map_err(|e| e.to_string())?,
    )?;
    let current = community_current_version();
    let (best, skipped_prerelease) = pick_latest_release(
        list.as_array().map(Vec::as_slice).unwrap_or_default(),
        include_prerelease,
    );
    let (latest, release_url, published_at) = best.unwrap_or_default();
    let update_available = !latest.is_empty()
        && compare_semver(&latest, current) == std::cmp::Ordering::Greater;
    Ok(json!({
        "current": current,
        "latest": latest,
        "updateAvailable": update_available,
        "releaseUrl": release_url,
        "pageUrl": COMMUNITY_RELEASES_PAGE,
        "publishedAt": published_at,
        "prereleaseIncluded": include_prerelease,
        "skippedPrerelease": skipped_prerelease,
        "message": if update_available {
            format!("发现新版本 v{latest}，当前 v{current}。")
        } else if latest.is_empty() && skipped_prerelease > 0 {
            "当前只发布了预发布版本；需要这类提醒可在设置里打开「包含预发布」。".to_string()
        } else if latest.is_empty() {
            "GitHub 上还没有已发布的安装包。".to_string()
        } else {
            format!("已是最新版本 v{current}。")
        },
    }))
}

fn official_update_status(region: crate::profile::Region) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("WuwaTappo-FullAtlas-Mac")
        .build()
        .map_err(|e| e.to_string())?;
    let api = format!("https://{}", region.api_host());
    let activity = get_json(
        &client,
        Url::parse(&format!("{api}/api/common/activity-config")).map_err(|e| e.to_string())?,
    )?;
    let host = activity
        .pointer("/data/assets_host")
        .and_then(Value::as_str)
        .ok_or("官方版本检查缺少资源源")?
        .trim_end_matches('/');
    let host = trusted_asset_url(region, host)?;
    // 两个地址都带一次性时间戳：CDN 边缘节点会保留旧对象，表现为官方清单长期
    // 停在上一版（我们已跟进 1.0.4，检查结果却还是 1.0.3）。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let pointer = get_json(
        &client,
        trusted_asset_url(
            region,
            &format!(
                "{}/app/json-host/{}?_={}",
                host,
                region.updater_host_file(),
                stamp
            ),
        )?,
    )?;
    let manifest_url = pointer
        .get("url")
        .and_then(Value::as_str)
        .ok_or("官方版本检查缺少版本清单")?;
    let mut manifest_url = trusted_asset_url(region, manifest_url)?;
    manifest_url
        .query_pairs_mut()
        .append_pair("_", &stamp.to_string());
    let manifest = get_json(&client, manifest_url)?;
    let latest = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("官方版本清单无效")?;
    let current = env!("WUWA_OFFICIAL_VERSION");
    let changed = official_is_newer(latest, current);
    // 官方清单落后于已整合基底时，界面不能再说「已是最新（旧号）」。
    let remote_older = official_is_newer(current, latest);
    let (installer_url, installer_name) = official_installer(region, &manifest, latest)?;
    Ok(json!({
        "currentVersion": current,
        "latestVersion": latest,
        "updateAvailable": changed,
        "remoteOlder": remote_older,
        "officialInstallerUrl": installer_url.as_str(),
        "officialInstallerFileName": installer_name,
        "officialInstallerPlatform": "windows-x86_64",
        "communityReleasesUrl": COMMUNITY_RELEASES_URL,
        "canDownloadOfficial": true,
        "supportsLocalAdaptation": false,
        "localAdaptationReason": "已安装的 Mac 应用没有源码、Node、Rust、Xcode 和 7-Zip，不能在应用内安全重打包。",
        "message": if changed {
            format!("检测到官方 {latest}，可下载官方 Windows 原包；Mac 全图鉴包需由本项目构建后从 Releases 更新。")
        } else if remote_older {
            format!("官方清单当前是 {latest}，本包基底已跟进到 {current}。")
        } else {
            format!("官方基底已是 {current}。")
        },
    }))
}

fn download_official_installer(
    app: &AppHandle,
    region: crate::profile::Region,
) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("WuwaTappo-Full-Community-Mac")
        .build()
        .map_err(|e| e.to_string())?;
    let api = format!("https://{}", region.api_host());
    let activity = get_json(
        &client,
        Url::parse(&format!("{api}/api/common/activity-config")).map_err(|e| e.to_string())?,
    )?;
    let host = activity
        .pointer("/data/assets_host")
        .and_then(Value::as_str)
        .ok_or("官方版本下载缺少资源源")?
        .trim_end_matches('/');
    let host = trusted_asset_url(region, host)?;
    let pointer = get_json(
        &client,
        trusted_asset_url(
            region,
            &format!("{}/app/json-host/{}", host, region.updater_host_file()),
        )?,
    )?;
    let manifest_url = pointer
        .get("url")
        .and_then(Value::as_str)
        .ok_or("官方版本下载缺少版本清单")?;
    let manifest = get_json(&client, trusted_asset_url(region, manifest_url)?)?;
    let latest = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("官方版本清单无效")?;
    let (url, filename) = official_installer(region, &manifest, latest)?;
    let current = env!("WUWA_OFFICIAL_VERSION");
    if !official_is_newer(latest, current) {
        return Err(format!("官方仍是 {current}，没有可下载的新版本"));
    }
    let response = client
        .get(url.clone())
        .send()
        .map_err(|e| format!("下载官方原包失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载官方原包失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_OFFICIAL_INSTALLER)
    {
        return Err("官方原包超过 512 MiB，已停止下载".into());
    }
    let directory = app.path().download_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let target = directory.join(&filename);
    let temporary = directory.join(format!(".{filename}.download"));
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    let mut limited = response.take(MAX_OFFICIAL_INSTALLER + 1);
    let copied = match std::io::copy(&mut limited, &mut file) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(format!("保存官方原包失败：{error}"));
        }
    };
    if copied > MAX_OFFICIAL_INSTALLER {
        let _ = fs::remove_file(&temporary);
        return Err("官方原包超过 512 MiB，已停止下载".into());
    }
    file.flush().map_err(|e| e.to_string())?;
    fs::rename(&temporary, &target).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "version": latest,
        "fileName": filename,
        "path": target,
        "platform": "windows-x86_64",
        "message": format!("官方 {latest} Windows 原包已下载到 {}。它不能直接安装到 Mac；请使用本项目 Releases 中的 Mac 包。", target.display()),
    }))
}
fn validate_catalog(value: &Value) -> Result<(), String> {
    let atlas = value
        .get("atlas")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or("官方图鉴列表为空")?;
    let catalog = value.get("catalog").ok_or("缺少外观目录")?;
    for kind in ["character_skin", "accessory", "emote"] {
        if !catalog.get(kind).is_some_and(Value::is_object)
            || !catalog
                .get(format!("{kind}_atlas"))
                .is_some_and(Value::is_object)
        {
            return Err("外观目录结构不兼容，原缓存保留".into());
        }
    }
    for row in atlas {
        let kind = row
            .get("asset_type")
            .and_then(Value::as_str)
            .ok_or("图鉴类型缺失")?;
        let id = row
            .get("asset_id")
            .and_then(Value::as_u64)
            .ok_or("图鉴 ID 无效")?
            .to_string();
        let usable = catalog
            .get(kind)
            .and_then(|v| v.get(&id))
            .and_then(|v| v.get("height_light_id"))
            .and_then(Value::as_u64)
            .ok_or("图鉴缺少外观 ID 映射")?
            .to_string();
        if !catalog
            .get(kind.trim_end_matches("_atlas"))
            .is_some_and(|v| v.get(usable).is_some())
        {
            return Err("图鉴外观映射不完整，原缓存保留".into());
        }
    }
    Ok(())
}

impl FullAtlasState {
    pub fn new(dir: PathBuf) -> Result<Self, String> {
        let path = dir.join("fullatlas.json");
        let saved = if path.exists() {
            serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("FullAtlas 状态文件损坏，请备份后修复：{e}"))?
        } else {
            Saved::default()
        };
        Ok(Self {
            dir,
            inner: Mutex::new(Inner {
                saved,
                runtime: Runtime::default(),
            }),
            catalog: Mutex::new(None),
            official_catalog: Mutex::new(None),
            custom_error: Mutex::new(None),
        })
    }
    pub fn enabled(&self) -> bool {
        self.inner.lock().map(|v| v.saved.enabled).unwrap_or(true)
    }
    fn save(&self, inner: &mut Inner, force: bool) -> Result<(), String> {
        if force
            || inner
                .runtime
                .last_save
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
        {
            atomic_json(&self.dir.join("fullatlas.json"), &inner.saved)?;
            inner.runtime.last_save = Some(Instant::now());
        }
        Ok(())
    }
    fn catalog(&self, app: &AppHandle, refresh: bool) -> Result<Value, String> {
        // Serialize catalog reads with Mod installation/removal so a stale
        // request cannot put a removed pack back into the memoized catalog.
        let mut memo = self.catalog.lock().map_err(|_| "图鉴缓存锁失败")?;
        if !refresh {
            if let Some(value) = memo.as_ref() {
                return Ok(value.clone());
            }
        }
        let official = self.official_catalog(app, refresh)?;
        let region = if crate::profile::Region::from_app(app) == crate::profile::Region::Cn {
            "CN"
        } else {
            "GLOBAL"
        };
        // Mod 叠加失败不能让全图鉴跟着不可用：降级返回官方数据，把原因留给
        // 设置页显示。没有导入 Mod 时这里本来就走透传路径。
        let mut value = match crate::custom_resources::merge_official(
            &official,
            &self.dir.join("custom-atlas"),
            region,
        ) {
            Ok(value) => {
                if let Ok(mut slot) = self.custom_error.lock() {
                    *slot = None;
                }
                value
            }
            Err(error) => {
                if let Ok(mut slot) = self.custom_error.lock() {
                    *slot = Some(error);
                }
                let mut fallback = official;
                fallback["atlas_count"] =
                    json!(fallback["atlas"].as_array().map(Vec::len).unwrap_or(0));
                fallback
            }
        };
        // Mod 图片必须从本机 HTTP 取：官方渲染器只把 http(s)/asset 原样交给
        // <img>，而 asset:// 在这个 WebView 里取不到。前端拿这个前缀拼
        // `custom-assets/...` 相对键，与 Windows 侧 local_host 的语义一致。
        value["customAssetBase"] = json!(app.state::<crate::mod_assets::ModAssetHost>().base());
        *memo = Some(value.clone());
        Ok(value)
    }

    fn official_catalog(&self, app: &AppHandle, refresh: bool) -> Result<Value, String> {
        let mut memo = self
            .official_catalog
            .lock()
            .map_err(|_| "官方图鉴缓存锁失败")?;
        if !refresh {
            if let Some(value) = memo.as_ref() {
                return Ok(value.clone());
            }
        }
        let path = self.dir.join("fullatlas-catalog.json");
        if !refresh {
            let bundled = app
                .path()
                .resource_dir()
                .map_err(|e| e.to_string())?
                .join("fullatlas-catalog.json");
            for source in [&path, &bundled] {
                if let Ok(bytes) = fs::read(source) {
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                        if validate_catalog(&value).is_ok()
                            && value.get("assets_host").and_then(Value::as_str)
                                == Some(
                                    format!(
                                        "https://{}",
                                        crate::profile::Region::from_app(app).asset_host()
                                    )
                                    .as_str(),
                                )
                        {
                            *memo = Some(value.clone());
                            return Ok(value);
                        }
                    }
                }
            }
        }
        let region = crate::profile::Region::from_app(app);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("WuwaTappo-FullAtlas-Mac")
            .build()
            .map_err(|e| e.to_string())?;
        let api = format!("https://{}", region.api_host());
        let activity = get_json(
            &client,
            Url::parse(&format!("{api}/api/common/activity-config")).map_err(|e| e.to_string())?,
        )?;
        let atlas = get_json(
            &client,
            Url::parse(&format!("{api}/api/common/atlas")).map_err(|e| e.to_string())?,
        )?;
        if activity.get("code") != Some(&json!(0)) || atlas.get("code") != Some(&json!(0)) {
            return Err("官方目录请求失败，原缓存保留".into());
        }
        let host = activity
            .pointer("/data/assets_host")
            .and_then(Value::as_str)
            .ok_or("官方素材源缺失")?
            .trim_end_matches('/');
        trusted_asset_url(region, host)?;
        let pointer = get_json(
            &client,
            trusted_asset_url(
                region,
                &format!("{host}/app/json-host/appearance_asset_host.json"),
            )?,
        )?;
        let url = pointer
            .get("url")
            .and_then(Value::as_str)
            .ok_or("官方目录地址缺失")?;
        let catalog = get_json(&client, trusted_asset_url(region, url)?)?;
        let value = json!({"atlas":atlas["data"], "catalog":catalog, "assets_host":host, "assets_json_ts":activity.pointer("/data/assets_json_ts"), "checked_at":unix_time()});
        validate_catalog(&value)?;
        atomic_json(&path, &value)?;
        *memo = Some(value.clone());
        Ok(value)
    }
    fn snapshot(inner: &Inner) -> Value {
        let today = Local::now().date_naive();
        let weekly: Vec<Value> = (0..7)
            .rev()
            .map(|n| {
                let day = (today - Days::days(n)).to_string();
                json!({"count":inner.saved.days.get(&day).copied().unwrap_or(0), "date":day})
            })
            .collect();
        json!({"settings":inner.saved.settings, "session":inner.saved.session, "confirmed":inner.saved.confirmed, "session_chests":inner.saved.session_chests,
            "rewards":inner.saved.rewards, "rooms":inner.saved.rooms, "monitor":inner.saved.monitor, "runtime":inner.runtime.values,
            "flush_generation":inner.runtime.flush_generation, "flush_done":inner.runtime.flush_done, "weekly":{"normal":weekly}, "update_required":false})
    }
    fn assistant(&self, action: &str, body: &Value, physical_quiet: bool) -> Result<Value, String> {
        let mut inner = self.inner.lock().map_err(|_| "全图鉴数据锁失败")?;
        match action {
            "state" | "renderer-start" => return Ok(Self::snapshot(&inner)),
            "configure" => {
                let changes = body
                    .get("settings")
                    .and_then(Value::as_object)
                    .ok_or("设置必须是对象")?;
                let mut next = inner.saved.settings.clone();
                for (key, val) in changes {
                    match key.as_str() {
                        "enabled" => next.enabled = val.as_bool().ok_or("enabled 必须是布尔值")?,
                        "auto_chest" => {
                            next.auto_chest = val.as_bool().ok_or("auto_chest 必须是布尔值")?
                        }
                        "remember_monitor" => {
                            next.remember_monitor =
                                val.as_bool().ok_or("remember_monitor 必须是布尔值")?
                        }
                        "prerelease_updates" => {
                            next.prerelease_updates =
                                val.as_bool().ok_or("prerelease_updates 必须是布尔值")?
                        }
                        "speed" => {
                            let speed = val
                                .as_u64()
                                .filter(|v| [1, 5, 10].contains(v))
                                .ok_or("速度必须是 1、5 或 10")?;
                            next.speed = speed as u32;
                        }
                        "threshold" => {
                            next.threshold = val
                                .as_u64()
                                .filter(|v| (1..=1_000_000_000).contains(v))
                                .ok_or("开箱阈值无效")?
                        }
                        _ => return Err(format!("未知设置：{key}")),
                    }
                }
                inner.saved.settings = next;
            }
            "tick" | "observe" => {
                let sent = body.get("sent").and_then(Value::as_u64).unwrap_or(0);
                let confirmed = body.get("confirmed").and_then(Value::as_u64).unwrap_or(0);
                if sent > 1 || confirmed > sent {
                    return Err("点击确认值无效".into());
                }
                inner.saved.session = inner.saved.session.saturating_add(sent);
                inner.saved.confirmed = inner.saved.confirmed.saturating_add(confirmed);
                if let Some(total) = body.get("total").and_then(Value::as_u64) {
                    if total > 9_007_199_254_740_991 {
                        return Err("累计点击值超出有效范围".into());
                    }
                    let account = body
                        .get("account")
                        .and_then(Value::as_str)
                        .unwrap_or("guest");
                    if account.len() > 200 {
                        return Err("账号标识过长".into());
                    }
                    inner.runtime.account = account.into();
                    if let Some(before) = inner.saved.baselines.insert(account.into(), total) {
                        if total >= before {
                            *inner
                                .saved
                                .days
                                .entry(Local::now().date_naive().to_string())
                                .or_default() += total - before;
                        }
                    }
                    if inner.saved.baselines.len() > 100 {
                        inner.saved.baselines.retain(|key, _| key == account);
                    }
                    let cutoff = (Local::now().date_naive() - Days::days(90)).to_string();
                    inner.saved.days.retain(|date, _| date >= &cutoff);
                    inner.runtime.values = json!({"total":total,"balance":body.get("balance"),"logged_in":body.get("logged_in")==Some(&Value::Bool(true)),"chests":body.get("chests"),"local_atlas":inner.saved.enabled,"quiet":physical_quiet,"connected":true,"seen_at":unix_time()});
                }
                if action == "tick" {
                    let allowed = inner.saved.settings.enabled
                        && physical_quiet
                        && inner.runtime.next_tick.is_none_or(|t| Instant::now() >= t);
                    if allowed {
                        inner.runtime.next_tick = Some(
                            Instant::now()
                                + Duration::from_secs_f64(
                                    1.0 / f64::from(inner.saved.settings.speed),
                                ),
                        );
                    }
                    self.save(&mut inner, false)?;
                    return Ok(
                        json!({"allowed":allowed,"quiet":physical_quiet,"settings":inner.saved.settings,"session":inner.saved.session,"confirmed":inner.saved.confirmed,
                        "flush_generation":if inner.runtime.flush_generation>inner.runtime.flush_done{inner.runtime.flush_generation}else{0},"update_required":false,"quit":false}),
                    );
                }
            }
            "reward" => {
                let reward = body
                    .get("reward")
                    .and_then(Value::as_object)
                    .ok_or("奖励必须是对象")?;
                let id = reward
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty() && v.len() <= 120)
                    .ok_or("奖励事件 ID 无效")?;
                if reward.get("confirmed") != Some(&Value::Bool(true))
                    || !matches!(
                        reward.get("side").and_then(Value::as_str),
                        Some("left" | "right")
                    )
                    || reward.get("asset_name").and_then(Value::as_str).is_none()
                    || reward.get("cost").and_then(Value::as_u64).unwrap_or(0) == 0
                {
                    return Err("只保存原版服务器确认的奖励".into());
                }
                if !inner
                    .saved
                    .rewards
                    .iter()
                    .any(|r| r.get("id").and_then(Value::as_str) == Some(id))
                {
                    let mut record = json!({"time":unix_time(),"confirmed":true});
                    for field in [
                        "id",
                        "side",
                        "asset_id",
                        "asset_type",
                        "asset_name",
                        "name",
                        "rarity",
                        "cost",
                        "source",
                    ] {
                        if let Some(value) = reward.get(field) {
                            record[field] = value.clone();
                        }
                    }
                    inner.saved.rewards.insert(0, record);
                    inner.saved.rewards.truncate(1000);
                    inner.saved.session_chests += 1;
                }
            }
            "room" => {
                if inner.saved.enabled {
                    return Err("本地图鉴开启时不能加入房间".into());
                }
                let code = body
                    .get("code")
                    .and_then(Value::as_str)
                    .filter(|s| {
                        !s.is_empty()
                            && s.len() <= 32
                            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                    .ok_or("房间号无效")?;
                let new_visit = body.get("new_visit") == Some(&Value::Bool(true));
                if inner.saved.deleted_rooms.iter().any(|item| item == code) && !new_visit {
                    return Ok(Self::snapshot(&inner));
                }
                if new_visit {
                    inner.saved.deleted_rooms.retain(|item| item != code);
                }
                let favorite = inner
                    .saved
                    .rooms
                    .iter()
                    .find(|r| r["code"] == code)
                    .is_some_and(|r| r["favorite"] == true);
                let old_visited = inner
                    .saved
                    .rooms
                    .iter()
                    .find(|r| r["code"] == code)
                    .and_then(|r| r["visited"].as_u64());
                inner.saved.rooms.retain(|r| r["code"] != code);
                inner.saved.rooms.insert(0,json!({"code":code,"host":body.get("host").and_then(Value::as_str).unwrap_or("").chars().take(100).collect::<String>(),"favorite":favorite,"visited":if new_visit || old_visited.is_none(){unix_time()}else{old_visited.unwrap()}}));
                inner.saved.rooms.sort_by(|a, b| {
                    (
                        a["favorite"] != true,
                        std::cmp::Reverse(a["visited"].as_u64().unwrap_or(0)),
                    )
                        .cmp(&(
                            b["favorite"] != true,
                            std::cmp::Reverse(b["visited"].as_u64().unwrap_or(0)),
                        ))
                });
                inner.saved.rooms.truncate(100);
            }
            "delete-room" => {
                let code = body
                    .get("code")
                    .and_then(Value::as_str)
                    .filter(|s| {
                        !s.is_empty()
                            && s.len() <= 32
                            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                    .ok_or("房间号无效")?;
                inner.saved.rooms.retain(|r| r["code"] != code);
                inner.saved.deleted_rooms.retain(|item| item != code);
                inner.saved.deleted_rooms.push(code.to_owned());
                if inner.saved.deleted_rooms.len() > 100 {
                    let start = inner.saved.deleted_rooms.len() - 100;
                    inner.saved.deleted_rooms.drain(..start);
                }
            }
            "favorite" => {
                for room in &mut inner.saved.rooms {
                    if room.get("code") == body.get("code") {
                        room["favorite"] = json!(body.get("favorite") == Some(&Value::Bool(true)));
                    }
                }
                inner.saved.rooms.sort_by(|a, b| {
                    (
                        a["favorite"] != true,
                        std::cmp::Reverse(a["visited"].as_u64().unwrap_or(0)),
                    )
                        .cmp(&(
                            b["favorite"] != true,
                            std::cmp::Reverse(b["visited"].as_u64().unwrap_or(0)),
                        ))
                });
            }
            "monitor" => {
                let value = body.get("monitor").cloned().unwrap_or(Value::Null);
                if !value.is_null() && value.get("name").and_then(Value::as_str).is_none() {
                    return Err("显示器信息无效".into());
                }
                inner.saved.monitor = Some(value);
            }
            "reset" => {
                inner.saved.session = 0;
                inner.saved.confirmed = 0;
                inner.saved.session_chests = 0;
            }
            "flush-request" => {
                inner.runtime.flush_generation += 1;
                return Ok(json!({"flush_generation":inner.runtime.flush_generation}));
            }
            "flush-ack" => {
                if let Some(n) = body.get("generation").and_then(Value::as_u64) {
                    if n >= inner.runtime.flush_done && n <= inner.runtime.flush_generation {
                        inner.runtime.flush_done = n;
                    }
                }
            }
            _ => return Err(format!("未知助手操作：{action}")),
        }
        self.save(
            &mut inner,
            action != "observe" || body.get("flush") == Some(&Value::Bool(true)),
        )?;
        Ok(Self::snapshot(&inner))
    }
    fn request(
        &self,
        app: &AppHandle,
        label: &str,
        action: &str,
        body: &Value,
    ) -> Result<Value, String> {
        if label != "main"
            && matches!(
                action,
                "assistant/tick"
                    | "assistant/observe"
                    | "assistant/reward"
                    | "assistant/room"
                    | "assistant/monitor"
                    | "assistant/renderer-start"
                    | "assistant/flush-ack"
            )
        {
            return Err("该操作仅允许主桌宠执行".into());
        }
        match action {
            "official/releases" => crate::system::open_community_releases(app)
                .map(|()| json!({"ok": true, "url": COMMUNITY_RELEASES_PAGE})),
            "official/pet-site" => {
                let region = crate::profile::Region::from_app(app);
                crate::system::open_pet_site(app, region)
                    .map(|()| json!({"ok": true, "url": region.pet_site()}))
            }
            "community/version" => Ok(json!({"current": community_current_version()})),
            "community/update-check" => {
                // 是否把预发布算进候选由设置决定；读不到设置时按“不含预发布”处理。
                let include = self
                    .inner
                    .lock()
                    .map(|inner| inner.saved.settings.prerelease_updates)
                    .unwrap_or(false);
                community_update_status(include)
            }
            "official/update-check" => {
                official_update_status(crate::profile::Region::from_app(app))
            }
            "official/download" => {
                download_official_installer(app, crate::profile::Region::from_app(app))
            }
            "status" => Ok(
                json!({"ready":true,"enabled":self.enabled(),"mode_verified":true,"network_policy":"rooms-only","sync":{"status":"idle"}}),
            ),
            "offline-data" => self.catalog(app, false),
            // 面板/背包要缩略图时不一定会先拉 offline-data，单独给一个轻量入口。
            "asset-base" => Ok(json!({
                "base": app.state::<crate::mod_assets::ModAssetHost>().base()
            })),
            "sync" => {
                let v = self.official_catalog(app, true)?;
                *self.catalog.lock().map_err(|_| "图鉴缓存锁失败")? = None;
                Ok(
                    json!({"status":"complete","checked_at":unix_time(),"result":{"changed":true,"assets_json_ts":v["assets_json_ts"],"atlas_count":v["atlas"].as_array().map(Vec::len).unwrap_or(0)}}),
                )
            }
            "custom/status" => {
                let _catalog = self.catalog.lock().map_err(|_| "图鉴缓存锁失败")?;
                let region = if crate::profile::Region::from_app(app) == crate::profile::Region::Cn
                {
                    "CN"
                } else {
                    "GLOBAL"
                };
                let packs = crate::custom_resources::list_details(
                    &self.dir.join("custom-atlas"),
                    region,
                )?;
                let items: usize = packs
                    .iter()
                    .map(|pack| pack["item_count"].as_u64().unwrap_or(0) as usize)
                    .sum();
                let assets: usize = packs
                    .iter()
                    .map(|pack| pack["asset_count"].as_u64().unwrap_or(0) as usize)
                    .sum();
                let error = self
                    .custom_error
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone());
                Ok(
                    json!({"enabled":self.enabled(),"region":region,"installed":!packs.is_empty(),"packs":packs,"atlas_count":items,"asset_count":assets,"error":error}),
                )
            }
            "custom/import" => {
                let mut memo = self.catalog.lock().map_err(|_| "图鉴缓存锁失败")?;
                let path = body
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("请选择 ZIP 资源包")?;
                let region = if crate::profile::Region::from_app(app) == crate::profile::Region::Cn
                {
                    "CN"
                } else {
                    "GLOBAL"
                };
                let summary = crate::custom_resources::install_archive(
                    Path::new(path),
                    &self.dir.join("custom-atlas"),
                    region,
                )?;
                *memo = None;
                Ok(
                    json!({"installed":true,"id":summary.id,"name":summary.name,"item_count":summary.item_count,"asset_count":summary.asset_count,"region":region}),
                )
            }
            "custom/remove" => {
                let mut memo = self.catalog.lock().map_err(|_| "图鉴缓存锁失败")?;
                let id = body
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("Mod ID 无效")?;
                let root = self.dir.join("custom-atlas");
                crate::custom_resources::remove_pack(&root, id)?;
                let region = if crate::profile::Region::from_app(app) == crate::profile::Region::Cn
                {
                    "CN"
                } else {
                    "GLOBAL"
                };
                let any = !crate::custom_resources::list_summaries(&root, region)?.is_empty();
                *memo = None;
                Ok(
                    json!({"removed":id,"any_installed":any}),
                )
            }
            "mode" => {
                let enabled = body
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("模式必须是布尔值")?;
                if enabled {
                    self.catalog(app, false)?;
                }
                let mut inner = self.inner.lock().map_err(|_| "全图鉴数据锁失败")?;
                inner.saved.enabled = enabled;
                self.save(&mut inner, true)?;
                drop(inner);
                if enabled {
                    crate::socket::shutdown(app);
                }
                app.emit("fullatlas-mode-changed", enabled)
                    .map_err(|e| e.to_string())?;
                Ok(json!({"enabled":enabled,"mode_verified":true,"network_policy":"rooms-only"}))
            }
            "update/status" | "update/check" | "update/healthy" => Ok(
                json!({"required":false,"version":env!("CARGO_PKG_VERSION"),"message":UPDATE_MESSAGE}),
            ),
            "update/install" => Err(UPDATE_MESSAGE.into()),
            "diagnostic" => Ok(json!({"ok":true})),
            name if name.starts_with("assistant/") => {
                let quiet = app.state::<crate::input::InputMonitor>().status().running
                    && crate::input::physical_input_quiet();
                self.assistant(name.trim_start_matches("assistant/"), body, quiet)
            }
            _ => Err(format!("未知全图鉴操作：{action}")),
        }
    }
}

#[tauri::command]
pub async fn fullatlas_request(
    app: AppHandle,
    window: WebviewWindow,
    action: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    if !local_window(window.label(), &window.url().map_err(|e| e.to_string())?) {
        return Err("全图鉴仅允许本机主窗口与面板访问".into());
    }
    let body = payload.unwrap_or_else(|| json!({}));
    if !body.is_object() || serde_json::to_vec(&body).map_err(|e| e.to_string())?.len() > 16384 {
        return Err("全图鉴请求格式无效或过大".into());
    }
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<FullAtlasState>()
            .request(&app, &label, &action, &body)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> FullAtlasState {
        FullAtlasState::new(std::env::temp_dir().join(format!(
                "fullatlas-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
        .unwrap()
    }

    #[test]
    fn official_version_check_only_reports_a_higher_release() {
        assert!(official_is_newer("1.0.4", "1.0.3"));
        assert!(official_is_newer("1.0.10", "1.0.3"));
        assert!(!official_is_newer("1.0.3", "1.0.3"));
        assert!(!official_is_newer("1.0.2", "1.0.3"));
    }

    #[test]
    fn community_version_check_understands_prerelease_ordering() {        // 预发布序列内部按数字比较：alpha.10 必须高于 alpha.9，而不是字典序的 alpha.1x。
        assert_eq!(compare_semver("1.3.0-alpha.10", "1.3.0-alpha.9"), std::cmp::Ordering::Greater);
        assert_eq!(compare_semver("1.3.0-alpha.2", "1.3.0-alpha.1"), std::cmp::Ordering::Greater);
        // 同号正式版高于预发布。
        assert_eq!(compare_semver("1.3.0", "1.3.0-alpha.2"), std::cmp::Ordering::Greater);
        assert_eq!(compare_semver("1.3.0-alpha.1", "1.3.0"), std::cmp::Ordering::Less);
        // 主体版本号优先于预发布段。
        assert_eq!(compare_semver("1.4.0-alpha.1", "1.3.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare_semver("1.3.0", "1.3.0"), std::cmp::Ordering::Equal);
        // tag 前缀 v 和空格都要能吃掉。
        assert_eq!(compare_semver(" v1.3.1 ", "1.3.0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn prerelease_releases_are_hidden_unless_the_setting_is_on() {
        let releases = vec![
            json!({"tag_name":"v1.3.0-alpha.2","prerelease":true,"html_url":"u2","published_at":"p2"}),
            json!({"tag_name":"v1.3.0-alpha.1","prerelease":true,"html_url":"u1","published_at":"p1"}),
            json!({"tag_name":"v1.2.8","prerelease":false,"html_url":"u0","published_at":"p0"}),
            json!({"tag_name":"v9.9.9","draft":true,"prerelease":false,"html_url":"ud"}),
        ];
        // 关闭时：跳过两个预发布、跳过草稿，只剩正式版 1.2.8。
        let (off, skipped) = pick_latest_release(&releases, false);
        assert_eq!(skipped, 2);
        assert_eq!(off.map(|(version, _, _)| version), Some("1.2.8".to_string()));
        // 打开时：alpha.2 高于 alpha.1，仍然是最高。
        let (on, skipped) = pick_latest_release(&releases, true);
        assert_eq!(skipped, 0);
        assert_eq!(on.map(|(version, _, _)| version), Some("1.3.0-alpha.2".to_string()));
        // 只有预发布时，关闭开关会得到空结果——界面据此提示“只有预发布”。
        let only_alpha = vec![json!({"tag_name":"v1.4.0-beta.1","prerelease":true})];
        let (empty, skipped) = pick_latest_release(&only_alpha, false);
        assert!(empty.is_none());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn official_installer_requires_the_current_region_and_versioned_filename() {
        let manifest = json!({
            "platforms": {
                "windows-x86_64": {
                    "url": "https://mc-wuwatappo.kurogames-global.com/app/WuwaTappo-cn_1.0.4_x64-setup.exe"
                }
            }
        });
        let (url, filename) =
            official_installer(crate::profile::Region::Cn, &manifest, "1.0.4").unwrap();
        assert_eq!(url.host_str(), Some("mc-wuwatappo.kurogames-global.com"));
        assert_eq!(filename, "WuwaTappo-cn_1.0.4_x64-setup.exe");
        for value in [
            "https://example.com/WuwaTappo-cn_1.0.4_x64-setup.exe",
            "https://mc-wuwatappo.kurogames-global.com/app/WuwaTappo-global_1.0.4_x64-setup.exe",
        ] {
            let invalid = json!({"platforms": {"windows-x86_64": {"url": value}}});
            assert!(official_installer(crate::profile::Region::Cn, &invalid, "1.0.4").is_err());
        }
    }
    #[test]
    fn assistant_uses_baselines_and_limits_rate() {
        let s = state();
        s.assistant("observe", &json!({"account":"a","total":100}), true)
            .unwrap();
        s.assistant("observe", &json!({"account":"a","total":115}), true)
            .unwrap();
        s.assistant("observe", &json!({"account":"b","total":900}), true)
            .unwrap();
        s.assistant(
            "configure",
            &json!({"settings":{"enabled":true,"speed":10}}),
            true,
        )
        .unwrap();
        assert_eq!(
            s.assistant("tick", &json!({}), false).unwrap()["allowed"],
            false
        );
        assert_eq!(
            s.assistant("tick", &json!({}), true).unwrap()["allowed"],
            true
        );
        assert_eq!(
            s.assistant("tick", &json!({}), true).unwrap()["allowed"],
            false
        );
        let snapshot = s.assistant("state", &json!({}), true).unwrap();
        assert_eq!(
            snapshot["weekly"]["normal"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["count"],
            15
        );
        assert!(s
            .assistant("configure", &json!({"settings":{"speed":20}}), true)
            .is_err());
    }
    #[test]
    fn rewards_require_confirmation_and_deduplicate() {
        let s = state();
        let r = json!({"reward":{"id":"a","side":"left","confirmed":true,"asset_name":"skin","cost":1000}});
        s.assistant("reward", &r, false).unwrap();
        s.assistant("reward", &r, false).unwrap();
        let v = s.assistant("state", &json!({}), false).unwrap();
        assert_eq!(v["rewards"].as_array().unwrap().len(), 1);
        assert_eq!(v["session_chests"], 1);
        assert!(s
            .assistant("reward", &json!({"reward":{"id":"fake"}}), false)
            .is_err());
        s.assistant("reset", &json!({}), false).unwrap();
        assert_eq!(
            s.assistant("state", &json!({}), false).unwrap()["rewards"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn saved_mode_and_auto_tasks_are_independent() {
        let s = state();
        s.assistant("configure", &json!({"settings":{"enabled":true}}), false)
            .unwrap();
        assert!(!s.enabled());
        let loaded = FullAtlasState::new(s.dir.clone()).unwrap();
        assert_eq!(
            loaded.assistant("state", &json!({}), false).unwrap()["settings"]["enabled"],
            true
        );
    }

    #[test]
    fn deleted_rooms_stay_hidden_until_a_new_visit() {
        let s = state();
        s.assistant(
            "room",
            &json!({"code":"123456","host":"Alice","new_visit":true}),
            false,
        )
        .unwrap();
        s.assistant("delete-room", &json!({"code":"123456"}), false)
            .unwrap();
        assert!(s
            .assistant(
                "room",
                &json!({"code":"123456","host":"Alice","new_visit":false}),
                false
            )
            .unwrap()["rooms"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            s.assistant(
                "room",
                &json!({"code":"123456","host":"Alice 2","new_visit":true}),
                false
            )
            .unwrap()["rooms"][0]["host"],
            "Alice 2"
        );
    }
    #[test]
    fn only_local_main_and_panel_can_control() {
        for label in ["main", "panel"] {
            assert!(local_window(
                label,
                &Url::parse("tauri://localhost/index.html").unwrap()
            ));
        }
        assert!(!local_window(
            "login",
            &Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(!local_window(
            "main",
            &Url::parse("https://example.com").unwrap()
        ));
        assert!(trusted_asset_url(
            crate::profile::Region::Cn,
            "https://evil.invalid/catalog.json"
        )
        .is_err());
    }
}
