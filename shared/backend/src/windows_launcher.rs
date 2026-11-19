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
    path::PathBuf,
    process::{Command, Stdio},
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

/// 探测响应是不是 200。
///
/// 只看状态码，不写死协议版本：`tiny_http` 按请求的版本回包，发 HTTP/1.0 就得到
/// `HTTP/1.0 200 OK`。这里原来断言 `HTTP/1.1 200`，于是探测永远失败——启动器每次都
/// 以为服务没起来，再起一个宿主；新宿主绑定同一端口失败直接退出，20 秒后报
/// 「本地服务启动超时」。
#[cfg_attr(not(windows), allow(dead_code))]
fn status_line_ok(response: &str) -> bool {
    response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .is_some_and(|code| code == "200")
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
    stream.read_to_string(&mut response).is_ok() && status_line_ok(&response)
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
    let config = wuwatappo_assistant_core::ensure_local_config(&root)?;
    let port = config
        .get("port")
        .and_then(Value::as_u64)
        .ok_or("本地服务端口无效")? as u16;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    if !ready(address) {
        let host = root.join("LocalGalleryHost.exe");
        // 启动器是 GUI 子系统，子进程的 stdout/stderr 没有控制台可去。
        // 落到日志里，宿主启动失败时才有东西可查。
        let capture = |name: &str| {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join(name))
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null())
        };
        Command::new(&host)
            .current_dir(&root)
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(capture("host.stdout.log"))
            .stderr(capture("host.stderr.log"))
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

#[cfg(test)]
mod tests {
    use super::status_line_ok;

    #[test]
    fn accepts_ok_from_either_protocol_version() {
        // tiny_http 对 HTTP/1.0 请求回 HTTP/1.0，而启动器发的正是 1.0；
        // 两种版本都得认，否则探测永远失败。
        assert!(status_line_ok("HTTP/1.0 200 OK\r\n\r\n"));
        assert!(status_line_ok("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"));
        assert!(status_line_ok("HTTP/1.0 200 OK\n"));
    }

    #[test]
    fn rejects_everything_that_is_not_200() {
        assert!(!status_line_ok("HTTP/1.1 403 Forbidden\r\n\r\n"));
        assert!(!status_line_ok("HTTP/1.0 404 Not Found\r\n\r\n"));
        assert!(!status_line_ok("HTTP/1.0 2000 nope\r\n\r\n"));
        assert!(!status_line_ok("HTTP/1.0\r\n\r\n"));
        assert!(!status_line_ok(""));
        // 端口被别的服务占着时，别把任意文本当成就绪。
        assert!(!status_line_ok("SSH-2.0-OpenSSH_9.0\r\n"));
    }
}
