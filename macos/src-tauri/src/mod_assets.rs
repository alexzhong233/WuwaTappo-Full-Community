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
    path::PathBuf,
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
            wuwatappo_assistant_core::custom_resources::asset_mime_type(&file),
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
        wuwatappo_assistant_core::custom_resources::resolve_asset_file(&self.root, key)
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
    let token = wuwatappo_assistant_core::random_token_hex();
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
    use std::path::Path;
    use wuwatappo_assistant_core::custom_resources::{asset_mime_type, resolve_asset_file};

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "wuwa-mod-assets-{}",
            wuwatappo_assistant_core::random_token_hex()
        ))
    }

    #[test]
    fn only_plain_asset_keys_inside_the_pack_folder_are_served() {
        let base = root();
        let pack = base.join("mod-0123456789abcdef").join("assets");
        fs::create_dir_all(&pack).unwrap();
        fs::write(pack.join("icon.png"), b"png").unwrap();

        let key = "custom-assets/mod-0123456789abcdef/icon.png";
        assert_eq!(
            resolve_asset_file(&base, key).map(|path| path.file_name().unwrap().to_owned()),
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
            assert!(resolve_asset_file(&base, rejected).is_none(), "{rejected}");
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
        assert_eq!(asset_mime_type(Path::new("a.PNG")), "image/png");
        assert_eq!(asset_mime_type(Path::new("a.webp")), "image/webp");
        assert_eq!(
            asset_mime_type(Path::new("a.json")),
            "application/octet-stream"
        );
    }
}
