//! Verified Windows in-place update package staging and transaction.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 5000;
const PROGRESS_MAX_AGE_SECS: u64 = 900;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn write_current_status(root: &Path, phase: &str, percent: f64, message: &str) {
    let payload = json!({
        "phase": phase,
        "percent": percent,
        "message": message,
        "updated_at": now_secs(),
    });
    let updates = root.join(".updates");
    let _ = fs::create_dir_all(&updates);
    let _ = atomic_json(&updates.join("install-current.json"), &payload);
}

pub fn write_progress(root: &Path, job: &Path, phase: &str, percent: f64, message: &str) {
    let payload = json!({
        "phase": phase,
        "percent": percent,
        "message": message,
        "updated_at": now_secs(),
    });
    if job.exists() {
        let _ = atomic_json(&job.join("progress.json"), &payload);
    }
    let updates = root.join(".updates");
    let _ = fs::create_dir_all(&updates);
    let _ = atomic_json(&updates.join("install-current.json"), &payload);
}

pub fn prune_old_jobs(root: &Path) {
    let updates = root.join(".updates");
    let Ok(entries) = fs::read_dir(&updates) else {
        return;
    };
    let cutoff = Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let stale = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .and_then(|mtime| SystemTime::now().duration_since(mtime).map_err(|e| std::io::Error::other(e)))
            .is_ok_and(|age| age > cutoff);
        if stale {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// 读取最近 15 分钟内（未过期）的升级进度快照。
pub fn read_progress(root: &Path) -> Option<Value> {
    let bytes = fs::read(root.join(".updates").join("install-current.json")).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let updated_at = value.get("updated_at").and_then(Value::as_u64)?;
    if now_secs().saturating_sub(updated_at) > PROGRESS_MAX_AGE_SECS {
        return None;
    }
    Some(value)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateContract {
    top_level: Vec<String>,
    directories: Vec<String>,
    protected: Vec<String>,
    reserved_names: Vec<String>,
}

static CONTRACT: OnceLock<UpdateContract> = OnceLock::new();

fn contract() -> &'static UpdateContract {
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!("../../contracts/windows-update.json"))
            .expect("shared/contracts/windows-update.json must be valid")
    })
}

fn safe_part(part: &str) -> bool {
    if part.is_empty()
        || part == "."
        || part == ".."
        || part.ends_with([' ', '.'])
        || part.contains(['<', '>', ':', '"', '|', '?', '*', '#', '%'])
        || part.chars().any(char::is_control)
    {
        return false;
    }
    let base = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !contract().reserved_names.iter().any(|name| name == &base)
}

fn safe_relative(raw: &str) -> bool {
    !raw.is_empty() && !raw.starts_with('/') && !raw.contains('\\') && raw.split('/').all(safe_part)
}

fn allowed(path: &str) -> bool {
    if !safe_relative(path) {
        return false;
    }
    let first = path.split('/').next().unwrap_or_default();
    if contract().top_level.iter().any(|name| name == first) {
        return path == first;
    }
    contract().directories.iter().any(|name| name == first)
}

fn protected(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or_default();
    contract().protected.iter().any(|name| name == first)
}

