#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
use serde_json::Value;
#[cfg(windows)]
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::windows::{ffi::OsStrExt, process::CommandExt},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: isize,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[cfg(windows)]
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn root_dir() -> Result<std::path::PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "无法确定程序目录".into())
}

#[cfg(windows)]
fn ready(address: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(700)));
    if write!(stream, "GET /status HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok() && response.starts_with("HTTP/1.1 200")
}

#[cfg(windows)]
fn show_error(message: &str) {
    let text = wide(OsStr::new(message));
    let title = wide(OsStr::new("WuwaTappo 全图鉴版"));
    unsafe {
        MessageBoxW(0, text.as_ptr(), title.as_ptr(), 0x10);
    }
}

#[cfg(windows)]
fn elevate_app(root: &std::path::Path, app: &std::path::Path) -> Result<(), String> {
    let verb = wide(OsStr::new("runas"));
    let file = wide(app.as_os_str());
    let directory = wide(root.as_os_str());
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            directory.as_ptr(),
            1,
        )
    };
    if result <= 32 {
        Err(format!("无法启动桌宠（系统错误 {result}）"))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    let root = root_dir()?;
    let config: Value = serde_json::from_slice(
        &fs::read(root.join("local_config.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let port = config
        .get("port")
        .and_then(Value::as_u64)
        .ok_or("本地服务端口无效")? as u16;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    if !ready(address) {
        let host = root.join("LocalGalleryHost.exe");
        Command::new(&host)
            .current_dir(&root)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| format!("无法启动本地服务：{error}"))?;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if ready(address) {
            let app = root.join("WuwaTappo.Local.exe");
            return elevate_app(&root, &app);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("本地服务启动超时。请检查端口占用或重新安装应用。".into())
}

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        show_error(&error);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("wuwatappo-launcher can only run on Windows");
}
