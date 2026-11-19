//! Shared release selection, official version lookup, and version comparison.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const REPOSITORY: &str = "alexzhong233/WuwaTappo-Full-Community";
const COMMUNITY_RELEASES: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";
const MAX_OFFICIAL_INSTALLER: u64 = 512 * 1024 * 1024;
const UPDATE_CACHE_TTL_SECS: u64 = 15 * 60;

#[derive(Clone)]
struct CachedCommunityCheck {
    key: (String, bool),
    at_secs: u64,
    value: Value,
}

/// 缓存最近一次社区版发布检查结果，避免页面轮询打爆 GitHub 匿名接口限流（60 次/小时）。
#[derive(Default)]
pub struct UpdateTracker {
    cache: Mutex<Option<CachedCommunityCheck>>,
}

impl UpdateTracker {
    pub fn check_community_cached(
        &self,
        current: &str,
        include_prerelease: bool,
        force: bool,
    ) -> Value {
        let now_secs = checked_at();
        if !force {
            let guard = self.cache.lock().unwrap();
            if let Some(entry) = guard.as_ref() {
                if entry.key.0 == current
                    && entry.key.1 == include_prerelease
                    && now_secs.saturating_sub(entry.at_secs) < UPDATE_CACHE_TTL_SECS
                {
                    return entry.value.clone();
                }
            }
        }
        match check_community(current, include_prerelease) {
            Ok(value) => {
                let mut guard = self.cache.lock().unwrap();
                *guard = Some(CachedCommunityCheck {
                    key: (current.to_owned(), include_prerelease),
                    at_secs: checked_at(),
                    value: value.clone(),
                });
                value
            }
            Err(error) => check_community_error(current, include_prerelease, &error),
        }
    }

    /// 仅读取内存缓存构造 `/update/status` 响应，绝不发起联网请求。
    pub fn policy_status(
        &self,
        current: &str,
        include_prerelease: bool,
        installable_platform: bool,
    ) -> Value {
        let cached = {
            let guard = self.cache.lock().unwrap();
            guard
                .as_ref()
                .filter(|entry| entry.key.0 == current && entry.key.1 == include_prerelease)
                .map(|entry| entry.value.clone())
        };
        match cached {
            Some(check) => policy_from_check(&check, installable_platform),
            None => json!({
                "required": false,
                "current": current,
                "repository": REPOSITORY,
                "checked_at": 0,
                "error": Value::Null,
                "github_version": "",
                "latest": "",
                "available": false,
                "installable": false,
                "release_notes": "",
                "release_url": COMMUNITY_RELEASES,
                "message": "点击“检查更新”查看最新整合包版本。",
            }),
        }
    }

    pub fn policy_check(
        &self,
        current: &str,
        include_prerelease: bool,
        installable_platform: bool,
        force: bool,
    ) -> Value {
        let check = self.check_community_cached(current, include_prerelease, force);
        policy_from_check(&check, installable_platform)
    }
}

pub fn check_community_error(current: &str, include_prerelease: bool, error: &str) -> Value {
    json!({
        "current": current,
        "latest": "",
        "updateAvailable": false,
        "checked_at": checked_at(),
        "error": error,
        "message": format!("检查更新失败：{error}"),
        "releaseUrl": COMMUNITY_RELEASES,
        "pageUrl": COMMUNITY_RELEASES,
        "releaseNotes": "",
        "prereleaseIncluded": include_prerelease,
    })
}

