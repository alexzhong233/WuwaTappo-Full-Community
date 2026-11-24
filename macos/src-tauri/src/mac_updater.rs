//! Verified macOS `.app` bundle staging and detached DMG in-place swap.
use serde_json::{json, Value};
use std::{
    ffi::{c_char, CString},
    fs,
    os::unix::fs::PermissionsExt,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use wuwatappo_assistant_core::release_installer;

const W_OK: i32 = 2;
const X_OK: i32 = 1;

unsafe extern "C" {
    fn access(path: *const c_char, amode: i32) -> i32;
}

fn can_access(path: &Path, mode: i32) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe { access(c_path.as_ptr(), mode) == 0 }
}

pub fn resolve_app_bundle(exe_path: &Path) -> Result<PathBuf, String> {
    let text = exe_path.to_string_lossy();
    if text.contains("/AppTranslocation/") {
        return Err(
            "当前应用正从系统临时隔离目录运行，请先将应用拖入「应用程序」文件夹后再使用一键更新"
                .into(),
        );
    }
    if text.starts_with("/Volumes/") {
        return Err(
            "当前正直接在磁盘映像（DMG）中运行，请先将应用拖入「应用程序」文件夹后再打开".into(),
        );
    }
    let Some(macos_dir) = exe_path.parent() else {
        return Err("无法定位当前应用主程序目录".into());
    };
    let Some(contents_dir) = macos_dir.parent() else {
        return Err("当前不是标准 .app 安装包结构".into());
    };
    let Some(bundle_dir) = contents_dir.parent() else {
        return Err("无法定位 .app 应用包目录".into());
    };
    let is_standard_bundle = macos_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && contents_dir.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && bundle_dir
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        && bundle_dir.is_dir();
    if !is_standard_bundle {
        return Err("仅已安装的 .app 应用支持一键原地更新".into());
    }
    let Some(parent_dir) = bundle_dir.parent() else {
        return Err("无法定位应用上级目录".into());
    };
    if !can_access(parent_dir, W_OK | X_OK) || !can_access(bundle_dir, W_OK) {
        return Err(
            "当前账号对应用所在目录没有写入权限，请前往 GitHub 下载安装包手动覆盖".into(),
        );
    }
    Ok(bundle_dir.to_path_buf())
}

pub fn current_installable_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("无法获取当前应用路径：{error}"))?;
    resolve_app_bundle(&exe)
}

struct MountGuard(PathBuf);

impl Drop for MountGuard {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = Command::new("/usr/bin/hdiutil")
                .args(["detach"])
                .arg(&self.0)
                .args(["-force"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = fs::remove_dir(&self.0);
        }
    }
}

fn find_app_in_mount(mount_dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(mount_dir)
        .map_err(|error| format!("无法读取镜像内容：{error}"))?;
    let mut apps = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取镜像条目失败：{error}"))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)
            .map_err(|error| format!("读取镜像文件属性失败：{error}"))?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            apps.push(path);
        }
    }
    match apps.len() {
        1 => Ok(apps.remove(0)),
        0 => Err("安装包镜像中未找到 .app 应用".into()),
        _ => Err("安装包镜像包含多个 .app 应用，无法自动选择".into()),
    }
}

