fn main() {
    let region = std::env::var("WUWA_REGION").unwrap_or_else(|_| "cn".into());
    let filename = if region == "global" {
        "../../shared/upstream/global.json"
    } else {
        "../../shared/upstream/cn.json"
    };
    let lock = std::fs::read_to_string(filename).expect("official resource lock must be readable");
    let version: serde_json::Value =
        serde_json::from_str(&lock).expect("official resource lock must be JSON");
    println!(
        "cargo:rustc-env=WUWA_OFFICIAL_VERSION={}",
        version["version"]
            .as_str()
            .expect("official version must be a string")
    );
    println!("cargo:rerun-if-changed={filename}");
    tauri_build::build()
}