pub fn policy_from_check(check: &Value, installable_platform: bool) -> Value {
    let current = check.get("current").and_then(Value::as_str).unwrap_or("");
    let latest = check.get("latest").and_then(Value::as_str).unwrap_or("");
    let available = check
        .get("updateAvailable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error = check.get("error").cloned().unwrap_or(Value::Null);
    let has_error = error.as_str().is_some_and(|s| !s.is_empty());
    let message = if has_error {
        check
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("检查更新失败")
            .to_owned()
    } else if available {
        if installable_platform {
            format!("发现新版本 {latest}，可点击下方按钮下载并原地升级，或到 GitHub Releases 下载安装包。")
        } else {
            format!("发现新版本 {latest}，请到 GitHub Releases 下载对应安装包。")
        }
    } else {
        check
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("检查更新失败")
            .to_owned()
    };
    json!({
        "required": false,
        "current": current,
        "repository": REPOSITORY,
        "checked_at": check.get("checked_at").cloned().unwrap_or_else(|| json!(0)),
        "error": error,
        "github_version": if latest.is_empty() { "" } else { latest },
        "latest": latest,
        "available": available,
        "installable": available && installable_platform,
        "release_notes": check.get("releaseNotes").and_then(Value::as_str).unwrap_or(""),
        "release_url": check.get("releaseUrl").and_then(Value::as_str).unwrap_or(COMMUNITY_RELEASES),
        "message": message,
    })
}

static RELEASE_CONTRACT: OnceLock<Value> = OnceLock::new();

fn release_contract() -> &'static Value {
    RELEASE_CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!("../../contracts/release-assets.json"))
            .expect("shared/contracts/release-assets.json must be valid")
    })
}

pub fn release_asset_name(kind: &str, prefix: &str, version: &str) -> Result<String, String> {
    let template = release_contract()["formats"][kind]
        .as_str()
        .ok_or_else(|| format!("共享发布契约缺少产物类型：{kind}"))?;
    Ok(template
        .replace("{prefix}", prefix)
        .replace("{version}", version))
}

fn client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(8))
        .redirect(Policy::none())
        .user_agent("WuwaTappo-Full-Community")
        .build()
        .map_err(|error| error.to_string())
}

fn checked_at() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_buster() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// 把 HTTP 失败响应翻译成能直接给人看的一句话。
///
/// GitHub 匿名接口每小时只给 60 次，超了回 403 加一段英文说明。原样透出去
/// 只会看到「更新服务器返回 HTTP 403」，看不出是限流还是真出错。
fn http_error(response: reqwest::blocking::Response) -> String {
    let status = response.status().as_u16();
    let mut body = Vec::new();
    let _ = response.take(4096).read_to_end(&mut body);
    let detail = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let limited = detail.to_ascii_lowercase().contains("rate limit");
    if (status == 403 || status == 429) && limited {
        return "GitHub 接口限流（匿名每小时 60 次），请过一会儿再检查".into();
    }
    if status == 403 || status == 429 {
        return format!("更新服务器拒绝了请求（HTTP {status}），请稍后再试");
    }
    if detail.is_empty() {
        format!("更新服务器返回 HTTP {status}")
    } else {
        format!("更新服务器返回 HTTP {status}：{detail}")
    }
}