fn plist_value(plist_path: &Path, key: &str) -> Result<String, String> {
    let expr = format!("Print :{key}");
    let output = Command::new("/usr/libexec/PlistBuddy")
        .arg("-c")
        .arg(expr)
        .arg(plist_path)
        .output()
        .map_err(|error| format!("读取 Info.plist 失败：{error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("无法读取 Info.plist 字段 {key}")
        } else {
            format!("无法读取 Info.plist 字段 {key}：{detail}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn verify_staged_app(
    staged_app: &Path,
    expected_identifier: &str,
    expected_version: &str,
) -> Result<(), String> {
    let codesign = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(staged_app)
        .output()
        .map_err(|error| format!("执行代码签名校验失败：{error}"))?;
    if !codesign.status.success() {
        let detail = String::from_utf8_lossy(&codesign.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            "新版本应用签名校验失败".into()
        } else {
            format!("新版本应用签名校验失败：{detail}")
        });
    }
    let plist = staged_app.join("Contents/Info.plist");
    if !plist.is_file() {
        return Err("新版本缺少 Contents/Info.plist".into());
    }
    let identifier = plist_value(&plist, "CFBundleIdentifier")?;
    if identifier != expected_identifier {
        return Err(format!(
            "安装包应用标识（{identifier}）与当前区域（{expected_identifier}）不匹配"
        ));
    }
    let bundle_version = plist_value(&plist, "CFBundleShortVersionString")?;
    if bundle_version != expected_version {
        return Err(format!(
            "安装包版本号（{bundle_version}）与发布版本（{expected_version}）不一致"
        ));
    }
    Ok(())
}

const APPLY_UPDATE_SCRIPT: &str = r#"#!/bin/sh
set -u
OLD_PID="$1"
TARGET_APP="$2"
STAGED_APP="$3"
BACKUP_APP="$4"
JOB_PROGRESS="$5"
CURRENT_PROGRESS="$6"
JOB_DIR="$7"
OPEN_BIN="${8:-/usr/bin/open}"

write_status() {
  phase="$1"
  percent="$2"
  msg="$3"
  ts="$(/bin/date +%s 2>/dev/null || echo 0)"
  json="{\"phase\":\"$phase\",\"percent\":$percent,\"message\":\"$msg\",\"updated_at\":$ts}"
  printf '%s' "$json" > "${JOB_PROGRESS}.tmp" && /bin/mv -f "${JOB_PROGRESS}.tmp" "$JOB_PROGRESS" 2>/dev/null || true
  printf '%s' "$json" > "${CURRENT_PROGRESS}.tmp" && /bin/mv -f "${CURRENT_PROGRESS}.tmp" "$CURRENT_PROGRESS" 2>/dev/null || true
}

i=0
while [ "$i" -lt 200 ] && /bin/kill -0 "$OLD_PID" 2>/dev/null; do
  /bin/sleep 0.3
  i=$((i + 1))
done
if /bin/kill -0 "$OLD_PID" 2>/dev/null; then
  write_status "failed" 0 "旧版本未及时退出，已取消更新"
  /bin/rm -rf "$STAGED_APP" "$JOB_DIR/pending.json"
  exit 1
fi

/bin/rm -rf "$BACKUP_APP"
if ! /bin/mv "$TARGET_APP" "$BACKUP_APP"; then
  write_status "failed" 0 "无法备份当前应用，请检查安装目录权限"
  exit 1
fi

if ! /bin/mv "$STAGED_APP" "$TARGET_APP"; then
  /bin/mv "$BACKUP_APP" "$TARGET_APP" 2>/dev/null || true
  "$OPEN_BIN" "$TARGET_APP" 2>/dev/null || true
  write_status "failed" 0 "替换新版本失败，已恢复原版本"
  exit 1
fi

/usr/bin/xattr -dr com.apple.quarantine "$TARGET_APP" 2>/dev/null || true

if ! "$OPEN_BIN" "$TARGET_APP"; then
  /bin/rm -rf "$TARGET_APP"
  /bin/mv "$BACKUP_APP" "$TARGET_APP" 2>/dev/null || true
  "$OPEN_BIN" "$TARGET_APP" 2>/dev/null || true
  write_status "failed" 0 "启动新版本失败，已恢复原版本"
  exit 1
fi

i=0
while [ "$i" -lt 60 ]; do
  if [ -f "$JOB_DIR/healthy" ]; then
    break
  fi
  if [ "$i" -ge 6 ]; then
    if ! /bin/ps -axo command= | /usr/bin/grep -Fq -- "$TARGET_APP/Contents/MacOS/"; then
      /bin/rm -rf "$TARGET_APP"
      /bin/mv "$BACKUP_APP" "$TARGET_APP" 2>/dev/null || true
      "$OPEN_BIN" "$TARGET_APP" 2>/dev/null || true
      write_status "failed" 0 "新版本启动异常退出，已恢复原版本"
      exit 1
    fi
  fi
  /bin/sleep 0.5
  i=$((i + 1))
done

if [ ! -f "$JOB_DIR/healthy" ]; then
  /usr/bin/pkill -f -- "$TARGET_APP/Contents/MacOS/" 2>/dev/null || true
  /bin/rm -rf "$TARGET_APP"
  /bin/mv "$BACKUP_APP" "$TARGET_APP" 2>/dev/null || true
  "$OPEN_BIN" "$TARGET_APP" 2>/dev/null || true
  write_status "failed" 0 "新版本启动超时，已恢复原版本"
  exit 1
fi

/bin/rm -rf "$BACKUP_APP" "$JOB_DIR/stage" "$JOB_DIR/pending.json"
write_status "complete" 100 "更新完成！"
exit 0
"#;

pub fn prepare_update(
    state_dir: &Path,
    expected_identifier: &str,
    candidate: &Value,
) -> Result<Value, String> {
    let target_app = current_installable_bundle()?;
    let bundle_name = target_app
        .file_name()
        .ok_or("无效的应用包名称")?
        .to_os_string();
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
    if !url.starts_with("https://github.com/")
        || !version.starts_with(char::is_numeric)
        || version.contains(['/', '\\', '\0'])
    {
        return Err("升级包来源或版本无效".into());
    }

    release_installer::prune_old_jobs(state_dir);

    let job_id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    let job = state_dir.join(".updates").join(&job_id);
    fs::create_dir_all(&job).map_err(|error| error.to_string())?;
    let _ = fs::set_permissions(&job, fs::Permissions::from_mode(0o700));
    release_installer::write_progress(state_dir, &job, "download", 0.0, "正在下载安装包…");

    let prepared = (|| -> Result<(), String> {
        let dmg_path = job.join("download.dmg");
        release_installer::download_with_progress(url, &dmg_path, size, sha, |done, total| {
            let ratio = done as f64 / total.max(1) as f64;
            let bar_pct = (ratio * 75.0).min(75.0);
            let dl_pct = (ratio * 100.0).min(100.0);
            release_installer::write_progress(
                state_dir,
                &job,
                "download",
                bar_pct,
                &format!("正在下载安装包（{dl_pct:.0}%）"),
            );
        })?;

        release_installer::write_progress(state_dir, &job, "extract", 80.0, "正在挂载并校验安装包…");
        let mount_dir = job.join("mnt");
        fs::create_dir_all(&mount_dir).map_err(|error| error.to_string())?;
        let attach = Command::new("/usr/bin/hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"])
            .arg(&mount_dir)
            .arg(&dmg_path)
            .output()
            .map_err(|error| format!("挂载 DMG 失败：{error}"))?;
        if !attach.status.success() {
            let detail = String::from_utf8_lossy(&attach.stderr).trim().to_owned();
            return Err(if detail.is_empty() {
                "挂载 DMG 失败".into()
            } else {
                format!("挂载 DMG 失败：{detail}")
            });
        }
        let guard = MountGuard(mount_dir.clone());

        let mounted_app = find_app_in_mount(&mount_dir)?;
        let stage_dir = job.join("stage");
        fs::create_dir_all(&stage_dir).map_err(|error| error.to_string())?;
        let staged_app = stage_dir.join(&bundle_name);
        let ditto = Command::new("/usr/bin/ditto")
            .args(["--norsrc", "--noextattr", "--noqtn", "--noacl"])
            .arg(&mounted_app)
            .arg(&staged_app)
            .output()
            .map_err(|error| format!("提取新版本应用失败：{error}"))?;
        if !ditto.status.success() {
            let detail = String::from_utf8_lossy(&ditto.stderr).trim().to_owned();
            return Err(if detail.is_empty() {
                "提取新版本应用失败".into()
            } else {
                format!("提取新版本应用失败：{detail}")
            });
        }
        drop(guard);
        let _ = fs::remove_file(&dmg_path);

        release_installer::write_progress(
            state_dir,
            &job,
            "extract",
            85.0,
            "正在核验应用签名与区域标识…",
        );
        verify_staged_app(&staged_app, expected_identifier, version)?;

        let pending = json!({"version": version, "id": job_id});
        fs::write(
            job.join("pending.json"),
            serde_json::to_vec(&pending).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;

        let script_path = job.join("apply_update.sh");
        fs::write(&script_path, APPLY_UPDATE_SCRIPT).map_err(|error| error.to_string())?;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;

        release_installer::write_progress(
            state_dir,
            &job,
            "apply",
            90.0,
            "校验通过，正在退出并替换应用…",
        );

        let log_file = fs::File::create(job.join("apply.log")).map_err(|error| error.to_string())?;
        let err_file = log_file.try_clone().map_err(|error| error.to_string())?;
        let mut command = Command::new("/bin/sh");
        command
            .arg(&script_path)
            .arg(std::process::id().to_string())
            .arg(&target_app)
            .arg(&staged_app)
            .arg(job.join("backup.app"))
            .arg(job.join("progress.json"))
            .arg(state_dir.join(".updates/install-current.json"))
            .arg(&job)
            .arg("/usr/bin/open")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(err_file))
            .process_group(0);
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("启动更新助手失败：{error}"))
    })();

    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&job);
        release_installer::write_current_status(
            state_dir,
            "failed",
            0.0,
            &format!("准备更新失败：{error}"),
        );
        return Err(error);
    }

    Ok(json!({
        "restarting": true,
        "job": job_id,
        "version": version,
        "message": "新版本已下载并校验，正在退出桌宠、备份并替换应用，随后将自动重启…",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wuwa-mac-updater-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_dummy_app(
        bundle: &Path,
        identifier: &str,
        version: &str,
        sign: bool,
    ) {
        let macos = bundle.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        let bin = macos.join("dummy");
        fs::copy("/usr/bin/true", &bin).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>dummy</string>
  <key>CFBundleIdentifier</key><string>{identifier}</string>
  <key>CFBundleName</key><string>Dummy</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
</dict></plist>"#
        );
        fs::write(bundle.join("Contents/Info.plist"), plist).unwrap();
        if sign {
            let status = Command::new("/usr/bin/codesign")
                .args(["--force", "--deep", "--sign", "-"])
                .arg(bundle)
                .status()
                .unwrap();
            assert!(status.success(), "codesign failed on dummy bundle");
        }
    }

    #[test]
    fn resolve_app_bundle_validates_bundle_location_and_writability() {
        let root = temp_dir("resolve");
        let apps = root.join("Applications");
        let app = apps.join("TestApp.app");
        let exe = app.join("Contents/MacOS/wuwatappo-mac");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"").unwrap();
        assert_eq!(resolve_app_bundle(&exe).unwrap(), app);

        assert!(resolve_app_bundle(Path::new(
            "/private/var/folders/xx/AppTranslocation/abc/d/Test.app/Contents/MacOS/bin"
        ))
        .is_err());
        assert!(resolve_app_bundle(Path::new(
            "/Volumes/Wuwa/Test.app/Contents/MacOS/bin"
        ))
        .is_err());
        assert!(resolve_app_bundle(&root.join("plain/bin")).is_err());

        fs::set_permissions(&apps, fs::Permissions::from_mode(0o555)).unwrap();
        let ro_result = resolve_app_bundle(&exe);
        fs::set_permissions(&apps, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ro_result.is_err());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_app_in_mount_selects_real_app_and_ignores_symlinks() {
        let mount = temp_dir("mount");
        assert!(find_app_in_mount(&mount).is_err());

        let real_app = mount.join("WuwaTappo.app");
        fs::create_dir_all(&real_app).unwrap();
        let target_dir = mount.join("RealApps");
        fs::create_dir_all(&target_dir).unwrap();
        symlink(&target_dir, mount.join("Applications")).unwrap();
        symlink(&target_dir, mount.join("Alias.app")).unwrap();

        assert_eq!(find_app_in_mount(&mount).unwrap(), real_app);

        fs::create_dir_all(mount.join("Second.app")).unwrap();
        assert!(find_app_in_mount(&mount).is_err());

        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    fn verify_staged_app_checks_codesign_identifier_and_version() {
        let root = temp_dir("verify");
        let signed = root.join("Signed.app");
        make_dummy_app(&signed, "dev.w33d.wuwatappo.fullatlas.mac", "1.3.2-beta.5", true);
        verify_staged_app(&signed, "dev.w33d.wuwatappo.fullatlas.mac", "1.3.2-beta.5").unwrap();
        assert!(
            verify_staged_app(&signed, "dev.w33d.wuwatappo.global.mac", "1.3.2-beta.5").is_err()
        );
        assert!(
            verify_staged_app(&signed, "dev.w33d.wuwatappo.fullatlas.mac", "9.9.9").is_err()
        );

        let unsigned = root.join("Unsigned.app");
        make_dummy_app(&unsigned, "dev.w33d.wuwatappo.fullatlas.mac", "1.3.2-beta.5", false);
        assert!(
            verify_staged_app(&unsigned, "dev.w33d.wuwatappo.fullatlas.mac", "1.3.2-beta.5")
                .is_err()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_script_swaps_bundle_and_rolls_back_on_launch_failure() {
        let root = temp_dir("apply");

        // Case 1: successful replacement + healthy marker
        let ok_case = root.join("ok");
        let ok_job = ok_case.join(".updates/job1");
        let ok_target = ok_case.join("App.app");
        let ok_staged = ok_job.join("stage/App.app");
        let ok_backup = ok_job.join("backup.app");
        fs::create_dir_all(ok_target.join("Contents")).unwrap();
        fs::create_dir_all(ok_staged.join("Contents")).unwrap();
        fs::write(ok_target.join("Contents/marker"), b"old").unwrap();
        fs::write(ok_staged.join("Contents/marker"), b"new").unwrap();
        let ok_script = ok_job.join("apply_update.sh");
        fs::write(&ok_script, APPLY_UPDATE_SCRIPT).unwrap();
        let ok_open = ok_job.join("mock_open.sh");
        fs::write(
            &ok_open,
            format!("#!/bin/sh\ntouch \"{}/healthy\"\nexit 0\n", ok_job.display()),
        )
        .unwrap();
        fs::set_permissions(&ok_open, fs::Permissions::from_mode(0o755)).unwrap();

        let status = Command::new("/bin/sh")
            .arg(&ok_script)
            .arg("999999")
            .arg(&ok_target)
            .arg(&ok_staged)
            .arg(&ok_backup)
            .arg(ok_job.join("progress.json"))
            .arg(ok_case.join(".updates/install-current.json"))
            .arg(&ok_job)
            .arg(&ok_open)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read(ok_target.join("Contents/marker")).unwrap(), b"new");
        assert!(!ok_backup.exists());
        let ok_progress: Value =
            serde_json::from_slice(&fs::read(ok_job.join("progress.json")).unwrap()).unwrap();
        assert_eq!(ok_progress["phase"], "complete");

        // Case 2: failed launch triggers rollback to backup
        let fail_case = root.join("fail");
        let fail_job = fail_case.join(".updates/job2");
        let fail_target = fail_case.join("App.app");
        let fail_staged = fail_job.join("stage/App.app");
        let fail_backup = fail_job.join("backup.app");
        fs::create_dir_all(fail_target.join("Contents")).unwrap();
        fs::create_dir_all(fail_staged.join("Contents")).unwrap();
        fs::write(fail_target.join("Contents/marker"), b"old").unwrap();
        fs::write(fail_staged.join("Contents/marker"), b"new").unwrap();
        let fail_script = fail_job.join("apply_update.sh");
        fs::write(&fail_script, APPLY_UPDATE_SCRIPT).unwrap();
        let fail_open = fail_job.join("mock_open_fail.sh");
        fs::write(&fail_open, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&fail_open, fs::Permissions::from_mode(0o755)).unwrap();

        let status = Command::new("/bin/sh")
            .arg(&fail_script)
            .arg("999999")
            .arg(&fail_target)
            .arg(&fail_staged)
            .arg(&fail_backup)
            .arg(fail_job.join("progress.json"))
            .arg(fail_case.join(".updates/install-current.json"))
            .arg(&fail_job)
            .arg(&fail_open)
            .status()
            .unwrap();
        assert!(!status.success());
        assert_eq!(fs::read(fail_target.join("Contents/marker")).unwrap(), b"old");
        let fail_progress: Value =
            serde_json::from_slice(&fs::read(fail_job.join("progress.json")).unwrap()).unwrap();
        assert_eq!(fail_progress["phase"], "failed");

        let _ = fs::remove_dir_all(&root);
    }
}
