//! Cross-platform process detection for automatic game mode.
use serde_json::{json, Value};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const GAME_PROCESSES: &[&str] = &[
    "wutheringwaves.exe",
    "wuthering waves.exe",
    "client-win64-shipping.exe",
    "wutheringwaves",
    "wuthering waves",
    "client-win64-shipping",
];

const STATUS_TTL: Duration = Duration::from_secs(2);
static STATUS_CACHE: OnceLock<Mutex<Option<(Instant, Value)>>> = OnceLock::new();

fn matches_process(name: &str) -> bool {
    let name = name.trim().trim_matches('"').to_ascii_lowercase();
    GAME_PROCESSES.iter().any(|candidate| name == *candidate)
}

fn name_from_path(raw: &str) -> &str {
    let trimmed = raw.trim().trim_matches('"');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

#[cfg(target_os = "macos")]
fn matching_processes() -> Vec<String> {
    use std::ffi::c_void;
    const PROC_ALL_PIDS: u32 = 1;
    unsafe extern "C" {
        fn proc_listpids(kind: u32, typeinfo: u32, buffer: *mut c_void, buffersize: i32) -> i32;
        fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
        fn proc_name(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
    }
    let bytes = unsafe { proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if bytes <= 0 {
        return Vec::new();
    }
    let cap = (bytes as usize / std::mem::size_of::<i32>()).saturating_add(64);
    let mut pids = vec![0i32; cap];
    let actual = unsafe {
        proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if actual <= 0 {
        return Vec::new();
    }
    let count = (actual as usize / std::mem::size_of::<i32>()).min(pids.len());
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    for &pid in &pids[..count] {
        if pid <= 0 {
            continue;
        }
        let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        let len = if len > 0 {
            len
        } else {
            unsafe { proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) }
        };
        if len <= 0 {
            continue;
        }
        let slice = &buf[..(len as usize).min(buf.len())];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        if let Ok(raw) = std::str::from_utf8(&slice[..end]) {
            let name = name_from_path(raw);
            if matches_process(name) {
                out.push(name.to_owned());
            }
        }
    }
    out
}

#[cfg(windows)]
fn matching_processes() -> Vec<String> {
    use std::ffi::c_void;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: *mut c_void = (-1isize) as *mut c_void;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, th32_process_id: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    struct Snapshot(*mut c_void);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let _snapshot = Snapshot(handle);
    let mut entry = ProcessEntry32W {
        dw_size: std::mem::size_of::<ProcessEntry32W>() as u32,
        cnt_usage: 0,
        th32_process_id: 0,
        th32_default_heap_id: 0,
        th32_module_id: 0,
        cnt_threads: 0,
        th32_parent_process_id: 0,
        pc_pri_class_base: 0,
        dw_flags: 0,
        sz_exe_file: [0; 260],
    };
    let mut out = Vec::new();
    if unsafe { Process32FirstW(handle, &mut entry) } != 0 {
        loop {
            let len = entry
                .sz_exe_file
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.sz_exe_file.len());
            let name = String::from_utf16_lossy(&entry.sz_exe_file[..len]);
            if matches_process(&name) {
                out.push(name);
            }
            if unsafe { Process32NextW(handle, &mut entry) } == 0 {
                break;
            }
        }
    }
    out
}

#[cfg(not(any(windows, target_os = "macos")))]
fn matching_processes() -> Vec<String> {
    std::process::Command::new("ps")
        .args(["-ax", "-o", "comm="])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(name_from_path)
                .filter(|name| matches_process(name))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn status() -> Value {
    let cache = STATUS_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, cached)) = guard.as_ref() {
            if at.elapsed() < STATUS_TTL {
                return cached.clone();
            }
        }
    }
    let processes = matching_processes();
    let value = json!({"running": !processes.is_empty(), "processes": processes});
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), value.clone()));
    }
    value
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

    #[test]
    fn path_basename_extraction_handles_unix_and_quoted_paths() {
        assert_eq!(
            name_from_path("/Applications/Wuthering Waves.app/Contents/MacOS/Client-Win64-Shipping"),
            "Client-Win64-Shipping"
        );
        assert_eq!(name_from_path(" \"WutheringWaves\" "), "WutheringWaves");
        assert!(matches_process(name_from_path(
            "/Games/Wuthering Waves/WutheringWaves.exe"
        )));
    }

    #[test]
    fn status_returns_boolean_and_process_list() {
        let value = status();
        assert!(value["running"].is_boolean());
        assert!(value["processes"].is_array());
    }
}