fn json(client: &Client, url: Url, max: u64) -> Result<Value, String> {
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(http_error(response));
    }
    if response.content_length().is_some_and(|size| size > max) {
        return Err("更新服务器响应超过大小限制".into());
    }
    let mut bytes = Vec::new();
    response
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max {
        return Err("更新服务器响应超过大小限制".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn version(value: &str) -> Result<(Vec<u64>, Option<String>), String> {
    let value = value.trim().strip_prefix('v').unwrap_or(value.trim());
    let (core, pre) = value
        .split_once('-')
        .map_or((value, None), |(core, pre)| (core, Some(pre.to_owned())));
    let core = core.split('+').next().unwrap_or(core);
    let parts = core
        .split('.')
        .map(|part| {
            part.parse::<u64>()
                .map_err(|_| format!("版本号无效：{value}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() != 3 || pre.as_ref().is_some_and(|pre| !valid_prerelease(pre)) {
        return Err(format!("版本号无效：{value}"));
    }
    Ok((parts, pre))
}

fn valid_prerelease(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !(identifier.len() > 1
                    && identifier.starts_with('0')
                    && identifier.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

pub fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, String> {
    let (mut left_core, left_pre) = version(left)?;
    let (mut right_core, right_pre) = version(right)?;
    left_core.resize(3, 0);
    right_core.resize(3, 0);
    let core = left_core.cmp(&right_core);
    if core != std::cmp::Ordering::Equal {
        return Ok(core);
    }
    Ok(match (left_pre, right_pre) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease(&left, &right),
    })
}

fn compare_prerelease(left: &str, right: &str) -> std::cmp::Ordering {
    for (left, right) in left.split('.').zip(right.split('.')) {
        let left_number = left.parse::<u64>();
        let right_number = right.parse::<u64>();
        let order = match (left_number, right_number) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => left.cmp(right),
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    left.split('.').count().cmp(&right.split('.').count())
}

fn select_community_release<'a>(
    releases: &'a [Value],
    _current: &str,
    include_prerelease: bool,
) -> Result<(Option<(String, &'a Value)>, usize), String> {
    let mut candidates = Vec::new();
    let mut skipped = 0;
    for release in releases {
        if release.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let candidate = tag.trim().trim_start_matches('v');
        if version(candidate).is_err() {
            continue;
        }
        let prerelease = release
            .get("prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if prerelease && !include_prerelease {
            skipped += 1;
            continue;
        }
        candidates.push((candidate.to_owned(), release));
    }
    candidates.sort_by(|left, right| {
        compare_versions(&left.0, &right.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok((candidates.pop(), skipped))
}

fn release_page_url(release: &Value) -> &str {
    release
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with(&format!("https://github.com/{REPOSITORY}/releases/tag/")))
        .unwrap_or(COMMUNITY_RELEASES)
}

pub fn check_community(current: &str, include_prerelease: bool) -> Result<Value, String> {
    let client = client(Duration::from_secs(12))?;
    let list_url = Url::parse(&format!(
        "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
    ))
    .map_err(|error| error.to_string())?;
    let releases = match json(&client, list_url, 4 * 1024 * 1024) {
        Ok(value) => value,
        Err(api_error) => {
            if !include_prerelease {
                if let Ok(fallback) = check_community_html_fallback(&client, current) {
                    return Ok(fallback);
                }
            }
            return Err(api_error);
        }
    };
    let Some(releases) = releases.as_array() else {
        return Err("GitHub Release 列表格式无效".into());
    };
    let (selected, skipped) = select_community_release(releases, current, include_prerelease)?;
    let Some((latest, release)) = selected else {
        return Ok(
            json!({"current":current,"latest":"","updateAvailable":false,"checked_at":checked_at(),"releaseUrl":COMMUNITY_RELEASES,"pageUrl":COMMUNITY_RELEASES,"releaseNotes":"","prereleaseIncluded":include_prerelease,"skippedPrerelease":skipped,"message":if skipped>0{"当前只有预发布版本；可在设置中打开预发布更新。"}else{"GitHub 上还没有已发布的版本。"}}),
        );
    };
    let release_url = release_page_url(release);
    let newer = compare_versions(&latest, current)? == std::cmp::Ordering::Greater;
    let notes = release.get("body").and_then(Value::as_str).unwrap_or("");
    Ok(json!({
        "current":current,"latest":latest,"updateAvailable":newer,
        "releaseUrl":release_url,"pageUrl":COMMUNITY_RELEASES,
        "checked_at":checked_at(),
        "publishedAt":release.get("published_at"),
        "releaseNotes":release_notes(notes),"prereleaseIncluded":include_prerelease,
        "skippedPrerelease":skipped,
        "message":if newer {format!("发现新版本 v{latest}，当前 v{current}。")} else {format!("已是最新版本 v{current}。")},
    }))
}

fn release_tag_from_location(location: &str) -> Result<(String, String), String> {
    let base = Url::parse("https://github.com").map_err(|e| e.to_string())?;
    let resolved = base.join(location.trim()).map_err(|_| "重定向地址无效")?;
    let prefix = format!("/{REPOSITORY}/releases/tag/");
    if resolved.scheme() != "https"
        || resolved.host_str() != Some("github.com")
        || !resolved.path().starts_with(&prefix)
    {
        return Err("发布页重定向地址不受信任".into());
    }
    let tag = resolved.path()[prefix.len()..]
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    let ver = tag.trim().trim_start_matches('v').to_owned();
    version(&ver)?;
    Ok((ver, resolved.to_string()))
}

fn release_notes_from_html(html: &str) -> String {
    let section = html
        .find("data-test-selector=\"body-content\"")
        .or_else(|| html.find("class=\"markdown-body"))
        .map(|start| {
            let slice = &html[start..];
            let after_open = slice.find('>').map(|i| &slice[i + 1..]).unwrap_or(slice);
            let mut limit = after_open.len().min(64 * 1024);
            while !after_open.is_char_boundary(limit) {
                limit -= 1;
            }
            let window = &after_open[..limit];
            let end = window
                .find("</div")
                .or_else(|| window.find("<details"))
                .unwrap_or(window.len());
            &window[..end]
        })
        .unwrap_or("");
    let with_newlines = section
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</li>", "\n")
        .replace("</h1>", "\n")
        .replace("</h2>", "\n")
        .replace("</h3>", "\n")
        .replace("</h4>", "\n");
    let mut stripped = String::with_capacity(with_newlines.len());
    let mut in_tag = false;
    for ch in with_newlines.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }
    let decoded = stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    release_notes(&decoded)
}

fn check_community_html_fallback(client: &Client, current: &str) -> Result<Value, String> {
    let latest_url = Url::parse(&format!("{COMMUNITY_RELEASES}/latest"))
        .map_err(|error| error.to_string())?;
    let response = client
        .get(latest_url)
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_redirection() {
        return Err("GitHub 最新版本页面未返回跳转".into());
    }
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .ok_or("GitHub 最新版本缺少 Location")?;
    let (latest, release_url) = release_tag_from_location(location)?;
    let parsed = Url::parse(&release_url).map_err(|error| error.to_string())?;
    let notes = fetch_text(client, parsed, 2 * 1024 * 1024)
        .map(|html| release_notes_from_html(&html))
        .unwrap_or_default();
    let newer = compare_versions(&latest, current)? == std::cmp::Ordering::Greater;
    Ok(json!({
        "current": current,
        "latest": latest,
        "updateAvailable": newer,
        "releaseUrl": release_url,
        "pageUrl": COMMUNITY_RELEASES,
        "checked_at": checked_at(),
        "publishedAt": Value::Null,
        "releaseNotes": notes,
        "prereleaseIncluded": false,
        "skippedPrerelease": 0,
        "message": if newer {
            format!("发现新版本 v{latest}，当前 v{current}。")
        } else {
            format!("已是最新版本 v{current}。")
        },
    }))
}

fn fetch_text(client: &Client, url: Url, max: u64) -> Result<String, String> {
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(http_error(response));
    }
    if response.content_length().is_some_and(|size| size > max) {
        return Err("更新校验文件超过大小限制".into());
    }
    let mut bytes = Vec::new();
    response
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max {
        return Err("更新校验文件超过大小限制".into());
    }
    String::from_utf8(bytes).map_err(|_| "更新校验文件不是 UTF-8".into())
}

pub fn community_installer(
    current: &str,
    include_prerelease: bool,
    region: &str,
) -> Result<Value, String> {
    let profile = crate::regions::profile(region).ok_or("未知官方服务区域")?;
    let client = client(Duration::from_secs(30))?;
    let list_url = Url::parse(&format!(
        "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
    ))
    .map_err(|error| error.to_string())?;
    let releases = json(&client, list_url, 4 * 1024 * 1024)?;
    let Some(releases) = releases.as_array() else {
        return Err("GitHub Release 列表格式无效".into());
    };
    let (selected, _) = select_community_release(releases, current, include_prerelease)?;
    let Some((version, release)) = selected else {
        return Err("当前没有可安装的新版本".into());
    };
    if compare_versions(&version, current)? != std::cmp::Ordering::Greater {
        return Err("当前已经是最新版本".into());
    }
    let asset_name = release_asset_name("windowsUpdate", &profile.artifact_prefix, &version)?;
    let sums_name = release_contract()["checksumFile"]
        .as_str()
        .ok_or("共享发布契约缺少校验文件名")?;
    let prefix = format!("https://github.com/{REPOSITORY}/releases/download/");
    let find_asset = |name: &str| {
        release
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|assets| {
                assets.iter().find(|asset| {
                    asset.get("name").and_then(Value::as_str) == Some(name)
                        && asset
                            .get("browser_download_url")
                            .and_then(Value::as_str)
                            .is_some_and(|url| url.starts_with(&prefix))
                })
            })
            .ok_or(format!("Release 缺少可用的 {name}"))
    };
    let asset = find_asset(&asset_name)?;
    let url = asset
        .get("browser_download_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
    if size == 0 || size > 500 * 1024 * 1024 {
        return Err("升级包大小无效".into());
    }
    let sums = find_asset(sums_name)?;
    let sums_url = sums
        .get("browser_download_url")
        .and_then(Value::as_str)
        .ok_or("校验文件缺少下载地址")?;
    let checksum_text = fetch_text(
        &client,
        Url::parse(sums_url).map_err(|e| e.to_string())?,
        256 * 1024,
    )?;
    let checksum = checksum_text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let file = parts.next()?.trim_start_matches('*');
            (file == asset_name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| hash.to_ascii_lowercase())
        })
        .next()
        .ok_or("升级包缺少有效 SHA256 校验")?;
    Ok(json!({
        "version":version,"url":url,"sha256":checksum,"size":size,
        "releaseUrl":release_page_url(release),"assetName":asset_name,
    }))
}

fn release_notes(body: &str) -> String {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with(['|', '!', '>', '`']) {
                return None;
            }
            let plain = line.replace("**", "").replace('*', "").replace('`', "");
            let plain = plain
                .split("http://")
                .next()
                .unwrap_or(&plain)
                .split("https://")
                .next()
                .unwrap_or(&plain)
                .trim();
            (!plain.is_empty()).then(|| plain.to_owned())
        })
        .take(40)
        .collect::<Vec<_>>()
        .join("\n")
}

fn trusted_url(raw: &str, host: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "官方更新地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some(host)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("官方更新地址不是当前区域的可信 HTTPS 地址".into());
    }
    Ok(url)
}

pub fn check_official(region: &str, current: &str) -> Result<Value, String> {
    let profile = crate::regions::profile(region).ok_or("未知官方服务区域")?;
    let api = profile.api_host.as_str();
    let asset_host = profile.assets_host.as_str();
    let pointer_file = profile.updater_host_file.as_str();
    let installer_prefix = profile.installer_prefix.as_str();
    let label = profile.installer_platform.as_str();
    let client_http = client(Duration::from_secs(10))?;
    let api_url = Url::parse(&format!("https://{api}/api/common/activity-config"))
        .map_err(|error| error.to_string())?;
    let activity = json(&client_http, api_url, 1024 * 1024)?;
    let returned_host = activity
        .pointer("/data/assets_host")
        .and_then(Value::as_str)
        .ok_or("官方配置缺少素材域名")?;
    let returned = trusted_url(returned_host, asset_host)?;
    let stamp = cache_buster();
    let pointer = trusted_url(
        &format!("https://{asset_host}/app/json-host/{pointer_file}?_={stamp}"),
        asset_host,
    )?;
    let pointer = json(&client_http, pointer, 1024 * 1024)?;
    let mut manifest_url = trusted_url(
        pointer["url"].as_str().ok_or("官方版本指针缺少 url")?,
        asset_host,
    )?;
    manifest_url
        .query_pairs_mut()
        .append_pair("_", &stamp.to_string());
    let manifest = json(&client_http, manifest_url, 2 * 1024 * 1024)?;
    let latest = manifest["version"]
        .as_str()
        .ok_or("官方版本清单缺少版本号")?;
    let installer_name = format!("{installer_prefix}{latest}_x64-setup.exe");
    let installer = manifest
        .pointer(&format!("/platforms/{label}/url"))
        .and_then(Value::as_str)
        .ok_or("官方版本清单缺少 Windows 安装包")?;
    let installer_url = trusted_url(installer, asset_host)?;
    if installer_url
        .path_segments()
        .and_then(|segments| segments.last())
        != Some(installer_name.as_str())
    {
        return Err("官方安装包文件名与版本或区域不匹配".into());
    }
    let newer = compare_versions(latest, current)? == std::cmp::Ordering::Greater;
    let remote_older = compare_versions(current, latest)? == std::cmp::Ordering::Greater;
    let community_url = COMMUNITY_RELEASES;
    let message = if newer {
        format!("检测到官方 {latest}，可下载官方原包；整合版需由维护环境重新构建后更新。")
    } else if remote_older {
        format!("官方清单当前是 {latest}，本包基底已跟进到 {current}。")
    } else {
        format!("官方基底已是 {current}。")
    };
    let _ = returned;
    Ok(json!({
        "local":current,"remote":latest,"newer":newer,"remoteOlder":remote_older,
        "url":installer_url.as_str(),"pub":manifest.get("pub_date").and_then(Value::as_str).unwrap_or(""),
        "currentVersion":current,"latestVersion":latest,"updateAvailable":newer,
        "checked_at":checked_at(),"error":"","message":message,
        "community_releases_url":community_url,"communityReleasesUrl":community_url,
        "officialInstallerUrl":installer_url.as_str(),"officialInstallerFileName":installer_name,
        "officialInstallerPlatform":label,"download_available":newer,"canDownloadOfficial":newer,
        "local_adaptation_supported":false,"supportsLocalAdaptation":false,
        "local_adaptation_reason":"请从项目 Releases 下载适配后的整合版。",
    }))
}

/// 下载当前区域官方最新的 Windows 安装包到指定目录，用于比对或手动更新参考。
pub fn download_official_installer(
    region: &str,
    current: &str,
    download_dir: &Path,
    is_mac: bool,
) -> Result<Value, String> {
    let profile = crate::regions::profile(region).ok_or("未知官方服务区域")?;
    let status = check_official(region, current)?;
    if status["newer"].as_bool() != Some(true) {
        return Err(status["message"]
            .as_str()
            .unwrap_or("没有可下载的新版本")
            .to_string());
    }
    let latest = status["remote"]
        .as_str()
        .ok_or("官方版本检查没有返回版本号")?;
    let raw_url = status["officialInstallerUrl"]
        .as_str()
        .ok_or("官方版本检查没有返回安装包地址")?;
    let url = trusted_url(raw_url, &profile.assets_host)?;
    let filename = status["officialInstallerFileName"]
        .as_str()
        .ok_or("官方版本检查没有返回安装包文件名")?;
    if filename.is_empty()
        || !filename.ends_with(".exe")
        || filename == "."
        || filename == ".."
        || filename.contains(['/', '\\', ':', '?', '#', '%'])
        || filename.chars().any(char::is_control)
    {
        return Err("官方安装包文件名无效".into());
    }
    let download_client = client(Duration::from_secs(300))?;
    let response = download_client
        .get(url)
        .send()
        .map_err(|error| format!("下载官方原包失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载官方原包失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_OFFICIAL_INSTALLER)
    {
        return Err("官方原包超过 512 MiB，已停止下载".into());
    }
    fs::create_dir_all(download_dir).map_err(|error| error.to_string())?;
    let target = download_dir.join(filename);
    let temporary = download_dir.join(format!(".{filename}.download"));
    let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    let mut limited = response.take(MAX_OFFICIAL_INSTALLER + 1);
    let copied = match std::io::copy(&mut limited, &mut file) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(format!("保存官方原包失败：{error}"));
        }
    };
    if copied > MAX_OFFICIAL_INSTALLER {
        let _ = fs::remove_file(&temporary);
        return Err("官方原包超过 512 MiB，已停止下载".into());
    }
    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
    let message = if is_mac {
        format!(
            "官方 {latest} Windows 原包已下载到 {}。它不能直接安装到 Mac；请使用本项目 Releases 中的 Mac 包。",
            target.display()
        )
    } else {
        format!(
            "官方 {latest} Windows 原包已下载到 {}。整合版更新请使用项目 Releases 安装包或原地更新包。",
            target.display()
        )
    };
    Ok(json!({
        "ok": true,
        "version": latest,
        "fileName": filename,
        "path": target,
        "platform": profile.installer_platform,
        "message": message,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numeric_components_and_prereleases() {
        assert_eq!(
            compare_versions("1.10.0", "1.9.9").unwrap(),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.2.0", "1.2.0-beta.2").unwrap(),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.2.0-alpha.2", "1.2.0-alpha.10").unwrap(),
            std::cmp::Ordering::Less
        );
        assert!(compare_versions("bad", "1.0.0").is_err());
        assert!(compare_versions("1.2", "1.2.0").is_err());
        assert!(compare_versions("1.2.0-alpha..1", "1.2.0").is_err());
        assert!(compare_versions("1.2.0-alpha.01", "1.2.0").is_err());
    }

    #[test]
    fn release_asset_names_come_from_the_shared_contract() {
        assert_eq!(
            release_asset_name(
                "windowsUpdate",
                "WuwaTappo-Full-CN-Community",
                "1.3.2-beta.1"
            )
            .unwrap(),
            "WuwaTappo-Full-CN-Community-1.3.2-beta.1-win-x64-update.zip"
        );
        assert_eq!(
            release_asset_name(
                "windowsInstaller",
                "WuwaTappo-Full-Global-Community",
                "1.3.2"
            )
            .unwrap(),
            "WuwaTappo-Full-Global-Community-Windows-x64-1.3.2.exe"
        );
        assert_eq!(
            release_contract()["checksumFile"].as_str(),
            Some("SHA256SUMS.txt")
        );
    }

    #[test]
    fn html_fallback_extracts_tag_and_plain_notes() {
        let (ver, url) = release_tag_from_location(
            "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases/tag/v1.3.2-beta.1",
        )
        .unwrap();
        assert_eq!(ver, "1.3.2-beta.1");
        assert!(url.ends_with("/releases/tag/v1.3.2-beta.1"));
        assert!(release_tag_from_location("https://evil.invalid/releases/tag/v1.3.2").is_err());

        let html = r#"<div data-test-selector="body-content" class="markdown-body"><p><strong>修复</strong> &amp; 对齐</p><ul><li>Mac &amp; Win 同步</li></ul></div>"#;
        assert_eq!(release_notes_from_html(html), "修复 & 对齐\nMac & Win 同步");
    }

    #[test]
    fn tracker_policy_status_never_fabricates_version_and_respects_prerelease_key() {
        let tracker = UpdateTracker::default();
        let uninitialized = tracker.policy_status("1.3.2-beta.2", false, true);
        assert_eq!(uninitialized["checked_at"], 0);
        assert_eq!(uninitialized["github_version"], "");
        assert_eq!(uninitialized["available"], false);

        *tracker.cache.lock().unwrap() = Some(CachedCommunityCheck {
            key: ("1.3.2-beta.2".into(), true),
            at_secs: checked_at(),
            value: json!({
                "current": "1.3.2-beta.2",
                "latest": "1.3.2",
                "updateAvailable": true,
                "checked_at": 12345,
                "releaseNotes": "fix",
                "releaseUrl": COMMUNITY_RELEASES,
                "message": "ok",
            }),
        });
        let cached_hit = tracker.policy_status("1.3.2-beta.2", true, false);
        assert_eq!(cached_hit["github_version"], "1.3.2");
        assert_eq!(cached_hit["installable"], false);
        assert_eq!(cached_hit["available"], true);

        let prerelease_changed = tracker.policy_status("1.3.2-beta.2", false, true);
        assert_eq!(prerelease_changed["checked_at"], 0);
        assert_eq!(prerelease_changed["github_version"], "");

        let err_policy = policy_from_check(
            &check_community_error("1.3.2-beta.2", false, "network down"),
            true,
        );
        assert_eq!(err_policy["github_version"], "");
        assert_eq!(err_policy["error"], "network down");
    }
}
