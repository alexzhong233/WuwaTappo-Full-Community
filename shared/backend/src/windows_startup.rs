//! Windows 开机自启托管：为本安装注册一条登录计划任务，优先拉起无控制台启动器 `WuwaTappoLauncher.exe`。
//!
//! 为什么不能沿用官方那条自启项：官方任务直接拉起桌宠 exe，开机后图鉴和控制台
//! 连不上本机服务。这里注册的任务优先运行 `WuwaTappoLauncher.exe`（先起本地服务再起桌宠），
//! 仅在旧布局缺少启动器 exe 时回退 `launch.ps1`；并且在启用/停用时顺手迁移掉官方旧任务。
//!
//! PowerShell 脚本保持与旧 Python 实现一致：越权、注册表、任务校验都交给系统，
//! 本模块只负责传参、超时和解析。
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x08000000;
const TIMEOUT: Duration = Duration::from_secs(45);

/// PowerShell 单引号字面量：内部的单引号要写成两个。
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 任务名跟着安装目录走，同一台机器上的多份安装互不影响。
fn task_name(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().to_lowercase().as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(6).map(|byte| format!("{byte:02x}")).collect();
    format!("FullAtlas-{hex}")
}

fn script(root: &Path, name: &str, action: &str) -> String {
    const TEMPLATE: &str = r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.Encoding]::UTF8
$taskRoot=ROOT_VALUE
$taskName=NAME_VALUE
$taskAction=ACTION_VALUE
$taskPath='\WuwaTappoFullAtlas\'
$taskNative=Join-Path $taskRoot 'WuwaTappo.Local.exe'
$taskGuiLauncher=Join-Path $taskRoot 'WuwaTappoLauncher.exe'
$taskLauncher=Join-Path $taskRoot 'launch.ps1'
$taskPowerShell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$taskArguments='-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$taskLauncher+'"'
$taskOwn=Get-ScheduledTask -TaskPath $taskPath -TaskName $taskName -ErrorAction SilentlyContinue
# 官方自启项用的是共享任务名。只有当它的动作正好是本安装的 exe 时才动它。
$taskLegacy=@(Get-ScheduledTask -TaskPath '\WuwaTappo\' -ErrorAction SilentlyContinue|Where-Object {
 @($_.Actions).Count -eq 1 -and $_.Actions[0].Execute.Trim('"') -ieq $taskNative
})
$taskOldEnabled=@($taskLegacy|Where-Object {$_.Settings.Enabled}).Count -gt 0
if($taskAction -eq 'enable' -or ($taskAction -eq 'migrate' -and -not $taskOwn -and $taskOldEnabled)){
 if(Test-Path -LiteralPath $taskGuiLauncher){
  $taskLaunch=New-ScheduledTaskAction -Execute $taskGuiLauncher -WorkingDirectory $taskRoot
 }elseif(Test-Path -LiteralPath $taskLauncher){
  $taskLaunch=New-ScheduledTaskAction -Execute $taskPowerShell -Argument $taskArguments -WorkingDirectory $taskRoot
 }else{
  throw '启动器不存在，未更改开机自启动。'
 }
 $taskUser=[Security.Principal.WindowsIdentity]::GetCurrent().Name
 $taskTrigger=New-ScheduledTaskTrigger -AtLogOn -User $taskUser
 $taskTrigger.Delay='PT15S'
 $taskPrincipal=New-ScheduledTaskPrincipal -UserId $taskUser -LogonType Interactive -RunLevel Highest
 $taskSettings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
 $taskOwn=Register-ScheduledTask -TaskPath $taskPath -TaskName $taskName -Action $taskLaunch -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description 'WuwaTappo全图鉴版：先启动本地服务，再启动桌宠。' -Force
}
if($taskAction -eq 'disable' -and $taskOwn){
 Unregister-ScheduledTask -TaskPath $taskPath -TaskName $taskName -Confirm:$false
 $taskOwn=$null
}
if($taskAction -in @('enable','disable','migrate') -and ($taskOwn -or $taskAction -eq 'disable')){
 foreach($taskOld in $taskLegacy){Unregister-ScheduledTask -TaskPath $taskOld.TaskPath -TaskName $taskOld.TaskName -Confirm:$false}
}
$taskValid=$false
if($taskOwn -and $taskOwn.Settings.Enabled -and @($taskOwn.Actions).Count -eq 1){
 $taskRegistered=$taskOwn.Actions[0]
 if($taskRegistered.Execute.Trim('"') -ieq $taskGuiLauncher){
  $taskValid=$true
 }elseif($taskRegistered.Execute -ieq $taskPowerShell -and $taskRegistered.Arguments -ceq $taskArguments -and (Test-Path -LiteralPath $taskLauncher)){
  $taskValid=$true
 }
}
@{enabled=[bool]($taskValid -or ($taskAction -eq 'status' -and $taskOldEnabled));launcher_verified=[bool]$taskValid;migrated=[bool]($taskAction -eq 'migrate' -and $taskValid -and $taskOldEnabled)}|ConvertTo-Json -Compress
"#;
    TEMPLATE
        .replace("ROOT_VALUE", &quote(&root.to_string_lossy()))
        .replace("NAME_VALUE", &quote(name))
        .replace("ACTION_VALUE", &quote(action))
}

