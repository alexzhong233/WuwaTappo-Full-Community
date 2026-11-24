use chrono::{Duration, Local, NaiveDate, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub mod custom_resources;
pub mod demo_packs;
pub mod injector;
pub mod lnk;
pub mod pe_repack;
pub mod regions;
pub mod release_installer;
pub mod release_updates;
pub mod resource_cache;
pub mod service;

pub const DEFAULTS: &str = include_str!("../../contracts/assistant-settings.json");

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct AssistantSettings {
    pub enabled: bool,
    pub speed: u32,
    pub auto_chest: bool,
    pub threshold: u64,
    pub auto_click_balance: bool,
    pub auto_click_below: u64,
    pub auto_click_above: u64,
    pub prerelease_updates: bool,
    pub auto_emote: bool,
    pub auto_emote_interval: u32,
    pub auto_emote_random: bool,
    pub auto_emote_slots: u32,
    pub emote_hotkeys: bool,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        let values = defaults();
        Self {
            enabled: values["enabled"].as_bool().unwrap(),
            speed: values["speed"].as_u64().unwrap() as u32,
            auto_chest: values["auto_chest"].as_bool().unwrap(),
            threshold: values["threshold"].as_u64().unwrap(),
            auto_click_balance: values["auto_click_balance"].as_bool().unwrap(),
            auto_click_below: values["auto_click_below"].as_u64().unwrap(),
            auto_click_above: values["auto_click_above"].as_u64().unwrap(),
            prerelease_updates: values["prerelease_updates"].as_bool().unwrap(),
            auto_emote: values["auto_emote"].as_bool().unwrap(),
            emote_hotkeys: values["emote_hotkeys"].as_bool().unwrap(),
            auto_emote_interval: values["auto_emote_interval"].as_u64().unwrap() as u32,
            auto_emote_random: values["auto_emote_random"].as_bool().unwrap(),
            auto_emote_slots: values["auto_emote_slots"].as_u64().unwrap() as u32,
        }
    }
}

impl AssistantSettings {
    pub fn apply(&mut self, changes: &Map<String, Value>) -> Result<(), String> {
        let mut values = serde_json::to_value(&*self)
            .map_err(|error| format!("设置序列化失败：{error}"))?
            .as_object()
            .cloned()
            .ok_or("设置结构无效")?;
        for (key, value) in changes {
            setting_error(key, value)?;
            values.insert(key.clone(), value.clone());
        }
        let next: Self = serde_json::from_value(Value::Object(values))
            .map_err(|error| format!("设置值无效：{error}"))?;
        if next.auto_click_below >= next.auto_click_above {
            return Err("自动点击下限必须低于上限".into());
        }
        *self = next;
        Ok(())
    }
}

fn defaults() -> Map<String, Value> {
    serde_json::from_str::<Value>(DEFAULTS)
        .expect("assistant settings contract is valid")
        .get("defaults")
        .and_then(Value::as_object)
        .cloned()
        .expect("assistant settings contract has defaults")
}

fn now_day(now: f64) -> NaiveDate {
    chrono::DateTime::<Utc>::from_timestamp(now.max(0.0) as i64, 0)
        .unwrap_or_else(Utc::now)
        .with_timezone(&Local)
        .date_naive()
}

fn json_size(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}

fn room_code(value: Option<&Value>) -> Option<&str> {
    let code = value?.as_str()?;
    if (1..=32).contains(&code.len())
        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Some(code)
    } else {
        None
    }
}

