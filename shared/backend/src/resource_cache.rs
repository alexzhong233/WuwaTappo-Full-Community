//! Verified, transactional download of the public atlas and its image cache.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_IMAGE: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SyncSummary {
    pub changed: bool,
    pub atlas_count: usize,
    pub asset_count: usize,
}

pub struct OfficialCatalog {
    pub activity: Value,
    pub atlas: Value,
    pub catalog: Value,
    pub assets_host: Url,
    client: Client,
}

fn safe_url(raw: &str, expected_host: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "官方资源地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some(expected_host)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("官方资源地址不是当前地区的 HTTPS 官方地址".into());
    }
    Ok(url)
}

fn get_json(client: &Client, url: Url) -> Result<Value, String> {
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "官方目录请求失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_JSON)
    {
        return Err("官方图鉴目录超过 16 MiB".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_JSON {
        return Err("官方图鉴目录超过 16 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("官方图鉴 JSON 无效：{error}"))
}

fn hash_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn rewrite_urls(
    value: &mut Value,
    base: &Url,
    jobs: &mut Map<String, Value>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if key == "url" {
                    if let Some(raw) = child.as_str() {
                        let url = base.join(raw).map_err(|_| "图鉴图片地址无效")?;
                        if url.scheme() != "https"
                            || url.host_str() != base.host_str()
                            || url.port_or_known_default() != Some(443)
                        {
                            return Err("图鉴包含非当前官方素材域名的图片".into());
                        }
                        let file = format!("{}.png", hash_hex(url.as_str().as_bytes()));
                        jobs.insert(file.clone(), json!(url.as_str()));
                        *child = Value::String(format!("assets/{file}"));
                    } else {
                        rewrite_urls(child, base, jobs)?;
                    }
                } else {
                    rewrite_urls(child, base, jobs)?;
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                rewrite_urls(child, base, jobs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut file, value).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

fn download_one(client: &Client, url: &str, target: &Path) -> Result<(u64, String), String> {
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "素材下载失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_IMAGE)
    {
        return Err("官方图片超过 32 MiB".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_IMAGE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("下载的官方素材不是有效 PNG".into());
    }
    fs::write(target, &bytes).map_err(|error| error.to_string())?;
    Ok((bytes.len() as u64, hash_hex(&bytes)))
}

fn download_one_with_retry(
    client: &Client,
    url: &str,
    target: &Path,
) -> Result<(u64, String), String> {
    let mut last_error = String::new();
    for attempt in 0..3 {
        match download_one(client, url, target) {
            Ok(result) => return Ok(result),
            Err(error) => {
                last_error = error;
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(250 * (attempt as u64 + 1)));
                }
            }
        }
    }
    Err(last_error)
}

fn id_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Some(Value::Number(number)) => number.as_u64().map(|v| v.to_string()),
        _ => None,
    }
}

/// 校验官方图鉴列表与外观映射表完整性，避免写入半残缓存。
pub fn validate_catalog(value: &Value) -> Result<(), String> {
    let atlas = value
        .get("atlas")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or("官方图鉴列表为空")?;
    let catalog = value.get("catalog").ok_or("缺少外观目录")?;
    for kind in ["character_skin", "accessory", "emote"] {
        if !catalog.get(kind).is_some_and(Value::is_object)
            || !catalog
                .get(format!("{kind}_atlas"))
                .is_some_and(Value::is_object)
        {
            return Err("外观目录结构不兼容，原缓存保留".into());
        }
    }
    for row in atlas {
        let kind = row
            .get("asset_type")
            .and_then(Value::as_str)
            .ok_or("图鉴类型缺失")?;
        if !matches!(
            kind,
            "character_skin_atlas" | "accessory_atlas" | "emote_atlas"
        ) {
            return Err("图鉴条目类型不受支持".into());
        }
        let id = id_text(row.get("asset_id")).ok_or("图鉴 ID 无效")?;
        let usable = id_text(
            catalog
                .get(kind)
                .and_then(|v| v.get(&id))
                .and_then(|v| v.get("height_light_id")),
        )
        .ok_or("图鉴缺少外观 ID 映射")?;
        if !catalog
            .get(kind.trim_end_matches("_atlas"))
            .is_some_and(|v| v.get(&usable).is_some())
        {
            return Err("图鉴外观映射不完整，原缓存保留".into());
        }
    }
    Ok(())
}

fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::PermissionDenied && attempt < 4 => {
                thread::sleep(Duration::from_millis(300));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

fn atomic_replace(root: &Path, stage: &Path) -> Result<(), String> {
    let cache = root.join("cache");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let previous = root.join(format!("cache_previous_{stamp}"));
    if cache.exists() {
        rename_with_retry(&cache, &previous)
            .map_err(|error| format!("无法保留旧素材缓存：{error}"))?;
    }
    if let Err(error) = rename_with_retry(stage, &cache) {
        if previous.exists() {
            let _ = rename_with_retry(&previous, &cache);
        }
        return Err(format!("无法启用新素材缓存：{error}"));
    }
    let mut old = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("cache_previous_")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    old.sort();
    old.reverse();
    for stale in old.into_iter().skip(2) {
        let _ = fs::remove_dir_all(stale);
    }
    Ok(())
}

/// Fetch and validate the selected region's official atlas and appearance catalog.
pub fn fetch_catalog(region: &str) -> Result<OfficialCatalog, String> {
    let profile = crate::regions::profile(region).ok_or("未知官方服务区域")?;
    let api_base = format!("https://{}", profile.api_host);
    let assets_host = profile.assets_origin.as_str();
    let api = Url::parse(&api_base).map_err(|_| "官方 API 地址无效")?;
    let host = Url::parse(assets_host).map_err(|_| "官方素材地址无效")?;
    if api.scheme() != "https"
        || host.scheme() != "https"
        || api.host_str().is_none()
        || host.host_str().is_none()
    {
        return Err("官方 API 和素材地址必须使用 HTTPS".into());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(40))
        .connect_timeout(Duration::from_secs(12))
        .redirect(Policy::none())
        .user_agent("WuwaTappo-LocalGallery-Cache")
        .build()
        .map_err(|error| error.to_string())?;
    let activity_url = api
        .join("/api/common/activity-config")
        .map_err(|error| error.to_string())?;
    let atlas_url = api
        .join("/api/common/atlas")
        .map_err(|error| error.to_string())?;
    let activity = get_json(&client, activity_url)?;
    let atlas = get_json(&client, atlas_url)?;
    if activity["code"].as_i64() != Some(0)
        || atlas["code"].as_i64() != Some(0)
        || !atlas["data"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    {
        return Err("官方目录响应结构异常，保留原缓存".into());
    }
    let returned_host = activity
        .pointer("/data/assets_host")
        .and_then(Value::as_str)
        .ok_or("官方活动配置缺少素材域名")?;
    let returned_host = Url::parse(returned_host).map_err(|_| "官方活动配置素材域名无效")?;
    if returned_host.scheme() != "https"
        || returned_host.host_str() != host.host_str()
        || returned_host.port_or_known_default() != Some(443)
        || !returned_host.username().is_empty()
        || returned_host.password().is_some()
        || returned_host.query().is_some()
        || returned_host.fragment().is_some()
        || returned_host.path() != "/"
    {
        return Err("官方活动配置返回了未锁定的素材域名".into());
    }
    let pointer_url = safe_url(
        &format!(
            "{}/app/json-host/appearance_asset_host.json",
            assets_host.trim_end_matches('/')
        ),
        host.host_str().unwrap(),
    )?;
    let pointer = get_json(&client, pointer_url)?;
    let manifest_url = safe_url(
        pointer["url"].as_str().ok_or("官方素材指针缺少 url")?,
        host.host_str().unwrap(),
    )?;
    let catalog = get_json(&client, manifest_url)?;
    validate_catalog(&json!({"atlas": atlas["data"], "catalog": catalog}))?;
    Ok(OfficialCatalog {
        activity,
        atlas,
        catalog,
        assets_host: host,
        client,
    })
}

fn read_cached_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn reuse_cached_asset(
    cache_assets: &Path,
    name: &str,
    target: &Path,
    expected: Option<&(u64, String)>,
) -> Option<(u64, String)> {
    let source = cache_assets.join(name);
    let meta = fs::metadata(&source).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE {
        return None;
    }
    if let Some((expected_size, expected_sha)) = expected {
        if meta.len() != *expected_size {
            return None;
        }
        let bytes = fs::read(&source).ok()?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || hash_hex(&bytes) != *expected_sha {
            return None;
        }
        if fs::hard_link(&source, target).is_err() {
            fs::copy(&source, target).ok()?;
        }
        return Some((*expected_size, expected_sha.clone()));
    }
    let bytes = fs::read(&source).ok()?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let digest = hash_hex(&bytes);
    let len = bytes.len() as u64;
    if fs::hard_link(&source, target).is_err() {
        fs::copy(&source, target).ok()?;
    }
    Some((len, digest))
}

/// 下载并校验选定区域完整图鉴缓存，支持增量复用与实时进度回调。
pub fn sync_with_progress<F>(
    root: &Path,
    region: &str,
    force: bool,
    on_progress: F,
) -> Result<SyncSummary, String>
where
    F: Fn(usize, usize) + Send + Sync,
{
    let OfficialCatalog {
        activity,
        atlas,
        mut catalog,
        assets_host,
        client,
    } = fetch_catalog(region)?;
    let host = assets_host;
    let atlas_rows = atlas["data"].as_array().ok_or("官方图鉴列表无效")?;
    let root = root.to_path_buf();
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.filter_map(Result::ok) {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("cache_staging_")
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }

    let mut jobs = Map::new();
    rewrite_urls(&mut catalog, &host, &mut jobs)?;
    let total = jobs.len();
    let cache_dir = root.join("cache");
    let cache_assets = cache_dir.join("assets");

    let mut existing_records: HashMap<String, (u64, String)> = HashMap::new();
    if let Some(manifest) = read_cached_json(&cache_dir.join("asset_manifest.json")) {
        if let Some(items) = manifest.get("records").and_then(Value::as_array) {
            for item in items {
                if let (Some(file), Some(size), Some(sha)) = (
                    item.get("file").and_then(Value::as_str),
                    item.get("size").and_then(Value::as_u64),
                    item.get("sha256").and_then(Value::as_str),
                ) {
                    existing_records.insert(file.to_owned(), (size, sha.to_owned()));
                }
            }
        }
    }

    if !force {
        let same_ts = read_cached_json(&cache_dir.join("activity-config.json"))
            .is_some_and(|cached| cached.pointer("/data/assets_json_ts") == activity.pointer("/data/assets_json_ts"));
        let same_atlas = read_cached_json(&cache_dir.join("atlas.json"))
            .is_some_and(|cached| cached.get("data") == atlas.get("data"));
        let same_catalog = read_cached_json(&cache_dir.join("offline_catalog.json"))
            .is_some_and(|cached| cached == catalog);
        let all_files_present = !jobs.is_empty()
            && existing_records.len() == total
            && jobs.keys().all(|name| {
                existing_records
                    .get(name)
                    .and_then(|(size, _)| {
                        fs::metadata(cache_assets.join(name))
                            .ok()
                            .filter(|meta| meta.is_file() && meta.len() == *size && *size > 0)
                    })
                    .is_some()
            });
        if same_ts && same_atlas && same_catalog && all_files_present {
            on_progress(total, total);
            return Ok(SyncSummary {
                changed: false,
                atlas_count: atlas_rows.len(),
                asset_count: total,
            });
        }
    }

    let stage = root.join(format!(
        "cache_staging_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(stage.join("assets")).map_err(|error| error.to_string())?;
    let jobs = Arc::new(jobs.into_iter().collect::<Vec<_>>());
    let next = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let records = Arc::new(Mutex::new(Vec::<Value>::new()));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let existing_records = Arc::new(existing_records);
    let callback = Arc::new(on_progress);
    let cache_assets = Arc::new(cache_assets);
    callback(0, total);

    std::thread::scope(|scope| {
        for _ in 0..8.min(jobs.len().max(1)) {
            let client = client.clone();
            let jobs = Arc::clone(&jobs);
            let next = Arc::clone(&next);
            let completed = Arc::clone(&completed);
            let records = Arc::clone(&records);
            let failures = Arc::clone(&failures);
            let existing_records = Arc::clone(&existing_records);
            let callback = Arc::clone(&callback);
            let cache_assets = Arc::clone(&cache_assets);
            let assets = stage.join("assets");
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some((name, source)) = jobs.get(index) else {
                    break;
                };
                let source = source.as_str().unwrap();
                let target = assets.join(name);
                let reused = if !force {
                    reuse_cached_asset(&cache_assets, name, &target, existing_records.get(name))
                } else {
                    None
                };
                let outcome = match reused {
                    Some(hit) => Ok(hit),
                    None => download_one_with_retry(&client, source, &target),
                };
                match outcome {
                    Ok((size, sha256)) => records
                        .lock()
                        .unwrap()
                        .push(json!({"file":name,"source":source,"size":size,"sha256":sha256})),
                    Err(error) => failures.lock().unwrap().push(format!("{name}: {error}")),
                }
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                callback(done, total);
            });
        }
    });

    let failures = failures.lock().unwrap();
    if !failures.is_empty() {
        let _ = fs::remove_dir_all(&stage);
        return Err(format!(
            "{} 个素材下载失败：{}",
            failures.len(),
            failures[0]
        ));
    }
    let records = records.lock().unwrap().clone();
    for (name, value) in [
        ("activity-config.json", activity),
        ("atlas.json", atlas.clone()),
        ("offline_catalog.json", catalog),
        (
            "asset_manifest.json",
            json!({"records":records,"errors":[]}),
        ),
    ] {
        if let Err(error) = write_json(&stage.join(name), &value) {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
    }
    let summary = SyncSummary {
        changed: true,
        atlas_count: atlas_rows.len(),
        asset_count: records.len(),
    };
    atomic_replace(&root, &stage)?;
    Ok(summary)
}

/// Download and verify the complete selected-region catalog before replacing cache/.
pub fn sync(root: &Path, region: &str) -> Result<SyncSummary, String> {
    sync_with_progress(root, region, false, |_, _| {})
}

fn read_json_value(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub fn safe_relative_key(raw: &str) -> bool {
    !raw.is_empty()
        && !raw.contains(['\\', ':', '?', '#', '%'])
        && !raw.chars().any(char::is_control)
        && raw
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn contained_file(root: &Path, relative: &str) -> Option<PathBuf> {
    if !safe_relative_key(relative) {
        return None;
    }
    let base = root.canonicalize().ok()?;
    let file = root.join(relative).canonicalize().ok()?;
    (file.starts_with(base) && file.is_file()).then_some(file)
}

/// 返回本地图鉴图片缓存摘要（实际已下载张数与预期张数）。
pub fn cache_summary(root: &Path, expected_override: Option<u64>) -> Value {
    let manifest = read_json_value(&root.join("cache/asset_manifest.json"));
    let assets = manifest
        .as_ref()
        .and_then(|value| value["records"].as_array())
        .map(Vec::len)
        .unwrap_or(0);
    json!({
        "assets": assets,
        "expected": expected_override.unwrap_or(assets as u64),
    })
}

/// 检查本地离线图鉴目录（JSON 与全部 PNG 缓存）是否完整就绪。
pub fn is_cache_ready(root: &Path, expected_override: Option<usize>) -> bool {
    let cache_dir = root.join("cache");
    if !cache_dir.join("offline_catalog.json").is_file() || !cache_dir.join("atlas.json").is_file()
    {
        return false;
    }
    let Some(manifest) = read_json_value(&cache_dir.join("asset_manifest.json")) else {
        return false;
    };
    let Some(records) = manifest["records"].as_array() else {
        return false;
    };
    if records.is_empty()
        || manifest["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    {
        return false;
    }
    if expected_override.is_some_and(|expected| expected > 0 && records.len() != expected) {
        return false;
    }
    let assets_dir = cache_dir.join("assets");
    records.iter().all(|record| {
        let Some(name) = record["file"].as_str() else {
            return false;
        };
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            return false;
        }
        let Some(expected_size) = record["size"].as_u64() else {
            return false;
        };
        fs::metadata(assets_dir.join(name))
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected_size)
    })
}

/// 解析离线缓存里的官方资源文件路径（支持 `assets/<sha>.png` 与官方按名称分类路径）。
pub fn resolve_cached_asset(cache_dir: &Path, relative_key: &str) -> Option<PathBuf> {
    let trimmed = relative_key.trim_start_matches('/');
    let trimmed = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    if !safe_relative_key(trimmed) {
        return None;
    }
    let assets_dir = cache_dir.join("assets");
    if let Some(name) = trimmed.strip_prefix("assets/") {
        if name.contains('/') {
            return None;
        }
        return contained_file(&assets_dir, name);
    }
    let is_category = [
        "character/",
        "accessory/",
        "emote/",
        "emote_atlas/",
        "character_atlas/",
        "character_skin_atlas/",
        "accessory_atlas/",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix));
    if !is_category {
        return None;
    }
    let requested = trimmed.rsplit('/').next().filter(|s| !s.is_empty())?;
    // The official renderer turns cached catalog keys such as
    // `assets/<sha256>.png` into `<category>/<sha256>.png` when it builds
    // pet/backpack image URLs. The cache file itself is already named by
    // that digest, so resolve it before consulting the source manifest.
    if !requested.contains('/') {
        if let Some(file) = contained_file(&assets_dir, requested) {
            return Some(file);
        }
    }
    let manifest = read_json_value(&cache_dir.join("asset_manifest.json"))?;
    let records = manifest["records"].as_array()?;
    fn clean_source(s: &str) -> &str {
        s.split(['?', '#']).next().unwrap_or(s)
    }
    let suffix = format!("/{trimmed}");
    let record = records
        .iter()
        .find(|record| {
            record["source"]
                .as_str()
                .is_some_and(|source| clean_source(source).ends_with(&suffix))
        })
        .or_else(|| {
            records.iter().find(|record| {
                record["source"]
                    .as_str()
                    .and_then(|source| clean_source(source).rsplit('/').next())
                    == Some(requested)
            })
        });
    if let Some(file) = record
        .and_then(|record| record["file"].as_str())
        .and_then(|file| contained_file(&assets_dir, file))
    {
        return Some(file);
    }
    resolve_named_catalog_asset(cache_dir, trimmed)
}

/// Resolve names emitted by the official renderer (for example
/// `character/role_1304_1007_body_1.png`). The download manifest is keyed by
/// official UUID URLs, while the renderer often composes paths from atlas
/// names. The cached atlas + catalog contain the authoritative mapping.
fn resolve_named_catalog_asset(cache_dir: &Path, relative_key: &str) -> Option<PathBuf> {
    let (category, requested) = relative_key.split_once('/')?;
    let category = category.to_ascii_lowercase();
    let stem = requested
        .strip_suffix(".png")
        .or_else(|| requested.strip_suffix(".webp"))?;
    let (atlas_type, primary_type) = match category.as_str() {
        "character" | "character_atlas" => ("character_skin_atlas", "character_skin"),
        "accessory" | "accessory_atlas" => ("accessory_atlas", "accessory"),
        "emote" | "emote_atlas" => ("emote_atlas", "emote"),
        _ => return None,
    };
    let atlas_doc = read_json_value(&cache_dir.join("atlas.json"))?;
    let rows = atlas_doc.get("data")?.as_array()?;
    let (row, base) = rows
        .iter()
        .filter(|row| row["asset_type"].as_str() == Some(atlas_type))
        .filter_map(|row| {
            let base = row["asset_name"].as_str()?.strip_suffix(".png")?;
            (stem == base || stem.starts_with(&format!("{base}_")))
                .then_some((row, base))
        })
        .max_by_key(|(_, base)| base.len())?;
    let atlas_id = row["asset_id"].as_u64()?;
    let catalog = read_json_value(&cache_dir.join("offline_catalog.json"))?;
    let atlas_meta = catalog.get(atlas_type)?.get(atlas_id.to_string())?;
    let url_key = if category.ends_with("_atlas") {
        "url".to_owned()
    } else if primary_type == "character_skin" {
        match stem.strip_prefix(base).unwrap_or_default() {
            "" => "url".to_owned(),
            suffix if suffix.starts_with("_body_") => format!("body{}", &suffix[6..]),
            suffix if suffix.starts_with("_head_") => format!("head{}", &suffix[6..]),
            _ => return None,
        }
    } else {
        "url".to_owned()
    };
    let id = atlas_meta["height_light_id"].as_u64().unwrap_or(atlas_id);
    let meta = catalog
        .get(if category.ends_with("_atlas") { atlas_type } else { primary_type })?
        .get(if category.ends_with("_atlas") { atlas_id } else { id }.to_string())?;
    let target = meta.get(&url_key)?;
    let asset = target
        .as_str()
        .or_else(|| target.get("url").and_then(Value::as_str))?;
    let file = asset.strip_prefix("assets/")?;
    contained_file(&cache_dir.join("assets"), file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloader_only_accepts_the_selected_official_region() {
        let root = std::env::temp_dir().join("wuwatappo-resource-cache-invalid-region");
        assert!(sync(&root, "other").unwrap_err().contains("区域"));
        assert!(safe_url(
            "http://mc-wuwatappo.kurogames-global.com/a",
            "mc-wuwatappo.kurogames-global.com"
        )
        .is_err());
        assert!(safe_url("https://example.com/a", "mc-wuwatappo.kurogames-global.com").is_err());
    }

    #[test]
    fn catalog_urls_are_rewritten_to_deterministic_local_keys() {
        let base = Url::parse("https://mc-wuwatappo.kurogames-global.com/").unwrap();
        let mut catalog = json!({"character_skin":{"1":{"url":"/assets/hero.png"}}});
        let mut jobs = Map::new();
        rewrite_urls(&mut catalog, &base, &mut jobs).unwrap();
        let key = format!(
            "{}.png",
            hash_hex(b"https://mc-wuwatappo.kurogames-global.com/assets/hero.png")
        );
        assert_eq!(
            catalog["character_skin"]["1"]["url"],
            format!("assets/{key}")
        );
        assert_eq!(
            jobs[&key],
            "https://mc-wuwatappo.kurogames-global.com/assets/hero.png"
        );
        let mut external = json!({"url":"https://example.com/asset.png"});
        assert!(rewrite_urls(&mut external, &base, &mut Map::new()).is_err());
    }

    #[test]
    fn catalog_validation_accepts_numeric_or_string_ids_and_rejects_broken_links() {
        let valid_numeric = json!({
            "atlas": [{"asset_type": "character_skin_atlas", "asset_id": 43}],
            "catalog": {
                "character_skin": {"42": {"url": "a.png"}},
                "character_skin_atlas": {"43": {"height_light_id": 42}},
                "accessory": {},
                "accessory_atlas": {},
                "emote": {},
                "emote_atlas": {}
            }
        });
        assert!(validate_catalog(&valid_numeric).is_ok());

        let valid_string = json!({
            "atlas": [{"asset_type": "emote_atlas", "asset_id": "101"}],
            "catalog": {
                "character_skin": {},
                "character_skin_atlas": {},
                "accessory": {},
                "accessory_atlas": {},
                "emote": {"202": {"url": "b.png"}},
                "emote_atlas": {"101": {"height_light_id": "202"}}
            }
        });
        assert!(validate_catalog(&valid_string).is_ok());

        let mut broken = valid_numeric.clone();
        broken["catalog"]["character_skin"]
            .as_object_mut()
            .unwrap()
            .remove("42");
        assert!(validate_catalog(&broken).is_err());
    }

    #[test]
    fn resolve_cached_asset_strips_version_queries_and_prefers_category_match() {
        let root = std::env::temp_dir().join(format!("wuwatappo-resolve-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let cache = root.join("cache");
        let assets = cache.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("skin.png"), b"skin").unwrap();
        fs::write(assets.join("atlas.png"), b"atlas").unwrap();
        fs::write(assets.join("char.png"), b"char").unwrap();
        fs::write(
            cache.join("asset_manifest.json"),
            json!({
                "records": [
                    {
                        "file": "atlas.png",
                        "source": "https://mc-wuwatappo.kurogames-global.com/character_atlas/same.png?v=1791270689726"
                    },
                    {
                        "file": "skin.png",
                        "source": "https://mc-wuwatappo.kurogames-global.com/character/same.png?v=1791270689726#frag"
                    },
                    {
                        "file": "char.png",
                        "source": "https://mc-wuwatappo.kurogames-global.com/character/41dd9bb6-2191-405b-be64-e09cb969add6.png?v=1791270689726"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let canon = |name: &str| fs::canonicalize(assets.join(name)).ok();
        assert_eq!(
            resolve_cached_asset(&cache, "character/41dd9bb6-2191-405b-be64-e09cb969add6.png"),
            canon("char.png")
        );
        assert_eq!(
            resolve_cached_asset(&cache, "character/41dd9bb6-2191-405b-be64-e09cb969add6.png?v=999"),
            canon("char.png")
        );
        assert_eq!(
            resolve_cached_asset(&cache, "character/same.png"),
            canon("skin.png")
        );
        assert_eq!(
            resolve_cached_asset(&cache, "character_atlas/same.png"),
            canon("atlas.png")
        );
        fs::write(assets.join("0123456789abcdef.png"), b"digest").unwrap();
        fs::write(assets.join("body1.png"), b"body1").unwrap();
        fs::write(assets.join("icon.png"), b"icon").unwrap();
        fs::write(
            cache.join("atlas.json"),
            serde_json::to_vec(&json!({"data":[{
                "asset_id": 20,
                "asset_type": "character_skin_atlas",
                "asset_name": "role_1304_1007.png"
            }]}))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            cache.join("offline_catalog.json"),
            serde_json::to_vec(&json!({
                "character_skin": {"890": {"url":"assets/char.png","body1":{"url":"assets/body1.png"}}},
                "character_skin_atlas": {"20": {"height_light_id":890,"url":"assets/icon.png"}}
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_cached_asset(&cache, "character/0123456789abcdef.png"),
            canon("0123456789abcdef.png")
        );
        let composed = format!("character/role_{}_{}_body_1.png", "1304", "1007");
        assert_eq!(resolve_cached_asset(&cache, &composed), canon("body1.png"));
        assert_eq!(
            resolve_cached_asset(&cache, "character_atlas/role_1304_1007.png"),
            canon("icon.png")
        );
        let _ = fs::remove_dir_all(&root);
    }
}
