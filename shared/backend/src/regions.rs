//! Region profiles shared by the Rust host, macOS shell, and build tooling.
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionProfile {
    pub id: String,
    pub region: String,
    pub label: String,
    pub api_host: String,
    pub assets_host: String,
    pub config_url: String,
    pub assets_origin: String,
    pub updater_host_file: String,
    pub installer_prefix: String,
    pub installer_platform: String,
    pub artifact_prefix: String,
    pub mac_product_name: String,
    pub windows_product_name: String,
    pub mac_identifier: String,
    pub pet_site: String,
    pub mac_lock: String,
    pub windows_lock: String,
    pub mac_frontend_dir: String,
    pub mac_prepared_dir: String,
    pub mac_icons_dir: String,
    pub mac_tauri_config: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Catalog {
    cn: RegionProfile,
    global: RegionProfile,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../config/regions.json"))
            .expect("shared/config/regions.json must be valid")
    })
}

pub fn profile(id: &str) -> Option<&'static RegionProfile> {
    match id.trim().to_ascii_lowercase().as_str() {
        "cn" => Some(&catalog().cn),
        "global" => Some(&catalog().global),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_complete_and_region_specific() {
        let cn = profile("cn").unwrap();
        let global = profile("GLOBAL").unwrap();
        assert_ne!(cn.api_host, global.api_host);
        assert_ne!(cn.assets_host, global.assets_host);
        assert_ne!(cn.mac_identifier, global.mac_identifier);
        assert_eq!(cn.installer_platform, "windows-x86_64");
        assert!(profile("international").is_none());
    }
}
