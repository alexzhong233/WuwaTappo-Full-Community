use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use wuwatappo_assistant_core::AssistantCore;

fn main() {
    let stdin = io::stdin();
    let mut core = AssistantCore::new(None);
    for line in stdin.lock().lines() {
        let line = match line { Ok(line) => line, Err(error) => { eprintln!("{error}"); break; } };
        if line.trim().is_empty() { continue; }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => { println!("{}", json!({"error":error.to_string()})); continue; }
        };
        if let Some(saved) = request.get("state") {
            core = AssistantCore::new(Some(saved.clone()));
        }
        let action = request.get("action").and_then(Value::as_str).unwrap_or("state");
        let body = request.get("body").cloned().unwrap_or_else(|| json!({}));
        let now = request.get("now").and_then(Value::as_f64).unwrap_or(0.0);
        let quiet = request.get("quiet").and_then(Value::as_bool).unwrap_or(false);
        let offline = request.get("offline").and_then(Value::as_bool).unwrap_or(false);
        let update_required = request.get("update_required").and_then(Value::as_bool).unwrap_or(false);
        let response = match core.handle(action, &body, now, quiet, offline, update_required) {
            Ok(result) => json!({"result":result,"state":core.saved()}),
            Err(error) => json!({"error":error,"state":core.saved()}),
        };
        println!("{}", response);
        let _ = io::stdout().flush();
    }
}
