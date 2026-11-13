use chrono::{Duration, Local, NaiveDate, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;

pub mod custom_resources;
pub mod release_updates;
pub mod resource_cache;

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
    pub remember_monitor: bool,
    pub prerelease_updates: bool,
    pub auto_emote: bool,
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
            remember_monitor: values["remember_monitor"].as_bool().unwrap(),
            prerelease_updates: values["prerelease_updates"].as_bool().unwrap(),
            auto_emote: values["auto_emote"].as_bool().unwrap(),
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
        if action == "typing-context" {
            return Ok(json!({"editable": false, "reason": "native-adapter"}));
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
                    self.runtime = json!({"total":total,"balance":body.get("balance"),"logged_in":body.get("logged_in")==Some(&Value::Bool(true)),"local_atlas":offline,"chests":body.get("chests"),"quiet":quiet,"connected":true,"seen_at":now});
                }
                if action == "tick" {
                    let allowed = !update_required
                        && settings["enabled"] == true
                        && quiet
                        && now >= self.next_tick;
                    if allowed {
                        let speed = settings["speed"].as_f64().unwrap_or(5.0);
                        self.next_tick = now + 1.0 / speed;
                    }
                    let open = std::mem::replace(&mut self.open_settings, Value::Bool(false));
                    let quit = std::mem::replace(&mut self.quit_requested, false);
                    return Ok(
                        json!({"allowed":allowed,"quiet":quiet,"open_settings":open,"quit":quit,"flush_generation":if self.flush_generation>self.flush_done{self.flush_generation}else{0},"settings":settings,"session":self.state["session"],"confirmed":self.state["confirmed"],"update_required":update_required}),
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
                if offline {
                    return Err("本地模式不能进入联机".into());
                }
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
                if new_visit {
                    self.state["deleted_rooms"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|x| x.as_str() != Some(code));
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
                self.state["rooms"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|r| r["code"] != code);
                let deleted = self.state["deleted_rooms"].as_array_mut().unwrap();
                deleted.retain(|x| x.as_str() != Some(&code));
                deleted.push(json!(code));
                if deleted.len() > 100 {
                    deleted.remove(0);
                }
            }
            "favorite" => {
                let code = body.get("code");
                for room in self.state["rooms"].as_array_mut().unwrap() {
                    if room.get("code") == code {
                        room["favorite"] = json!(body.get("favorite") == Some(&Value::Bool(true)));
                    }
                }
                self.state["rooms"]
                    .as_array_mut()
                    .unwrap()
                    .sort_by_key(|r| {
                        (
                            r["favorite"] != true,
                            Reverse(r["visited"].as_u64().unwrap_or(0)),
                        )
                    });
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
