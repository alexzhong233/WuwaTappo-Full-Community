//! Cross-platform process detection for automatic game mode.
use serde_json::{json, Value};
use std::process::Command;

const GAME_PROCESSES: &[&str] = &[
    "wutheringwaves.exe",
    "wuthering waves.exe",
    "client-win64-shipping.exe",
    "wutheringwaves",
    "wuthering waves",
    "client-win64-shipping",
];

fn matches_process(name: &str) -> bool {
    let name = name.trim().trim_matches('"').to_ascii_lowercase();
    GAME_PROCESSES.iter().any(|candidate| name == *candidate)
}

#[cfg(windows)]
fn process_names() -> Vec<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.split(',').next())
                .map(|name| name.trim().trim_matches('"').to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn process_names() -> Vec<String> {
    Command::new("ps")
        .args(["-ax", "-o", "comm="])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().rsplit('/').next())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn status() -> Value {
    let processes: Vec<String> = process_names()
        .into_iter()
        .filter(|name| matches_process(name))
        .collect();
    json!({"running": !processes.is_empty(), "processes": processes})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_matching_is_case_insensitive_and_exact() {
        assert!(matches_process("Client-Win64-Shipping.exe"));
        assert!(matches_process("Wuthering Waves.exe"));
        assert!(!matches_process("not-WutheringWaves.exe"));
    }
}
