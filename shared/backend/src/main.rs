use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};
use wuwatappo_assistant_core::{demo_packs, injector, lnk, pe_repack, AssistantCore};

fn flag_value(args: &[String], name: &str) -> Result<Option<String>, String> {
    let prefix = format!("{name}=");
    for i in 0..args.len() {
        if args[i] == name {
            let val = args
                .get(i + 1)
                .ok_or_else(|| format!("参数 {name} 缺少值"))?;
            return Ok(Some(val.clone()));
        }
        if let Some(rest) = args[i].strip_prefix(&prefix) {
            return Ok(Some(rest.to_owned()));
        }
    }
    Ok(None)
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn default_repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn run_cli(args: &[String]) -> Result<(), String> {
    let cmd = args[0].as_str();
    let rest = &args[1..];
    match cmd {
        "inject" => {
            let platform_raw = flag_value(rest, "--platform")?
                .ok_or("用法：assistant-core inject --platform <mac|win> [--build-root <dir>] [--repo-root <dir>] [--pack-pe]")?;
            let platform = injector::Platform::parse(&platform_raw)?;
            let build_root = flag_value(rest, "--build-root")?
                .or_else(|| env::var("WUWA_BUILD_ROOT").ok())
                .ok_or("缺少 --build-root 或环境变量 WUWA_BUILD_ROOT")?;
            let repo_root = flag_value(rest, "--repo-root")?
                .map(PathBuf::from)
                .unwrap_or_else(default_repo_root);
            let result = injector::integrate_build(platform, Path::new(&build_root), &repo_root)?;
            if has_flag(rest, "--pack-pe") {
                if platform != injector::Platform::Windows {
                    return Err("--pack-pe 仅支持 Windows 目标".into());
                }
                let report = pe_repack::repack_pe(
                    Path::new(&build_root),
                    &result.base_dir,
                    &result.frontend_dir,
                    &result.shared_name,
                    &result.main_name,
                    &result.panel_name,
                    &result.extra_patched_assets,
                    &result.patch_log,
                )?;
                let mut printed = report.clone();
                if let Some(obj) = printed.as_object_mut() {
                    obj.remove("patches");
                }
                println!("{}", serde_json::to_string_pretty(&printed).map_err(|e| e.to_string())?);
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result.manifest).map_err(|e| e.to_string())?
                );
            }
            Ok(())
        }
        "extract-pe" => {
            let exe = flag_value(rest, "--exe")?
                .ok_or("用法：assistant-core extract-pe --exe <file> [--extract-root <dir>] --manifest <json>")?;
            let extract_root = flag_value(rest, "--extract-root")?
                .or_else(|| env::var("WUWA_EXTRACT_ROOT").ok())
                .ok_or("缺少 --extract-root 或环境变量 WUWA_EXTRACT_ROOT")?;
            let manifest_path = flag_value(rest, "--manifest")?
                .ok_or("缺少 --manifest <json>")?;
            let manifest_json = fs::read_to_string(&manifest_path)
                .map_err(|e| format!("读取 manifest 失败：{e}"))?;
            let meta = pe_repack::extract_pe_metadata(
                Path::new(&exe),
                Path::new(&extract_root),
                &manifest_json,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        "make-shortcut" => {
            let out = flag_value(rest, "--out")?.ok_or("缺少 --out <path.lnk>")?;
            let target = flag_value(rest, "--target")?.ok_or("缺少 --target <exe>")?;
            let arguments = flag_value(rest, "--arguments")?;
            let workdir = flag_value(rest, "--workdir")?;
            let icon = flag_value(rest, "--icon")?;
            let description = flag_value(rest, "--description")?;
            let window = match flag_value(rest, "--window")? {
                Some(raw) => lnk::WindowMode::parse(&raw)?,
                None => lnk::WindowMode::Normal,
            };
            let size = lnk::create_shortcut(
                &lnk::ShortcutSpec {
                    out: Path::new(&out),
                    target: &target,
                    arguments: arguments.as_deref(),
                    workdir: workdir.as_deref(),
                    icon: icon.as_deref(),
                    description: description.as_deref(),
                    window,
                },
                !has_flag(rest, "--skip-exists-check"),
            )?;
            println!("{size} bytes -> {out}");
            Ok(())
        }
        "make-demo-packs" => {
            let out_dir = flag_value(rest, "--out-dir")?.unwrap_or_else(|| {
                default_repo_root().join("mod-examples").to_string_lossy().into_owned()
            });
            demo_packs::generate_all(Path::new(&out_dir))?;
            println!("generated demo packs at {out_dir}");
            Ok(())
        }
        other => Err(format!(
            "未知子命令：{other}（支持 inject / extract-pe / make-shortcut / make-demo-packs）"
        )),
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        if let Err(error) = run_cli(&args) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    let stdin = io::stdin();
    let mut core = AssistantCore::new(None);
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("{error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                println!("{}", json!({"error":error.to_string()}));
                continue;
            }
        };
        if let Some(saved) = request.get("state") {
            core = AssistantCore::new(Some(saved.clone()));
        }
        let action = request
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("state");
        let body = request.get("body").cloned().unwrap_or_else(|| json!({}));
        let now = request.get("now").and_then(Value::as_f64).unwrap_or(0.0);
        let quiet = request
            .get("quiet")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let offline = request
            .get("offline")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let update_required = request
            .get("update_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let response = match core.handle(action, &body, now, quiet, offline, update_required) {
            Ok(result) => json!({"result":result,"state":core.saved()}),
            Err(error) => json!({"error":error,"state":core.saved()}),
        };
        println!("{}", response);
        let _ = io::stdout().flush();
    }
}
