//! User appearance Mods. The official catalog remains the immutable base.
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zip::ZipArchive;

const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 5000;

#[derive(Clone, Debug)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub item_count: usize,
    pub asset_count: usize,
}

fn text(value: Option<&Value>, label: &str, max: usize, pattern: bool) -> Result<String, String> {
    let value = value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or(format!("{label} 无效"))?;
    if value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("{label} 无效"));
    }
    if pattern
        && (value.len() > max
            || !value.as_bytes()[0].is_ascii_alphanumeric()
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-'))
    {
        return Err(format!("{label} 只能使用字母、数字、点、下划线和短横线"));
    }
    Ok(value.to_owned())
}
fn asset_path(value: Option<&Value>) -> Result<String, String> {
    let raw = value.and_then(Value::as_str).ok_or("图片路径必须是文本")?;
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains(['\\', '#', '?', ':'])
        || raw.chars().any(char::is_control)
    {
        return Err("资源 URL 必须是包内相对路径".into());
    }
    let path = raw;
    if path
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':'))
    {
        return Err("资源路径包含不允许的目录段".into());
    }
    Ok(if path.starts_with("assets/") {
        path.to_owned()
    } else {
        format!("assets/{path}")
    })
}
fn json_file(path: &Path, name: &str) -> Result<Value, String> {
    let meta = fs::metadata(path).map_err(|_| format!("资源包缺少 {name}"))?;
    if meta.len() > MAX_JSON {
        return Err(format!("{name} 超过 16 MiB"));
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| format!("{name} 不是有效 UTF-8 JSON"))
}
fn image_ok(path: &Path, bytes: &[u8]) -> bool {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        _ => false,
    }
}
fn unsafe_zip_part(part: &str) -> bool {
    let base = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    part.is_empty()
        || part == "."
        || part == ".."
        || part.ends_with([' ', '.'])
        || part.contains(['<', '>', ':', '"', '|', '?', '*'])
        || part.chars().any(char::is_control)
        || matches!(base.as_str(), "con" | "prn" | "aux" | "nul")
        || ((base.starts_with("com") || base.starts_with("lpt"))
            && base.len() == 4
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}
fn parse_pack(root: &Path, region: &str) -> Result<(Summary, Value), String> {
    let manifest = json_file(&root.join("manifest.json"), "manifest.json")?;
    if manifest.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("资源包 schemaVersion 必须是 1".into());
    }
    let name = text(manifest.get("name"), "资源包名称", 80, false)?;
    let id = format!("mod-{:016x}", hash(&name));
    let pack_region = manifest
        .get("region")
        .and_then(Value::as_str)
        .unwrap_or("ANY");
    if !matches!(pack_region, "ANY" | "CN" | "GLOBAL")
        || (pack_region != "ANY" && pack_region != region)
    {
        return Err(format!("资源包属于 {pack_region}，当前客户端是 {region}"));
    }
    let doc = json_file(&root.join("items.json"), "items.json")?;
    let rows = doc
        .get("items")
        .and_then(Value::as_array)
        .ok_or("items.json 必须包含 items 数组")?;
    if rows.is_empty() || rows.len() > MAX_ITEMS {
        return Err(format!("资源包必须包含 1 到 {MAX_ITEMS} 条资源"));
    }
    let mut ids = HashSet::new();
    let mut urls = HashSet::new();
    let mut items = Vec::new();
    for row in rows {
        let kind = row
            .get("type")
            .and_then(Value::as_str)
            .ok_or("资源条目 type 缺失")?;
        if !matches!(kind, "character" | "accessory" | "emote") {
            return Err("资源条目 type 无效".into());
        }
        let item_name = text(row.get("name"), "资源名称", 180, false)?;
        if !ids.insert((kind.to_owned(), item_name.clone())) {
            return Err(format!("同一资源包内存在同类型同名资源：{item_name}"));
        }
        let item_id = format!("item-{:016x}", hash(&format!("{kind}\0{item_name}")));
        let assets = row
            .get("assets")
            .and_then(Value::as_object)
            .ok_or("资源条目缺少 assets")?;
        let required: &[&str] = match kind {
            "character" => &["body1", "head1"],
            _ => &["image"],
        };
        for key in required {
            if !assets.contains_key(*key) {
                return Err(format!("{kind} 缺少 {key} 图片"));
            }
        }
        let mut normalized = Map::new();
        for (key, value) in assets {
            if !matches!(
                key.as_str(),
                "icon" | "preview" | "image" | "body1" | "head1" | "body2" | "head2" | "body3"
            ) {
                return Err(format!("资源条目包含未知图片字段：{key}"));
            }
            let path = asset_path(Some(value))?;
            if !root.join(&path).is_file() {
                return Err(format!("资源包缺少图片：{path}"));
            }
            urls.insert(path.clone());
            normalized.insert(key.clone(), Value::String(path));
        }
        items.push(json!({"id":item_id,"type":kind,"name":item_name,"assets":normalized}));
    }
    items.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok((
        Summary {
            id: id.clone(),
            name: name.clone(),
            item_count: items.len(),
            asset_count: urls.len(),
        },
        json!({"id":id,"name":name,"region":pack_region,"items":items,"root":root.to_string_lossy()}),
    ))
}
pub fn summary(root: &Path, region: &str) -> Result<Summary, String> {
    parse_pack(root, region).map(|v| v.0)
}
pub fn list_summaries(root: &Path, region: &str) -> Result<Vec<Summary>, String> {
    let mut directories = match fs::read_dir(root) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    directories.sort();
    let mut summaries = Vec::new();
    for path in directories {
        if path.is_dir()
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
        {
            if let Ok(pack) = summary(&path, region) {
                summaries.push(pack);
            }
        }
    }
    Ok(summaries)
}
/// 管理面板用：每个包的条目明细 + 可直接渲染的缩略图引用。
/// 引用格式与 catalog 里的 `url` 完全一致，前端用同一套解析还原成本地文件。
pub fn list_details(root: &Path, region: &str) -> Result<Vec<Value>, String> {
    let mut directories = match fs::read_dir(root) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    directories.sort();
    let mut packs = Vec::new();
    for path in directories {
        if !path.is_dir()
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let Ok((summary, pack)) = parse_pack(&path, region) else {
            continue;
        };
        let pack_id = pack["id"].as_str().unwrap_or_default();
        let mut items = Vec::new();
        for item in pack["items"].as_array().into_iter().flatten() {
            let assets = item["assets"].as_object();
            let primary = ["icon", "preview", "image", "body1"]
                .iter()
                .find_map(|key| assets.and_then(|assets| assets.get(*key)))
                .and_then(Value::as_str)
                .unwrap_or("assets/missing.png");
            items.push(json!({
                "id": item["id"],
                "name": item["name"],
                "type": item["type"],
                "icon": format!("custom-assets/{pack_id}/{}", primary.trim_start_matches("assets/")),
            }));
        }
        packs.push(json!({
            "id": summary.id,
            "name": summary.name,
            "item_count": summary.item_count,
            "asset_count": summary.asset_count,
            "items": items,
        }));
    }
    Ok(packs)
}
pub fn remove_pack(root: &Path, pack_id: &str) -> Result<(), String> {
    text(Some(&json!(pack_id)), "Mod ID", 64, true)?;
    let target = root.join(pack_id);
    if !target.is_dir() || target.parent() != Some(root) {
        return Err("未找到该 Mod".into());
    }
    fs::remove_dir_all(target).map_err(|e| e.to_string())
}
fn hash(seed: &str) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for b in seed.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}
fn stable_id(seed: &str, used: &mut HashSet<u64>) -> u64 {
    let mut id = 1_000_000_000 + hash(seed) % 9_007_198_254_740_992;
    while !used.insert(id) {
        id = if id >= 9_007_199_254_740_991 {
            1_000_000_000
        } else {
            id + 1
        };
    }
    id
}
fn used_ids(official: &Value) -> HashSet<u64> {
    let mut used = HashSet::new();
    if let Some(c) = official.get("catalog").and_then(Value::as_object) {
        for v in c.values().filter_map(Value::as_object) {
            for id in v.keys().filter_map(|v| v.parse().ok()) {
                used.insert(id);
            }
        }
    }
    if let Some(a) = official.get("atlas").and_then(Value::as_array) {
        for row in a {
            if let Some(id) = row.get("asset_id").and_then(Value::as_u64) {
                used.insert(id);
            }
        }
    }
    used
}
fn materialize(
    pack: &Value,
    used: &mut HashSet<u64>,
) -> (Vec<Value>, Map<String, Value>, Map<String, Value>) {
    let id = pack["id"].as_str().unwrap();
    let name = pack["name"].as_str().unwrap();
    let root = PathBuf::from(pack["root"].as_str().unwrap());
    let mut atlas = Vec::new();
    let mut catalog = Map::new();
    for k in [
        "character_skin",
        "character_skin_atlas",
        "accessory",
        "accessory_atlas",
        "emote",
        "emote_atlas",
    ] {
        catalog.insert(k.to_owned(), Value::Object(Map::new()));
    }
    let mut paths = Map::new();
    for item in pack["items"].as_array().unwrap() {
        let iid = item["id"].as_str().unwrap();
        let kind = item["type"].as_str().unwrap();
        let cat_kind = if kind == "character" {
            "character_skin"
        } else {
            kind
        };
        let aid = stable_id(&format!("{id}\0{kind}\0{iid}"), used);
        let atlas_id = stable_id(&format!("{id}\0{cat_kind}_atlas\0{iid}"), used);
        let assets = item["assets"].as_object().unwrap();
        let primary = assets
            .get(if kind == "character" {
                "preview"
            } else {
                "image"
            })
            .or_else(|| assets.get("icon"))
            .or_else(|| assets.get("body1"))
            .expect("validated item has a primary image");
        let primary_rel = asset_path(Some(primary)).unwrap();
        let url = format!(
            "custom-assets/{id}/{}",
            primary_rel.trim_start_matches("assets/")
        );
        for value in assets.values() {
            let rel = asset_path(Some(value)).unwrap();
            paths.insert(
                format!("custom-assets/{id}/{}", rel.trim_start_matches("assets/")),
                Value::String(root.join(rel).to_string_lossy().to_string()),
            );
        }
        let mut entry = json!({"game_role_id":0,"url":url,"offset_x":0,"offset_y":0,"my_offset_x":0,"my_offset_y":0});
        if kind == "character" {
            if let Some(obj) = entry.as_object_mut() {
                for layer in ["body1", "head1", "body2", "head2", "body3"] {
                    if let Some(value) = assets.get(layer) {
                        let rel = asset_path(Some(value)).unwrap();
                        obj.insert(layer.to_owned(),json!({"url":format!("custom-assets/{id}/{}",rel.trim_start_matches("assets/")),"offset_x":0,"offset_y":0,"my_offset_x":0,"my_offset_y":0}));
                    }
                }
                // 渲染器切换动作帧时不回退：缺帧会拿到空 URL，那一瞬间整层消失
                // （表现为每 5 秒眨眼时头部闪一下）。用基础帧补齐，等价于动作时保持静止。
                for (action, base) in [("body2", "body1"), ("body3", "body1"), ("head2", "head1")] {
                    if !obj.contains_key(action) {
                        if let Some(value) = obj.get(base).cloned() {
                            obj.insert(action.to_owned(), value);
                        }
                    }
                }
            }
        }
        catalog
            .get_mut(cat_kind)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(aid.to_string(), entry);
        let icon_rel = asset_path(assets.get("icon").or(Some(primary))).unwrap();
        let icon_url = format!(
            "custom-assets/{id}/{}",
            icon_rel.trim_start_matches("assets/")
        );
        catalog.get_mut(&format!("{cat_kind}_atlas")).unwrap().as_object_mut().unwrap().insert(atlas_id.to_string(),json!({"height_light_id":aid,"game_role_id":0,"url":icon_url,"offset_x":0,"offset_y":0,"my_offset_x":0,"my_offset_y":0}));
        atlas.push(json!({"asset_id":atlas_id,"asset_name":item["name"],"display_name":item["name"],"asset_code":format!("mod:{id}:{iid}"),"asset_type":format!("{cat_kind}_atlas"),"star_level":0,"region_code":"","series_name":name,"rarity":"MOD","custom_pack_id":id,"custom_item_id":iid}));
    }
    (atlas, catalog, paths)
}
pub fn merge_official(official: &Value, root: &Path, region: &str) -> Result<Value, String> {
    // 先收集包，再决定要不要碰官方数据。没有 Mod 的用户走的是纯透传路径：
    // 官方结构怎么变都不该因为「Mod 叠加」这一步失败。
    let mut packs = Vec::new();
    if let Ok(directories) = fs::read_dir(root) {
        let mut directories = directories
            .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        directories.sort();
        for dir in directories {
            if !dir.is_dir() {
                continue;
            }
            if dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            if let Ok((_, pack)) = parse_pack(&dir, region) {
                packs.push(pack);
            }
        }
    }
    let mut merged = official.clone();
    if packs.is_empty() {
        merged["atlas_count"] = json!(merged["atlas"].as_array().map(Vec::len).unwrap_or(0));
        return Ok(merged);
    }
    for kind in [
        "character_skin",
        "character_skin_atlas",
        "accessory",
        "accessory_atlas",
        "emote",
        "emote_atlas",
    ] {
        if !official["catalog"][kind].is_object() || !official["atlas"].is_array() {
            return Err("官方图鉴缓存不完整，无法叠加 Mod".into());
        }
    }
    let mut used = used_ids(official);
    let mut paths = Map::new();
    let mut packs_meta = Vec::new();
    let mut addon_atlas = Vec::new();
    let mut addon_catalog = Map::new();
    for k in [
        "character_skin",
        "character_skin_atlas",
        "accessory",
        "accessory_atlas",
        "emote",
        "emote_atlas",
    ] {
        addon_catalog.insert(k.to_owned(), Value::Object(Map::new()));
    }
    for pack in &packs {
        let (atlas, catalog, map) = materialize(pack, &mut used);
        addon_atlas.extend(atlas);
        for (k, v) in catalog {
            addon_catalog
                .get_mut(&k)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .extend(v.as_object().unwrap().clone());
        }
        paths.extend(map);
        packs_meta.push(json!({"id":pack["id"],"name":pack["name"],"item_count":pack["items"].as_array().map(Vec::len).unwrap_or(0),"asset_count":count_assets(pack)}));
    }
    if let Some(a) = merged.get_mut("atlas").and_then(Value::as_array_mut) {
        a.extend(addon_atlas);
    }
    if let Some(c) = merged.get_mut("catalog").and_then(Value::as_object_mut) {
        for (k, v) in addon_catalog {
            c.get_mut(&k)
                .and_then(Value::as_object_mut)
                .unwrap()
                .extend(v.as_object().unwrap().clone());
        }
    }
    merged["customAssetPaths"] = Value::Object(paths);
    merged["custom_resources"] = json!({"packs":packs_meta});
    merged["atlas_count"] = json!(merged["atlas"].as_array().map(Vec::len).unwrap_or(0));
    Ok(merged)
}