fn hash_scope(scope: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(scope.as_bytes());
    digest
        .finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn settings_from(value: Option<&Value>) -> Map<String, Value> {
    let mut result = defaults();
    if let Some(object) = value.and_then(Value::as_object) {
        for (key, value) in object {
            if result.contains_key(key) {
                result.insert(key.clone(), value.clone());
            }
        }
    }
    result
}

fn setting_error(key: &str, value: &Value) -> Result<(), String> {
    let properties =
        serde_json::from_str::<Value>(DEFAULTS).map_err(|e| e.to_string())?["properties"].clone();
    let rule = properties
        .get(key)
        .ok_or_else(|| format!("未知设置：{key}"))?;
    match rule.get("type").and_then(Value::as_str) {
        Some("boolean") if !value.is_boolean() => return Err(format!("{key} 必须是布尔值")),
        Some("integer") if value.as_i64().is_none() || value.as_i64().unwrap() < 0 => {
            return Err(format!("{key} 必须是非负整数"))
        }
        _ => {}
    }
    if let Some(number) = value.as_i64() {
        if let Some(min) = rule.get("minimum").and_then(Value::as_i64) {
            if number < min {
                return Err(format!("{key} 小于允许范围"));
            }
        }
        if let Some(max) = rule.get("maximum").and_then(Value::as_i64) {
            if number > max {
                return Err(format!("{key} 超出允许范围"));
            }
        }
        if let Some(enums) = rule.get("enum").and_then(Value::as_array) {
            if !enums.iter().any(|item| item.as_i64() == Some(number)) {
                return Err(format!("{key} 不是允许的选项"));
            }
        }
    }
    Ok(())
}

pub struct AssistantCore {
    state: Value,
    runtime: Value,
    next_tick: f64,
    open_settings: Value,
    quit_requested: bool,
    flush_generation: u64,
    flush_done: u64,
}

impl AssistantCore {
    pub fn new(saved: Option<Value>) -> Self {
        let mut state = json!({
            "schema": 2,
            "settings": defaults(),
            "session": 0,
            "confirmed": 0,
            "baselines": {},
            "days": {"normal": {}},
            "rooms": [],
            "deleted_rooms": [],
            "monitor": null,
            "rewards": [],
            "session_chests": 0
        });
        if let Some(saved) = saved {
            if let (Some(target), Some(source)) = (state.as_object_mut(), saved.as_object()) {
                for (key, value) in source {
                    target.insert(key.clone(), value.clone());
                }
                // Mac 1.x stored daily counters as a flat map and account
                // baselines as raw identifiers. Normalize those once on load.
                if source.get("schema").is_none()
                    && source
                        .get("days")
                        .and_then(Value::as_object)
                        .is_some_and(|days| !days.contains_key("normal"))
                {
                    let old_days = source.get("days").cloned().unwrap_or_else(|| json!({}));
                    target.insert("days".into(), json!({"normal": old_days}));
                    if let Some(baselines) = source.get("baselines").and_then(Value::as_object) {
                        let normalized: Map<String, Value> = baselines
                            .iter()
                            .map(|(account, value)| {
                                (hash_scope(&format!("normal:{account}")), value.clone())
                            })
                            .collect();
                        target.insert("baselines".into(), Value::Object(normalized));
                    }
                }
                target.insert(
                    "settings".into(),
                    Value::Object(settings_from(source.get("settings"))),
                );
            }
        }
        if let Some(object) = state.as_object_mut() {
            object.remove("local");
            object.insert("schema".into(), json!(2));
            object
                .entry("days")
                .or_insert_with(|| json!({"normal": {}}));
            object.entry("baselines").or_insert_with(|| json!({}));
            object.entry("deleted_rooms").or_insert_with(|| json!([]));
        }
        Self {
            state,
            runtime: json!({}),
            next_tick: 0.0,
            open_settings: Value::Bool(false),
            quit_requested: false,
            flush_generation: 0,
            flush_done: 0,
        }
    }

    pub fn saved(&self) -> Value {
        self.state.clone()
    }

    pub fn app_enabled(&self) -> bool {
        self.state
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_app_enabled(&mut self, enabled: bool) {
        self.state["enabled"] = json!(enabled);
    }

    pub fn snapshot(&self, now: f64, update_required: bool) -> Value {
        let mut result = self.state.clone();
        if let Some(object) = result.as_object_mut() {
            object.remove("baselines");
            object.remove("deleted_rooms");
            let today = now_day(now);
            let days = self
                .state
                .pointer("/days/normal")
                .and_then(Value::as_object);
            let weekly: Vec<Value> = (0..7)
                .rev()
                .map(|offset| {
                    let date = (today - Duration::days(offset)).to_string();
                    let count = days
                        .and_then(|v| v.get(&date))
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    json!({"date": date, "count": count})
                })
                .collect();
            object.remove("days");
            object.insert("weekly".into(), json!({"normal": weekly, "local": weekly}));
            object.insert("runtime".into(), self.runtime.clone());
            object.insert("flush_generation".into(), json!(self.flush_generation));
            object.insert("flush_done".into(), json!(self.flush_done));
            object.insert("update_required".into(), json!(update_required));
        }
        result
    }

    fn settings(&self) -> Map<String, Value> {
        settings_from(self.state.get("settings"))
    }

    pub fn handle(
        &mut self,
        action: &str,
        body: &Value,
        now: f64,
        quiet: bool,
        offline: bool,
        update_required: bool,
    ) -> Result<Value, String> {
        if matches!(action, "state" | "renderer-start") {
            if action == "renderer-start" {
                self.runtime = json!({"seen_at": now, "connected": true});
            }
            return Ok(self.snapshot(now, update_required));
        }
        let settings = self.settings();
        match action {
            "configure" => {
                let changes = body
                    .get("settings")
                    .and_then(Value::as_object)
                    .ok_or("设置必须是对象")?;
                let mut typed: AssistantSettings =
                    serde_json::from_value(Value::Object(settings.clone()))
                        .map_err(|error| format!("设置值无效：{error}"))?;
                typed.apply(changes)?;
                self.state["settings"] = serde_json::to_value(typed).map_err(|e| e.to_string())?;
            }
            "reset" => {
                self.state["session"] = json!(0);
                self.state["confirmed"] = json!(0);
                self.state["session_chests"] = json!(0);
            }
            "tick" | "observe" => {
                let sent = body.get("sent").and_then(Value::as_u64).unwrap_or(0);
                let confirmed = body.get("confirmed").and_then(Value::as_u64).unwrap_or(0);
                let balance_paused = body.get("balance_paused") == Some(&Value::Bool(true));
                if sent > 1 || confirmed > sent {
                    return Err("点击确认值无效".into());
                }
                self.state["session"] = json!(self.state["session"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(sent));
                self.state["confirmed"] = json!(self.state["confirmed"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(confirmed));
                if let Some(total) = body.get("total").and_then(Value::as_u64) {
                    let account = body
                        .get("account")
                        .and_then(Value::as_str)
                        .unwrap_or("guest");
                    if account.len() > 200 {
                        return Err("账号标识过长".into());
                    }
                    let key = hash_scope(&format!("normal:{account}"));
                    let old = self.state["baselines"].get(&key).and_then(Value::as_u64);
                    self.state["baselines"][key.clone()] = json!(total);
                    if let Some(before) = old.filter(|before| total >= *before) {
                        let date = now_day(now).to_string();
                        let count = self.state["days"]["normal"][&date].as_u64().unwrap_or(0);
                        self.state["days"]["normal"][date] = json!(count + total - before);
                    }
                    let cutoff = (now_day(now) - Duration::days(90)).to_string();
                    if let Some(days) = self.state["days"]["normal"].as_object_mut() {
                        days.retain(|date, _| date >= &cutoff);
                    }
                    if self.state["baselines"]
                        .as_object()
                        .map(|v| v.len())
                        .unwrap_or(0)
                        > 100
                    {
                        self.state["baselines"] = json!({key: total});
                    }
                    let prev_emote = self.runtime.get("emote").cloned();
                    self.runtime = json!({"total":total,"balance":body.get("balance"),"logged_in":body.get("logged_in")==Some(&Value::Bool(true)),"local_atlas":offline,"chests":body.get("chests"),"quiet":quiet,"connected":true,"seen_at":now});
                    if let Some(emote) = body
                        .get("emote")
                        .filter(|v| v.is_object())
                        .cloned()
                        .or(prev_emote)
                    {
                        self.runtime["emote"] = emote;
                    }
                } else if let Some(emote) = body.get("emote").filter(|v| v.is_object()) {
                    if !self.runtime.is_object() {
                        self.runtime = json!({});
                    }
                    self.runtime["emote"] = emote.clone();
                }
                self.runtime["balance_paused"] = json!(balance_paused);
                if action == "tick" {
                    let allowed = !update_required
                        && settings["enabled"] == true
                        && quiet
                        && !balance_paused
                        && now >= self.next_tick;
                    if allowed {
                        let speed = settings["speed"].as_f64().unwrap_or(5.0);
                        self.next_tick = now + 1.0 / speed;
                    }
                    let open = std::mem::replace(&mut self.open_settings, Value::Bool(false));
                    let quit = std::mem::replace(&mut self.quit_requested, false);
                    return Ok(
                        json!({"allowed":allowed,"quiet":quiet,"open_settings":open,"quit":quit,"flush_generation":if self.flush_generation>self.flush_done{self.flush_generation}else{0},"settings":settings,"session":self.state["session"],"confirmed":self.state["confirmed"],"update_required":update_required,"balance_paused":balance_paused}),
                    );
                }
            }
            "flush-request" => {
                self.flush_generation += 1;
                return Ok(json!({"flush_generation": self.flush_generation}));
            }
            "flush-ack" => {
                if let Some(generation) = body.get("generation").and_then(Value::as_u64) {
                    if generation >= self.flush_done && generation <= self.flush_generation {
                        self.flush_done = generation;
                    }
                }
            }
            "open-settings" => {
                let view = body.get("view").cloned().unwrap_or(Value::Bool(true));
                if !view.is_boolean()
                    && !matches!(
                        view.as_str(),
                        Some("settings" | "overview" | "rewards" | "daily" | "atlas")
                    )
                {
                    return Err("未知 view".into());
                }
                self.open_settings = view;
            }
            "quit" => self.quit_requested = true,
            "local-chest" => return Err("模拟开箱已移除，请使用原版账号开箱".into()),
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
                    || json_size(&Value::Object(reward.clone())) > 8192
                {
                    return Err("只保存原版服务器确认的奖励".into());
                }
                let rewards = self.state["rewards"].as_array_mut().ok_or("奖励状态损坏")?;
                if !rewards
                    .iter()
                    .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
                {
                    let mut record = reward.clone();
                    record.insert("time".into(), json!(now as u64));
                    record.insert("confirmed".into(), json!(true));
                    rewards.insert(0, Value::Object(record));
                    rewards.truncate(1000);
                    self.state["session_chests"] =
                        json!(self.state["session_chests"].as_u64().unwrap_or(0) + 1);
                }
            }
            "room" => {
                let code = room_code(body.get("code")).ok_or("房间号无效")?;
                let new_visit = body.get("new_visit") == Some(&Value::Bool(true));
                if self.state["deleted_rooms"]
                    .as_array()
                    .map(|v| v.iter().any(|x| x.as_str() == Some(code)))
                    .unwrap_or(false)
                    && !new_visit
                {
                    return Ok(self.snapshot(now, update_required));
                }
                if !self.state["rooms"].is_array() {
                    self.state["rooms"] = json!([]);
                }
                if !self.state["deleted_rooms"].is_array() {
                    self.state["deleted_rooms"] = json!([]);
                }
                if new_visit {
                    if let Some(deleted) = self.state["deleted_rooms"].as_array_mut() {
                        deleted.retain(|x| x.as_str() != Some(code));
                    }
                }
                let rooms = self.state["rooms"].as_array_mut().ok_or("房间状态损坏")?;
                let old = rooms.iter().find(|r| r["code"] == code).cloned();
                rooms.retain(|r| r["code"] != code);
                rooms.push(json!({"code":code,"host":body.get("host").and_then(Value::as_str).unwrap_or("").chars().take(100).collect::<String>(),"favorite":old.as_ref().map(|r| r["favorite"]==true).unwrap_or(false),"visited":if new_visit || old.is_none(){now as u64}else{old.unwrap()["visited"].as_u64().unwrap_or(now as u64)}}));
                rooms.sort_by_key(|r| {
                    (
                        r["favorite"] != true,
                        Reverse(r["visited"].as_u64().unwrap_or(0)),
                    )
                });
                rooms.truncate(100);
            }
            "delete-room" => {
                let code = room_code(body.get("code")).ok_or("房间号无效")?.to_string();
                if let Some(rooms) = self.state["rooms"].as_array_mut() {
                    rooms.retain(|r| r["code"] != code);
                }
                if !self.state["deleted_rooms"].is_array() {
                    self.state["deleted_rooms"] = json!([]);
                }
                if let Some(deleted) = self.state["deleted_rooms"].as_array_mut() {
                    deleted.retain(|x| x.as_str() != Some(&code));
                    deleted.push(json!(code));
                    if deleted.len() > 100 {
                        deleted.remove(0);
                    }
                }
            }
            "favorite" => {
                let code = body.get("code");
                if let Some(rooms) = self.state["rooms"].as_array_mut() {
                    for room in rooms.iter_mut() {
                        if room.get("code") == code {
                            room["favorite"] =
                                json!(body.get("favorite") == Some(&Value::Bool(true)));
                        }
                    }
                    rooms.sort_by_key(|r| {
                        (
                            r["favorite"] != true,
                            Reverse(r["visited"].as_u64().unwrap_or(0)),
                        )
                    });
                }
            }
            "monitor" => {
                let monitor = body.get("monitor").cloned().unwrap_or(Value::Null);
                if !monitor.is_null() && monitor.get("name").and_then(Value::as_str).is_none() {
                    return Err("显示器信息无效".into());
                }
                if json_size(&monitor) > 2048 {
                    return Err("显示器信息过大".into());
                }
                self.state["monitor"] = monitor;
            }
            _ => return Err(format!("未知助手操作：{action}")),
        }
        Ok(self.snapshot(now, update_required))
    }
}

const MAX_DIAGNOSTIC_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SYNTH_LOG_BYTES: u64 = 16 * 1024 * 1024;
const SYNTH_LOG_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
const REDACTED: &str = "[REDACTED]";
const DEFAULT_HOST_PORT: u64 = 43229;

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 生成 32 位十六进制随机令牌，供本机图片/控制路由鉴权使用。
pub fn random_token_hex() -> String {
    let mut bytes = [0u8; 16];
    let filled = {
        #[cfg(windows)]
        {
            const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
            #[link(name = "bcrypt")]
            unsafe extern "system" {
                fn BCryptGenRandom(
                    algorithm: *mut std::ffi::c_void,
                    buffer: *mut u8,
                    count: u32,
                    flags: u32,
                ) -> i32;
            }
            unsafe {
                BCryptGenRandom(
                    std::ptr::null_mut(),
                    bytes.as_mut_ptr(),
                    bytes.len() as u32,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                ) == 0
            }
        }
        #[cfg(not(windows))]
        {
            fs::File::open("/dev/urandom")
                .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
                .is_ok()
        }
    };
    if !filled {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let seed = format!(
            "{}-{}-{:?}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default(),
            std::thread::current().id(),
            seq
        );
        let digest = Sha256::digest(seed.as_bytes());
        bytes.copy_from_slice(&digest[..16]);
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 敏感凭据键名片段（小写比较），命中的值一律替换为 `[REDACTED]`。
const SENSITIVE_KEY_PARTS: [&str; 7] = [
    "token",
    "cookie",
    "authorization",
    "password",
    "secret",
    "x-local-token",
    "kuro_infotoken",
];

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEY_PARTS
        .iter()
        .any(|part| lower.contains(part))
}

/// 递归脱敏 JSON 中的凭据字段，供诊断与合成日志落盘前调用。
pub fn sanitize_diagnostic_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut result = Map::with_capacity(map.len());
            for (key, inner) in map {
                let next = if is_sensitive_key(key) {
                    Value::String(REDACTED.into())
                } else {
                    sanitize_diagnostic_value(inner)
                };
                result.insert(key.clone(), next);
            }
            Value::Object(result)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_diagnostic_value).collect()),
        other => other.clone(),
    }
}

