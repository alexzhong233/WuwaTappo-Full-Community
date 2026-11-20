//! Cross-platform runtime business service shared by macOS (Tauri IPC) and
//! Windows (`LocalGalleryHost` HTTP bridge).
//!
//! Unifies assistant state, offline catalog/PNG cache readiness & background sync,
//! custom Mod overlays, official & community version checks, update policy status,
//! and diagnostics so both desktop shells expose identical contracts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    append_diagnostic, custom_resources, release_installer, release_updates, resource_cache,
    AssistantCore,
};

pub const COMMUNITY_RELEASES_URL: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

#[derive(Clone, Debug)]
pub enum PersistenceMode {
    /// macOS stores `{ enabled, ...AssistantCore }` atomically in a single file (`fullatlas.json`).
    CombinedFile(String),
    /// Windows stores `assistant_state.json` + `offline_state.json` separately.
    SplitFiles,
}

#[derive(Clone)]
pub struct ServiceConfig {
    pub root: PathBuf,
    pub persistence: PersistenceMode,
    pub region_id: String,
    pub region_code: String,
    pub community_version: String,
    pub client_version: String,
    pub expected_asset_count: Option<u64>,
    pub on_sync_complete: Option<Arc<dyn Fn(&resource_cache::SyncSummary) + Send + Sync>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    BadRequest,
    Conflict,
    Internal,
}

#[derive(Clone, Debug)]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ServiceError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::BadRequest,
            message: msg.into(),
        }
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Conflict,
            message: msg.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            message: msg.into(),
        }
    }

    pub fn status_code(&self) -> u16 {
        match self.kind {
            ErrorKind::BadRequest => 400,
            ErrorKind::Conflict => 409,
            ErrorKind::Internal => 500,
        }
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct CombinedSaved {
    enabled: bool,
}

struct AssistantInner {
    enabled: bool,
    assistant: AssistantCore,
    last_save: Option<Instant>,
}

pub struct InstallGuard {
    service: Arc<FullAtlasService>,
    armed: bool,
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if self.armed {
            self.service.installing.store(false, Ordering::Release);
        }
    }
}

impl InstallGuard {
    pub fn disarm(mut self) {
        self.armed = false;
    }
}

pub struct FullAtlasService {
    root: PathBuf,
    persistence: PersistenceMode,
    region_id: String,
    region_code: String,
    community_version: String,
    client_version: String,
    expected_asset_count: Mutex<Option<u64>>,
    inner: Mutex<AssistantInner>,
    catalog: Mutex<Option<Value>>,
    custom_error: Mutex<Option<String>>,
    sync: Mutex<Value>,
    updates: release_updates::UpdateTracker,
    installing: AtomicBool,
    on_sync_complete: Option<Arc<dyn Fn(&resource_cache::SyncSummary) + Send + Sync>>,
}

fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT_TMP: AtomicUsize = AtomicUsize::new(0);
    let parent = path.parent().ok_or("Missing state directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temp_or_tmp(&tmp), fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(unix)]
fn temp_or_tmp(p: &Path) -> &Path {
    p
}

fn read_json_value(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

impl FullAtlasService {
    pub fn new(config: ServiceConfig) -> Result<Self, String> {
        let (enabled, assistant_saved) = match &config.persistence {
            PersistenceMode::CombinedFile(filename) => {
                let path = config.root.join(filename);
                let bytes = if path.exists() {
                    Some(fs::read(&path).map_err(|e| e.to_string())?)
                } else {
                    None
                };
                let saved_flag: CombinedSaved = bytes
                    .as_deref()
                    .map(serde_json::from_slice)
                    .transpose()
                    .map_err(|e| format!("FullAtlas 状态文件损坏，请备份后修复：{e}"))?
                    .unwrap_or_default();
                let core_state = bytes.as_deref().and_then(|v| serde_json::from_slice(v).ok());
                (saved_flag.enabled, core_state)
            }
            PersistenceMode::SplitFiles => {
                let core_state = fs::read(config.root.join("assistant_state.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok());
                let offline_enabled = fs::read_to_string(config.root.join("offline_state.json"))
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|v| v.get("enabled").and_then(Value::as_bool))
                    .unwrap_or(false);
                (offline_enabled, core_state)
            }
        };

        let mut assistant = AssistantCore::new(assistant_saved);
        assistant.set_app_enabled(enabled);

        Ok(Self {
            root: config.root,
            persistence: config.persistence,
            region_id: config.region_id,
            region_code: config.region_code,
            community_version: config.community_version,
            client_version: config.client_version,
            expected_asset_count: Mutex::new(config.expected_asset_count),
            inner: Mutex::new(AssistantInner {
                enabled,
                assistant,
                last_save: None,
            }),
            catalog: Mutex::new(None),
            custom_error: Mutex::new(None),
            sync: Mutex::new(json!({"status":"idle","done":0,"total":0,"checked_at":0})),
            updates: Default::default(),
            installing: AtomicBool::new(false),
            on_sync_complete: config.on_sync_complete,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn custom_root(&self) -> PathBuf {
        self.root.join("custom-atlas")
    }

    pub fn region_id(&self) -> &str {
        &self.region_id
    }

    pub fn region_code(&self) -> &str {
        &self.region_code
    }

    pub fn community_version(&self) -> Value {
        json!({ "current": self.community_version })
    }

    pub fn current_community_version(&self) -> &str {
        &self.community_version
    }

    pub fn current_client_version(&self) -> &str {
        &self.client_version
    }

    pub fn pet_site_url(&self) -> String {
        crate::regions::profile(&self.region_id)
            .map(|p| p.pet_site.clone())
            .unwrap_or_else(|| "https://download-mc-wuwatappo.kurogames-global.com/download/".into())
    }

    pub fn enabled(&self) -> bool {
        self.inner.lock().map(|i| i.enabled).unwrap_or(false)
    }

    pub fn include_prerelease(&self) -> bool {
        self.inner
            .lock()
            .map(|i| {
                i.assistant.saved()["settings"]["prerelease_updates"]
                    .as_bool()
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    fn expected_assets(&self) -> Option<u64> {
        *self.expected_asset_count.lock().unwrap()
    }

    pub fn set_expected_asset_count(&self, count: Option<u64>) {
        *self.expected_asset_count.lock().unwrap() = count;
    }

    pub fn cache_ready(&self) -> bool {
        resource_cache::is_cache_ready(
            &self.root,
            self.expected_assets().map(|v| v as usize),
        )
    }

    pub fn cache_summary(&self) -> Value {
        resource_cache::cache_summary(&self.root, self.expected_assets())
    }

    fn save_assistant(&self, inner: &mut AssistantInner, force: bool) -> Result<(), String> {
        if !force
            && inner
                .last_save
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        {
            return Ok(());
        }
        let path = match &self.persistence {
            PersistenceMode::CombinedFile(filename) => self.root.join(filename),
            PersistenceMode::SplitFiles => self.root.join("assistant_state.json"),
        };
        let bytes = serde_json::to_vec(&inner.assistant.saved()).map_err(|e| e.to_string())?;
        atomic_write_bytes(&path, &bytes)?;
        inner.last_save = Some(Instant::now());
        Ok(())
    }

    pub fn status_snapshot(&self, extra_fields: Option<Map<String, Value>>) -> Value {
        let mut obj = Map::new();
        obj.insert("ready".into(), json!(self.cache_ready()));
        obj.insert("enabled".into(), json!(self.enabled()));
        obj.insert("mode_verified".into(), json!(true));
        obj.insert("network_policy".into(), json!("hybrid"));
        obj.insert("sync".into(), self.sync_status());
        obj.insert("cache".into(), self.cache_summary());
        if let Some(extra) = extra_fields {
            for (k, v) in extra {
                obj.insert(k, v);
            }
        }
        Value::Object(obj)
    }

    pub fn sync_status(&self) -> Value {
        self.sync.lock().unwrap().clone()
    }

    pub fn start_sync(self: &Arc<Self>, force: bool) -> Value {
        let mut guard = self.sync.lock().unwrap();
        if guard["status"] == "running" {
            return guard.clone();
        }
        let checked_at = guard["checked_at"].as_f64().unwrap_or(0.0);
        if !force && checked_at > 0.0 && now_f64() - checked_at < 900.0 {
            return guard.clone();
        }
        *guard = json!({
            "status": "running",
            "done": 0,
            "total": 0,
            "checked_at": checked_at,
        });
        let snapshot = guard.clone();
        drop(guard);

        let svc = Arc::clone(self);
        std::thread::spawn(move || {
            let progress_svc = Arc::clone(&svc);
            let result = resource_cache::sync_with_progress(
                &svc.root,
                &svc.region_id,
                force,
                move |done, total| {
                    let mut g = progress_svc.sync.lock().unwrap();
                    *g = json!({
                        "status": "running",
                        "done": done,
                        "total": total,
                        "checked_at": now_f64(),
                    });
                },
            );
            match result {
                Ok(summary) => {
                    svc.set_expected_asset_count(Some(summary.asset_count as u64));
                    if let Ok(mut memo) = svc.catalog.lock() {
                        *memo = None;
                    }
                    if let Some(cb) = &svc.on_sync_complete {
                        cb(&summary);
                    }
                    let mut g = svc.sync.lock().unwrap();
                    *g = json!({
                        "status": "complete",
                        "done": summary.asset_count,
                        "total": summary.asset_count,
                        "checked_at": now_f64(),
                        "result": {
                            "changed": summary.changed,
                            "atlas_count": summary.atlas_count,
                            "asset_count": summary.asset_count,
                        }
                    });
                }
                Err(error) => {
                    let mut g = svc.sync.lock().unwrap();
                    *g = json!({
                        "status": "failed",
                        "done": 0,
                        "total": 0,
                        "checked_at": now_f64(),
                        "error": error,
                    });
                }
            }
        });
        snapshot
    }

    pub fn ensure_initial_sync(self: &Arc<Self>) {
        if !self.cache_ready() {
            let _ = self.start_sync(false);
        }
    }

    pub fn offline_bundle(
        &self,
        custom_asset_base: Option<&str>,
    ) -> Result<Value, ServiceError> {
        if !self.cache_ready() {
            return Err(ServiceError::conflict(
                "本地素材缓存不完整，请重新同步后再开启本地图鉴",
            ));
        }
        let mut memo = self
            .catalog
            .lock()
            .map_err(|_| ServiceError::internal("图鉴缓存锁失败"))?;
        let mut bundle = if let Some(cached) = memo.as_ref() {
            cached.clone()
        } else {
            let cache = self.cache_dir();
            let catalog = read_json_value(&cache.join("offline_catalog.json"))
                .map_err(ServiceError::conflict)?;
            let atlas_doc =
                read_json_value(&cache.join("atlas.json")).map_err(ServiceError::conflict)?;
            let atlas = atlas_doc
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| ServiceError::conflict("图鉴缓存结构无效"))?;
            let official = json!({
                "catalog": catalog,
                "atlas": atlas,
                "atlas_count": atlas.len(),
                "asset_count": self.expected_assets().unwrap_or(0),
            });
            let (merged, custom_err) = custom_resources::merge_or_official(
                &official,
                &self.custom_root(),
                &self.region_code,
            );
            if let Ok(mut slot) = self.custom_error.lock() {
                *slot = custom_err;
            }
            *memo = Some(merged.clone());
            merged
        };
        if let Some(base) = custom_asset_base {
            bundle["customAssetBase"] = json!(base);
        }
        Ok(bundle)
    }

    pub fn custom_status(&self) -> Value {
        let enabled = self.enabled();
        let error = self.custom_error.lock().ok().and_then(|s| s.clone());
        custom_resources::status(&self.custom_root(), &self.region_code, enabled, error)
            .unwrap_or_else(|err| {
                json!({
                    "installed": false,
                    "enabled": enabled,
                    "name": "",
                    "atlas_count": 0,
                    "asset_count": 0,
                    "packs": [],
                    "region": self.region_code,
                    "error": err,
                })
            })
    }

    pub fn custom_import(&self, archive_path: &Path) -> Result<Value, ServiceError> {
        let mut memo = self
            .catalog
            .lock()
            .map_err(|_| ServiceError::internal("图鉴缓存锁失败"))?;
        let summary = custom_resources::install_archive(
            archive_path,
            &self.custom_root(),
            &self.region_code,
        )
        .map_err(ServiceError::conflict)?;
        *memo = None;
        if let Ok(mut slot) = self.custom_error.lock() {
            *slot = None;
        }
        Ok(json!({
            "installed": true,
            "id": summary.id,
            "name": summary.name,
            "item_count": summary.item_count,
            "asset_count": summary.asset_count,
            "region": self.region_code,
            "enabled": self.enabled(),
        }))
    }

    pub fn custom_remove(&self, id: &str) -> Result<Value, ServiceError> {
        let mut memo = self
            .catalog
            .lock()
            .map_err(|_| ServiceError::internal("图鉴缓存锁失败"))?;
        let root = self.custom_root();
        custom_resources::remove_pack(&root, id).map_err(ServiceError::conflict)?;
        let any = !custom_resources::list_summaries(&root, &self.region_code)
            .unwrap_or_default()
            .is_empty();
        *memo = None;
        if let Ok(mut slot) = self.custom_error.lock() {
            *slot = None;
        }
        Ok(json!({
            "removed": id,
            "any_installed": any,
            "enabled": self.enabled(),
        }))
    }

    pub fn set_mode(
        &self,
        enabled: bool,
        include_firewall_verified: bool,
    ) -> Result<Value, ServiceError> {
        if enabled && !self.cache_ready() {
            return Err(ServiceError::conflict("离线素材缓存不完整，未开启。"));
        }
        if matches!(self.persistence, PersistenceMode::SplitFiles) {
            let state_path = self.root.join("offline_state.json");
            atomic_write_bytes(
                &state_path,
                json!({"enabled": enabled}).to_string().as_bytes(),
            )
            .map_err(|_| ServiceError::internal("无法保存图鉴模式"))?;
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ServiceError::internal("全图鉴数据锁失败"))?;
        inner.enabled = enabled;
        inner.assistant.set_app_enabled(enabled);
        self.save_assistant(&mut inner, true)
            .map_err(ServiceError::internal)?;

        let mut response = json!({
            "enabled": enabled,
            "mode_verified": true,
            "network_policy": "rooms-only",
            "ready": self.cache_ready(),
        });
        if include_firewall_verified {
            response["firewall_verified"] = json!(false);
        }
        Ok(response)
    }

    pub fn community_update_check(
        &self,
        force: bool,
        install_check: Result<(), String>,
    ) -> Value {
        let mut value = self.updates.check_community_cached(
            &self.community_version,
            self.include_prerelease(),
            force,
        );
        let available = value.get("updateAvailable").and_then(Value::as_bool) == Some(true);
        value["installable"] = json!(available && install_check.is_ok());
        if available {
            if let Err(reason) = install_check {
                value["installableReason"] = json!(reason);
            }
        }
        value
    }

    pub fn official_update_check(&self, is_macos: bool) -> Result<Value, ServiceError> {
        if is_macos {
            let mut result = release_updates::check_official(&self.region_id, &self.client_version)
                .map_err(ServiceError::bad_request)?;
            result["communityReleasesUrl"] = json!(COMMUNITY_RELEASES_URL);
            result["supportsLocalAdaptation"] = json!(false);
            let reason = "已安装的 Mac 应用没有源码、Node、Rust、Xcode 和 7-Zip，不能在应用内安全重打包。";
            result["localAdaptationReason"] = json!(reason);
            result["local_adaptation_supported"] = json!(false);
            result["local_adaptation_reason"] = json!(reason);
            result["message"] = json!(if result["newer"].as_bool() == Some(true) {
                format!(
                    "检测到官方 {}，可下载官方 Windows 原包；Mac 全图鉴包需由本项目构建后从 Releases 更新。",
                    result["remote"].as_str().unwrap_or("")
                )
            } else {
                result["message"]
                    .as_str()
                    .unwrap_or("官方版本检查完成")
                    .to_string()
            });
            Ok(result)
        } else {
            Ok(
                match release_updates::check_official(&self.region_id, &self.client_version) {
                    Ok(value) => value,
                    Err(error) => json!({
                        "currentVersion": self.client_version,
                        "latestVersion": "",
                        "updateAvailable": false,
                        "checked_at": now_f64(),
                        "error": error,
                        "message": format!("检查失败：{error}"),
                        "communityReleasesUrl": COMMUNITY_RELEASES_URL,
                    }),
                },
            )
        }
    }

    pub fn official_download(
        &self,
        download_dir: &Path,
        is_macos: bool,
    ) -> Result<Value, ServiceError> {
        release_updates::download_official_installer(
            &self.region_id,
            &self.client_version,
            download_dir,
            is_macos,
        )
        .map_err(ServiceError::bad_request)
    }

    pub fn update_status(&self, install_check: Result<(), String>) -> Value {
        let mut status = self.updates.policy_status(
            &self.community_version,
            self.include_prerelease(),
            install_check.is_ok(),
        );
        if status.get("available").and_then(Value::as_bool) == Some(true) {
            if let Err(reason) = install_check {
                status["installableReason"] = json!(reason);
            }
        }
        if let Some(installation) = release_installer::read_progress(&self.root) {
            status["installation"] = installation;
        }
        status
    }

    pub fn update_check(&self, force: bool, install_check: Result<(), String>) -> Value {
        let mut status = self.updates.policy_check(
            &self.community_version,
            self.include_prerelease(),
            install_check.is_ok(),
            force,
        );
        if status.get("available").and_then(Value::as_bool) == Some(true) {
            if let Err(reason) = install_check {
                status["installableReason"] = json!(reason);
            }
        }
        if let Some(installation) = release_installer::read_progress(&self.root) {
            status["installation"] = installation;
        }
        status
    }

    pub fn update_healthy(&self) -> Value {
        release_installer::ack_healthy(&self.root, &self.community_version);
        json!({ "healthy": true })
    }

    pub fn begin_install(self: &Arc<Self>) -> Result<InstallGuard, ServiceError> {
        if self
            .installing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ServiceError::conflict("更新正在进行中，请稍候…"));
        }
        Ok(InstallGuard {
            service: Arc::clone(self),
            armed: true,
        })
    }

    pub fn diagnostic(&self, body: &Value) -> Value {
        append_diagnostic(&self.root, body);
        json!({ "ok": true })
    }

    pub fn assistant(
        &self,
        action: &str,
        body: &Value,
        physical_quiet: bool,
    ) -> Result<Value, ServiceError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ServiceError::internal("全图鉴数据锁失败"))?;
        let offline = inner.enabled;
        let result = inner
            .assistant
            .handle(action, body, now_f64(), physical_quiet, offline, false)
            .map_err(ServiceError::bad_request)?;
        let force_save = match self.persistence {
            PersistenceMode::CombinedFile(_) => {
                action != "observe" || body.get("flush") == Some(&Value::Bool(true))
            }
            PersistenceMode::SplitFiles => action != "observe" || action == "tick",
        };
        self.save_assistant(&mut inner, force_save)
            .map_err(ServiceError::internal)?;
        Ok(result)
    }

    /// Dispatch routes that behave uniformly across platforms.
    pub fn handle_shared_action(
        self: &Arc<Self>,
        action: &str,
        body: &Value,
        quiet: bool,
        custom_asset_base: Option<&str>,
    ) -> Option<Result<Value, ServiceError>> {
        match action {
            "community/version" => Some(Ok(self.community_version())),
            "status" => Some(Ok(self.status_snapshot(None))),
            "game/status" => Some(Ok(crate::game_watch::status())),
            "sync/status" => Some(Ok(self.sync_status())),
            "sync" => {
                let force = body.get("force") == Some(&Value::Bool(true));
                Some(Ok(self.start_sync(force)))
            }
            "offline-data" => Some(self.offline_bundle(custom_asset_base)),
            "custom/status" => Some(Ok(self.custom_status())),
            "custom/import" => {
                let Some(path) = body.get("path").and_then(Value::as_str) else {
                    return Some(Err(ServiceError::bad_request("请选择有效的 ZIP 资源包")));
                };
                Some(self.custom_import(Path::new(path)))
            }
            "custom/remove" => {
                let Some(id) = body.get("id").and_then(Value::as_str) else {
                    return Some(Err(ServiceError::bad_request("Mod ID 无效")));
                };
                Some(self.custom_remove(id))
            }
            "update/healthy" => Some(Ok(self.update_healthy())),
            "diagnostic" => Some(Ok(self.diagnostic(body))),
            name if name.starts_with("assistant/") => {
                let sub = name.trim_start_matches("assistant/");
                Some(self.assistant(sub, body, quiet))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "wuwa-service-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn seed_valid_cache(root: &Path) {
        let assets = root.join("cache/assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("a.png"), b"\x89PNG\r\n\x1a\nok").unwrap();
        fs::write(
            root.join("cache/offline_catalog.json"),
            json!({
                "character_skin": {"10": {"url": "assets/a.png"}},
                "character_skin_atlas": {"11": {"height_light_id": 10}},
                "accessory": {},
                "accessory_atlas": {},
                "emote": {},
                "emote_atlas": {}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            root.join("cache/atlas.json"),
            json!({"data": [{"asset_id": 11, "asset_type": "character_skin_atlas", "asset_name": "Hero"}]}).to_string(),
        )
        .unwrap();
        fs::write(
            root.join("cache/asset_manifest.json"),
            json!({
                "records": [{"file": "a.png", "source": "https://official.example/character/Hero.png", "size": 10}],
                "errors": []
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn mode_requires_complete_cache_and_persists_across_restarts() {
        let root = temp_dir("mode-persist");
        let cfg = ServiceConfig {
            root: root.clone(),
            persistence: PersistenceMode::CombinedFile("fullatlas.json".into()),
            region_id: "cn".into(),
            region_code: "CN".into(),
            community_version: "1.3.2-beta.5".into(),
            client_version: "1.0.4".into(),
            expected_asset_count: None,
            on_sync_complete: None,
        };
        let svc = Arc::new(FullAtlasService::new(cfg.clone()).unwrap());
        assert!(!svc.cache_ready());
        assert_eq!(svc.set_mode(true, false).unwrap_err().kind, ErrorKind::Conflict);

        seed_valid_cache(&root);
        assert!(svc.cache_ready());
        let mode_res = svc.set_mode(true, false).unwrap();
        assert_eq!(mode_res["enabled"], true);
        let bundle = svc.offline_bundle(Some("http://127.0.0.1:9999/tok/")).unwrap();
        assert_eq!(bundle["customAssetBase"], "http://127.0.0.1:9999/tok/");
        assert_eq!(bundle["atlas_count"], 1);

        let reloaded = FullAtlasService::new(cfg).unwrap();
        assert!(reloaded.enabled());
        assert_eq!(reloaded.community_version()["current"], "1.3.2-beta.5");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_files_mode_preserves_windows_state_layout() {
        let root = temp_dir("split-mode");
        seed_valid_cache(&root);
        let cfg = ServiceConfig {
            root: root.clone(),
            persistence: PersistenceMode::SplitFiles,
            region_id: "global".into(),
            region_code: "GLOBAL".into(),
            community_version: "1.3.2-beta.5".into(),
            client_version: "1.0.4".into(),
            expected_asset_count: Some(1),
            on_sync_complete: None,
        };
        let svc = FullAtlasService::new(cfg.clone()).unwrap();
        svc.assistant("configure", &json!({"settings":{"enabled":true}}), false)
            .unwrap();
        assert!(!svc.enabled());
        svc.set_mode(true, true).unwrap();
        assert!(root.join("offline_state.json").is_file());
        assert!(root.join("assistant_state.json").is_file());

        let reloaded = FullAtlasService::new(cfg).unwrap();
        assert!(reloaded.enabled());
        assert_eq!(
            reloaded.assistant("state", &json!({}), false).unwrap()["settings"]["enabled"],
            true
        );
        let _ = fs::remove_dir_all(root);
    }
}
