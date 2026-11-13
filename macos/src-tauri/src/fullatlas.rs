//! Native transport and state for the MIT FullAtlas controllers. No OS input
//! injection, Windows updater, or simulated rewards. The only local HTTP
//! listener is the read-only Mod image route in `mod_assets`.
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use wuwatappo_assistant_core::AssistantCore;

const UPDATE_MESSAGE: &str =
    "这是 Mac 整合版；Windows FullAtlas 更新包不能安装到 Mac。请使用本项目的 Mac 安装包更新。";
const MAX_OFFICIAL_INSTALLER: u64 = 512 * 1024 * 1024;
const COMMUNITY_RELEASES_URL: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    enabled: bool,
}

#[derive(Default)]
struct Runtime {
    last_save: Option<Instant>,
}
struct Inner {
    saved: Saved,
    assistant: AssistantCore,
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
const COMMUNITY_RELEASES_PAGE: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

/// 全图鉴整合包的发布版本；与 `package.json` 同源（bump-version 同步 Cargo.toml）。
fn community_current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn community_update_status(include_prerelease: bool) -> Result<Value, String> {
    wuwatappo_assistant_core::release_updates::check_community(
        community_current_version(),
        include_prerelease,
    )
}

fn official_update_status(region: crate::profile::Region) -> Result<Value, String> {
    let region = if region == crate::profile::Region::Cn {
        "cn"
    } else {
        "global"
    };
    let mut result = wuwatappo_assistant_core::release_updates::check_official(
        region,
        env!("WUWA_OFFICIAL_VERSION"),
    )?;
    result["communityReleasesUrl"] = json!(COMMUNITY_RELEASES_URL);
    result["supportsLocalAdaptation"] = json!(false);
    result["localAdaptationReason"] =
        json!("已安装的 Mac 应用没有源码、Node、Rust、Xcode 和 7-Zip，不能在应用内安全重打包。");
    result["local_adaptation_supported"] = json!(false);
    result["local_adaptation_reason"] = result["localAdaptationReason"].clone();
    result["message"] = json!(if result["newer"].as_bool() == Some(true) {
        format!("检测到官方 {}，可下载官方 Windows 原包；Mac 全图鉴包需由本项目构建后从 Releases 更新。", result["remote"].as_str().unwrap_or(""))
    } else {
        result["message"]
            .as_str()
            .unwrap_or("官方版本检查完成")
            .to_string()
    });
    Ok(result)
}

fn download_official_installer(
    app: &AppHandle,
    region: crate::profile::Region,
) -> Result<Value, String> {
    let region_id = if region == crate::profile::Region::Cn {
        "cn"
    } else {
        "global"
    };
    let status = wuwatappo_assistant_core::release_updates::check_official(
        region_id,
        env!("WUWA_OFFICIAL_VERSION"),
    )?;
    if status["newer"].as_bool() != Some(true) {
        return Err(status["message"]
            .as_str()
            .unwrap_or("没有可下载的新版本")
            .to_string());
    }
    let latest = status["remote"]
        .as_str()
        .ok_or("官方版本检查没有返回版本号")?;
    let url = Url::parse(
        status["officialInstallerUrl"]
            .as_str()
            .ok_or("官方版本检查没有返回安装包地址")?,
    )
    .map_err(|_| "官方安装包地址无效")?;
    let filename = status["officialInstallerFileName"]
        .as_str()
        .ok_or("官方版本检查没有返回安装包文件名")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("WuwaTappo-Full-Community-Mac")
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
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
        let bytes = if path.exists() {
            Some(fs::read(&path).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let saved = bytes
            .as_deref()
            .map(|bytes| serde_json::from_slice(bytes))
            .transpose()
            .map_err(|e| format!("FullAtlas 状态文件损坏，请备份后修复：{e}"))?
            .unwrap_or_default();
        let core_state = bytes
            .as_deref()
            .and_then(|value| serde_json::from_slice(value).ok());
        Ok(Self {
            dir,
            inner: Mutex::new(Inner {
                saved,
                assistant: AssistantCore::new(core_state),
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
            atomic_json(&self.dir.join("fullatlas.json"), &inner.assistant.saved())?;
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
        let region = if crate::profile::Region::from_app(app) == crate::profile::Region::Cn {
            "cn"
        } else {
            "global"
        };
        let snapshot = wuwatappo_assistant_core::resource_cache::fetch_catalog(region)?;
        let host = snapshot.assets_host.as_str().trim_end_matches('/');
        let value = json!({
            "atlas": snapshot.atlas["data"],
            "catalog": snapshot.catalog,
            "assets_host": host,
            "assets_json_ts": snapshot.activity.pointer("/data/assets_json_ts"),
            "checked_at": unix_time()
        });
        validate_catalog(&value)?;
        atomic_json(&path, &value)?;
        *memo = Some(value.clone());
        Ok(value)
    }
    fn assistant(&self, action: &str, body: &Value, physical_quiet: bool) -> Result<Value, String> {
        let mut inner = self.inner.lock().map_err(|_| "全图鉴数据锁失败")?;
        if action == "typing-context" {
            if !inner.assistant.saved()["settings"]["auto_emote"]
                .as_bool()
                .unwrap_or(false)
            {
                return Ok(json!({"editable": false, "reason": "disabled"}));
            }
            drop(inner);
            return Ok(crate::input::focused_text_context());
        }
        let offline = inner.assistant.app_enabled();
        let result = inner.assistant.handle(
            action,
            body,
            unix_time() as f64,
            physical_quiet,
            offline,
            false,
        )?;
        self.save(
            &mut inner,
            action != "observe" || body.get("flush") == Some(&Value::Bool(true)),
        )?;
        Ok(result)
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
                    .map(|inner| {
                        inner.assistant.saved()["settings"]["prerelease_updates"]
                            .as_bool()
                            .unwrap_or(false)
                    })
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
                let packs =
                    crate::custom_resources::list_details(&self.dir.join("custom-atlas"), region)?;
                let items: usize = packs
                    .iter()
                    .map(|pack| pack["item_count"].as_u64().unwrap_or(0) as usize)
                    .sum();
                let assets: usize = packs
                    .iter()
                    .map(|pack| pack["asset_count"].as_u64().unwrap_or(0) as usize)
                    .sum();
                let error = self.custom_error.lock().ok().and_then(|slot| slot.clone());
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
                Ok(json!({"removed":id,"any_installed":any}))
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
                inner.assistant.set_app_enabled(enabled);
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
    fn tick_does_not_own_shared_balance_threshold_policy() {
        let s = state();
        s.assistant(
            "configure",
            &json!({"settings":{"auto_click_balance":true}}),
            true,
        )
        .unwrap();
        assert_eq!(
            s.assistant("tick", &json!({"balance":9_999}), true)
                .unwrap()["settings"]["enabled"],
            false
        );
        assert!(s
            .assistant(
                "configure",
                &json!({"settings":{"auto_click_below":40_000}}),
                true
            )
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
    }
}
