//! Native transport and state for the MIT FullAtlas controllers. No localhost
//! HTTP server, OS input injection, Windows updater, or simulated rewards.
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
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            speed: 5,
            auto_chest: false,
            threshold: 1000,
            remember_monitor: true,
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
        .ok_or("官方版本检查缺少版本清单")?;
    let manifest_url = trusted_asset_url(region, manifest_url)?;
    let manifest = get_json(&client, manifest_url)?;
    let latest = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("官方版本清单无效")?;
    let current = env!("WUWA_OFFICIAL_VERSION");
    let changed = official_is_newer(latest, current);
    let (installer_url, installer_name) = official_installer(region, &manifest, latest)?;
    Ok(json!({
        "currentVersion": current,
        "latestVersion": latest,
        "updateAvailable": changed,
        "officialInstallerUrl": installer_url.as_str(),
        "officialInstallerFileName": installer_name,
        "officialInstallerPlatform": "windows-x86_64",
        "communityReleasesUrl": COMMUNITY_RELEASES_URL,
        "canDownloadOfficial": true,
        "supportsLocalAdaptation": false,
        "localAdaptationReason": "已安装的 Mac 应用没有源码、Node、Rust、Xcode 和 7-Zip，不能在应用内安全重打包。",
        "message": if changed { format!("检测到官方 {latest}，可下载官方 Windows 原包；Mac 全图鉴包需由本项目构建后从 Releases 更新。") } else { format!("官方基底已是 {current}。") },
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
        let mut memo = self.catalog.lock().map_err(|_| "图鉴缓存锁失败")?;
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
            "official/update-check" => official_update_status(crate::profile::Region::from_app(app)),
            "official/download" => download_official_installer(app, crate::profile::Region::from_app(app)),
            "status"=>Ok(json!({"ready":true,"enabled":self.enabled(),"mode_verified":true,"network_policy":"rooms-only","sync":{"status":"idle"}})),
            "offline-data"=>self.catalog(app,false),
            "sync"=>self.catalog(app,true).map(|v|json!({"status":"complete","checked_at":unix_time(),"result":{"changed":true,"assets_json_ts":v["assets_json_ts"],"atlas_count":v["atlas"].as_array().map(Vec::len).unwrap_or(0)}})),
            "mode"=>{let enabled=body.get("enabled").and_then(Value::as_bool).ok_or("模式必须是布尔值")?; if enabled{self.catalog(app,false)?;}let mut inner=self.inner.lock().map_err(|_|"全图鉴数据锁失败")?;inner.saved.enabled=enabled;self.save(&mut inner,true)?;drop(inner);if enabled{crate::socket::shutdown(app);}app.emit("fullatlas-mode-changed",enabled).map_err(|e|e.to_string())?;Ok(json!({"enabled":enabled,"mode_verified":true,"network_policy":"rooms-only"}))},
            "update/status"|"update/check"|"update/healthy"=>Ok(json!({"required":false,"version":env!("CARGO_PKG_VERSION"),"message":UPDATE_MESSAGE})),
            "update/install"=>Err(UPDATE_MESSAGE.into()),
            "diagnostic"=>Ok(json!({"ok":true})),
            name if name.starts_with("assistant/")=>{let quiet=app.state::<crate::input::InputMonitor>().status().running&&crate::input::physical_input_quiet();self.assistant(name.trim_start_matches("assistant/"),body,quiet)},
            _=>Err(format!("未知全图鉴操作：{action}")),
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
