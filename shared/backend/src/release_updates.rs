//! Shared release selection, official version lookup, and version comparison.
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const REPOSITORY: &str = "alexzhong233/WuwaTappo-Full-Community";
const COMMUNITY_RELEASES: &str =
    "https://github.com/alexzhong233/WuwaTappo-Full-Community/releases";

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

fn json(client: &Client, url: Url, max: u64) -> Result<Value, String> {
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "更新服务器返回 HTTP {}",
            response.status().as_u16()
        ));
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

use std::io::Read;

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

pub fn check_community(current: &str, include_prerelease: bool) -> Result<Value, String> {
    let client = client(Duration::from_secs(12))?;
    let list_url = Url::parse(&format!(
        "https://api.github.com/repos/{REPOSITORY}/releases?per_page=100"
    ))
    .map_err(|error| error.to_string())?;
    let releases = json(&client, list_url, 4 * 1024 * 1024)?;
    let Some(releases) = releases.as_array() else {
        return Err("GitHub Release 列表格式无效".into());
    };
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
    let Some((latest, release)) = candidates.pop() else {
        return Ok(
            json!({"current":current,"latest":"","updateAvailable":false,"checked_at":checked_at(),"releaseUrl":COMMUNITY_RELEASES,"pageUrl":COMMUNITY_RELEASES,"releaseNotes":"","prereleaseIncluded":include_prerelease,"skippedPrerelease":skipped,"message":if skipped>0{"当前只有预发布版本；可在设置中打开预发布更新。"}else{"GitHub 上还没有已发布的版本。"}}),
        );
    };
    let release_url = release
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with(&format!("https://github.com/{REPOSITORY}/releases/tag/")))
        .unwrap_or(COMMUNITY_RELEASES);
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
    let (api, asset_host, pointer_file, installer_prefix, label) = match region {
        "cn" => (
            "api-mc-wuwatappo.kurogames-global.com",
            "mc-wuwatappo.kurogames-global.com",
            "latest_host.json",
            "WuwaTappo-cn_",
            "windows-x86_64",
        ),
        "global" => (
            "api-wutheringwaves-wuwatappo.kurogames-global.com",
            "wutheringwaves-wuwatappo.kurogames-global.com",
            "latest-global_host.json",
            "WuwaTappo-global_",
            "windows-x86_64",
        ),
        _ => return Err("未知官方服务区域".into()),
    };
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
        .pointer("/platforms/windows-x86_64/url")
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
}
