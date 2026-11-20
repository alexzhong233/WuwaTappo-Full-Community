//! Unified frontend injector for both macOS and Windows.
//!
//! Replaces the historical Python pipeline (`inject_core.py`, `fullatlas_core.py`,
//! `assistant_core.py`, `fullatlas-integration.py`, `build_integration.py`, and
//! `integrate_assistant.py`) while rendering the desktop overlay modules directly from
//! `shared/frontend/` + `shared/contracts/assistant-settings.json`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const SETTINGS_CONTRACT: &str = include_str!("../../contracts/assistant-settings.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Mac,
    Windows,
}

impl Platform {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mac" | "macos" => Ok(Self::Mac),
            "win" | "windows" => Ok(Self::Windows),
            other => Err(format!("未知注入目标平台：{other}（期望 mac 或 win）")),
        }
    }

    pub fn brand(self) -> &'static str {
        match self {
            Self::Mac => "WUWA TAPPO FULLATLAS / MAC",
            Self::Windows => "WUWA TAPPO / 后台助手",
        }
    }

    pub fn platform_label(self) -> &'static str {
        match self {
            Self::Mac => "macOS",
            Self::Windows => "Windows",
        }
    }

    pub fn startup_error_prefix(self) -> &'static str {
        match self {
            Self::Mac => "Mac 全图鉴启动失败：",
            Self::Windows => {
                "本地素材服务没有启动，请用桌面快捷方式重新打开；仍不行请查看安装目录下的 host.stderr.log。"
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PatchLogEntry {
    pub anchor: String,
    pub matches: usize,
}

#[derive(Default)]
pub struct PatchRecorder {
    pub entries: Vec<PatchLogEntry>,
}

impl PatchRecorder {
    pub fn replace(&mut self, text: &str, old: &str, new: &str) -> Result<String, String> {
        self.replace_n(text, old, new, 1)
    }

    pub fn replace_n(
        &mut self,
        text: &str,
        old: &str,
        new: &str,
        expected: usize,
    ) -> Result<String, String> {
        if old.is_empty() {
            return Err("替换锚点不能为空".into());
        }
        let actual = text.match_indices(old).count();
        if actual != expected {
            let preview: String = old.chars().take(100).collect();
            return Err(format!(
                "注入锚点匹配数量异常：'{preview}'（实际 {actual}，期望 {expected}）"
            ));
        }
        let preview: String = old.chars().take(100).collect();
        self.entries.push(PatchLogEntry {
            anchor: preview,
            matches: actual,
        });
        Ok(text.replace(old, new))
    }
}

/// Convert `inspect_js.cjs` Unicode codepoint (char) indices into UTF-8 byte offsets.
pub fn char_to_byte_map(text: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(text.chars().count() + 1);
    for (byte_idx, _) in text.char_indices() {
        map.push(byte_idx);
    }
    map.push(text.len());
    map
}

fn map_offset(map: &[usize], char_offset: usize, label: &str) -> Result<usize, String> {
    map.get(char_offset)
        .copied()
        .ok_or_else(|| format!("AST 偏移量越界 ({label}): {char_offset} >= {}", map.len()))
}

#[derive(Clone, Debug, Deserialize)]
pub struct FunctionMeta {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub body: usize,
    #[serde(rename = "bodyEnd")]
    pub body_end: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ComponentMeta {
    pub name: String,
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub variable: Option<String>,
    pub body: usize,
    #[serde(rename = "bodyEnd")]
    pub body_end: usize,
    #[serde(rename = "returnStart", default)]
    pub return_start: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContainerMeta {
    pub start: usize,
    #[allow(dead_code)]
    pub end: usize,
    pub array: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RoomConditionMeta {
    pub start: usize,
    #[allow(dead_code)]
    pub end: usize,
    pub test: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SnapshotMeta {
    pub start: usize,
    pub end: usize,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub imports: BTreeMap<String, String>,
    #[serde(default)]
    pub functions: Vec<FunctionMeta>,
    #[serde(default)]
    pub components: Vec<ComponentMeta>,
    #[serde(default)]
    pub containers: Vec<ContainerMeta>,
    #[serde(rename = "roomConditions", default)]
    pub room_conditions: Vec<RoomConditionMeta>,
    #[serde(default)]
    pub cases: BTreeMap<String, String>,
    #[serde(default)]
    pub snapshots: Vec<SnapshotMeta>,
}

impl FileMeta {
    fn convert_offsets_to_bytes(&mut self, map: &[usize]) -> Result<(), String> {
        for f in &mut self.functions {
            f.start = map_offset(map, f.start, &f.name)?;
            f.end = map_offset(map, f.end, &f.name)?;
            f.body = map_offset(map, f.body, &f.name)?;
            f.body_end = map_offset(map, f.body_end, &f.name)?;
        }
        for c in &mut self.components {
            c.start = map_offset(map, c.start, &c.name)?;
            c.end = map_offset(map, c.end, &c.name)?;
            c.body = map_offset(map, c.body, &c.name)?;
            c.body_end = map_offset(map, c.body_end, &c.name)?;
            if let Some(rs) = c.return_start {
                c.return_start = Some(map_offset(map, rs, &c.name)?);
            }
        }
        for container in &mut self.containers {
            container.start = map_offset(map, container.start, "container")?;
            container.end = map_offset(map, container.end, "container")?;
            container.array = map_offset(map, container.array, "container.array")?;
        }
        for cond in &mut self.room_conditions {
            cond.start = map_offset(map, cond.start, "roomCondition")?;
            cond.end = map_offset(map, cond.end, "roomCondition")?;
        }
        for snap in &mut self.snapshots {
            snap.start = map_offset(map, snap.start, "snapshot")?;
            snap.end = map_offset(map, snap.end, "snapshot")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct JsStructure {
    pub shared: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub exports: BTreeMap<String, String>,
    pub files: BTreeMap<String, FileMeta>,
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

impl Edit {
    pub fn new(start: usize, end: usize, value: impl Into<String>) -> Self {
        Self {
            start,
            end,
            value: value.into(),
        }
    }
}

/// Apply span edits in reverse offset order so earlier indices stay valid.
pub fn apply_edits(text: &str, edits: Vec<Edit>) -> Result<String, String> {
    let mut indexed: Vec<(usize, Edit)> = edits.into_iter().enumerate().collect();
    // Preserve reverse tie-break behavior on identical `(start, end)` if ever present.
    indexed.sort_by(|(idx_a, a), (idx_b, b)| {
        b.start
            .cmp(&a.start)
            .then_with(|| b.end.cmp(&a.end))
            .then_with(|| idx_b.cmp(idx_a))
    });
    let mut out = text.to_owned();
    let mut last = out.len();
    for (_, edit) in indexed {
        if !(edit.start <= edit.end && edit.end <= last) {
            return Err(format!(
                "编辑区间无效或重叠：({}, {}), last={last}",
                edit.start, edit.end
            ));
        }
        if !out.is_char_boundary(edit.start) || !out.is_char_boundary(edit.end) {
            return Err(format!(
                "编辑区间不在 UTF-8 字符边界上：({}, {})",
                edit.start, edit.end
            ));
        }
        out.replace_range(edit.start..edit.end, &edit.value);
        last = edit.start;
    }
    Ok(out)
}

pub fn function_for<'a>(
    functions: &'a [FunctionMeta],
    text: &str,
    needle: &str,
) -> Result<&'a FunctionMeta, String> {
    functions
        .iter()
        .filter(|f| {
            text.get(f.start..f.end)
                .is_some_and(|slice| slice.contains(needle))
        })
        .min_by_key(|f| f.end.saturating_sub(f.start))
        .ok_or_else(|| format!("未找到包含特征 '{needle}' 的函数"))
}

pub fn named<'a>(functions: &'a [FunctionMeta], name: &str) -> Result<&'a FunctionMeta, String> {
    let mut matches = functions.iter().filter(|f| f.name == name);
    let first = matches
        .next()
        .ok_or_else(|| format!("未找到命名函数：{name}"))?;
    if matches.next().is_some() {
        return Err(format!("命名函数不唯一：{name}"));
    }
    Ok(first)
}

fn require_import<'a>(imports: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    imports
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("js_structure 缺少导入绑定：{key}"))
}

fn with_alias(imports: &mut BTreeMap<String, String>, canonical: &str) {
    if imports.contains_key(canonical) {
        return;
    }
    let alt = format!("{canonical}$1");
    if let Some(value) = imports.get(&alt).cloned() {
        imports.insert(canonical.to_owned(), value);
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Render a module directly from `shared/frontend/<name>` for the target platform,
/// substituting `__WUWA_BRAND__`, `__WUWA_PLATFORM__`, and `__WUWA_ASSISTANT_DEFAULTS__`.
pub fn render_frontend_module(
    repo_root: &Path,
    name: &str,
    platform: Platform,
) -> Result<String, String> {
    let source_path = repo_root.join("shared/frontend").join(name);
    let raw = fs::read_to_string(&source_path)
        .map_err(|e| format!("读取共享前端模块 {} 失败：{e}", source_path.display()))?;
    render_frontend_source(name, &raw, platform)
}

pub fn render_frontend_source(
    name: &str,
    raw: &str,
    platform: Platform,
) -> Result<String, String> {
    let mut rendered = raw
        .replace("__WUWA_BRAND__", platform.brand())
        .replace("__WUWA_PLATFORM__", platform.platform_label());
    if name == "desktop_assistant.js" {
        let contract: Value = serde_json::from_str(SETTINGS_CONTRACT)
            .map_err(|e| format!("解析 assistant-settings.json 失败：{e}"))?;
        let defaults = contract
            .get("defaults")
            .ok_or("assistant-settings.json 缺少 defaults")?;
        let defaults_json = serde_json::to_string(defaults).map_err(|e| e.to_string())?;
        rendered = rendered.replace("__WUWA_ASSISTANT_DEFAULTS__", &defaults_json);
    }
    for placeholder in [
        "__WUWA_BRAND__",
        "__WUWA_PLATFORM__",
        "__WUWA_ASSISTANT_DEFAULTS__",
    ] {
        if rendered.contains(placeholder) {
            return Err(format!("{name} 渲染后仍保留占位符 {placeholder}"));
        }
    }
    Ok(rendered)
}

const MOD_NAME_FALLBACK_OLD: &str = r#"function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  return s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : ""
}"#;

const MOD_NAME_FALLBACK_NEW: &str = r#"function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  const resolved = s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : "";
  // 本机 Mod 的名字不在官方远端字典里：查不到就直接用条目名，否则图鉴/背包只剩空白或占位符。
  const mods = typeof window !== "undefined" ? window.__WUWA_MOD_NAMES__ : null;
  return resolved || (mods && mods.has(n) ? n : "")
}"#;

pub fn patch_mod_display_name(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    rec.replace(text, MOD_NAME_FALLBACK_OLD, MOD_NAME_FALLBACK_NEW)
}

pub fn patch_mod_asset_url(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let re = Regex::new(r"function buildAssetUrl\(([^)]*)\)\s*\{").unwrap();
    let caps = re
        .captures(text)
        .ok_or("official buildAssetUrl contract changed")?;
    let whole = caps.get(0).unwrap().as_str().to_owned();
    let params: Vec<&str> = caps[1].split(',').map(str::trim).collect();
    if params.len() != 2 || params.iter().any(|p| p.is_empty()) {
        return Err(format!("buildAssetUrl 参数列表异常：{:?}", params));
    }
    let injection = format!(
        "{whole}\n  const modAssetPath = (typeof window !== \"undefined\" && window.__WUWA_MOD_ASSET_PATH__) ? window.__WUWA_MOD_ASSET_PATH__({}) : \"\";\n  if (modAssetPath) return withAssetUrlCacheBust(modAssetPath);",
        params[1]
    );
    rec.replace(text, &whole, &injection)
}

pub fn patch_online_services(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let old = "loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();";
    let new = r#"let localOnlineServicesStarted = false;
function localStartOnlineServices() {
 if (localOnlineServicesStarted) return;
 localOnlineServicesStarted = true;
 loaderModule.init(kurodataConfig); window.kurodata = loaderModule;
 initTrackCommonProps(); initFrontendAlertReporting();
}"#;
    rec.replace(text, old, new)
}

pub fn patch_display_version(
    text: &str,
    community_version: &str,
    official_version: &str,
) -> Result<String, String> {
    let community_json = serde_json::to_string(community_version.trim()).unwrap();
    let official_json = serde_json::to_string(official_version.trim()).unwrap();

    let telemetry_re = Regex::new(r"cachedAppVersion\s*=\s*await getVersion\(\)").unwrap();
    let telemetry_count = telemetry_re.find_iter(text).count();
    if telemetry_count != 1 {
        return Err(format!(
            "cachedAppVersion telemetry anchor count={telemetry_count} (expected 1)"
        ));
    }
    let text = telemetry_re
        .replace(text, format!("cachedAppVersion = {official_json}").as_str())
        .into_owned();

    let version_re =
        Regex::new(r#"async function getVersion\(\)\s*\{\s*return invoke(?:\$1)?\("plugin:app\|version"\);?\s*\}"#)
            .unwrap();
    let version_count = version_re.find_iter(&text).count();
    if version_count != 1 {
        return Err(format!(
            "official getVersion anchor count={version_count} (expected 1)"
        ));
    }
    let replacement = format!(
        "async function getVersion() {{\n  let c = {community_json};\n  try {{\n    const r = await window.__WUWA_LOCAL_ATLAS__?.request?.(\"/community/version\", {{}});\n    if (r && typeof r.current === \"string\" && r.current.trim()) c = r.current.trim();\n  }} catch {{}}\n  const b = {official_json};\n  return b ? c + \" · 基底 \" + b : c;\n}}"
    );
    Ok(version_re.replace(&text, replacement.as_str()).into_owned())
}

pub fn patch_invoke_guard(
    rec: &mut PatchRecorder,
    text: &str,
    strict: bool,
) -> Result<String, String> {
    let mut found = Vec::new();
    for name in ["invoke$1", "invoke"] {
        let anchor = format!("async function {name}(e, t = {{}}, n) {{");
        if text.contains(&anchor) {
            found.push(anchor);
        }
    }
    if strict && found.is_empty() {
        return Err("tauri invoke wrapper contract missing".into());
    }
    let mut out = text.to_owned();
    for anchor in found {
        let replacement = format!(
            "{anchor}\n window.__WUWA_LOCAL_ATLAS__?.checkInvoke(e, t);"
        );
        out = rec.replace(&out, &anchor, &replacement)?;
    }
    Ok(out)
}

pub fn patch_taskbar_default(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let taskbar_default = "showTaskbarIcon: !0";
    let clamp_anchor = "function clampPetScale(e) {";
    if text.matches(taskbar_default).count() != 1 || text.matches(clamp_anchor).count() != 1 {
        eprintln!("warn: 官方前端里找不到任务栏图标默认值，跳过「默认隐藏」这一项");
        return Ok(text.to_owned());
    }
    let out = rec.replace(text, taskbar_default, "showTaskbarIcon: !1")?;
    let migration = r#"// 社区版：默认不占任务栏/程序坞图标；历史设置只迁移一次，避免每次启动覆盖用户选择。
(function () {
  try {
    const key = "wuwatappo-settings", marker = "wuwatappo-settings-community-taskbar-v1";
    if (window.localStorage.getItem(marker)) return;
    const saved = JSON.parse(window.localStorage.getItem(key) || "null");
    if (saved && typeof saved === "object" && "showTaskbarIcon" in saved) {
      saved.showTaskbarIcon = false;
      window.localStorage.setItem(key, JSON.stringify(saved));
    }
    window.localStorage.setItem(marker, "1");
  } catch (error) {
    console.warn("[community] taskbar default skipped", error);
  }
})();

function clampPetScale(e) {"#;
    rec.replace(&out, clamp_anchor, migration)
}

pub const ACTIVITY_LOADING_GUARD: &str = r#"
 // 活动配置是后台刷新（挂机开箱、切页、重连都会触发），不该用全屏遮罩。
 if (t && typeof t.url === "string" && t.url.indexOf("api/common/activity-config") !== -1) t.loading = !1;"#;

pub fn patch_atlas_guards(
    rec: &mut PatchRecorder,
    text: &str,
    extra_interceptor: &str,
) -> Result<String, String> {
    let text = rec.replace(
        text,
        "async function ensureAppearanceAssetCatalog(e) {",
        "async function ensureAppearanceAssetCatalog(e) {\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return true;",
    )?;
    let text = rec.replace(
        &text,
        "function applyCatalog(e, t, n) {",
        "function applyCatalog(e, t, n) {\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && t !== window.__WUWA_LOCAL_ATLAS__.assetsHost) return;",
    )?;
    let text = rec.replace(
        &text,
        "function atlas() {",
        "function atlas() {\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return Promise.resolve({code: 0, data: window.__WUWA_LOCAL_ATLAS__.offlineData.value.atlas});",
    )?;
    let interceptor = format!(
        "this.instance.interceptors.request.use(t => {{\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && window.__WUWA_LOCAL_ATLAS__.restricted(t.url,t.method)) throw window.__WUWA_LOCAL_ATLAS__.offlineError();{ACTIVITY_LOADING_GUARD}{extra_interceptor}"
    );
    rec.replace(
        &text,
        "this.instance.interceptors.request.use(t => {",
        &interceptor,
    )
}

fn prepare_mac_shared(rec: &mut PatchRecorder, shared: &str) -> Result<String, String> {
    let mut out = shared.to_owned();
    for name in [
        "resolveAppearanceAssetUrl",
        "resolveAppearanceAssetDisplayUrl",
        "resolveAppearanceAtlasUrl",
    ] {
        let sig_re = Regex::new(&format!(r"function {name}\([^\n]*\) \{{")).unwrap();
        let m = sig_re
            .find(&out)
            .ok_or_else(|| format!("Mac shared patch missing function {name}"))?;
        let start = m.start();
        let after_sig = &out[m.end()..];
        let mut end = (start + 3500).min(out.len());
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        let next_fn = ["\nfunction ", "\nasync function "]
            .into_iter()
            .filter_map(|needle| after_sig.find(needle).map(|off| m.end() + off))
            .min();
        if let Some(boundary) = next_fn {
            if boundary <= end {
                end = boundary;
            }
        }
        let original = out[start..end].to_owned();
        if original[1..].contains("\nfunction ") {
            return Err(format!("{name} regex matched past function boundary"));
        }
        let updated = original.replace("/^https?:\\/\\//i", "/^(?:https?|asset):\\/\\//i");
        if updated == original {
            return Err(format!("{name} did not contain https scheme regex"));
        }
        out = rec.replace(&out, &original, &updated)?;
    }
    Ok(out)
}

fn transport_options(platform: Platform, config: &Value) -> Result<String, String> {
    match platform {
        Platform::Mac => Ok(concat!(
            "origin: window.__WUWATAPPO_PROFILE__.assetHost, ",
            "request: (route,payload) => window.__TAURI_INTERNALS__.invoke('fullatlas_request',{action:route.replace(/^\\//,''),payload:payload||{}}), ",
            "token: null, customAssetUrl: (reference, file) => window.__TAURI_INTERNALS__.convertFileSrc(file)"
        )
        .to_owned()),
        Platform::Windows => {
            let port = config
                .get("port")
                .and_then(Value::as_u64)
                .ok_or("local_config.json 缺少有效的 port")?;
            let origin = serde_json::to_string(&format!("http://127.0.0.1:{port}")).unwrap();
            Ok(format!(
                "origin: {origin}, token: null, customAssetUrl: reference => {origin} + \"/\" + reference.split(\"/\").map(encodeURIComponent).join(\"/\")"
            ))
        }
    }
}

fn controller_init(platform: Platform, config: &Value, controller: &str) -> Result<String, String> {
    let transport = transport_options(platform, config)?;
    let prefix_json = serde_json::to_string(platform.startup_error_prefix()).unwrap();
    Ok(format!(
        "{controller}\nconst localAtlasRoute = ['/', '/panel'].includes((window.location.hash || '#/').slice(1).split('?')[0]);\n\
const localAtlasController = localAtlasRoute ? createLocalAtlasController({{\n\
ref, window, storage: localStorage, {transport}, normalizeAtlasList, resolvePetAssetIdFromAppearanceId, getAppearanceAssetById,\n\
snapshotCatalog: () => ({{assets: appearanceAssetCatalog.value, atlases: appearanceAtlasCatalog.value, host: loadedAssetsHost, ts: loadedAssetsJsonTs}}),\n\
getAccountCumulative: () => globalStore.userInfo ? isFrozenClickCount(globalStore.serverClickTotal) ? globalStore.serverClickTotal : clampClickCountSum(globalStore.serverClickTotal+globalStore.localClickCount) : parseSafeClickCount(globalStore.localClickCount),\n\
getAccountPending: () => parseSafeClickCount(globalStore.localClickCount),\n\
catalogConsumer: localAtlasRoute,\n\
applyOfflineCatalog: (payload, host) => applyCatalog(buildCatalogMaps(payload), host, Date.now()),\n\
restoreCatalog: saved => applyCatalog({{assets: saved.assets, atlases: saved.atlases}}, saved.host, saved.ts),\n\
onError: error => console.error('[local-gallery]', error)\n\
}}) : null;\n\
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;\n\
if (localAtlasController) {{\n\
localAtlasController.ready.then(() => {{ localStartOnlineServices(); return bootstrap(); }}).catch(error => {{\n\
 console.error('[local-gallery] startup failed', error);\n\
 document.getElementById('app').textContent = {prefix_json} + (error?.message || '');\n\
}});\n\
watch(localAtlasController.enabled, on => {{ if (!on) localStartOnlineServices(); }});\n\
}} else {{ localStartOnlineServices(); bootstrap(); }}\n"
    ))
}

fn find_component<'a>(meta: &'a FileMeta, name: &str) -> Result<&'a ComponentMeta, String> {
    meta.components
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("缺少组件 {name}"))
}

fn find_char_boundary_backward(text: &str, pos: usize) -> usize {
    let mut i = pos.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

struct WindowsContextMenuPatch {
    context_actions: String,
}

fn patch_windows_context_menu(
    main: &str,
    source: &str,
    main_meta: &FileMeta,
    mi: &BTreeMap<String, String>,
    open_tab: &str,
) -> Result<(String, WindowsContextMenuPatch), String> {
    let popup_re = Regex::new(
        r#"await ([A-Za-z0-9_$]+)\("popup_main_dock_context_menu", \{\s*x: ([A-Za-z0-9_$]+),\s*y: ([A-Za-z0-9_$]+)\s*\}\)"#,
    )
    .unwrap();
    let popup = popup_re
        .captures(main)
        .ok_or("native dock menu anchor changed")?;
    let popup_whole = popup[0].to_owned();
    let popup_x = popup[2].to_owned();
    let popup_y = popup[3].to_owned();

    let popup_fn = function_for(&main_meta.functions, source, "\"popup_main_dock_context_menu\"")?;
    let popup_source = &source[popup_fn.start..popup_fn.end];
    let cancel_re =
        Regex::new(r"finally\s*\{[\s\S]*?,\s*([A-Za-z0-9_$]+)\(\),\s*[A-Za-z0-9_$]+ && window\.setTimeout")
            .unwrap();
    let cancel_gesture = cancel_re
        .captures(popup_source)
        .ok_or("dock cleanup binding changed")?[1]
        .to_owned();
    let ptr_re = Regex::new(r"cancelDockPointerGesture:\s*([A-Za-z0-9_$]+)").unwrap();
    let ptr_matches: Vec<String> = ptr_re
        .captures_iter(source)
        .map(|c| c[1].to_owned())
        .collect();
    if !ptr_matches.contains(&cancel_gesture) {
        return Err("dock cleanup binding changed (cancelDockPointerGesture mismatch)".into());
    }

    let swallow_fn = function_for(&main_meta.functions, source, "\"set_main_swallow_mouse\"")?;
    let swallow_source = &source[swallow_fn.start..swallow_fn.end];
    let swallow_re = Regex::new(r"function ([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) \{").unwrap();
    let swallow_caps = swallow_re
        .captures(swallow_source)
        .ok_or("mouse interception binding changed")?;
    let swallow_whole = swallow_caps[0].to_owned();
    let swallow_arg = swallow_caps[2].to_owned();

    let mut rec = PatchRecorder::default();
    let main = rec.replace(
        main,
        &swallow_whole,
        &format!("{swallow_whole}\n if(__lg.contextMenu?.isOpen()) {swallow_arg}=false;"),
    )?;
    let native_invoke = mi
        .get("invoke$1")
        .or_else(|| mi.get("invoke"))
        .map(String::as_str)
        .ok_or("Windows context menu 缺少 invoke 导入")?;
    let context_actions = format!(
        "{{openTab:{open_tab},openConsole:async()=>{{await __lg.assistant.openPanel();}},before:async()=>{{await {cancel_gesture}();await {native_invoke}('set_main_swallow_mouse',{{enabled:false}});}},after:async()=>{{await {native_invoke}('set_main_swallow_mouse',{{enabled:false}});await {cancel_gesture}();}}}}"
    );
    let main = rec.replace(
        &main,
        &popup_whole,
        &format!(
            "try{{await __lg.contextMenu.popup({popup_x},{popup_y},{context_actions});}}\n            catch(error){{console.warn('[context-menu] native extension unavailable',error);{popup_whole};}}"
        ),
    )?;
    Ok((main, WindowsContextMenuPatch { context_actions }))
}

fn patch_windows_monitor_autostart(monitor: &str) -> String {
    let mut out = monitor.to_owned();
    let status_re = Regex::new(
        r#"return Fe\(\) \? await [A-Za-z0-9_$]+\("is_elevated_autostart_enabled"\) : await [A-Za-z0-9_$]+\(\)"#,
    )
    .unwrap();
    if let Some(m) = status_re.find(&out) {
        let rep = r#"return (await window.__WUWA_LOCAL_ATLAS__.request("/startup",{action:"status"})).enabled"#;
        out.replace_range(m.start()..m.end(), rep);
    } else {
        eprintln!("warn: 官方开机自启状态查询锚点变了，设置里的开关仍走官方实现");
    }

    let set_re =
        Regex::new(r"(async function ([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) \{)\s*try \{\s*if \(Fe\(\)\)")
            .unwrap();
    if let Some(caps) = set_re.captures(&out) {
        let header = caps.get(1).unwrap();
        let arg = &caps[3];
        let insert_at = header.end();
        let snippet = format!(
            "\n return window.__WUWA_LOCAL_ATLAS__.request(\"/startup\",{{action:{arg}?\"enable\":\"disable\"}});"
        );
        out.insert_str(insert_at, &snippet);
    } else {
        eprintln!("warn: 官方开机自启开关锚点变了，设置里的开关仍走官方实现");
    }
    out
}

fn read_normalized_js(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取 JS 失败 ({}): {e}", path.display()))?;
    if raw.contains("\r\n") {
        Ok(raw.replace("\r\n", "\n"))
    } else {
        Ok(raw)
    }
}

pub struct IntegrationResult {
    pub manifest: Value,
    pub patch_log: Vec<PatchLogEntry>,
    pub shared_name: String,
    pub main_name: String,
    pub panel_name: String,
    pub extra_patched_assets: Vec<String>,
    pub base_dir: PathBuf,
    pub frontend_dir: PathBuf,
}

/// Execute the complete official frontend integration pipeline for either macOS or Windows.
pub fn integrate_build(
    platform: Platform,
    build_root: &Path,
    repo_root: &Path,
) -> Result<IntegrationResult, String> {
    let base_text = fs::read_to_string(build_root.join("current_base.txt"))
        .map_err(|e| format!("读取 current_base.txt 失败：{e}"))?;
    let base_dir = PathBuf::from(base_text.trim().trim_start_matches('\u{feff}'));
    let structure_raw = fs::read_to_string(base_dir.join("js_structure.json"))
        .map_err(|e| format!("读取 js_structure.json 失败：{e}"))?;
    let mut structure: JsStructure = serde_json::from_str(&structure_raw)
        .map_err(|e| format!("解析 js_structure.json 失败：{e}"))?;

    let config_raw = fs::read_to_string(build_root.join("local_config.json"))
        .map_err(|e| format!("读取 local_config.json 失败：{e}"))?;
    let config: Value = serde_json::from_str(config_raw.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("解析 local_config.json 失败：{e}"))?;

    let shared_name = structure.shared.clone();
    let readable_assets = base_dir.join("readable_js/assets");
    let shared_orig = read_normalized_js(&readable_assets.join(&shared_name))?;

    let router_pos = shared_orig
        .find("const router = createRouter")
        .ok_or("shared 模块缺少 const router = createRouter")?;
    let route_region = &shared_orig[router_pos..];
    let route_re =
        Regex::new(r#"(?s)path:\s*"([^"]+)"[\s\S]*?import\("\./([^"]+)"\)"#).unwrap();
    let mut routes = BTreeMap::new();
    for caps in route_re.captures_iter(route_region).take(9) {
        routes.insert(caps[1].to_owned(), caps[2].to_owned());
    }
    let main_name = routes
        .get("/")
        .cloned()
        .ok_or("路由表缺少 '/' 入口模块")?;
    let panel_name = routes
        .get("/panel")
        .cloned()
        .ok_or("路由表缺少 '/panel' 入口模块")?;

    let main_orig = read_normalized_js(&readable_assets.join(&main_name))?;
    let panel_orig = read_normalized_js(&readable_assets.join(&panel_name))?;

    // Convert inspect_js char offsets to UTF-8 byte offsets for main and panel before slicing.
    let main_map = char_to_byte_map(&main_orig);
    let panel_map = char_to_byte_map(&panel_orig);
    if let Some(m) = structure.files.get_mut(&main_name) {
        m.convert_offsets_to_bytes(&main_map)?;
    }
    if let Some(p) = structure.files.get_mut(&panel_name) {
        p.convert_offsets_to_bytes(&panel_map)?;
    }

    let frontend_dir = build_root.join("frontend_v2");
    if frontend_dir.exists() {
        fs::remove_dir_all(&frontend_dir).map_err(|e| e.to_string())?;
    }
    copy_dir_recursive(&base_dir.join("frontend"), &frontend_dir)?;

    let mut rec = PatchRecorder::default();

    // Aligned cross-platform order: prepare_shared -> mod_display_name -> rest.
    let mut shared = match platform {
        Platform::Mac => prepare_mac_shared(&mut rec, &shared_orig)?,
        Platform::Windows => shared_orig.clone(),
    };
    shared = patch_mod_display_name(&mut rec, &shared)?;
    shared = patch_taskbar_default(&mut rec, &shared)?;
    shared = patch_mod_asset_url(&mut rec, &shared)?;
    shared = patch_online_services(&mut rec, &shared)?;

    let pkg_json: Value = serde_json::from_str(
        &fs::read_to_string(repo_root.join("package.json"))
            .map_err(|e| format!("读取 package.json 失败：{e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let community_version = config
        .get("community_version")
        .and_then(Value::as_str)
        .or_else(|| config.get("mod_version").and_then(Value::as_str))
        .or_else(|| pkg_json.get("version").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_owned();
    let official_version = config
        .get("client_version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if official_version.is_empty() {
        return Err("Official client version missing from app config".into());
    }
    shared = patch_display_version(&shared, &community_version, &official_version)?;
    shared = patch_invoke_guard(&mut rec, &shared, true)?;
    shared = patch_atlas_guards(&mut rec, &shared, "")?;

    let controller = render_frontend_module(repo_root, "local_atlas_controller.js", platform)?;
    let init = controller_init(platform, &config, &controller)?;
    shared = rec.replace(
        &shared,
        "bootstrap();\nexport {",
        &format!("{init}\nexport {{ localAtlasController as zzLocalAtlas,\n"),
    )?;

    // Main renderer edits
    let m_raw = structure
        .files
        .get(&main_name)
        .ok_or_else(|| format!("js_structure 缺少 {main_name}"))?
        .clone();
    let mut mi = m_raw.imports.clone();
    for canonical in ["listen", "invoke", "watch", "onMounted", "onUnmounted"] {
        with_alias(&mut mi, canonical);
    }
    if !mi.contains_key("invoke$1") {
        if let Some(inv) = mi.get("invoke").cloned() {
            mi.insert("invoke$1".into(), inv);
        }
    }

    let main_component = find_component(&m_raw, "index")?;
    let component_functions: Vec<FunctionMeta> = m_raw
        .functions
        .iter()
        .filter(|fn_meta| {
            fn_meta.start > main_component.body && fn_meta.start < main_component.body_end
        })
        .cloned()
        .collect();
    if m_raw.snapshots.len() != 1 {
        return Err(format!(
            "主渲染器 snapshot 数量异常：{}（期望 1）",
            m_raw.snapshots.len()
        ));
    }
    let snapshot = &m_raw.snapshots[0];
    let f = &snapshot.fields;
    let req_field = |k: &str| -> Result<&str, String> {
        f.get(k)
            .map(String::as_str)
            .ok_or_else(|| format!("snapshot 缺少字段：{k}"))
    };

    let store = require_import(&mi, "globalStore")?;
    let watch_sym = require_import(&mi, "watch")?;
    let computed_sym = require_import(&mi, "computed")?;
    let show_toast_sym = require_import(&mi, "showToast")?;
    let appear_catalog_sym = require_import(&mi, "appearanceAssetCatalog")?;
    let getchar_sym = require_import(&mi, "getUserCharacterAssetName")?;
    let getaccess_sym = require_import(&mi, "getUserAccessoryAssetName")?;
    let getemotes_sym = require_import(&mi, "getUserEmoteAssets")?;
    let role = req_field("selectedRoleId")?;
    let accessory = req_field("selectedPartId")?;
    let host = req_field("assetsCdnHost")?;
    let bag = req_field("bagList")?;
    let atlas = req_field("atlasList")?;
    let atlas_loading = req_field("atlasListLoading")?;
    let preset_list_field = req_field("presetList")?;
    let preset_loading_field = req_field("presetListLoading")?;
    let user_info_field = req_field("userInfo")?;
    let selected_emotes_field = req_field("selectedEmoteAssetIds")?;

    let emote_re = Regex::new(&format!(
        r"(?s){}\(\(\) => {}\.isLoggedIn \? {}\({}\.userInfo\)[\s\S]{{0,250}}?\}}\)\) : ([A-Za-z0-9_$]+)\.value\)",
        regex::escape(computed_sym),
        regex::escape(store),
        regex::escape(getemotes_sym),
        regex::escape(store)
    ))
    .unwrap();
    let emotes_cap = emote_re
        .captures(&main_orig)
        .ok_or("Emote selection computed layout changed")?;
    let emotes = format!("{}.value", &emotes_cap[1]);

    let mut edits = vec![Edit::new(
        0,
        0,
        format!("import {{zzLocalAtlas as __lg}} from \"./{shared_name}\";\n"),
    )];
    let insert_fn = |edits: &mut Vec<Edit>, name: &str, code: String| -> Result<(), String> {
        let pos = named(&component_functions, name)?.body + 1;
        edits.push(Edit::new(pos, pos, format!("\n{code}\n")));
        Ok(())
    };

    let get_bag_list = require_import(&mi, "getBagList")?;
    let atlas_imp = require_import(&mi, "atlas")?;
    let activity_info_imp = require_import(&mi, "activityInfo")?;
    let get_preset_list = require_import(&mi, "getPresetList")?;

    let bag_loader = function_for(&component_functions, &main_orig, &format!("await {get_bag_list}()"))?;
    let atlas_loader = function_for(&component_functions, &main_orig, &format!("await {atlas_imp}()"))?;
    let activity_loader =
        function_for(&component_functions, &main_orig, &format!("await {activity_info_imp}()"))?;

    insert_fn(
        &mut edits,
        &atlas_loader.name,
        format!(
            "if(__lg.enabled.value){{{atlas}=__lg.offlineData.value.atlas;{atlas_loading}=false;return;}}"
        ),
    )?;
    insert_fn(&mut edits, &activity_loader.name, "try {".into())?;
    edits.push(Edit::new(
        activity_loader.body_end - 1,
        activity_loader.body_end - 1,
        "}finally{if(__lg.enabled.value)__localRefresh();}",
    ));

    for action in ["create-room", "join-room"] {
        let fn_name = m_raw
            .cases
            .get(action)
            .ok_or_else(|| format!("缺少 switch case {action}"))?;
        insert_fn(
            &mut edits,
            fn_name,
            format!(
                "if(__lg.enabled.value){{{show_toast_sym}('本地全图鉴已开启，无法进入联机。请先关闭开关。');return;}}"
            ),
        )?;
    }

    let param_of = |fn_name: &str| -> Result<Option<String>, String> {
        let re = Regex::new(&format!(r"function\s+{}\((\w+)", regex::escape(fn_name))).unwrap();
        Ok(re.captures(&main_orig).map(|c| c[1].to_owned()))
    };

    let select_name = m_raw
        .cases
        .get("select-appearance")
        .ok_or("缺少 case select-appearance")?;
    let select_param = param_of(select_name)?.ok_or("select-appearance 缺少参数")?;
    let select_fn = named(&component_functions, select_name)?;
    let select_source = &main_orig[select_fn.start..select_fn.end];
    let max_emotes = Regex::new(r"\.length >= ([A-Za-z0-9_$]+) \|\|")
        .unwrap()
        .captures(select_source)
        .ok_or("Emote capacity guard changed")?[1]
        .to_owned();
    insert_fn(
        &mut edits,
        select_name,
        format!(
            "if(__lg.enabled.value){{try{{__lg.select({select_param},{store}.userInfo,{max_emotes});__localRefresh();}}catch(error){{{show_toast_sym}(error.message);}}return;}}"
        ),
    )?;

    let reorder_name = m_raw
        .cases
        .get("reorder-emotes")
        .ok_or("缺少 case reorder-emotes")?;
    let reorder_param = param_of(reorder_name)?.ok_or("reorder-emotes 缺少参数")?;
    insert_fn(
        &mut edits,
        reorder_name,
        format!("if(__lg.enabled.value){{__lg.reorder({reorder_param},{store}.userInfo);__localRefresh();return;}}"),
    )?;

    let favorite_name = m_raw
        .cases
        .get("collect-change")
        .ok_or("缺少 case collect-change")?;
    let favorite_param = param_of(favorite_name)?.ok_or("collect-change 缺少参数")?;
    insert_fn(
        &mut edits,
        favorite_name,
        format!(
            "if(__lg.enabled.value){{__lg.favorite({favorite_param}.assetId,{favorite_param}.is_collect===1);return;}}"
        ),
    )?;

    let preset_loader =
        function_for(&component_functions, &main_orig, &format!("await {get_preset_list}()"))?;
    let preset_source = &main_orig[preset_loader.start..preset_loader.end];
    let preset_normalize_re = Regex::new(&format!(
        r"{} = [^\n]+\.map\(([A-Za-z0-9_$]+)\)",
        regex::escape(preset_list_field)
    ))
    .unwrap();
    let preset_normalize = preset_normalize_re
        .captures(preset_source)
        .ok_or("Preset list normalization changed")?[1]
        .to_owned();
    let create_preset_fn = named(
        &component_functions,
        m_raw
            .cases
            .get("create-preset")
            .ok_or("缺少 case create-preset")?,
    )?;
    let preset_limit_re =
        Regex::new(&format!(r"{}\.length >= ([A-Za-z0-9_$]+)", regex::escape(preset_list_field)))
            .unwrap();
    let preset_limit = preset_limit_re
        .captures(&main_orig[create_preset_fn.start..create_preset_fn.end])
        .ok_or("Preset capacity guard changed")?[1]
        .to_owned();

    let create_op = format!("__lg.createLocalPreset({store}.userInfo,{preset_limit})");
    for (action, template) in [
        ("create-preset", create_op.as_str()),
        ("delete-preset", "__lg.deleteLocalPreset({arg})"),
        ("apply-preset", "__lg.applyPreset({arg}.preset_id)"),
    ] {
        let action_name = m_raw
            .cases
            .get(action)
            .ok_or_else(|| format!("缺少 case {action}"))?;
        let arg = param_of(action_name)?.unwrap_or_else(|| "null".into());
        let op = template.replace("{arg}", &arg);
        insert_fn(
            &mut edits,
            action_name,
            format!(
                "if(__lg.enabled.value){{try{{{op};__localRefresh();}}catch(error){{{show_toast_sym}(error.message);}}return;}}"
            ),
        )?;
    }

    let mut snap_text = main_orig[snapshot.start..snapshot.end].to_owned();
    snap_text = snap_text.replace(
        &format!("bagList: {bag}"),
        &format!("bagList: __lg.inventory({atlas},{bag})"),
    );
    snap_text = snap_text.replace(
        &format!("atlasList: {atlas}"),
        &format!("atlasList: __lg.enabled.value ? __lg.offlineData.value.atlas : {atlas}"),
    );
    snap_text = snap_text.replace(
        &format!("presetList: {preset_list_field}"),
        &format!(
            "presetList: __lg.enabled.value ? __lg.mergePresets(__lg.presetList().map({preset_normalize}), {preset_list_field}) : {preset_list_field}"
        ),
    );
    snap_text = snap_text.replace(
        &format!("presetListLoading: {preset_loading_field}"),
        &format!("presetListLoading: {preset_loading_field}"),
    );
    snap_text = snap_text.replace(
        user_info_field,
        &user_info_field.replace(
            &format!("{store}.userInfo"),
            &format!("__lg.viewUser({store}.userInfo)"),
        ),
    );
    snap_text = snap_text.replace(
        &format!("selectedEmoteAssetIds: {selected_emotes_field}"),
        &format!(
            "selectedEmoteAssetIds: {getemotes_sym}(__lg.viewUser({store}.userInfo)).map(item=>item.id)"
        ),
    );
    edits.push(Edit::new(snapshot.start, snapshot.end, snap_text));

    for meaning in [
        "getUserCharacterAssetId",
        "getUserCharacterAssetName",
        "getUserAccessoryAssetId",
        "getUserAccessoryAssetName",
        "getUserEmoteAssets",
        "getUserAccessoryOffset",
        "getUserCharacterMyOffset",
    ] {
        let alias = require_import(&mi, meaning)?;
        let call_re =
            Regex::new(&format!(r"{}\({}\.userInfo\)", regex::escape(alias), regex::escape(store)))
                .unwrap();
        for m in call_re.find_iter(&main_orig) {
            if !(snapshot.start <= m.start() && m.start() < snapshot.end) {
                edits.push(Edit::new(
                    m.start(),
                    m.end(),
                    format!("{alias}(__lg.viewUser({store}.userInfo))"),
                ));
            }
        }
    }

    let local_code = format!(
        r#"
let __localSaved=null;
function __localRefresh(){{
 if(!__lg.enabled.value)return;
 const user=__lg.viewUser({store}.userInfo);
 {host}=__lg.assetsHost;
 const name={getchar_sym}(user);if(name){role}=name.replace(/\.png$/i,'');
 {accessory}={getaccess_sym}(user)||null;
 {emotes}={getemotes_sym}(user).map(item=>({{id:item.id,name:item.name}}));
 {atlas}=__lg.offlineData.value.atlas;
}}
{watch_sym}(__lg.enabled,on=>{{
 if(on){{__localSaved={{role:{role},accessory:{accessory},emotes:[...{emotes}],host:{host},atlas:{atlas}}};__localRefresh();}}
 else if(__localSaved){{{role}=__localSaved.role;{accessory}=__localSaved.accessory;{emotes}=__localSaved.emotes;{host}=__localSaved.host;{atlas}=__localSaved.atlas;__localSaved=null;{act_fn}({{force:true}}).then(()=>{{{bag_fn}()}}).catch(()=>{{}});}}
}},{{immediate:true}});
{watch_sym}([__lg.selection,__lg.revision,{appear_catalog_sym}],()=>{{if(__lg.enabled.value)__localRefresh();}});
{watch_sym}([()=>{role},()=>{accessory},()=>{host}],()=>{{if(__lg.enabled.value)__localRefresh();}});
"#,
        act_fn = activity_loader.name,
        bag_fn = bag_loader.name,
    );
    let main_return = main_component
        .return_start
        .ok_or("主组件缺少 returnStart")?;
    edits.push(Edit::new(main_return, main_return, local_code));
    let mut main = apply_edits(&main_orig, edits)?;
    for key in ["canCreateRoom", "canJoinRoom"] {
        let field_val = req_field(key)?;
        main = main.replace(
            &format!("{key}: {field_val}"),
            &format!("{key}: !__lg.enabled.value && {field_val}"),
        );
    }

    // Panel edits
    let p_raw = structure
        .files
        .get(&panel_name)
        .ok_or_else(|| format!("js_structure 缺少 {panel_name}"))?;
    let mut pi = p_raw.imports.clone();
    for canonical in ["listen", "invoke", "watch", "onMounted", "onUnmounted"] {
        with_alias(&mut pi, canonical);
    }
    let settings = find_component(p_raw, "SettingsModule")?;
    let atlas_component = find_component(p_raw, "MainPanelAtlasTab")?;
    let container = p_raw
        .containers
        .iter()
        .find(|c| settings.body < c.start && c.start < settings.body_end)
        .ok_or("SettingsModule 缺少 panel-content 容器")?;
    let h = require_import(&pi, "createBaseVNode")?;
    let with_modifiers = require_import(&pi, "withModifiers")?;
    let show_toast_p = require_import(&pi, "showToast")?;

    let setup = format!(
        "const __localEnabled=__lg.enabled,__localBusy=__lg.busy,__localSync=__lg.syncStatus,__localCache=__lg.cacheReady;\n\
async function __toggleLocal(event){{try{{await __lg.setEnabled(event.target.checked);}}catch(error){{{show_toast_p}(error.message);}}event.target.checked=__localEnabled.value;}}\n\
async function __syncLocal(){{try{{const status=await __lg.syncCache(true);{show_toast_p}('本地图鉴已同步：'+(status.result?.atlas_count||'')+'项');}}catch(error){{{show_toast_p}(error.message);}}}}\n\
function __openCommunityUpdate(event){{try{{event&&event.stopPropagation&&event.stopPropagation();__lg.assistant.openPanel('update');}}catch(error){{console.error('[assistant]',error);}}}}\n"
    );
    let row = format!(
        "{h}('div',{{class:'setting-item','data-local-gallery-setting':'true'}},[\n\
 {h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'外观模式'),{h}('div',{{class:'setting-sub'}},__localBusy.value?'正在切换本地显示模式…':!__localCache.value?(__localSync.value.status==='running'?'正在下载本地素材 '+__localSync.value.done+'/'+__localSync.value.total+'，完成后自动可用':__localSync.value.status==='failed'?'本地素材下载失败，点下方「检查并同步」重试':'本地素材还没准备好，点下方「检查并同步」开始下载'):__localEnabled.value?'当前：全图鉴 + Mod · 联机不可用，其他网络功能正常':'当前：官方 · 开启后使用全图鉴和已导入 Mod',1)]),\n\
 {h}('div',{{class:'setting-content'}},[{h}('label',{{class:'setting-switch'}},[{h}('input',{{type:'checkbox',role:'switch','aria-label':'全图鉴 + Mod',style:'display:block;position:absolute;width:1px;height:1px;opacity:0',checked:__localEnabled.value,disabled:__localBusy.value||!__localCache.value||__localSync.value.status==='running',onChange:__toggleLocal,onMousedown:{with_modifiers}(()=>{{}},['stop'])}},null,40,['checked','disabled']),{h}('span',{{class:'setting-slider'}})]),{h}('button',{{class:'setting-btn',type:'button',style:'margin-left:10px;white-space:nowrap',onMousedown:{with_modifiers}(()=>{{}},['stop']),onClick:()=>__lg.assistant.openPanel('mods')}},'Mod 管理')])]),\n\
 {h}('div',{{class:'setting-item','data-local-gallery-sync':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'更新本地图鉴'),{h}('div',{{class:'setting-sub'}},__localSync.value.status==='running'?'正在同步 '+__localSync.value.done+'/'+__localSync.value.total:!__localCache.value?'首次启动要联网补齐本地素材（后台进行，不耽误玩）':'联网检查新增图鉴，本地全图鉴开启时也可同步',1)]),{h}('button',{{class:'setting-btn',type:'button',disabled:__localSync.value.status==='running',onClick:__syncLocal}},'检查并同步',8,['disabled'])]),\n"
    );

    let mut panel_edits = vec![
        Edit::new(
            0,
            0,
            format!("import {{zzLocalAtlas as __lg}} from \"./{shared_name}\";\n"),
        ),
        Edit::new(settings.body + 1, settings.body + 1, setup),
        Edit::new(container.array + 1, container.array + 1, row),
    ];

    // Native Console tab in panel (extend_panel)
    let module_component = find_component(p_raw, "MainModulePanel")?;
    let module_text = &panel_orig[module_component.start..module_component.end];
    let nav_iter =
        Regex::new(r"([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\), ([A-Za-z0-9_$]+) =>")
            .unwrap()
            .captures(module_text)
            .ok_or("navigation iterator changed")?;
    let (nav_render, nav_unref, nav_items, nav_item) =
        (&nav_iter[1], &nav_iter[2], &nav_iter[3], &nav_iter[4]);
    let nav_state = Regex::new(r#"([A-Za-z0-9_$]+)\.activeTab === "bag""#)
        .unwrap()
        .captures(module_text)
        .ok_or("navigation state binding changed")?[1]
        .to_owned();
    let nav_label = Regex::new(r#"return ([A-Za-z0-9_$]+)\.key === "toggle""#)
        .unwrap()
        .captures(module_text)
        .ok_or("navigation label binding changed")?[1]
        .to_owned();

    let define_component = require_import(&pi, "defineComponent")?;
    let ref_p = require_import(&pi, "ref")?;
    let on_mounted_p = require_import(&pi, "onMounted")?;
    let on_unmounted_p = require_import(&pi, "onUnmounted")?;
    let computed_p = require_import(&pi, "computed")?;
    let unref_p = require_import(&pi, "unref")?;
    let open_block_p = require_import(&pi, "openBlock")?;
    let create_elem_block_p = require_import(&pi, "createElementBlock")?;
    let create_block_p = require_import(&pi, "createBlock")?;

    let console_component = format!(
        "\nconst __ConsolePage={define_component}({{\n\
 __name:'WuwaNativeConsole',setup(){{\n\
  const target={ref_p}(null);\n\
  {on_mounted_p}(()=>__lg.assistant.mountConsole(target.value));\n\
  {on_unmounted_p}(()=>__lg.assistant.closePanel());\n\
  return ()=>{h}('div',{{class:'module-block',style:{{height:'100%',minHeight:0,position:'relative',overflow:'hidden',paddingTop:'6px'}}}},[{h}('div',{{ref:target,style:{{height:'100%',minHeight:0,overflow:'auto'}}}})]);\n\
 }}\n\
}});\n\
const __consoleTabs={computed_p}(()=>{unref_p}({nav_items}).flatMap(item=>item.key==='setting'?[{{key:'console',labelKey:'console'}},item]:[item]));\n"
    );
    panel_edits.push(Edit::new(
        module_component.body + 1,
        module_component.body + 1,
        console_component,
    ));

    let icon_re = Regex::new(&format!(
        r#"{}\("span", \{{\s*class: [A-Za-z0-9_$]+\(\["tab-btn__icon", `tab-btn__icon--\$\{{{}\.key\}}`\]\)\s*\}}, null, 2\)"#,
        regex::escape(h),
        regex::escape(nav_item)
    ))
    .unwrap();
    let icon_match = icon_re
        .find(module_text)
        .ok_or("native navigation icon layout changed")?;
    let console_icon = format!(
        "{nav_item}.key==='console'?({open_block_p}(),{create_elem_block_p}('svg',{{key:'console-icon',class:'tab-btn__icon tab-btn__icon--console',viewBox:'0 0 40 54','aria-hidden':'true','data-console-icon':'sliders',style:{{color:{nav_state}.activeTab==='console'?'#9c6541':'#f4d5a5'}}}},[\n\
 {h}('rect',{{x:7,y:15,width:26,height:25,rx:5,fill:'none',stroke:'currentColor','stroke-width':2.5}}),\n\
 {h}('path',{{d:'M14 21v13M20 21v13M26 21v13',fill:'none',stroke:'currentColor','stroke-width':2.2,'stroke-linecap':'round'}}),\n\
 {h}('circle',{{cx:14,cy:25,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:20,cy:30,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:26,cy:24,r:2.6,fill:'currentColor'}})\n\
],4)):{}",
        icon_match.as_str()
    );
    panel_edits.push(Edit::new(
        module_component.start + icon_match.start(),
        module_component.start + icon_match.end(),
        console_icon,
    ));

    for (old, new) in [
        (
            format!("return {nav_label}.key === \"toggle\""),
            format!(
                "return {nav_label}.key === \"console\" ? \"控制台\" : {nav_label}.key === \"toggle\""
            ),
        ),
        (
            format!("{nav_render}({nav_unref}({nav_items}),"),
            format!("{nav_render}(__consoleTabs.value,"),
        ),
        (
            format!("{nav_state}.activeTab === \"bag\" ?"),
            format!(
                "{nav_state}.activeTab === \"console\" ? ({open_block_p}(),{create_block_p}(__ConsolePage,{{key:\"console\"}})) : {nav_state}.activeTab === \"bag\" ?"
            ),
        ),
    ] {
        if module_text.match_indices(&old).count() != 1 {
            return Err(format!("native console anchor '{old}' not unique"));
        }
        let pos = module_component.start + module_text.find(&old).unwrap();
        panel_edits.push(Edit::new(pos, pos + old.len(), new));
    }

    let panel_root = find_component(p_raw, "index")?;
    let root_source = &panel_orig[panel_root.body..panel_root.body_end];
    let state_ref_re = Regex::new(&format!(
        r"const ([A-Za-z0-9_$]+) = {}\(null\)",
        regex::escape(ref_p)
    ))
    .unwrap();
    let state_ref_name = if let Some(caps) = state_ref_re.captures(root_source) {
        caps[1].to_owned()
    } else {
        let sub_re = Regex::new(
            r"(?s)=\s*await\s+[A-Za-z0-9_$]+\(\s*([A-Za-z0-9_$]+)\s*=>\s*\{(.{0,800}?)\n\s*\}\)",
        )
        .unwrap();
        let sub_caps = sub_re
            .captures(root_source)
            .ok_or("Panel snapshot subscription changed")?;
        let param = &sub_caps[1];
        let body = &sub_caps[2];
        let got_re =
            Regex::new(&format!(r"([A-Za-z0-9_$]+)\.value = {}\b", regex::escape(param))).unwrap();
        got_re
            .captures(body)
            .ok_or("Panel snapshot state assignment changed")?[1]
            .to_owned()
    };
    let ready_retry = format!(
        "\nlet __panelDisposed=false;\n\
{on_unmounted_p}(()=>{{__panelDisposed=true;}});\n\
{on_mounted_p}(()=>{{
 for(const delay of [500,1500,3000,6000,10000])window.setTimeout(()=>{{
  if(!__panelDisposed&&!{state_ref_name}.value)window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{{event:'cat-panel-action',payload:{{type:'ready'}}}}).catch(()=>{{}});
 }},delay);
}});\n"
    );
    let panel_return = panel_root
        .return_start
        .ok_or("面板根组件缺少 returnStart")?;
    panel_edits.push(Edit::new(panel_return, panel_return, ready_retry));

    for condition in &p_raw.room_conditions {
        if condition.test.contains("activeTab") {
            let message = format!(
                "{open_block_p}(), {create_elem_block_p}('div',{{class:'module-block',style:'padding:24px'}},'本地全图鉴已开启，无法进入联机。请先关闭开关。')"
            );
            panel_edits.push(Edit::new(
                condition.start,
                condition.start,
                format!("__lg.enabled.value && {} ? ({message}) : ", condition.test),
            ));
        }
    }

    let atlas_text = &panel_orig[atlas_component.start..atlas_component.end];
    let merge_check_re = Regex::new(&format!(
        r"[A-Za-z0-9_$]+ = {}\(\(\) => [A-Za-z0-9_$]+\([A-Za-z0-9_$]+\([A-Za-z0-9_$]+\.atlasList, [A-Za-z0-9_$]+\.bagList\)\)\)",
        regex::escape(computed_p)
    ))
    .unwrap();
    if !merge_check_re.is_match(atlas_text) {
        return Err("atlas row merge computed changed".into());
    }
    let category_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => \{{\s*const [A-Za-z0-9_$]+ = [A-Za-z0-9_$]+\[([A-Za-z0-9_$]+)\.value\];",
        regex::escape(computed_p)
    ))
    .unwrap();
    let cat_caps = category_re
        .captures(atlas_text)
        .ok_or("atlas category computed changed")?;
    let (category_rows, category_index) = (&cat_caps[1], &cat_caps[2]);
    let atlas_loading_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => [A-Za-z0-9_$]+\.bagListLoading \|\| [A-Za-z0-9_$]+\.atlasListLoading\)",
        regex::escape(computed_p)
    ))
    .unwrap();
    let atlas_loading_var = atlas_loading_re
        .captures(atlas_text)
        .ok_or("atlas loading computed changed")?[1]
        .to_owned();
    let atlas_return = atlas_component
        .return_start
        .ok_or("MainPanelAtlasTab 缺少 returnStart")?;
    let global_store_p = require_import(&pi, "globalStore")?;
    panel_edits.push(Edit::new(
        atlas_return,
        atlas_return,
        format!(
            "const __atlasProgress={computed_p}(()=>__lg.collectionProgress({category_rows}.value,{atlas_loading_var}.value)),__atlasProgressTitle={computed_p}(()=>['全图鉴收集进度','全角色收集进度','全配件收集进度','全表情收集进度'][{category_index}.value]||'全图鉴收集进度');\n"
        ),
    ));
    let tab_anchor = "null, 8, [\"modelValue\", \"tabs\"]),";
    if atlas_text.match_indices(tab_anchor).count() != 1 {
        return Err("atlas tab header changed".into());
    }
    let progress_ui = format!(
        " ({open_block_p}(true),{create_elem_block_p}('section',{{key:'account-atlas-progress','data-atlas-progress':'online',style:{{padding:'12px 4px 14px',width:'100%',boxSizing:'border-box'}}}},[\n\
 {h}('div',{{style:{{display:'flex',justifyContent:'space-between',alignItems:'center',gap:'12px',fontSize:'14px',color:'#927b65',marginBottom:'8px'}}}},[\n\
 {h}('span',null,__atlasProgressTitle.value),{h}('strong',{{style:{{fontVariantNumeric:'tabular-nums',color:'#996644'}}}},\n\
 !{global_store_p}.isLoggedIn?'登录后查看':{atlas_loading_var}.value?'正在读取…':__atlasProgress.value.collected+'/'+__atlasProgress.value.total+' · '+__atlasProgress.value.percent.toFixed(1)+'%')]),\n\
 {h}('div',{{role:'progressbar','aria-label':'真实账号'+__atlasProgressTitle.value,'aria-valuemin':0,'aria-valuemax':__atlasProgress.value.total,'aria-valuenow':__atlasProgress.value.collected,'aria-busy':{atlas_loading_var}.value,style:{{height:'9px',width:'100%',background:'#ecddc4',borderRadius:'20px',overflow:'hidden'}}}},[\n\
 {h}('span',{{style:{{display:'block',height:'100%',width:({global_store_p}.isLoggedIn?__atlasProgress.value.percent:0)+'%',background:'linear-gradient(90deg,#e4b56f,#c58b51)',borderRadius:'inherit',transition:'width .25s ease'}}}})])],-2)),"
    );
    let progress_at = atlas_component.start + atlas_text.find(tab_anchor).unwrap() + tab_anchor.len();
    panel_edits.push(Edit::new(progress_at, progress_at, progress_ui));

    let bag_merge_re = Regex::new(
        r"\(\s*([A-Za-z0-9_$]+)\(\s*([A-Za-z0-9_$]+)\.atlasList\s*,\s*([A-Za-z0-9_$]+)\.bagList\s*\)\s*\)",
    )
    .unwrap();
    if let Some(m) = bag_merge_re.captures(&panel_orig) {
        let span = m.get(0).unwrap();
        panel_edits.push(Edit::new(
            span.start(),
            span.end(),
            format!(
                "({}({}.atlasList, window.__WUWA_LOCAL_ATLAS__?.ownedBag?.() || {}.bagList))",
                &m[1], &m[2], &m[3]
            ),
        ));
    } else {
        eprintln!("warn: 找不到图鉴与背包的合并点，跳过「未拥有角标」");
    }

    let clickable_re = Regex::new(r#""is-item-clickable":\s*([A-Za-z0-9_$]+)"#).unwrap();
    for caps in clickable_re.captures_iter(&panel_orig) {
        let m = caps.get(0).unwrap();
        if panel_orig.as_bytes().get(m.end()) == Some(&b'.') {
            continue;
        }
        let name = &caps[1];
        panel_edits.push(Edit::new(
            m.start(),
            m.end(),
            format!(
                "\"is-item-clickable\": (__c => window.__WUWA_LOCAL_ATLAS__?.enabled.value ? !0 : {name}(__c))"
            ),
        ));
    }

    let preset_add_re = Regex::new(
        r#"\{\s*"preset-item--add":\s*([A-Za-z0-9_$]+)\.kind\s*===\s*"add"\s*\}"#,
    )
    .unwrap();
    for caps in preset_add_re.captures_iter(&panel_orig) {
        let m = caps.get(0).unwrap();
        let item = &caps[1];
        panel_edits.push(Edit::new(
            m.start(),
            m.end(),
            format!(
                "{{\"preset-item--add\": {item}.kind === \"add\", \"preset-item--account\": {item}.kind === \"preset\" && ({item}.preset?.preset_id ?? 0) > 0, \"preset-item--local\": {item}.kind === \"preset\" && ({item}.preset?.preset_id ?? 0) <= 0}}"
            ),
        ));
    }

    let bag_component = find_component(p_raw, "MainPanelBagTab")?;
    let bag_text = &panel_orig[bag_component.start..bag_component.end];
    let bag_guard = format!("if (!{global_store_p}.isLoggedIn)");
    if bag_text.match_indices(&bag_guard).count() != 2 {
        return Err("preset login guard layout changed".into());
    }
    for (idx, _) in bag_text.match_indices(&bag_guard) {
        panel_edits.push(Edit::new(
            bag_component.start + idx,
            bag_component.start + idx + bag_guard.len(),
            format!("if (!{global_store_p}.isLoggedIn && !__lg.enabled.value)"),
        ));
    }
    let delete_guard_re = Regex::new(&format!(
        r"if \([A-Za-z0-9_$]+\.stopPropagation\(\), !{}\.isLoggedIn\)",
        regex::escape(global_store_p)
    ))
    .unwrap();
    let del_m = delete_guard_re
        .find(bag_text)
        .ok_or("preset delete login guard changed")?;
    let del_str = del_m.as_str();
    panel_edits.push(Edit::new(
        bag_component.start + del_m.start(),
        bag_component.start + del_m.end(),
        format!(
            "{} && !__lg.enabled.value)",
            &del_str[..del_str.len() - 1]
        ),
    ));

    let mut panel = apply_edits(&panel_orig, panel_edits)?;
    let update_marker = "\"setting-btn btn-check-update\"";
    if panel.match_indices(update_marker).count() != 1 {
        return Err("official update button contract changed".into());
    }
    let marker_at = panel.find(update_marker).unwrap();
    let call_at = panel[marker_at..]
        .find("onClick:")
        .map(|o| marker_at + o)
        .ok_or("official update button onClick missing")?;
    let handler_rel = panel[call_at..]
        .find("R(")
        .map(|o| call_at + o + 2)
        .ok_or("official update button R( missing")?;
    let handler_end = panel[handler_rel..]
        .find(',')
        .map(|o| handler_rel + o)
        .ok_or("official update button handler delimiter missing")?;
    let raw_handler = &panel[handler_rel..handler_end];
    let handler = raw_handler.trim();
    if handler.is_empty()
        || !handler
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
    {
        return Err(format!("official update handler invalid: {raw_handler}"));
    }
    let trimmed_start = handler_rel + raw_handler.find(handler).unwrap();
    panel.replace_range(
        trimmed_start..trimmed_start + handler.len(),
        "__openCommunityUpdate",
    );

    let unfav = require_import(&pi, "unfavoriteAsset")?;
    let fav = require_import(&pi, "favoriteAsset")?;
    let fav_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+)\.value \? await ([A-Za-z0-9_$]+)\(await {}\(([A-Za-z0-9_$]+)\.item\.asset_id\)\) : await ([A-Za-z0-9_$]+)\(await {}\(([A-Za-z0-9_$]+)\.item\.asset_id\)\)",
        regex::escape(unfav),
        regex::escape(fav)
    ))
    .unwrap();
    let fav_caps = fav_re
        .captures_iter(&panel)
        .find(|c| &c[2] == &c[4] && &c[3] == &c[5])
        .ok_or("Favorite toggle layout changed")?;
    let fav_pos = fav_caps.get(0).unwrap().start();
    let fav_flag = fav_caps[1].to_owned();
    let fav_item = fav_caps[3].to_owned();
    panel.insert_str(
        fav_pos,
        &format!("__lg.enabled.value ? __lg.favorite({fav_item}.item.asset_id,!{fav_flag}.value) : "),
    );

    // Phase 2: Desktop assistant + platform hooks (assistant_core + WindowsHooks)
    let mut facade_files = vec!["assistant_dashboard.js", "desktop_assistant.js"];
    if platform == Platform::Windows {
        facade_files.push("desktop_context_menu.js");
    }
    let mut factory_parts = Vec::with_capacity(facade_files.len());
    for fname in facade_files {
        factory_parts.push(render_frontend_module(repo_root, fname, platform)?);
    }
    let factory = factory_parts.join("\n");

    let mut handlers = Vec::with_capacity(2);
    for event in ["key-press", "key-release"] {
        let listen_sym = require_import(&mi, "listen")?;
        let re = Regex::new(&format!(
            r#"{}\("{event}", (\w+) => \{{\s*([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\.payload\)"#,
            regex::escape(listen_sym)
        ))
        .unwrap();
        let caps = re
            .captures_iter(&main_orig)
            .find(|c| &c[1] == &c[3])
            .ok_or_else(|| format!("缺少键盘事件监听：{event}"))?;
        handlers.push(caps[2].to_owned());
    }
    let press_fn = named(&m_raw.functions, &handlers[0])?;
    let press_text = &main_orig[press_fn.start..press_fn.end];
    let first_guard_caps = Regex::new(r"if \(([^\n]+)\) return;")
        .unwrap()
        .captures(press_text)
        .ok_or("按键处理缺少守卫条件")?;
    let held_guard = Regex::new(r"[A-Za-z0-9_$]+\.has\(e\)")
        .unwrap()
        .find_iter(&first_guard_caps[1])
        .last()
        .ok_or("按键守卫缺少 held_guard")?
        .as_str()
        .to_owned();
    main = rec.replace(
        &main,
        &format!("function {}(e) {{", handlers[0]),
        &format!("function {}(e,__assistantVirtual=false) {{", handlers[0]),
    )?;
    main = rec.replace(
        &main,
        &first_guard_caps[0],
        &format!(
            "if (__assistantVirtual ? {held_guard} : ({})) return;",
            &first_guard_caps[1]
        ),
    )?;

    let chest_fn = function_for(&m_raw.functions, &main_orig, "openChest failed")?;
    let chest_text = &main_orig[chest_fn.start..chest_fn.end];
    let cost = Regex::new(&format!(r"{}\.totalClickCount < ([A-Za-z0-9_$]+)", regex::escape(store)))
        .unwrap()
        .captures(chest_text)
        .ok_or("chest cost anchor missing")?[1]
        .to_owned();
    let animation = Regex::new(r"const \w+ = ([A-Za-z0-9_$]+)\(e\);")
        .unwrap()
        .captures(chest_text)
        .ok_or("chest animation anchor missing")?[1]
        .to_owned();

    let ready_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => ([A-Za-z0-9_$]+)\.value \|\| ![A-Za-z0-9_$]+\.isLoggedIn \? !0 : ([A-Za-z0-9_$]+)\.value <= 0\)",
        regex::escape(computed_sym)
    ))
    .unwrap();
    let ready_pairs: Vec<(String, String)> = ready_re
        .captures_iter(&main_orig)
        .map(|c| (c[1].to_owned(), c[3].to_owned()))
        .collect();
    if ready_pairs.len() != 2 {
        return Err(format!(
            "chest readiness contract (expected two ready computeds, got {})",
            ready_pairs.len()
        ));
    }
    let (left_ready, left_rem) = &ready_pairs[0];
    let (right_ready, right_rem) = &ready_pairs[1];
    let busy_caps = Regex::new(r"if \(([A-Za-z0-9_$]+)\.value \|\| ([A-Za-z0-9_$]+)\.value\) return;")
        .unwrap()
        .captures(chest_text)
        .ok_or("chest busy guard contract missing")?;
    let (busy_open, blocked_chest) = (&busy_caps[1], &busy_caps[2]);

    shared = rec.replace(
        &shared,
        "c === 10002 || c === 10003 || r === 401 || r === 403",
        "c === 10002 || c === 10003 || r === 401",
    )?;
    shared = rec.replace(
        &shared,
        "function isAppUpdaterEnabled() {",
        "function isAppUpdaterEnabled() {\n return false;",
    )?;
    shared = rec.replace(
        &shared,
        "function isForceUpdateLocked() {",
        "function isForceUpdateLocked() {\n return window.__WUWA_LOCAL_ATLAS__?.assistant?.updateRequired.value === true;",
    )?;
    shared = rec.replace(
        &shared,
        "function schedulePreloadAppUiImages() {",
        "function schedulePreloadAppUiImages() {\n const route=(window.location.hash||'#/').slice(1).split('?')[0];\n if(!['/','/panel'].includes(route))return;",
    )?;

    let report_sticker = require_import(&mi, "reportSendSticker")?;
    let emote_fn = function_for(&m_raw.functions, &main_orig, &format!("{report_sticker}({{"))?;
    let emote_text = &main_orig[emote_fn.start..emote_fn.end];
    if !emote_text.contains("e.src") || !emote_text.contains(".push({") {
        return Err("manual emote rendering contract changed".into());
    }
    let emote_items = Regex::new(&format!(
        r#""emote-items": ([A-Za-z0-9_$]+)\.value,\s*onSelect: {}"#,
        regex::escape(&emote_fn.name)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("emote selection binding changed")?[1]
        .to_owned();
    let emote_url = Regex::new(&format!(
        r#""asset-url": ([A-Za-z0-9_$]+),\s*"emote-items": {}"#,
        regex::escape(&emote_items)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("emote asset url binding changed")?[1]
        .to_owned();
    let sync_call_caps = Regex::new(&format!(
        r"{}\.isLoggedIn && ([A-Za-z0-9_$]+)\(e\.id\);",
        regex::escape(store)
    ))
    .unwrap()
    .captures(emote_text)
    .ok_or("original room emote send contract changed")?;
    main = rec.replace(
        &main,
        &sync_call_caps[0],
        &format!(
            "{store}.isLoggedIn && !__lg.enabled.value && {}(e.id);",
            &sync_call_caps[1]
        ),
    )?;

    let mut facade_locals = String::from(
        "\nif(localAtlasController)localAtlasController.assistant=createDesktopAssistant({ref,window,atlas:localAtlasController,displayName:resolveAssetDisplayName});\nwindow.__WUWA_OPEN_PANEL_TAB__=tab=>invoke(\"open_panel_window\",{payload:{tab}}).then(()=>true).catch(()=>false);",
    );
    if platform == Platform::Windows {
        facade_locals.push_str(
            "\nif(localAtlasController)localAtlasController.contextMenu=createDesktopContextMenu({invoke,request:(path,body)=>localAtlasController.request(path,body),settings:loadDesktopCatSettings,visible:()=>petDockVisibleForMenu,labels:async()=>{try{return await getNativeMenuLabels(formatShortcut((await getAppShortcuts()).togglePet));}catch(error){return {};}},report:details=>localAtlasController.request('/diagnostic',details)});",
        );
    }
    shared = rec.replace(
        &shared,
        "window.__WUWA_LOCAL_ATLAS__ = localAtlasController;",
        &format!("{factory}\nwindow.__WUWA_LOCAL_ATLAS__ = localAtlasController;{facade_locals}"),
    )?;
    shared = rec.replace(
        &shared,
        "localAtlasController.ready.then(() => { localStartOnlineServices(); return bootstrap(); })",
        "Promise.all([localAtlasController.ready,localAtlasController.assistant.ready]).then(() => { localStartOnlineServices(); return bootstrap(); })",
    )?;

    main = rec.replace(
        &main,
        &format!("const f = {animation}(e);"),
        &format!(
            "const __assistantAttempt=__lg.assistant.beginChest(e);\n        const f = {animation}(e);"
        ),
    )?;
    let report_reward_succ = require_import(&mi, "reportRewardClaimSucc")?;
    let reward_anchor = format!("          {report_reward_succ}({{\n            itemId: M.data.asset_id,");
    main = rec.replace(
        &main,
        &reward_anchor,
        &format!("          __lg.assistant.chestSucceeded(__assistantAttempt,M.data);\n{reward_anchor}"),
    )?;

    let room_field = req_field("currentRoomId")?;
    let room_members_field = req_field("roomMembers")?;
    let members = format!(
        "{}.value",
        &Regex::new(r"^([A-Za-z0-9_$]+)\.value")
            .unwrap()
            .captures(room_members_field)
            .ok_or("roomMembers 格式异常")?[1]
    );
    let input_room = req_field("roomIdInput")?;
    let join_fn = m_raw
        .cases
        .get("join-room")
        .ok_or("缺少 case join-room")?;
    let with_mod_m = require_import(&mi, "withModifiers")?;
    let open_settings = Regex::new(&format!(
        r#"class: "settings-btn"[\s\S]{{0,300}}?onClick: {}\(([A-Za-z0-9_$]+),"#,
        regex::escape(with_mod_m)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("settings-btn onClick 锚点丢失")?[1]
        .to_owned();
    let open_tab = function_for(&m_raw.functions, &main_orig, "[panel] open tab failed")?
        .name
        .clone();
    let quit_fn = Regex::new(r#""tray-quit", \(\) => \{\s*([A-Za-z0-9_$]+)\(\)"#)
        .unwrap()
        .captures(&main_orig)
        .ok_or("tray-quit 锚点丢失")?[1]
        .to_owned();
    let persist_imp = require_import(&mi, "listenPersistClicksBeforeExit")?;
    let persist_original = Regex::new(&format!(
        r"(?s){}\(async \(\) => \{{\s*([\s\S]+?)\n\s*\}}\)",
        regex::escape(persist_imp)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("listenPersistClicksBeforeExit 锚点丢失")?[1]
        .to_owned();
    main = rec.replace(
        &main,
        &format!("async function {quit_fn}() {{"),
        &format!("async function {quit_fn}() {{\n try{{await __lg.assistant.flush();}}catch{{}}"),
    )?;

    let tab_pos = main
        .find(&format!("function {open_tab}("))
        .ok_or("open_tab definition missing in patched main")?;
    let search_start = find_char_boundary_backward(&main, tab_pos.saturating_sub(6));
    let search_end = {
        let mut e = (tab_pos + 600).min(main.len());
        while e < main.len() && !main.is_char_boundary(e) {
            e += 1;
        }
        e
    };
    let tab_guard_re = Regex::new(&format!(
        r"(?:async )?function {}\(([A-Za-z0-9_$]+)\)\s*\{{\s*if \(([A-Za-z0-9_$]+)\(\)\) \{{\s*{}\(",
        regex::escape(&open_tab),
        regex::escape(show_toast_sym)
    ))
    .unwrap();
    let tab_caps = tab_guard_re
        .captures(&main[search_start..search_end])
        .ok_or("panel update-lock guard changed")?;
    let old_guard = tab_caps[0].to_owned();
    let tab_param = &tab_caps[1];
    let lock_fn = &tab_caps[2];
    let new_guard = old_guard.replacen(
        &format!("if ({lock_fn}()) {{"),
        &format!("if ({tab_param} !== \"console\" && {lock_fn}()) {{"),
        1,
    );
    main = rec.replace(&main, &old_guard, &new_guard)?;

    let mut code_prefix = String::new();
    let mut blocked_expr = "__lg.assistant.updateRequired.value".to_owned();
    let on_mounted_m = require_import(&mi, "onMounted")?;
    if platform == Platform::Windows {
        let (patched_main, ctx_patch) =
            patch_windows_context_menu(&main, &main_orig, &m_raw, &mi, &open_tab)?;
        main = patched_main;
        code_prefix = format!(
            "\n{on_mounted_m}(()=>{{window.setTimeout(()=>__lg?.contextMenu?.initialize({}).catch(error=>console.warn('[context-menu] initialize',error)),1500);}});",
            ctx_patch.context_actions
        );
        blocked_expr = "__lg.assistant.updateRequired.value||Boolean(__lg.contextMenu?.isOpen())".into();
    }

    let invoke_m = require_import(&mi, "invoke$1")?;
    let current_monitor_m = require_import(&mi, "currentMonitor")?;
    let listen_m = require_import(&mi, "listen")?;
    let code = format!(
        "{code_prefix}\n\
{on_mounted_m}(()=>{{window.setTimeout(()=>__lg.assistant.startMain({{\n\
 total:()=>{store}.cumulativeClickCount,balance:()=>{store}.totalClickCount,\n\
 account:()=>String({store}.userInfo?.cuid||{store}.userInfo?.user_id||{store}.userInfo?.uid||'guest'),\n\
 loggedIn:()=>{store}.isLoggedIn,blocked:()=>{blocked_expr},\n\
 press:key=>{press_h}(key,true),release:{release_h},chestCost:{cost},\n\
 chests:()=>({{left:{{remaining:Math.max(0,{left_rem}.value),ready:{left_ready}.value}},right:{{remaining:Math.max(0,{right_rem}.value),ready:{right_ready}.value}},cooldown:1800000,busy:{busy_open}.value||{blocked_chest}.value}}),\n\
 emotes:()=>({emote_items}.value||[]).map(item=>({{id:item.id,name:item.name,src:item.src||(item.name?((window.__WUWA_MOD_ASSET_PATH__&&window.__WUWA_MOD_ASSET_PATH__('emote/'+item.name))||{emote_url}('emote/'+item.name)):'')}})),\n\
 sendEmote:item=>{emote_send}(item),\n\
 monitors:__assistantMonitors,restoreMonitor:__assistantRestoreMonitor,\n\
 currentMonitor:{current_monitor_m},toast:{show_toast_sym},\n\
 openSettings:{open_settings},\n\
 openView:async view=>{{\n\
   if(__lg.assistant.updateRequired.value)return;\n\
   await {open_tab}(view==='atlas'?'atlas':'settings');\n\
   const send=()=>{{{invoke_m}('plugin:event|emit',{{event:'local-assistant-view',payload:view}}).catch(()=>{{}});}};\n\
   send();window.setTimeout(send,800);window.setTimeout(send,2200);\n\
 }},\n\
 quit:{quit_fn},\n\
 persistOriginal:async()=>{{{persist_original}}},\n\
 listenClicks:callback=>{watch_sym}(()=>{store}.localClickCount,(next,prev)=>{{if(Number(next)===Number(prev)+1)callback(1);}},{{flush:'sync'}}),\n\
 listenRoom:callback=>{watch_sym}([()=>{room_field},()=>{members}.map(x=>[x.id,x.nickname,x.isOwner]).join('|')],()=>{{
   const owner={members}.find(x=>x.isOwner);if({room_field})callback(String({room_field}),owner?.nickname||'').catch(()=>{{}});
 }},{{immediate:true}}),\n\
 listenJoin:callback=>{listen_m}('local-assistant-join',event=>callback(String(event.payload))),\n\
 join:async code=>{{{input_room}=code;await {join_fn}();}}\n\
}}).catch(error=>console.error('[assistant]',error)),3000);}});\n",
        press_h = handlers[0],
        release_h = handlers[1],
        emote_send = emote_fn.name,
    );
    main = rec.replace(&main, "let __localSaved=null;", &format!("{code}\nlet __localSaved=null;"))?;

    let monitor_name = fs::read_dir(&readable_assets)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("switchDisplayMonitor-") && name.ends_with(".js"))
        .ok_or("缺少 switchDisplayMonitor-*.js 模块")?;
    let mut monitor = read_normalized_js(&readable_assets.join(&monitor_name))?;
    let sw_caps = Regex::new(
        r"async function ([A-Za-z0-9_$]+)\(\) \{\n  let e = !1;\n  const t = `\$\{Date\.now\(\)\}",
    )
    .unwrap()
    .captures(&monitor)
    .ok_or("display switch contract changed")?;
    let switch_fn = sw_caps[1].to_owned();
    monitor = rec.replace(
        &monitor,
        &format!("async function {switch_fn}() {{"),
        &format!("async function {switch_fn}(targetName=null) {{"),
    )?;
    monitor = rec.replace(
        &monitor,
        "if (d.length <= 1)",
        "if (!targetName && d.length <= 1)",
    )?;
    monitor = rec.replace(
        &monitor,
        "const g = (N + 1) % d.length,",
        "if(targetName){const __t=d.find(x=>x.name===targetName);if(!__t)return false;if(_&&p(_)===p(__t))return false;}\n    const g = targetName ? d.findIndex(x=>x.name===targetName) : (N + 1) % d.length,",
    )?;
    let mon_fn = Regex::new(
        r"async function ([A-Za-z0-9_$]+)\(\) \{\n  const e = await me\(\),\n    t = await H\(\);\n  return qe\(e, t\)",
    )
    .unwrap()
    .captures(&monitor)
    .ok_or("display list contract changed")?[1]
        .to_owned();
    monitor.push_str(&format!(
        "\nexport{{{mon_fn} as __zzMonitors,{switch_fn} as __zzSwitchMonitor}};\n"
    ));
    let tab_guard =
        "return e === \"bag\" || e === \"atlas\" || e === \"exchange\" || e === \"room\" || e === \"setting\"";
    monitor = rec.replace(&monitor, tab_guard, &format!("{tab_guard} || e === \"console\""))?;
    if platform == Platform::Windows {
        monitor = patch_windows_monitor_autostart(&monitor);
    }
    fs::write(frontend_dir.join("assets").join(&monitor_name), monitor.as_bytes())
        .map_err(|e| e.to_string())?;
    main = format!(
        "import {{__zzMonitors as __assistantMonitors,__zzSwitchMonitor as __assistantRestoreMonitor}} from \"./{monitor_name}\";\n{main}"
    );

    let assistant_row = format!(
        "{h}('div',{{class:'setting-item','data-desktop-assistant':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'桌宠助手'),{h}('div',{{class:'setting-sub'}},'自动点击与开箱、每日统计、Mod 管理与版本更新')]),{h}('button',{{class:'setting-btn',type:'button',onClick:()=>__lg.assistant.openPanel()}},'打开控制台')]),"
    );
    let gallery_anchor =
        format!("{h}('div',{{class:'setting-item','data-local-gallery-setting':'true'}},[");
    panel = rec.replace(
        &panel,
        &gallery_anchor,
        &format!("{assistant_row}\n{gallery_anchor}"),
    )?;

    let pair_re = Regex::new(r"await [A-Za-z0-9_$]+\(\), await [A-Za-z0-9_$]+\(\)").unwrap();
    let pair_calls: HashSet<String> = pair_re
        .find_iter(&panel)
        .map(|m| m.as_str().to_owned())
        .collect();
    if !pair_calls
        .iter()
        .any(|call| panel.match_indices(call.as_str()).count() == 2)
        && panel.match_indices("await ds(), await xt()").count() != 2
    {
        return Err("panel close lifecycle changed".into());
    }
    let listen_p = require_import(&pi, "listen")?;
    panel = rec.replace(
        &panel,
        "let __panelDisposed=false;",
        &format!(
            "let __assistantViewUnlisten=null;\n\
{on_mounted_p}(async()=>{{__assistantViewUnlisten=await {listen_p}('local-assistant-view',event=>{{\n\
 const view=event.payload;\n\
 if(['overview','rewards','daily'].includes(view))__lg.assistant.openPanel(view).catch(error=>console.error('[assistant-view]',error));\n\
 else if(['atlas','settings'].includes(view))__lg.assistant.closePanel();\n\
}});}});\n\
{on_unmounted_p}(()=>{{__assistantViewUnlisten?.();}});\n\
let __panelDisposed=false;"
        ),
    )?;

    for (name, text) in [
        (&shared_name, &shared),
        (&main_name, &main),
        (&panel_name, &panel),
    ] {
        fs::write(frontend_dir.join("assets").join(name), text.as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let mut extra_patched_assets = Vec::new();
    for entry in fs::read_dir(base_dir.join("frontend/assets")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let fname = entry.file_name().to_string_lossy().into_owned();
        if !fname.starts_with("switchDisplayMonitor-") || !fname.ends_with(".js") {
            continue;
        }
        let orig_bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
        let new_bytes = fs::read(frontend_dir.join("assets").join(&fname)).map_err(|e| e.to_string())?;
        if orig_bytes != new_bytes {
            extra_patched_assets.push(fname);
        }
    }

    let manifest = json!({
        "shared": shared_name,
        "main": main_name,
        "panel": panel_name,
        "settingsSymbol": settings.variable,
        "atlasSymbol": atlas_component.variable,
        "base": base_dir.to_string_lossy(),
        "version": official_version,
    });
    fs::write(
        build_root.join("build_manifest_v2.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(IntegrationResult {
        manifest,
        patch_log: rec.entries,
        shared_name,
        main_name,
        panel_name,
        extra_patched_assets,
        base_dir,
        frontend_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_asserts_anchor_count() {
        let mut rec = PatchRecorder::default();
        assert_eq!(rec.replace_n("a b a", "a", "x", 2).unwrap(), "x b x");
        assert!(rec.replace("a b a", "a", "x").is_err());
        assert!(rec.replace("a b", "zzz", "x").is_err());
    }

    #[test]
    fn apply_edits_keeps_earlier_offsets_stable() {
        let edits = vec![Edit::new(8, 10, "AB"), Edit::new(0, 1, "zero")];
        assert_eq!(apply_edits("0123456789", edits).unwrap(), "zero1234567AB");
        assert!(apply_edits("abc", vec![Edit::new(0, 99, "x")]).is_err());
    }

    #[test]
    fn char_to_byte_map_handles_multibyte_chinese_and_astral_chars() {
        let text = "const 提示='中文😀'; function target() { return 1; }";
        let map = char_to_byte_map(text);
        let char_pos = text.chars().take_while(|&c| c != '{').count();
        let byte_pos = map_offset(&map, char_pos, "body").unwrap();
        assert_eq!(&text[byte_pos..byte_pos + 1], "{");
        let meta = vec![FunctionMeta {
            name: "target".into(),
            start: map[text.chars().count() - 28],
            end: text.len(),
            body: byte_pos,
            body_end: text.len(),
        }];
        assert_eq!(function_for(&meta, text, "return 1").unwrap().name, "target");
    }

    #[test]
    fn function_for_picks_innermost_and_named_requires_unique() {
        let functions = vec![
            FunctionMeta {
                name: "outer".into(),
                start: 0,
                end: 20,
                body: 5,
                body_end: 20,
            },
            FunctionMeta {
                name: "inner".into(),
                start: 5,
                end: 15,
                body: 10,
                body_end: 15,
            },
        ];
        let text = &"x".repeat(30);
        assert_eq!(function_for(&functions, text, "x").unwrap().name, "inner");
        assert!(function_for(&functions[..1], "abc", "zzz").is_err());
        assert_eq!(named(&functions, "outer").unwrap().end, 20);
        let dup = vec![functions[0].clone(), functions[0].clone()];
        assert!(named(&dup, "outer").is_err());
    }

    #[test]
    fn shared_patches_match_contract() {
        let mut rec = PatchRecorder::default();
        let url_out =
            patch_mod_asset_url(&mut rec, "function buildAssetUrl(e, t) {\n  return 1\n}")
                .unwrap();
        assert!(url_out.contains("__WUWA_MOD_ASSET_PATH__(t)"));
        assert!(url_out.contains("withAssetUrlCacheBust(modAssetPath)"));
        assert!(patch_mod_asset_url(&mut rec, "function buildAssetUrl(e) {\n}").is_err());

        let name_out = patch_mod_display_name(&mut rec, MOD_NAME_FALLBACK_OLD).unwrap();
        assert!(name_out.contains("__WUWA_MOD_NAMES__"));
        assert!(!name_out.contains("return s === \"character_skin\""));

        let online_out = patch_online_services(
            &mut rec,
            "loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();",
        )
        .unwrap();
        assert!(online_out.contains("function localStartOnlineServices() {"));
        assert!(online_out.contains("if (localOnlineServicesStarted) return;"));

        let inv_out = patch_invoke_guard(
            &mut rec,
            "async function invoke$1(e, t = {}, n) {\n}\n",
            false,
        )
        .unwrap();
        assert!(inv_out.contains("checkInvoke(e, t)"));
        assert!(patch_invoke_guard(&mut rec, "no wrapper here", true).is_err());
        assert_eq!(
            patch_invoke_guard(&mut rec, "no wrapper here", false).unwrap(),
            "no wrapper here"
        );

        let guard_src = "async function ensureAppearanceAssetCatalog(e) {\n}\nfunction applyCatalog(e, t, n) {\n}\nfunction atlas() {\n}\nthis.instance.interceptors.request.use(t => {\n";
        let guard_out = patch_atlas_guards(&mut rec, guard_src, "\n PLATFORM_EXTRA").unwrap();
        assert!(guard_out.contains("api/common/activity-config"));
        assert!(guard_out.contains("PLATFORM_EXTRA"));
        assert!(guard_out.contains("offlineError()"));
        assert_eq!(
            guard_out
                .match_indices("this.instance.interceptors.request.use")
                .count(),
            1
        );

        let taskbar_out = patch_taskbar_default(
            &mut rec,
            "const defaults = { showTaskbarIcon: !0 };\nfunction clampPetScale(e) {}",
        )
        .unwrap();
        assert!(taskbar_out.contains("showTaskbarIcon: !1"));
        assert!(taskbar_out.contains("wuwatappo-settings-community-taskbar-v1"));

        for invoke_fn in ["invoke", "invoke$1"] {
            let ver_src = format!(
                "try {{ cachedAppVersion = await getVersion() }} catch {{}}\nasync function getVersion() {{ return {invoke_fn}(\"plugin:app|version\") }}\n"
            );
            let ver_out = patch_display_version(&ver_src, "1.3.2-beta.2", "1.0.4").unwrap();
            assert!(ver_out.contains("cachedAppVersion = \"1.0.4\""));
            assert!(ver_out.contains("c + \" · 基底 \" + b"));
            assert!(ver_out.contains("\"/community/version\""));
            assert!(!ver_out.contains("plugin:app|version"));
        }
    }

    #[test]
    fn render_frontend_module_substitutes_platform_placeholders() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let mac_da = render_frontend_module(repo, "desktop_assistant.js", Platform::Mac).unwrap();
        let win_da =
            render_frontend_module(repo, "desktop_assistant.js", Platform::Windows).unwrap();
        assert!(mac_da.contains("macOS input queue"));
        assert!(win_da.contains("Windows input queue"));
        assert!(!mac_da.contains("__WUWA_ASSISTANT_DEFAULTS__"));

        let mac_dash =
            render_frontend_module(repo, "assistant_dashboard.js", Platform::Mac).unwrap();
        let win_dash =
            render_frontend_module(repo, "assistant_dashboard.js", Platform::Windows).unwrap();
        assert!(mac_dash.contains("WUWA TAPPO FULLATLAS / MAC"));
        assert!(win_dash.contains("WUWA TAPPO / 后台助手"));
    }

    #[test]
    fn patch_log_records_every_hit() {
        let mut rec = PatchRecorder::default();
        assert_eq!(rec.replace_n("a b a", "a", "x", 2).unwrap(), "x b x");
        assert_eq!(rec.entries.len(), 1);
        assert_eq!(rec.entries[0].matches, 2);
        assert_eq!(rec.entries[0].anchor, "a");
    }
}
