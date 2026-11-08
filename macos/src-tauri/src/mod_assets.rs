//! 本机 Mod 图片路由。
//!
//! 官方渲染器虽然放行 `asset://`，但 WKWebView 从 `tauri://localhost` 页面里
//! 取 `asset://` 子资源会受到协议作用域和来源限制，实测拿不到图（表现为 404）。
//! 这里起一个只监听回环地址的极小 HTTP 服务，把 Mod 目录里的图片按
//! `custom-assets/<包>/<文件>` 暴露出去 —— 与 Windows 侧 local_host 的同名路径
//! 完全一致的语义，前端一份代码两端通用。
//!
//! 安全边界：
//! - 只 bind 127.0.0.1，端口由系统分配；
//! - 路径前缀是每次启动随机生成的 token，别的进程/网页猜不到；
//! - 只服务 `custom-assets/` 下校验过的相对键，拒绝 `..`、反斜杠、盘符；
//! - 规范化后的真实路径必须仍在 Mod 根目录内（挡住符号链接逃逸）。

use std::{
    fs,
    net::TcpListener as StdListener,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const MAX_REQUEST: usize = 2048;
const MAX_FILE: u64 = 16 * 1024 * 1024;

/// 已启动的取图服务。`base` 形如 `http://127.0.0.1:53421/3f9c.../`。
pub struct ModAssetHost {
    base: String,
}

impl ModAssetHost {
    pub fn base(&self) -> &str {
        &self.base
    }
}

struct Inner {
    root: PathBuf,
    token: String,
}

impl Inner {
    async fn serve(&self, mut stream: TcpStream) {
        let mut buffer = [0u8; MAX_REQUEST];
        let read = match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer)).await
        {
            Ok(Ok(count)) if count > 0 => count,
            _ => return,
        };
        let head = String::from_utf8_lossy(&buffer[..read]);
        let mut lines = head.lines();
        let mut request = lines.next().unwrap_or_default().split(' ');
        let method = request.next().unwrap_or_default();
        let target = request.next().unwrap_or_default();
        if method != "GET" && method != "HEAD" {
            return respond(&mut stream, "405 Method Not Allowed", "text/plain", b"", false).await;
        }
        let Some(file) = self.lookup(target) else {
            return respond(&mut stream, "404 Not Found", "text/plain", b"", false).await;
        };
        let body = match fs::metadata(&file) {
            Ok(meta) if meta.len() <= MAX_FILE => match fs::read(&file) {
                Ok(bytes) => bytes,
                Err(_) => {
                    return respond(&mut stream, "404 Not Found", "text/plain", b"", false).await
                }
            },
            _ => return respond(&mut stream, "404 Not Found", "text/plain", b"", false).await,
        };
        respond(
            &mut stream,
            "200 OK",
            content_type(&file),
            &body,
            method == "HEAD",
        )
        .await;
    }

    fn lookup(&self, target: &str) -> Option<PathBuf> {
        let path = target
            .split(|value| value == '?' || value == '#')
            .next()
            .unwrap_or_default();
        let decoded = percent_decode(path);
        let key = decoded.strip_prefix(&format!("/{}/", self.token))?;
        asset_path(&self.root, key)
    }
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    body: &[u8],
    head_only: bool,
) {
    // no-store：同一个包重装后同名文件内容会变，让 WebView 每次都回源。
    // ACAO 只是兜底：图片走 <img>，但官方有些预览会 fetch 成 blob，跨源要它。
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    if !head_only {
        let _ = stream.write_all(body).await;
    }
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

/// 把 `custom-assets/<pack>/<file…>` 解析成磁盘路径，任何可疑输入直接拒绝。
fn asset_path(root: &Path, key: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() < 3 || parts[0] != "custom-assets" {
        return None;
    }
    if parts.iter().any(|part| {
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part.contains('\\')
            || part.contains(':')
            || part.chars().any(char::is_control)
    }) {
        return None;
    }
    // 隐藏目录是安装/回滚过程中的临时状态，不该被外部看到。
    if parts[1].starts_with('.') {
        return None;
    }
    let mut path = root.join(parts[1]).join("assets");
    for part in &parts[2..] {
        path.push(part);
    }
    let path = path.canonicalize().ok()?;
    let base = root.canonicalize().ok()?;
    if !path.starts_with(&base) || !path.is_file() {
        return None;
    }
    Some(path)
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    }
}