/// `-EncodedCommand` 要求 UTF-16LE + Base64；脚本里有中文，不能直接拼命令行参数。
fn encode_command(script: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        out.push(TABLE[(triple >> 18) as usize & 63] as char);
        out.push(TABLE[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn run(root: &Path, action: &str) -> Result<Value, String> {
    if !matches!(action, "status" | "enable" | "disable" | "migrate") {
        return Err("Unknown startup action".into());
    }
    let name = task_name(root);
    let encoded = encode_command(&script(root, &name, action));
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let powershell = Path::new(&system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut child = Command::new(powershell)
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            encoded.as_str(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 PowerShell：{error}"))?;
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("开机自启动设置超时".into());
            }
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    if !status.success() {
        let message = String::from_utf8_lossy(&stderr).trim().to_owned();
        return Err(if message.is_empty() {
            "开机自启动设置失败，请通过启动器以管理员权限运行。".into()
        } else {
            message
        });
    }
    let text = String::from_utf8_lossy(&stdout);
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err("开机自启动设置没有返回结果".into());
    }
    serde_json::from_str(text).map_err(|error| format!("开机自启动输出无法解析：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_name_is_stable_and_installation_scoped() {
        let first = task_name(Path::new("C:/Game/WuwaTappo"));
        assert_eq!(first, task_name(Path::new("c:/game/wuwatappo")));
        assert_ne!(first, task_name(Path::new("D:/Game/WuwaTappo")));
        assert!(first.starts_with("FullAtlas-"));
        assert_eq!(first.len(), "FullAtlas-".len() + 12);
    }

    #[test]
    fn quotes_are_doubled_so_paths_cannot_break_out_of_the_literal() {
        assert_eq!(quote("C:/it's"), "'C:/it''s'");
        let rendered = script(Path::new("C:/a'b"), "FullAtlas-1", "status");
        assert!(rendered.contains("'C:/a''b'"), "{rendered}");
        assert!(rendered.contains("$taskAction='status'"), "{rendered}");
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        // "A" -> UTF-16LE 41 00 -> "QQA="（少一个 "=" 就会被 PowerShell 当作坏串）
        assert_eq!(encode_command("A"), "QQA=");
        // 4 字节落在两个 chunk 上：第二块只剩 1 字节，必须补满 "=="
        assert_eq!(encode_command("AB"), "QQBCAA==");
        assert!(encode_command("启动").chars().all(|c| c.is_ascii()));
    }

    #[test]
    fn rejects_unknown_actions_before_spawning_anything() {
        assert!(run(Path::new("C:/Game/WuwaTappo"), "delete").is_err());
    }

    #[test]
    fn startup_script_prefers_gui_launcher_and_keeps_legacy_fallback() {
        let rendered = script(Path::new("C:/Game/WuwaTappo"), "FullAtlas-1", "enable");
        let gui_pos = rendered.find("$taskGuiLauncher").unwrap();
        let legacy_pos = rendered.find("$taskPowerShell -Argument $taskArguments").unwrap();
        assert!(rendered.contains("WuwaTappoLauncher.exe"));
        assert!(rendered.contains("launch.ps1"));
        assert!(gui_pos < legacy_pos);
    }
}
