//! Verified, transactional download of the public atlas and its image cache.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
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

fn atomic_replace(root: &Path, stage: &Path) -> Result<(), String> {
    let cache = root.join("cache");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let previous = root.join(format!("cache_previous_{stamp}"));
    if cache.exists() {
        fs::rename(&cache, &previous).map_err(|error| format!("无法保留旧素材缓存：{error}"))?;
    }
    if let Err(error) = fs::rename(stage, &cache) {
        if previous.exists() {
            let _ = fs::rename(&previous, &cache);
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
    let (api_base, assets_host) = match region {
        "cn" => (
            "https://api-mc-wuwatappo.kurogames-global.com",
            "https://mc-wuwatappo.kurogames-global.com",
        ),
        "global" => (
            "https://api-wutheringwaves-wuwatappo.kurogames-global.com",
            "https://wutheringwaves-wuwatappo.kurogames-global.com",
        ),
        _ => return Err("未知官方服务区域".into()),
    };
    let api = Url::parse(api_base).map_err(|_| "官方 API 地址无效")?;
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
    let kinds = ["character_skin", "accessory", "emote"];
    for kind in kinds {
        if !catalog[kind].is_object() || !catalog[format!("{kind}_atlas")].is_object() {
            return Err("新版素材结构不兼容，保留旧缓存".into());
        }
    }
    let atlas_rows = atlas["data"].as_array().unwrap();
    for row in atlas_rows {
        let kind = row["asset_type"].as_str().ok_or("图鉴条目缺少类型")?;
        let id = row["asset_id"]
            .as_u64()
            .ok_or("图鉴条目 ID 无效")?
            .to_string();
        if !kinds.contains(&kind) {
            return Err("图鉴条目类型不受支持".into());
        }
        let entry = catalog[kind]
            .get(&id)
            .ok_or("图鉴到资源映射缺失，保留旧缓存")?;
        let highlight = entry["height_light_id"]
            .as_str()
            .ok_or("图鉴资源映射缺少外观 ID")?;
        if !catalog[kind.trim_end_matches("_atlas")]
            .get(highlight)
            .is_some()
        {
            return Err("图鉴到外观映射缺失，保留旧缓存".into());
        }
    }
    Ok(OfficialCatalog {
        activity,
        atlas,
        catalog,
        assets_host: host,
        client,
    })
}

/// Download and verify the complete selected-region catalog before replacing cache/.
pub fn sync(root: &Path, region: &str) -> Result<SyncSummary, String> {
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
    let stage = root.join(format!(
        "cache_staging_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(stage.join("assets")).map_err(|error| error.to_string())?;
    let mut jobs = Map::new();
    rewrite_urls(&mut catalog, &host, &mut jobs)?;
    let jobs = Arc::new(jobs.into_iter().collect::<Vec<_>>());
    let next = Arc::new(AtomicUsize::new(0));
    let records = Arc::new(Mutex::new(Vec::<Value>::new()));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut workers = Vec::new();
    for _ in 0..8.min(jobs.len().max(1)) {
        let client = client.clone();
        let jobs = Arc::clone(&jobs);
        let next = Arc::clone(&next);
        let records = Arc::clone(&records);
        let failures = Arc::clone(&failures);
        let assets = stage.join("assets");
        workers.push(thread::spawn(move || loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some((name, source)) = jobs.get(index) else {
                break;
            };
            let source = source.as_str().unwrap();
            match download_one(&client, source, &assets.join(name)) {
                Ok((size, sha256)) => records
                    .lock()
                    .unwrap()
                    .push(json!({"file":name,"source":source,"size":size,"sha256":sha256})),
                Err(error) => failures.lock().unwrap().push(format!("{name}: {error}")),
            }
        }));
    }
    for worker in workers {
        let _ = worker.join();
    }
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
}