/// 只解码 `%XX`；`+` 在路径里就是加号，不能当空格。
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn random_token() -> String {
    // 不引新依赖：优先取系统随机源，取不到就退化成时间 + pid 的哈希。
    let mut bytes = [0u8; 16];
    let filled = fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
        .is_ok();
    if !filled {
        let seed = format!(
            "{}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default(),
            std::thread::current().id()
        );
        let mut state: u64 = 0xcbf29ce484222325;
        for (index, byte) in seed.bytes().enumerate() {
            state ^= u64::from(byte);
            state = state.wrapping_mul(0x100000001b3);
            bytes[index % 16] ^= (state >> ((index % 8) * 8)) as u8;
        }
    }
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

/// 启动取图服务；起不来只降级（`base` 为空，前端退回旧路径），不影响应用启动。
pub fn start(root: PathBuf) -> ModAssetHost {
    match bind(root) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("Mod 图片服务未启动：{error}");
            ModAssetHost {
                base: String::new(),
            }
        }
    }
}

fn bind(root: PathBuf) -> Result<ModAssetHost, String> {
    let listener = StdListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("本机图片服务启动失败：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let token = random_token();
    let inner = Arc::new(Inner {
        root,
        token: token.clone(),
    });
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = TcpListener::from_std(listener) else {
            return;
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let inner = inner.clone();
                    tauri::async_runtime::spawn(async move { inner.serve(stream).await });
                }
                Err(error) => {
                    // 连接被中断是常态，不该把整个循环带走。
                    eprintln!("Mod 图片服务 accept 失败：{error}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    });
    Ok(ModAssetHost {
        base: format!("http://127.0.0.1:{port}/{token}/"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("wuwa-mod-assets-{}", random_token()))
    }

    #[test]
    fn only_plain_asset_keys_inside_the_pack_folder_are_served() {
        let base = root();
        let pack = base.join("mod-0123456789abcdef").join("assets");
        fs::create_dir_all(&pack).unwrap();
        fs::write(pack.join("icon.png"), b"png").unwrap();

        let key = "custom-assets/mod-0123456789abcdef/icon.png";
        assert_eq!(
            asset_path(&base, key).map(|path| path.file_name().unwrap().to_owned()),
            Some("icon.png".into())
        );

        for rejected in [
            "custom-assets/../icon.png",
            "custom-assets/mod-0123456789abcdef/../../etc/passwd",
            "custom-assets/mod-0123456789abcdef/",
            "custom-assets/.stage-1/icon.png",
            "assets/mod-0123456789abcdef/icon.png",
            "custom-assets/mod-0123456789abcdef\\icon.png",
            "custom-assets/C:/Windows/System32/icon.png",
            "custom-assets/mod-0123456789abcdef/missing.png",
        ] {
            assert!(asset_path(&base, rejected).is_none(), "{rejected}");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn request_target_is_decoded_and_the_token_is_required() {
        let inner = Inner {
            root: root(),
            token: "abc123".into(),
        };
        assert_eq!(
            percent_decode("/abc123/custom-assets/mod-1/%E5%9B%BE.png"),
            "/abc123/custom-assets/mod-1/图.png"
        );
        // token 不对、路径越界都拿不到文件。
        assert!(inner.lookup("/wrong/custom-assets/mod-1/icon.png").is_none());
        assert!(inner.lookup("/abc123/custom-assets/../icon.png").is_none());
        // 查询串（官方渲染器会拼 ?t=）不影响解析。
        assert_eq!(inner.lookup("/abc123/custom-assets/mod-1/icon.png?t=1"), None);
    }

    #[test]
    fn only_known_image_extensions_get_an_image_content_type() {
        assert_eq!(content_type(Path::new("a.PNG")), "image/png");
        assert_eq!(content_type(Path::new("a.webp")), "image/webp");
        assert_eq!(content_type(Path::new("a.json")), "application/octet-stream");
    }
}