/// 追加一条客户端诊断事件，超过 8 MiB 自动轮转为 `.jsonl.1`。
pub fn append_diagnostic(root: &Path, body: &Value) {
    append_diagnostic_with_limit(root, body, MAX_DIAGNOSTIC_BYTES);
}

static LOG_IO_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

fn append_diagnostic_with_limit(root: &Path, body: &Value, max_bytes: u64) {
    let _guard = LOG_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = fs::create_dir_all(root);
    let path = root.join("client_diagnostics.jsonl");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= max_bytes) {
        let rotated = root.join("client_diagnostics.jsonl.1");
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(&path, &rotated);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let line = json!({
        "time": unix_time_secs(),
        "event": sanitize_diagnostic_value(body),
    });
    if let Ok(text) = serde_json::to_string(&line) {
        let _ = file.write_all(format!("{text}\n").as_bytes());
    }
}

/// 追加一条合成日志，按天分文件并做体积与时间轮转。
pub fn append_synth_log(root: &Path, body: &Value) {
    append_synth_log_with_limit(root, body, MAX_SYNTH_LOG_BYTES);
}

fn synth_log_dir(root: &Path) -> PathBuf {
    root.join("logs")
}

/// 列出 `logs/synth-*.jsonl*`，按修改时间升序返回（同日 `.1` 视为更早）。
fn synth_log_files(dir: &Path) -> Vec<(SystemTime, String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("synth-") || !name.contains(".jsonl") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        files.push((modified, name.to_string(), path));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    files
}