#[cfg(windows)]
fn replace_path(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source = wide(source);
    let target = wide(target);
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_path(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let temp = path.with_extension("tmp");
    fs::write(
        &temp,
        serde_json::to_vec(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    replace_path(&temp, path).map_err(|error| error.to_string())
}

pub fn download_with_progress<F>(
    url: &str,
    target: &Path,
    expected_size: u64,
    expected_sha: &str,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let parsed = reqwest::Url::parse(url).map_err(|error| error.to_string())?;
    if !crate::release_updates::is_trusted_github_download_url(&parsed) {
        return Err("升级包下载地址不受信任".into());
    }
    let client = crate::release_updates::github_download_client(Duration::from_secs(300))?;
    let mut response = client.get(parsed).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "升级包下载失败：HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE || size != expected_size)
    {
        return Err("升级包大小与 Release 清单不一致".into());
    }
    let mut file = File::create(target).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut last_reported = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_ARCHIVE || total > expected_size {
            return Err("升级包超过大小限制".into());
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        if total == expected_size || total.saturating_sub(last_reported) >= 256 * 1024 {
            last_reported = total;
            on_progress(total, expected_size);
        }
    }
    file.flush().map_err(|error| error.to_string())?;
    if total != expected_size {
        return Err("升级包大小不匹配".into());
    }
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected_sha {
        return Err("升级包 SHA-256 不匹配".into());
    }
    Ok(())
}

fn installed_version(config: &Value) -> String {
    config
        .get("community_version")
        .and_then(Value::as_str)
        .or_else(|| config.get("mod_version").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn validate_manifest(
    manifest: &Value,
    version: &str,
) -> Result<Vec<(String, u64, String)>, String> {
    if manifest.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err("未知升级包格式".into());
    }
    if manifest.get("version").and_then(Value::as_str) != Some(version) {
        return Err("升级包版本与 Release 不一致".into());
    }
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or("升级包文件清单无效")?;
    if !(3..=MAX_ENTRIES).contains(&files.len()) {
        return Err("升级包文件清单数量无效".into());
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut total = 0u64;
    for item in files {
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or("升级包文件路径无效")?;
        let size = item
            .get("size")
            .and_then(Value::as_u64)
            .ok_or("升级包文件大小无效")?;
        let digest = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("升级包文件哈希无效")?
            .to_ascii_lowercase();
        if !allowed(path) || protected(path) {
            return Err(format!("升级包包含不允许覆盖的文件：{path}"));
        }
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("升级包文件哈希格式无效".into());
        }
        if size == 0 || size > MAX_EXPANDED || !seen.insert(path.to_ascii_lowercase()) {
            return Err("升级包文件大小或路径重复".into());
        }
        total = total.checked_add(size).ok_or("升级包展开大小溢出")?;
        if total > MAX_EXPANDED {
            return Err("升级包展开大小超过限制".into());
        }
        result.push((path.to_owned(), size, digest));
    }
    let required = [
        "WuwaTappo.Local.exe".to_ascii_lowercase(),
        "localgalleryhost.exe".into(),
        "package_config.json".into(),
    ];
    for required in required {
        if !seen
            .iter()
            .any(|path| path.to_ascii_lowercase() == required)
        {
            return Err(format!("升级包缺少必要文件：{required}"));
        }
    }
    Ok(result)
}
fn extract_archive(archive_path: &Path, stage: &Path, version: &str) -> Result<(), String> {
    let mut manifest_archive =
        ZipArchive::new(File::open(archive_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if manifest_archive.len() > MAX_ENTRIES + 1 {
        return Err("升级包条目过多".into());
    }
    let manifest = {
        let mut entry = manifest_archive
            .by_name("update_manifest.json")
            .map_err(|_| "升级包缺少 update_manifest.json")?;
        if entry.size() > MAX_MANIFEST {
            return Err("升级包清单过大".into());
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())?
    };
    let files = validate_manifest(&manifest, version)?;
    let mut archive = ZipArchive::new(File::open(archive_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let expected: HashSet<&str> = files.iter().map(|(path, _, _)| path.as_str()).collect();
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_owned();
        if !names.insert(name.clone()) {
            return Err(format!("升级包包含重复条目：{name}"));
        }
        if entry.is_dir() {
            return Err(format!("升级包包含目录条目：{name}"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("升级包包含符号链接：{name}"));
        }
        if name != "update_manifest.json" && !expected.contains(name.as_str()) {
            return Err(format!("升级包包含清单外文件：{name}"));
        }
    }
    if names.len() != files.len() + 1 {
        return Err("升级包条目与清单不一致".into());
    }
    for path in &expected {
        if !names.contains(*path) {
            return Err(format!("升级包缺少文件：{path}"));
        }
    }
    fs::create_dir_all(stage).map_err(|error| error.to_string())?;
    for (path, size, digest) in &files {
        let target = stage.join(path);
        if !target.starts_with(stage) {
            return Err("升级包路径越界".into());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut entry = archive.by_name(path).map_err(|error| error.to_string())?;
        let mut file = File::create(&target).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > *size {
                return Err(format!("升级包文件展开后过大：{path}"));
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
        }
        file.flush().map_err(|error| error.to_string())?;
        let actual = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if total != *size || &actual != digest {
            return Err(format!("升级包文件校验失败：{path}"));
        }
    }
    let package =
        fs::read_to_string(stage.join("package_config.json")).map_err(|error| error.to_string())?;
    let package: Value = serde_json::from_str(&package).map_err(|error| error.to_string())?;
    if package.get("token").is_some() || installed_version(&package) != version {
        return Err("升级包公开配置无效".into());
    }
    atomic_json(&stage.join("update_manifest.json"), &manifest)?;
    Ok(())
}

pub fn prepare(root: &Path, candidate: &Value) -> Result<Value, String> {
    let url = candidate
        .get("url")
        .and_then(Value::as_str)
        .ok_or("升级包地址缺失")?;
    let version = candidate
        .get("version")
        .and_then(Value::as_str)
        .ok_or("升级包版本缺失")?;
    let sha = candidate
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or("升级包哈希缺失")?;
    let size = candidate
        .get("size")
        .and_then(Value::as_u64)
        .ok_or("升级包大小缺失")?;
    if !url.starts_with("https://github.com/") || !version.starts_with(char::is_numeric) {
        return Err("升级包来源无效".into());
    }
    prune_old_jobs(root);
    let job_id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    let job = root.join(".updates").join(&job_id);
    fs::create_dir_all(&job).map_err(|error| error.to_string())?;
    write_progress(root, &job, "download", 0.0, "正在下载升级包…");
    let prepared = (|| {
        let archive = job.join("download.zip");
        download_with_progress(url, &archive, size, sha, |done, total| {
            let ratio = done as f64 / total.max(1) as f64;
            let bar_pct = (ratio * 75.0).min(75.0);
            let dl_pct = (ratio * 100.0).min(100.0);
            write_progress(
                root,
                &job,
                "download",
                bar_pct,
                &format!("正在下载升级包（{dl_pct:.0}%）"),
            );
        })?;
        write_progress(root, &job, "extract", 80.0, "正在解压并校验升级包…");
        let stage = job.join("stage");
        extract_archive(&archive, &stage, version)?;
        write_progress(root, &job, "apply", 85.0, "准备重启并应用更新…");
        let helper = job.join("helper");
        fs::create_dir_all(&helper).map_err(|error| error.to_string())?;
        fs::copy(
            root.join("LocalGalleryHost.exe"),
            helper.join("LocalGalleryHost.exe"),
        )
        .map_err(|error| format!("复制升级助手失败：{error}"))?;
        atomic_json(
            &job.join("pending.json"),
            &json!({"version":version,"id":job_id}),
        )?;
        spawn_helper(root, &stage)
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&job);
        write_progress(root, &job, "failed", 0.0, &format!("准备更新失败：{error}"));
        return Err(error);
    }
    Ok(json!({"restarting":true,"job":job_id,"version":version,
        "message":"升级包已校验。正在退出桌宠、备份并原地升级…"}))
}

fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::copy(source, target).map_err(|error| error.to_string())?;
    Ok(())
}

fn update_local_config(root: &Path, stage: &Path) -> Result<(), String> {
    let package: Value = serde_json::from_slice(
        &fs::read(stage.join("package_config.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let path = root.join("local_config.json");
    let mut current = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_else(|| json!({}));
    if let (Some(target), Some(source)) = (current.as_object_mut(), package.as_object()) {
        for key in [
            "region",
            "label",
            "cdn_host",
            "api_base",
            "asset_count",
            "atlas_count",
            "client_version",
            "mod_version",
            "community_version",
        ] {
            if let Some(value) = source.get(key) {
                target.insert(key.to_owned(), value.clone());
            }
        }
    }
    atomic_json(&path, &current)
}

fn apply_files(root: &Path, stage: &Path, job: &Path) -> Result<Vec<(String, bool)>, String> {
    let manifest: Value = serde_json::from_slice(
        &fs::read(stage.join("update_manifest.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("升级包版本缺失")?;
    let files = validate_manifest(&manifest, version)?;
    let backup = job.join("backup");
    let mut changed = Vec::new();
    let mut apply = || -> Result<(), String> {
        for (name, _, _) in &files {
            let target = root.join(name);
            if !target.starts_with(root) {
                return Err("升级目标越界".into());
            }
            let existed = target.exists();
            if existed {
                copy_file(&target, &backup.join(name))?;
            }
            changed.push((name.clone(), existed));
            atomic_json(&job.join("transaction.json"), &json!(changed))?;
            let temp = target.with_extension("update-tmp");
            copy_file(&stage.join(name), &temp)?;
            replace_path(&temp, &target).map_err(|error| error.to_string())?;
        }
        let local = root.join("local_config.json");
        let local_existed = local.exists();
        if local_existed {
            copy_file(&local, &backup.join("local_config.json"))?;
        }
        changed.push(("local_config.json".into(), local_existed));
        atomic_json(&job.join("transaction.json"), &json!(changed))?;
        update_local_config(root, stage)?;
        Ok(())
    };
    if let Err(error) = apply() {
        rollback(root, job, &changed);
        return Err(error);
    }
    Ok(changed)
}

fn rollback(root: &Path, job: &Path, changed: &[(String, bool)]) {
    for (name, existed) in changed.iter().rev() {
        let target = root.join(name);
        if *existed {
            let _ = copy_file(&job.join("backup").join(name), &target);
        } else {
            let _ = fs::remove_file(&target);
        }
    }
}

fn launch(root: &Path) -> Result<(), String> {
    let launcher = root.join("WuwaTappoLauncher.exe");
    let target = if launcher.is_file() {
        launcher
    } else {
        root.join("WuwaTappo.Local.exe")
    };
    let mut command = Command::new(target);
    command.current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn app_running() -> bool {
    Command::new("tasklist")
        .args([
            "/FI",
            "IMAGENAME eq WuwaTappo.Local.exe",
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .ok()
        .is_some_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .to_ascii_lowercase()
                .contains(&"WuwaTappo.Local.exe".to_ascii_lowercase())
        })
}

#[cfg(not(windows))]
fn app_running() -> bool {
    false
}

fn wait_for_app_exit() -> Result<(), String> {
    let deadline = SystemTime::now() + Duration::from_secs(60);
    while app_running() {
        if SystemTime::now() >= deadline {
            return Err("旧桌宠仍在运行，升级取消；原文件未替换。".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn wait_healthy(job: &Path) -> Result<(), String> {
    let deadline = SystemTime::now() + Duration::from_secs(90);
    while !job.join("healthy").exists() {
        if SystemTime::now() >= deadline {
            return Err("新版本未通过启动检查，正在恢复旧版本。".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

pub fn apply_update(root: &Path, stage: &Path) -> Result<(), String> {
    let job = stage.parent().ok_or("升级舞台路径无效")?;
    std::thread::sleep(Duration::from_secs(2));
    wait_for_app_exit()?;
    write_progress(root, job, "apply", 90.0, "正在替换程序文件…");
    let manifest: Value = serde_json::from_slice(
        &fs::read(stage.join("update_manifest.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let changed = match apply_files(root, stage, job) {
        Ok(changed) => changed,
        Err(error) => {
            write_progress(root, job, "failed", 0.0, &format!("替换文件失败：{error}"));
            let _ = launch(root);
            let _ = atomic_json(&job.join("result.json"), &json!({"ok":false,"error":error}));
            return Err(error);
        }
    };
    if let Err(error) = launch(root).and_then(|_| wait_healthy(job)) {
        rollback(root, job, &changed);
        write_progress(
            root,
            job,
            "failed",
            0.0,
            &format!("新版本启动失败，已回滚：{error}"),
        );
        let _ = launch(root);
        let _ = atomic_json(&job.join("result.json"), &json!({"ok":false,"error":error}));
        return Err(error);
    }
    if job.join("backup").exists() {
        let _ = fs::remove_dir_all(job.join("backup"));
    }
    let _ = fs::remove_file(job.join("download.zip"));
    let _ = fs::remove_file(job.join("pending.json"));
    let _ = fs::remove_dir_all(stage);
    write_progress(root, job, "complete", 100.0, "更新完成！");
    atomic_json(
        &job.join("result.json"),
        &json!({"ok":true,"version":version}),
    )
}

pub fn ack_healthy(root: &Path, version: &str) {
    let updates = root.join(".updates");
    let Ok(entries) = fs::read_dir(updates) else {
        return;
    };
    for entry in entries.flatten() {
        let pending = entry.path().join("pending.json");
        let Ok(value) = fs::read(&pending)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .ok_or(())
        else {
            continue;
        };
        if value.get("version").and_then(Value::as_str) == Some(version) {
            let _ = fs::write(entry.path().join("healthy"), version);
        }
    }
}

#[cfg(windows)]
fn spawn_helper(root: &Path, stage: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let helper = stage
        .parent()
        .ok_or("升级舞台路径无效")?
        .join("helper")
        .join("LocalGalleryHost.exe");
    Command::new(helper)
        .arg("--apply-update")
        .arg(root)
        .arg(stage)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn spawn_helper(_root: &Path, _stage: &Path) -> Result<(), String> {
    Err("Windows 更新助手只能在本平台运行".into())
}

fn collect_payload_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "打包目录不允许包含符号链接：{}",
                entry.path().display()
            ));
        }
        let path = entry.path();
        if meta.is_dir() {
            collect_payload_files(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if relative != "update_manifest.json" && allowed(&relative) && !protected(&relative) {
            out.push(relative);
        }
    }
    Ok(())
}

/// 从公开构建目录生成带白名单与 SHA-256 清单的原地更新 ZIP 包。
pub fn build_update_archive(
    payload: &Path,
    out_dir: &Path,
    region_override: Option<&str>,
) -> Result<Value, String> {
    let config_raw =
        fs::read_to_string(payload.join("package_config.json")).map_err(|error| error.to_string())?;
    let config_trimmed = config_raw.strip_prefix('\u{feff}').unwrap_or(&config_raw);
    let mut config: Value =
        serde_json::from_str(config_trimmed).map_err(|error| error.to_string())?;
    if let Some(object) = config.as_object_mut() {
        object.remove("token");
    }
    let version = installed_version(&config);
    if version.is_empty() {
        return Err("package_config.json 缺少版本号".into());
    }
    let region = region_override
        .or_else(|| config.get("region").and_then(Value::as_str))
        .unwrap_or("cn");
    let profile = crate::regions::profile(region).ok_or("未知官方服务区域")?;

    let mut paths = Vec::new();
    collect_payload_files(payload, payload, &mut paths)?;
    paths.sort();

    let public_config = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    let mut file_records = Vec::with_capacity(paths.len());
    let mut contents: Vec<(String, Vec<u8>)> = Vec::with_capacity(paths.len());
    for relative in paths {
        let bytes = if relative == "package_config.json" {
            public_config.clone()
        } else {
            fs::read(payload.join(&relative)).map_err(|error| error.to_string())?
        };
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        file_records.push(json!({
            "path": relative,
            "size": bytes.len() as u64,
            "sha256": digest,
        }));
        contents.push((relative, bytes));
    }
    let manifest = json!({
        "schema": 1,
        "format": "wuwatappo-windows-update",
        "version": version,
        "client_version": config.get("client_version").cloned().unwrap_or(Value::Null),
        "region": profile.id,
        "files": file_records,
    });
    validate_manifest(&manifest, &version)?;

    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    let asset_name = crate::release_updates::release_asset_name(
        "windowsUpdate",
        &profile.artifact_prefix,
        &version,
    )?;
    let archive_path = out_dir.join(&asset_name);
    let temp_path = out_dir.join(format!(".{asset_name}.tmp"));
    {
        let mut writer = ZipWriter::new(File::create(&temp_path).map_err(|error| error.to_string())?);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o100644);
        writer
            .start_file("update_manifest.json", options)
            .map_err(|error| error.to_string())?;
        let manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
        writer
            .write_all(&manifest_bytes)
            .map_err(|error| error.to_string())?;
        for (relative, bytes) in &contents {
            writer
                .start_file(relative, options)
                .map_err(|error| error.to_string())?;
            writer.write_all(bytes).map_err(|error| error.to_string())?;
        }
        writer.finish().map_err(|error| error.to_string())?;
    }
    if let Err(error) = replace_path(&temp_path, &archive_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error.to_string());
    }
    let zip_bytes = fs::read(&archive_path).map_err(|error| error.to_string())?;
    let zip_sha = Sha256::digest(&zip_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(json!({
        "archive": archive_path,
        "fileName": asset_name,
        "sha256": zip_sha,
        "size": zip_bytes.len() as u64,
        "version": version,
        "region": profile.id,
        "fileCount": contents.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::SystemTime};

    fn manifest(path: &str) -> Value {
        json!({
            "schema": 1,
            "version": "1.3.2",
            "files": [
                {"path":"WuwaTappo.Local.exe","size":1,"sha256":"a".repeat(64)},
                {"path":"LocalGalleryHost.exe","size":1,"sha256":"b".repeat(64)},
                {"path":path,"size":1,"sha256":"c".repeat(64)}
            ]
        })
    }

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wuwa-update-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn digest(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn package_entries() -> Vec<(String, Vec<u8>)> {
        vec![
            ("WuwaTappo.Local.exe".into(), b"MZ-local".to_vec()),
            ("LocalGalleryHost.exe".into(), b"MZ-host".to_vec()),
            (
                "package_config.json".into(),
                br#"{"community_version":"1.3.2","region":"cn"}"#.to_vec(),
            ),
        ]
    }

    fn package_manifest(entries: &[(String, Vec<u8>)]) -> Value {
        json!({
            "schema": 1,
            "version": "1.3.2",
            "files": entries.iter().map(|(path, bytes)| json!({
                "path": path,
                "size": bytes.len(),
                "sha256": digest(bytes),
            })).collect::<Vec<_>>()
        })
    }

    fn write_zip(path: &Path, manifest: &Value, entries: &[(String, Vec<u8>)]) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        writer
            .start_file(
                "update_manifest.json",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer
            .write_all(serde_json::to_string(manifest).unwrap().as_bytes())
            .unwrap();
        for (name, bytes) in entries {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .unix_permissions(0o100644);
            writer.start_file(name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    fn mark_zip_entry_symlink(path: &Path, entry_name: &str) {
        let mut bytes = fs::read(path).unwrap();
        let mut offset = 0usize;
        let mut found = false;
        while offset + 46 <= bytes.len() {
            if bytes.get(offset..offset + 4) != Some(b"PK\x01\x02") {
                offset += 1;
                continue;
            }
            let name_len =
                u16::from_le_bytes(bytes[offset + 28..offset + 30].try_into().unwrap()) as usize;
            let extra_len =
                u16::from_le_bytes(bytes[offset + 30..offset + 32].try_into().unwrap()) as usize;
            let comment_len =
                u16::from_le_bytes(bytes[offset + 32..offset + 34].try_into().unwrap()) as usize;
            let name = &bytes[offset + 46..offset + 46 + name_len];
            if name == entry_name.as_bytes() {
                bytes[offset + 4..offset + 6].copy_from_slice(&0x031eu16.to_le_bytes());
                bytes[offset + 38..offset + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
                found = true;
                break;
            }
            offset += 46 + name_len + extra_len + comment_len;
        }
        assert!(found, "zip entry not found: {entry_name}");
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn replace_path_overwrites_an_existing_file() {
        let root = temp_dir("replace");
        let source = root.join("source.tmp");
        let target = root.join("target.json");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();
        replace_path(&source, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(!source.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn update_manifest_rejects_protected_and_traversal_paths() {
        assert!(validate_manifest(&manifest("package_config.json"), "1.3.2").is_ok());
        for path in [
            "local_config.json",
            "../escape",
            "/escape",
            "cache/state.json",
            r"runtime\x",
            "docs/../escape",
            "WuwaTappo.Local.exe/child",
            "docs/CON",
            "docs/file.",
            "docs/file ",
            "docs/a?b",
            "docs/a<b",
        ] {
            assert!(
                validate_manifest(&manifest(path), "1.3.2").is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn update_manifest_requires_the_executable_host_and_public_config() {
        let mut value = manifest("package_config.json");
        value["files"].as_array_mut().unwrap().pop();
        assert!(validate_manifest(&value, "1.3.2").is_err());
        assert!(validate_manifest(&manifest("package_config.json"), "9.9.9").is_err());
    }

    #[test]
    fn update_manifest_rejects_case_insensitive_duplicate_paths() {
        let mut value = manifest("README.md");
        value["files"].as_array_mut().unwrap().push(json!({
            "path":"readme.md","size":1,"sha256":"d".repeat(64)
        }));
        assert!(validate_manifest(&value, "1.3.2").is_err());
    }

    #[test]
    fn valid_update_zip_is_verified_and_extracted() {
        let root = temp_dir("valid");
        let zip = root.join("update.zip");
        let entries = package_entries();
        write_zip(&zip, &package_manifest(&entries), &entries);
        let stage = root.join("stage");
        extract_archive(&zip, &stage, "1.3.2").unwrap();
        assert_eq!(
            fs::read(stage.join("WuwaTappo.Local.exe")).unwrap(),
            b"MZ-local"
        );
        assert!(stage.join("update_manifest.json").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tampered_or_extra_zip_entries_are_rejected() {
        let root = temp_dir("tampered");
        let entries = package_entries();
        let manifest = package_manifest(&entries);

        let tampered = root.join("tampered.zip");
        let mut changed = entries.clone();
        changed[0].1 = b"MZ-changed".to_vec();
        write_zip(&tampered, &manifest, &changed);
        assert!(extract_archive(&tampered, &root.join("tampered-stage"), "1.3.2").is_err());

        let extra = root.join("extra.zip");
        let mut extra_entries = entries.clone();
        extra_entries.push(("extra.txt".into(), b"extra".to_vec()));
        write_zip(&extra, &manifest, &extra_entries);
        assert!(extract_archive(&extra, &root.join("extra-stage"), "1.3.2").is_err());

        let symlink = root.join("symlink.zip");
        let mut symlink_entries = entries.clone();
        symlink_entries.push(("README.md".into(), b"target".to_vec()));
        let symlink_manifest = package_manifest(&symlink_entries);
        write_zip(&symlink, &symlink_manifest, &symlink_entries);
        mark_zip_entry_symlink(&symlink, "README.md");
        assert!(extract_archive(&symlink, &root.join("symlink-stage"), "1.3.2").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_restores_existing_files_and_removes_new_files() {
        let root = temp_dir("rollback");
        let job = root.join(".updates/job");
        fs::create_dir_all(job.join("backup")).unwrap();
        fs::write(root.join("kept.txt"), b"new").unwrap();
        fs::write(root.join("created.txt"), b"new").unwrap();
        fs::write(job.join("backup/kept.txt"), b"old").unwrap();
        rollback(
            &root,
            &job,
            &[("kept.txt".into(), true), ("created.txt".into(), false)],
        );
        assert_eq!(fs::read(root.join("kept.txt")).unwrap(), b"old");
        assert!(!root.join("created.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn built_update_archive_strips_token_and_passes_extract_verification() {
        let root = temp_dir("build-archive");
        let payload = root.join("payload");
        let out_dir = root.join("dist");
        fs::create_dir_all(payload.join("docs")).unwrap();
        fs::create_dir_all(payload.join("cache")).unwrap();
        fs::write(payload.join("WuwaTappo.Local.exe"), b"MZ-local").unwrap();
        fs::write(payload.join("LocalGalleryHost.exe"), b"MZ-host").unwrap();
        fs::write(
            payload.join("package_config.json"),
            "\u{feff}{\"community_version\":\"1.3.2\",\"client_version\":\"1.0.4\",\"region\":\"cn\",\"token\":\"secret-token\"}",
        )
        .unwrap();
        fs::write(payload.join("docs/README.md"), b"doc").unwrap();
        // Should be excluded by protected/whitelist rules:
        fs::write(payload.join("local_config.json"), b"{}").unwrap();
        fs::write(payload.join("cache/state.json"), b"{}").unwrap();

        let summary = build_update_archive(&payload, &out_dir, None).unwrap();
        assert_eq!(
            summary["fileName"],
            "WuwaTappo-Full-CN-Community-1.3.2-win-x64-update.zip"
        );
        assert_eq!(summary["fileCount"], 4);

        let stage = root.join("stage");
        let zip_path = PathBuf::from(summary["archive"].as_str().unwrap());
        extract_archive(&zip_path, &stage, "1.3.2").unwrap();
        let extracted_cfg: Value =
            serde_json::from_slice(&fs::read(stage.join("package_config.json")).unwrap()).unwrap();
        assert!(extracted_cfg.get("token").is_none());
        assert!(!stage.join("local_config.json").exists());
        assert!(!stage.join("cache").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn install_progress_writes_and_expires_after_ttl() {
        let root = temp_dir("progress");
        let job = root.join(".updates/job-1");
        fs::create_dir_all(&job).unwrap();
        write_progress(&root, &job, "extract", 80.0, "正在解压");
        let current = read_progress(&root).unwrap();
        assert_eq!(current["phase"], "extract");
        assert_eq!(current["percent"], 80.0);
        assert!(job.join("progress.json").exists());

        atomic_json(
            &root.join(".updates/install-current.json"),
            &json!({"phase": "download", "percent": 10.0, "updated_at": 1}),
        )
        .unwrap();
        assert!(read_progress(&root).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