fn count_assets(pack: &Value) -> usize {
    let mut seen = HashSet::new();
    if let Some(items) = pack["items"].as_array() {
        for item in items {
            if let Some(assets) = item["assets"].as_object() {
                for value in assets.values().filter_map(Value::as_str) {
                    seen.insert(value.to_owned());
                }
            }
        }
    }
    seen.len()
}
pub fn install_archive(
    archive_path: &Path,
    destination: &Path,
    region: &str,
) -> Result<Summary, String> {
    if archive_path
        .extension()
        .and_then(|v| v.to_str())
        .map(|v| v.to_ascii_lowercase())
        != Some("zip".into())
    {
        return Err("请选择 ZIP 资源包".into());
    }
    if !archive_path.is_absolute() || !archive_path.is_file() {
        return Err("请选择有效的 ZIP 资源包".into());
    }
    if fs::metadata(archive_path).map_err(|e| e.to_string())?.len() > MAX_ARCHIVE {
        return Err("ZIP 资源包不能超过 512 MiB".into());
    }
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let stage = destination.join(format!(
        ".stage-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut zip = ZipArchive::new(File::open(archive_path).map_err(|e| e.to_string())?)
            .map_err(|_| "ZIP 资源包无法读取".to_owned())?;
        if zip.len() > MAX_ENTRIES {
            return Err("ZIP 文件数量超过限制".into());
        }
        let mut seen = HashSet::new();
        let mut expanded = 0;
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).map_err(|_| "ZIP 文件目录损坏".to_owned())?;
            let raw = e.name().trim_end_matches('/').to_owned();
            if raw.starts_with('/') || raw.contains('\\') || raw.split('/').any(unsafe_zip_part) {
                return Err("ZIP 中包含不安全路径".into());
            }
            if !seen.insert(raw.to_lowercase()) {
                return Err("ZIP 中存在重复文件路径".into());
            }
            if e.unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("ZIP 不允许包含符号链接".into());
            }
            if e.is_dir()
                || raw.starts_with("__MACOSX/")
                || raw.rsplit('/').next() == Some(".DS_Store")
            {
                continue;
            }
            if raw != "manifest.json" && raw != "items.json" && !raw.starts_with("assets/") {
                return Err(format!("ZIP 包含未知文件：{raw}"));
            }
            if matches!(raw.as_str(), "manifest.json" | "items.json") && e.size() > MAX_JSON {
                return Err(format!("{raw} 超过 16 MiB"));
            }
            expanded += e.size();
            if expanded > MAX_EXPANDED {
                return Err("ZIP 解压后不能超过 1 GiB".into());
            }
            if e.size() > 16 * 1024 * 1024
                && (e.compressed_size() == 0 || e.size() / e.compressed_size().max(1) > 1000)
            {
                return Err("ZIP 压缩率超过安全限制".into());
            }
            let target = stage.join(&raw);
            if let Some(p) = target.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(&target).map_err(|e| e.to_string())?;
            let mut prefix = [0u8; 16];
            let count = e.read(&mut prefix).map_err(|e| e.to_string())?;
            if raw.starts_with("assets/") && !image_ok(&target, &prefix[..count]) {
                return Err(format!("资源文件不是受支持的图片：{raw}"));
            }
            out.write_all(&prefix[..count]).map_err(|e| e.to_string())?;
            let copied = std::io::copy(&mut e, &mut out).map_err(|e| e.to_string())?;
            if count as u64 + copied != e.size() {
                return Err("ZIP 解压内容大小无效".into());
            }
        }
        let (summary, _) = parse_pack(&stage, region)?;
        Ok(summary)
    })();
    match result {
        Ok(summary) => {
            let target = destination.join(&summary.id);
            let backup = destination.join(format!(".{}.previous", summary.id));
            if target.exists() {
                let _ = fs::remove_dir_all(&backup);
                fs::rename(&target, &backup).map_err(|e| e.to_string())?;
            }
            if let Err(error) = fs::rename(&stage, &target) {
                if backup.exists() {
                    let _ = fs::rename(&backup, &target);
                }
                let _ = fs::remove_dir_all(&stage);
                return Err(error.to_string());
            }
            let _ = fs::remove_dir_all(backup);
            Ok(summary)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(stage);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "custom-mod-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_pack(root: &Path, id: &str, region: &str, items: Value) {
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(
            root.join("manifest.json"),
            json!({"schemaVersion":1,"name":id,"region":region}).to_string(),
        )
        .unwrap();
        fs::write(root.join("items.json"), json!({"items":items}).to_string()).unwrap();
        for (item, path) in [
            ("hero", "assets/body.png"),
            ("hero", "assets/head.png"),
            ("ears", "assets/ears.png"),
            ("happy", "assets/happy.png"),
        ] {
            let _ = item;
            if items_has_path(&root.join("items.json"), path) {
                fs::write(root.join(path), b"\x89PNG\r\n\x1a\nfixture").unwrap();
            }
        }
    }

    fn items_has_path(path: &Path, file: &str) -> bool {
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        value["items"].as_array().unwrap().iter().any(|item| {
            item["assets"]
                .as_object()
                .unwrap()
                .values()
                .any(|value| value.as_str() == Some(file))
        })
    }

    fn official() -> Value {
        let mut catalog = Map::new();
        for kind in [
            "character_skin",
            "character_skin_atlas",
            "accessory",
            "accessory_atlas",
            "emote",
            "emote_atlas",
        ] {
            catalog.insert(kind.into(), Value::Object(Map::new()));
        }
        catalog
            .get_mut("character_skin")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("42".into(), json!({"url":"assets/official.png"}));
        catalog
            .get_mut("character_skin_atlas")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("43".into(), json!({"height_light_id":42}));
        json!({"atlas":[{"asset_id":43,"asset_type":"character_skin_atlas","asset_name":"official","display_name":"Official"}],"catalog":catalog,"assets_host":"official","asset_count":1})
    }

    fn sample_items() -> Value {
        json!([
            {"type":"character","name":"Hero","assets":{"body1":"assets/body.png","head1":"assets/head.png"}},
            {"type":"accessory","name":"Ears","assets":{"image":"assets/ears.png"}},
            {"type":"emote","name":"Happy","assets":{"image":"assets/happy.png"}}
        ])
    }

    #[test]
    fn mods_overlay_official_data_and_leave_the_official_snapshot_unchanged() {
        let root = temp_dir("merge");
        let one = root.join("mods/one");
        let two = root.join("mods/two");
        write_pack(&one, "pack-one", "ANY", sample_items());
        write_pack(
            &two,
            "pack-two",
            "ANY",
            json!([{"id":"solo","type":"emote","name":"Solo","assets":{"image":"assets/happy.png"}}]),
        );
        let original = official();
        let before = original.clone();
        let merged = merge_official(&original, &root.join("mods"), "CN").unwrap();
        assert_eq!(original, before);
        assert_eq!(merged["atlas"].as_array().unwrap().len(), 5);
        assert_eq!(
            merged["catalog"]["character_skin"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(merged["catalog"]["accessory"].as_object().unwrap().len(), 1);
        assert_eq!(merged["catalog"]["emote"].as_object().unwrap().len(), 2);
        assert_eq!(merged["atlas"][0]["asset_id"], 43);
        assert_eq!(
            merged["atlas"][1]["rarity"], "MOD",
            "背包装配用 rarity 生成 atlas-thumb--mod 容器"
        );
        assert!(merged["customAssetPaths"]
            .as_object()
            .unwrap()
            .contains_key(&format!(
                "custom-assets/mod-{:016x}/body.png",
                hash("pack-one")
            )));
        // 只给了 body1/head1 的角色也要能切换动作帧：缺帧会让渲染器拿到空 URL，
        // 表现为每 5 秒眨眼时头部闪一下。这里断言动作帧被基础帧补齐。
        let hero = merged["catalog"]["character_skin"]
            .as_object()
            .unwrap()
            .values()
            .find(|entry| entry["body1"].is_object())
            .unwrap()
            .clone();
        for action in ["body2", "body3"] {
            assert_eq!(
                hero[action]["url"], hero["body1"]["url"],
                "{} 应回退到 body1",
                action
            );
        }
        assert_eq!(
            hero["head2"]["url"], hero["head1"]["url"],
            "head2 应回退到 head1"
        );
        let again = merge_official(&original, &root.join("mods"), "CN").unwrap();
        assert_eq!(merged, again);
        let mut reordered = sample_items().as_array().unwrap().clone();
        reordered.reverse();
        write_pack(&one, "pack-one", "ANY", json!(reordered));
        assert_eq!(
            merge_official(&original, &root.join("mods"), "CN").unwrap(),
            merged
        );
        fs::remove_dir_all(&one).unwrap();
        let remaining = merge_official(&original, &root.join("mods"), "CN").unwrap();
        assert_eq!(remaining["atlas"].as_array().unwrap().len(), 2);
        assert_eq!(remaining["atlas"][0], original["atlas"][0]);
        assert_eq!(
            remaining["atlas"][1]["asset_id"],
            merged["atlas"][4]["asset_id"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_duplicate_names_wrong_region_and_path_traversal() {
        let root = temp_dir("validation");
        let duplicate = root.join("duplicate");
        let item = json!({"id":"hero","type":"character","name":"Hero","assets":{"body1":"assets/body.png","head1":"assets/head.png"}});
        write_pack(&duplicate, "duplicate", "ANY", json!([item.clone(), item]));
        assert!(parse_pack(&duplicate, "CN")
            .unwrap_err()
            .contains("同类型同名资源"));

        let wrong_region = root.join("wrong-region");
        write_pack(
            &wrong_region,
            "wrong-region",
            "GLOBAL",
            json!([{"id":"m","type":"emote","name":"M","assets":{"image":"assets/happy.png"}}]),
        );
        assert!(parse_pack(&wrong_region, "CN")
            .unwrap_err()
            .contains("当前客户端"));

        let traversal = root.join("traversal");
        write_pack(
            &traversal,
            "traversal",
            "ANY",
            json!([{"id":"m","type":"emote","name":"M","assets":{"image":"../outside.png"}}]),
        );
        assert!(parse_pack(&traversal, "CN").unwrap_err().contains("不允许"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_ids_match_the_cross_platform_fnv_mapping() {
        let mut used = HashSet::new();
        assert_eq!(
            stable_id("pack\0character\0hero", &mut used),
            1_407_742_458_454_113
        );
        assert_eq!(hash("hello"), 0xa430_d846_80aa_bd0b);
        assert_eq!(hash("自制包"), 0x8a47_7546_7a40_fe51);
    }

    #[test]
    fn unsafe_removal_and_incomplete_official_catalog_are_rejected() {
        let root = temp_dir("unsafe");
        fs::create_dir_all(&root).unwrap();
        for id in ["..", ".", "../other", "a/b", "-bad"] {
            assert!(remove_pack(&root, id).is_err());
        }
        for part in [".. ", "CON.png", "nul", "image.png.", "a\0.png"] {
            assert!(unsafe_zip_part(part));
        }
        let mut invalid = official();
        invalid["catalog"].as_object_mut().unwrap().remove("emote");
        // 没有装 Mod 时，官方目录结构再怪也必须原样透传：Mod 层不能拖垮基础功能。
        let passthrough = merge_official(&invalid, &root, "CN").unwrap();
        assert_eq!(passthrough["catalog"], invalid["catalog"]);
        assert_eq!(passthrough["atlas_count"], 1);
        assert!(passthrough.get("customAssetPaths").is_none());
        // 装了 Mod 才需要官方目录完整，缺分类时明确报错而不是产出半套数据。
        write_pack(
            &root.join("one"),
            "pack-one",
            "ANY",
            json!([{"id":"solo","type":"emote","name":"Solo","assets":{"image":"assets/happy.png"}}]),
        );
        assert!(merge_official(&invalid, &root, "CN").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_import_replaces_the_named_pack_and_rolls_back_invalid_input() {
        use zip::{write::SimpleFileOptions, ZipWriter};
        let root = temp_dir("archive");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        write_pack(&source, "Example", "ANY", sample_items());
        let zip_path = root.join("example.zip");
        let mut zip = ZipWriter::new(File::create(&zip_path).unwrap());
        for name in [
            "manifest.json",
            "items.json",
            "assets/body.png",
            "assets/head.png",
            "assets/ears.png",
            "assets/happy.png",
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&fs::read(source.join(name)).unwrap())
                .unwrap();
        }
        zip.start_file(".DS_Store", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"metadata").unwrap();
        zip.finish().unwrap();
        let destination = root.join("mods");
        let installed = install_archive(&zip_path, &destination, "CN").unwrap();
        let before = merge_official(&official(), &destination, "CN").unwrap();
        assert_eq!(
            install_archive(&zip_path, &destination, "CN").unwrap().id,
            installed.id
        );
        assert_eq!(list_summaries(&destination, "CN").unwrap().len(), 1);
        let mut bad = ZipWriter::new(File::create(root.join("bad.zip")).unwrap());
        bad.start_file("../escape.png", SimpleFileOptions::default())
            .unwrap();
        bad.write_all(b"not an image").unwrap();
        bad.finish().unwrap();
        assert!(install_archive(&root.join("bad.zip"), &destination, "CN").is_err());
        assert_eq!(
            merge_official(&official(), &destination, "CN").unwrap(),
            before
        );
        remove_pack(&destination, &installed.id).unwrap();
        assert!(list_summaries(&destination, "CN").unwrap().is_empty());
        let restored = merge_official(&official(), &destination, "CN").unwrap();
        assert_eq!(restored["atlas"], official()["atlas"]);
        assert_eq!(restored["catalog"], official()["catalog"]);
        fs::remove_dir_all(root).unwrap();
    }
}
