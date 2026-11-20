//! Thin macOS Tauri IPC adapter delegating business state, offline catalog/PNG cache,
//! Mod overlays, and update checks to the shared `wuwatappo_assistant_core::service::FullAtlasService`.

use reqwest::Url;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use wuwatappo_assistant_core::service::{
    FullAtlasService, PersistenceMode, ServiceConfig, COMMUNITY_RELEASES_URL,
};

pub struct FullAtlasState {
    service: Arc<FullAtlasService>,
}

fn local_window(label: &str, url: &Url) -> bool {
    matches!(label, "main" | "panel")
        && url.port().is_none()
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost")))
}

impl FullAtlasState {
    pub fn new(dir: PathBuf, region: crate::profile::Region) -> Result<Self, String> {
        let is_cn = region == crate::profile::Region::Cn;
        let service = FullAtlasService::new(ServiceConfig {
            root: dir,
            persistence: PersistenceMode::CombinedFile("fullatlas.json".into()),
            region_id: if is_cn { "cn".into() } else { "global".into() },
            region_code: if is_cn { "CN".into() } else { "GLOBAL".into() },
            community_version: env!("CARGO_PKG_VERSION").into(),
            client_version: env!("WUWA_OFFICIAL_VERSION").into(),
            expected_asset_count: None,
            on_sync_complete: None,
        })?;
        Ok(Self {
            service: Arc::new(service),
        })
    }

    pub fn ensure_initial_sync(&self) {
        self.service.ensure_initial_sync();
    }

    #[cfg(test)]
    pub fn enabled(&self) -> bool {
        self.service.enabled()
    }

    #[cfg(test)]
    pub fn assistant(
        &self,
        action: &str,
        body: &Value,
        physical_quiet: bool,
    ) -> Result<Value, String> {
        self.service
            .assistant(action, body, physical_quiet)
            .map_err(|e| e.message)
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
        let asset_base = app
            .try_state::<crate::mod_assets::ModAssetHost>()
            .map(|h| h.base().to_owned())
            .filter(|s| !s.is_empty());

        if matches!(
            action,
            "community/version"
                | "status"
                | "sync/status"
                | "sync"
                | "offline-data"
                | "game/status"
                | "custom/status"
                | "custom/import"
                | "custom/remove"
                | "update/healthy"
                | "diagnostic"
        ) || action.starts_with("assistant/")
        {
            let quiet = app.state::<crate::input::InputMonitor>().status().running
                && crate::input::physical_input_quiet();
            if let Some(res) = self.service.handle_shared_action(
                action,
                body,
                quiet,
                asset_base.as_deref(),
            ) {
                return res.map_err(|e| e.message);
            }
        }

        match action {
            "official/releases" => crate::system::open_community_releases(app)
                .map(|()| json!({"ok": true, "url": COMMUNITY_RELEASES_URL})),
            "official/pet-site" => {
                let region = crate::profile::Region::from_app(app);
                crate::system::open_pet_site(app, region)
                    .map(|()| json!({"ok": true, "url": region.pet_site()}))
            }
            "community/update-check" => {
                let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);
                let bundle_check = crate::mac_updater::current_installable_bundle().map(|_| ());
                Ok(self.service.community_update_check(force, bundle_check))
            }
            "official/update-check" | "official/check" => self
                .service
                .official_update_check(true)
                .map_err(|e| e.message),
            "official/download" => {
                let directory = app.path().download_dir().map_err(|e| e.to_string())?;
                self.service
                    .official_download(&directory, true)
                    .map_err(|e| e.message)
            }
            "asset-base" => Ok(json!({
                "base": asset_base.unwrap_or_default(),
            })),
            "mode" => {
                let enabled = body
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("模式必须是布尔值")?;
                let response = self
                    .service
                    .set_mode(enabled, false)
                    .map_err(|e| e.message)?;
                app.emit("fullatlas-mode-changed", enabled)
                    .map_err(|e| e.to_string())?;
                Ok(response)
            }
            "update/status" => {
                let bundle_check = crate::mac_updater::current_installable_bundle().map(|_| ());
                Ok(self.service.update_status(bundle_check))
            }
            "update/check" => {
                let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);
                let bundle_check = crate::mac_updater::current_installable_bundle().map(|_| ());
                Ok(self.service.update_check(force, bundle_check))
            }
            "update/install" => {
                crate::mac_updater::current_installable_bundle()?;
                let guard = self.service.begin_install().map_err(|e| e.message)?;
                wuwatappo_assistant_core::release_installer::write_current_status(
                    self.service.root(),
                    "download",
                    2.0,
                    "正在获取更新信息并后台下载安装包…",
                );
                let service = Arc::clone(&self.service);
                let region = crate::profile::Region::from_app(app);
                let handle = app.clone();
                std::thread::spawn(move || {
                    let _guard = guard;
                    let candidate =
                        match wuwatappo_assistant_core::release_updates::community_installer_for_kind(
                            service.current_community_version(),
                            service.include_prerelease(),
                            region.id(),
                            "macos",
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
                    if let Err(error) = crate::mac_updater::prepare_update(
                        service.root(),
                        region.identifier(),
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
                    std::thread::sleep(Duration::from_millis(400));
                    let _ = handle.emit("tray-quit", Value::Null);
                    std::thread::sleep(Duration::from_millis(1200));
                    if let Some(windows) = handle.try_state::<crate::windows::WindowState>() {
                        windows.mark_stopping();
                    }
                    handle.exit(0);
                });
                Ok(json!({
                    "started": true,
                    "message": "已开始后台下载并校验安装包…",
                }))
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
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn state() -> FullAtlasState {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        FullAtlasState::new(
            std::env::temp_dir().join(format!(
                "fullatlas-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            )),
            crate::profile::Region::Cn,
        )
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
        let loaded =
            FullAtlasState::new(s.service.root().to_path_buf(), crate::profile::Region::Cn)
                .unwrap();
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