fn prune_synth_logs(dir: &Path) {
    for (modified, _, path) in synth_log_files(dir) {
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        if age.as_secs() > SYNTH_LOG_RETENTION_SECS {
            let _ = fs::remove_file(path);
        }
    }
}

fn append_synth_log_with_limit(root: &Path, body: &Value, max_bytes: u64) {
    let _guard = LOG_IO_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = synth_log_dir(root);
    let _ = fs::create_dir_all(&dir);
    prune_synth_logs(&dir);
    let name = format!("synth-{}.jsonl", Local::now().format("%Y-%m-%d"));
    let path = dir.join(&name);
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= max_bytes) {
        let rotated = dir.join(format!("{name}.1"));
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(&path, &rotated);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let line = json!({
        "time": unix_time_secs(),
        "entry": sanitize_diagnostic_value(body),
    });
    if let Ok(text) = serde_json::to_string(&line) {
        let _ = file.write_all(format!("{text}\n").as_bytes());
    }
}

/// 读取最近的合成日志，最新条目在前，最多返回 `limit.clamp(1, 200)` 条。
pub fn read_recent_synth_logs(root: &Path, limit: usize) -> Vec<Value> {
    let limit = limit.clamp(1, 200);
    let dir = synth_log_dir(root);
    let mut entries = Vec::new();
    for (_, _, path) in synth_log_files(&dir) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                entries.push(value);
            }
        }
    }
    entries.reverse();
    entries.truncate(limit);
    entries
}

fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    let text = fs::read_to_string(path).ok()?;
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(&text);
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|value| value.as_object().cloned())
}

/// 读取或初始化本机运行配置：从 `package_config.json` 补齐公开字段并确保有效端口与随机令牌。
const SYNC_PACKAGE_KEYS: [&str; 7] = [
    "region",
    "label",
    "cdn_host",
    "api_base",
    "client_version",
    "mod_version",
    "community_version",
];

pub fn ensure_local_config(root: &Path) -> Result<Value, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let local_path = root.join("local_config.json");
    let mut changed = !local_path.exists();
    let mut map = read_json_object(&local_path).unwrap_or_default();
    let pkg = read_json_object(&root.join("package_config.json"));
    let pkg_port = pkg
        .as_ref()
        .and_then(|p| p.get("port"))
        .and_then(Value::as_u64)
        .filter(|port| (1..=65535).contains(port));
    if let Some(pkg) = pkg {
        for (key, value) in pkg {
            if key == "token" || key == "port" {
                continue;
            }
            let sync = SYNC_PACKAGE_KEYS.contains(&key.as_str());
            if !map.contains_key(&key) || (sync && map.get(&key) != Some(&value)) {
                map.insert(key, value);
                changed = true;
            }
        }
    }
    let valid_port = map
        .get("port")
        .and_then(Value::as_u64)
        .is_some_and(|port| (1..=65535).contains(&port));
    if !valid_port {
        map.insert("port".into(), json!(pkg_port.unwrap_or(DEFAULT_HOST_PORT)));
        changed = true;
    }
    let valid_token = map
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|token| token.len() >= 32);
    if !valid_token {
        map.insert("token".into(), json!(random_token_hex()));
        changed = true;
    }
    let config = Value::Object(map);
    if changed {
        let temp = local_path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
        fs::write(&temp, bytes).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&temp, &local_path).map_err(|error| error.to_string())?;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wuwatappo-core-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn shared_core_keeps_settings_and_daily_counts() {
        let mut core = AssistantCore::new(None);
        core.handle(
            "configure",
            &json!({"settings":{"enabled":true,"speed":10}}),
            1.0,
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(
            core.handle("tick", &json!({}), 1.0, true, false, false)
                .unwrap()["allowed"],
            true
        );
        assert_eq!(
            core.handle("tick", &json!({}), 1.0, true, false, false)
                .unwrap()["allowed"],
            false
        );
        core.handle(
            "observe",
            &json!({"total":100,"account":"a"}),
            1.0,
            true,
            false,
            false,
        )
        .unwrap();
        let value = core
            .handle(
                "observe",
                &json!({"total":105,"account":"a"}),
                2.0,
                true,
                false,
                false,
            )
            .unwrap();
        assert_eq!(
            value["weekly"]["normal"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["count"],
            5
        );
    }

    #[test]
    fn random_token_hex_generates_distinct_32_char_hex_strings() {
        let first = random_token_hex();
        let second = random_token_hex();
        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        assert!(first.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert_ne!(first, second);
    }

    #[test]
    fn diagnostic_log_records_timestamp_and_rotates_on_limit() {
        let root = temp_root("diag");
        append_diagnostic_with_limit(&root, &json!({"kind":"first"}), 32);
        append_diagnostic_with_limit(&root, &json!({"kind":"second"}), 32);
        let active = fs::read_to_string(root.join("client_diagnostics.jsonl")).unwrap();
        let rotated = fs::read_to_string(root.join("client_diagnostics.jsonl.1")).unwrap();
        assert!(rotated.contains("\"first\""));
        assert!(active.contains("\"second\""));
        let parsed: Value = serde_json::from_str(active.lines().next().unwrap()).unwrap();
        assert!(parsed.get("time").and_then(Value::as_u64).is_some());
        assert_eq!(parsed["event"]["kind"], "second");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_local_config_merges_package_config_and_generates_token() {
        let root = temp_root("config");
        fs::write(
            root.join("package_config.json"),
            "\u{feff}{\"region\":\"cn\",\"community_version\":\"1.3.2\",\"token\":\"leaked\"}",
        )
        .unwrap();
        let created = ensure_local_config(&root).unwrap();
        assert_eq!(created["region"], "cn");
        assert_eq!(created["community_version"], "1.3.2");
        assert_eq!(created["port"], DEFAULT_HOST_PORT);
        let token = created["token"].as_str().unwrap().to_owned();
        assert_eq!(token.len(), 32);
        assert_ne!(token, "leaked");

        let reloaded = ensure_local_config(&root).unwrap();
        assert_eq!(reloaded["token"], token);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_local_config_refreshes_version_fields_without_clobbering_token_or_port() {
        let root = temp_root("config-upgrade");
        fs::write(
            root.join("local_config.json"),
            json!({
                "region": "cn",
                "community_version": "1.3.1",
                "mod_version": "1.3.1",
                "client_version": "1.0.3",
                "port": 49123,
                "token": "0123456789abcdef0123456789abcdef",
                "user_custom": "keep"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            root.join("package_config.json"),
            json!({
                "region": "cn",
                "community_version": "1.3.2-beta.2",
                "mod_version": "1.3.2-beta.2",
                "client_version": "1.0.4",
                "port": 51731,
                "token": "ffffffffffffffffffffffffffffffff"
            })
            .to_string(),
        )
        .unwrap();
        let upgraded = ensure_local_config(&root).unwrap();
        assert_eq!(upgraded["community_version"], "1.3.2-beta.2");
        assert_eq!(upgraded["mod_version"], "1.3.2-beta.2");
        assert_eq!(upgraded["client_version"], "1.0.4");
        assert_eq!(upgraded["port"], 49123);
        assert_eq!(upgraded["token"], "0123456789abcdef0123456789abcdef");
        assert_eq!(upgraded["user_custom"], "keep");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn assistant_persists_emote_stats_even_without_total_observation() {
        let mut core = AssistantCore::new(None);
        core.handle(
            "tick",
            &json!({"emote": {"sent": 3, "reason": "ok"}}),
            1.0,
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(core.snapshot(2.0, false)["runtime"]["emote"]["sent"], 3);
        assert_eq!(
            core.snapshot(2.0, false)["runtime"]["emote"]["reason"],
            "ok"
        );
        core.handle(
            "observe",
            &json!({"total": 10, "account": "a"}),
            3.0,
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(core.snapshot(4.0, false)["runtime"]["emote"]["sent"], 3);
    }

    #[test]
    fn auto_emote_interval_and_slots_validate_bounds_and_fallback_defaults() {
        let mut core = AssistantCore::new(Some(json!({
            "settings": { "auto_emote": true }
        })));
        let initial = core.snapshot(1.0, false);
        assert_eq!(initial["settings"]["auto_emote"], true);
        assert_eq!(initial["settings"]["auto_emote_interval"], 25);
        assert_eq!(initial["settings"]["auto_emote_random"], true);
        assert_eq!(initial["settings"]["auto_emote_slots"], 31);

        assert!(core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_interval": 0}}),
                1.0,
                true,
                false,
                false,
            )
            .is_err());
        assert!(core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_interval": 1001}}),
                1.0,
                true,
                false,
                false,
            )
            .is_err());
        assert!(core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_slots": 0}}),
                1.0,
                true,
                false,
                false,
            )
            .is_err());
        assert!(core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_slots": 32}}),
                1.0,
                true,
                false,
                false,
            )
            .is_err());

        let updated = core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_interval": 40, "auto_emote_random": false, "auto_emote_slots": 6}}),
                2.0,
                true,
                false,
                false,
            )
            .unwrap();
        assert_eq!(updated["settings"]["auto_emote_interval"], 40);
        assert_eq!(updated["settings"]["auto_emote_random"], false);
        assert_eq!(updated["settings"]["auto_emote_slots"], 6);
        assert!(core
            .handle(
                "configure",
                &json!({"settings": {"auto_emote_random": "no"}}),
                2.0,
                true,
                false,
                false,
            )
            .is_err());
    }

    #[test]
    fn balance_pause_pauses_ticks_without_disabling_master_switch() {
        let mut core = AssistantCore::new(None);
        core.handle(
            "configure",
            &json!({"settings": {"enabled": true, "speed": 10}}),
            1.0,
            true,
            false,
            false,
        )
        .unwrap();
        let paused = core
            .handle(
                "tick",
                &json!({"balance_paused": true}),
                1.0,
                true,
                false,
                false,
            )
            .unwrap();
        assert_eq!(paused["allowed"], false);
        assert_eq!(paused["balance_paused"], true);
        assert_eq!(paused["settings"]["enabled"], true);
        assert_eq!(core.snapshot(2.0, false)["runtime"]["balance_paused"], true);
        assert_eq!(core.snapshot(2.0, false)["settings"]["enabled"], true);

        core.handle(
            "observe",
            &json!({"balance_paused": true}),
            3.0,
            true,
            false,
            false,
        )
        .unwrap();
        assert_eq!(core.snapshot(4.0, false)["runtime"]["balance_paused"], true);

        let resumed = core
            .handle(
                "tick",
                &json!({"balance_paused": false}),
                1.0,
                true,
                false,
                false,
            )
            .unwrap();
        assert_eq!(resumed["allowed"], true);
        assert_eq!(resumed["balance_paused"], false);
        assert_eq!(
            core.snapshot(2.0, false)["runtime"]["balance_paused"],
            false
        );
        assert_eq!(core.snapshot(2.0, false)["settings"]["enabled"], true);
    }

    #[test]
    fn offline_mode_allows_room_history_recording() {
        let mut core = AssistantCore::new(None);
        let value = core
            .handle(
                "room",
                &json!({"code": "abc123", "host": "tester"}),
                10.0,
                true,
                true,
                false,
            )
            .unwrap();
        let rooms = value["rooms"].as_array().unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0]["code"], "abc123");
        assert_eq!(rooms[0]["host"], "tester");
        assert_eq!(rooms[0]["visited"], 10);
    }

    #[test]
    fn sanitize_diagnostic_value_redacts_nested_credentials() {
        let value = json!({
            "token": "abc",
            "Cookie": "session=1",
            "headers": {"Authorization": "Bearer secret-value", "Accept": "json"},
            "nested": [{"password": "p"}, {"x-local-token": "x"}, {"kuro_infotoken": "k"}],
            "keep": "value"
        });
        let sanitized = sanitize_diagnostic_value(&value);
        assert_eq!(sanitized["token"], REDACTED);
        assert_eq!(sanitized["Cookie"], REDACTED);
        assert_eq!(sanitized["headers"]["Authorization"], REDACTED);
        assert_eq!(sanitized["headers"]["Accept"], "json");
        assert_eq!(sanitized["nested"][0]["password"], REDACTED);
        assert_eq!(sanitized["nested"][1]["x-local-token"], REDACTED);
        assert_eq!(sanitized["nested"][2]["kuro_infotoken"], REDACTED);
        assert_eq!(sanitized["keep"], "value");
        assert!(!sanitized.to_string().contains("Bearer secret-value"));
    }

    #[test]
    fn diagnostic_log_never_writes_unredacted_credentials() {
        let root = temp_root("diag-redact");
        append_diagnostic(&root, &json!({"token": "leaked-token", "note": "ok"}));
        let text = fs::read_to_string(root.join("client_diagnostics.jsonl")).unwrap();
        assert!(!text.contains("leaked-token"));
        assert!(text.contains(REDACTED));
        assert!(text.contains("\"note\":\"ok\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn synth_log_round_trips_sanitized_entries_newest_first() {
        let root = temp_root("synth");
        append_synth_log(&root, &json!({"step": "first", "token": "leaked-token"}));
        append_synth_log(&root, &json!({"step": "second"}));

        let dir = root.join("logs");
        let files = synth_log_files(&dir);
        assert_eq!(files.len(), 1);
        assert!(files[0].1.starts_with("synth-") && files[0].1.ends_with(".jsonl"));
        let text = fs::read_to_string(&files[0].2).unwrap();
        assert!(!text.contains("leaked-token"));
        assert_eq!(text.lines().count(), 2);

        let entries = read_recent_synth_logs(&root, 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["entry"]["step"], "second");
        assert_eq!(entries[1]["entry"]["step"], "first");
        assert_eq!(entries[1]["entry"]["token"], REDACTED);
        assert!(entries[0]["time"].as_u64().is_some());
        assert_eq!(read_recent_synth_logs(&root, 1).len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn synth_log_rotates_to_dot_one_at_size_limit() {
        let root = temp_root("synth-rotate");
        append_synth_log_with_limit(&root, &json!({"step": "first"}), 32);
        append_synth_log_with_limit(&root, &json!({"step": "second"}), 32);

        let dir = root.join("logs");
        let name = format!("synth-{}.jsonl", Local::now().format("%Y-%m-%d"));
        let active = fs::read_to_string(dir.join(&name)).unwrap();
        let rotated = fs::read_to_string(dir.join(format!("{name}.1"))).unwrap();
        assert!(rotated.contains("\"first\""));
        assert!(active.contains("\"second\""));
        assert_eq!(active.lines().count(), 1);
        let parsed: Value = serde_json::from_str(active.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["entry"]["step"], "second");
        assert!(parsed["time"].as_u64().is_some());

        let entries = read_recent_synth_logs(&root, 5);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["entry"]["step"], "second");
        assert_eq!(entries[1]["entry"]["step"], "first");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ensure_local_config_port_fallback_and_corrupted_rooms_recovery() {
        let root = temp_root("cfg-rooms");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("package_config.json"),
            r#"{"region":"cn","port":45123,"asset_count":99}"#,
        )
        .unwrap();
        let cfg = ensure_local_config(&root).unwrap();
        assert_eq!(cfg["port"], 45123);

        fs::write(root.join("local_config.json"), r#"{"port":0}"#).unwrap();
        fs::write(root.join("package_config.json"), r#"{"region":"cn"}"#).unwrap();
        let defaulted = ensure_local_config(&root).unwrap();
        assert_eq!(defaulted["port"], 43229);

        let mut core = AssistantCore::new(None);
        core.state["rooms"] = json!(null);
        core.state["deleted_rooms"] = json!({"bad": 1});
        core.handle(
            "room",
            &json!({"code":"123456","host":"p","new_visit":true}),
            100.0,
            true,
            true,
            false,
        )
        .unwrap();
        assert_eq!(core.state["rooms"].as_array().unwrap().len(), 1);
        core.handle("delete-room", &json!({"code":"123456"}), 101.0, true, true, false)
            .unwrap();
        assert_eq!(core.state["rooms"].as_array().unwrap().len(), 0);
        assert_eq!(core.state["deleted_rooms"][0], "123456");
        let _ = fs::remove_dir_all(root);
    }
}
