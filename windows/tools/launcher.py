"""WuwaTappo 全图鉴版 无控制台启动器。

流程：
  1. 以 CREATE_NO_WINDOW 拉起本地素材服务 LocalGalleryHost.exe
  2. 轮询 /status 就绪后再启动主程序 WuwaTappo.Local.exe
  3. 主程序起来后，后台查一次官方版本清单（24 小时内不重复查）
     发现有比本地基底更高的版本，弹一次提示，同一版本只提示一次
  4. 自身退出

关于提权：主程序 WuwaTappo.Local.exe 的清单里是 requireAdministrator（沿用官方
客户端），所以它必须由系统提权启动。启动器自己不提权（不带 --uac-admin），
改用 ShellExecuteW("runas") 拉起主程序，让 Windows 按主程序的清单弹一次 UAC。
这样做的原因：
  - 启动器自身 --uac-admin 会让每次双击先弹一次针对启动器的 UAC，再弹一次主程序的；
  - 不提权的进程用 CreateProcess 直接起 requireAdministrator 程序会失败 [WinError 740]。
ShellExecuteW 走的是 shell 的提权通道，非提权进程也能拉起它。

注意：改过的程序过不了官方 minisign 校验，所以程序内自动更新必须保持关闭。
      这里只做「告诉你官方出新版了」，绝不自动下载或替换，避免静默错位。

编译：tools\\build-launcher.ps1（PyInstaller --onefile --noconsole）
"""
import ctypes
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.request
from urllib.parse import urlparse

HOST_EXE = 'LocalGalleryHost.exe'
APP_EXE = 'WuwaTappo.Local.exe'
STATUS_URL = 'http://127.0.0.1:43229/status'
WAIT_SECONDS = 20

# 官方更新接口（客户端自己用的同一条链，CN 渠道）
MANIFEST_HOST = 'https://mc-wuwatappo.kurogames-global.com'
HINT_FILE = 'update_hint.json'
CHECK_THROTTLE = 24 * 3600   # 多久才允许重新查一次
HTTP_TIMEOUT = 7             # 单个请求超时
CHECK_BUDGET = 16            # 后台检查最多占用多久，超时直接放弃（静默）

CREATE_NO_WINDOW = 0x08000000
DETACHED_PROCESS = 0x00000008

MB_ICONINFORMATION = 0x40
MB_TOPMOST = 0x40000
MB_SETFOREGROUND = 0x10000


def app_dir():
    if getattr(sys, 'frozen', False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def say(msg):
    """--noconsole 下 sys.stderr 可能为 None，失败时才弹框。"""
    try:
        if sys.stderr:
            sys.stderr.write(msg + '\n')
    except Exception:
        pass


def fail_box(text):
    try:
        ctypes.windll.user32.MessageBoxW(None, text, 'WuwaTappo 全图鉴版', 0x10)
    except Exception:
        pass


def port_open():
    """不经过系统代理探测本地服务。"""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(STATUS_URL, timeout=2) as resp:
            return resp.status == 200
    except Exception:
        return False


# ---------------------------------------------------------------- 更新检查

def _json_url(url, timeout=HTTP_TIMEOUT):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(
        url, headers={'User-Agent': 'WuwaTappo-FullAtlas-Launcher/1.0'})
    with opener.open(req, timeout=timeout) as resp:
        final = urlparse(resp.geturl())
        expected = urlparse(MANIFEST_HOST)
        if (final.scheme != 'https' or final.hostname != expected.hostname
                or final.port not in (None, 443) or final.username or final.password
                or final.fragment):
            raise ValueError('official update response left the trusted HTTPS host')
        data = json.loads(resp.read().decode('utf-8'))
    return data if isinstance(data, dict) else {}


def local_version(root):
    try:
        with open(os.path.join(root, 'local_config.json'), 'r', encoding='utf-8') as fh:
            cfg = json.load(fh)
        return str(cfg.get('client_version') or '')
    except Exception:
        return ''


def vtup(text):
    """'1.0.10' -> (1, 0, 10)，取不到数字就当 0。"""
    nums = re.findall(r'\d+', str(text or '')[:16])
    return tuple(int(x) for x in nums[:3]) or (0,)


def read_hint(root):
    try:
        with open(os.path.join(root, HINT_FILE), 'r', encoding='utf-8') as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def write_hint(root, data):
    try:
        with open(os.path.join(root, HINT_FILE), 'w', encoding='utf-8') as fh:
            json.dump(data, fh, ensure_ascii=False, indent=2)
    except Exception:
        pass


def probe_update(root, out):
    """查官方清单，只把「确实有新版」写进 out；任何异常都静默。"""
    try:
        entry = _json_url(MANIFEST_HOST + '/app/json-host/latest_host.json')
        url = str(entry.get('url') or '')
        if not url:
            return
        # 必须用 latest_host.json 给带 ?t= 的地址，否则命中 CDN 旧清单
        manifest = _json_url(url)
        remote = str(manifest.get('version') or '')
        local = local_version(root)
        if not remote or not local:
            return
        if vtup(remote) <= vtup(local):
            return
        platforms = manifest.get('platforms')
        plat = platforms.get('windows-x86_64') if isinstance(platforms, dict) else None
        expected_name = 'WuwaTappo-cn_%s_x64-setup.exe' % remote
        installer = str((plat or {}).get('url') or '')
        parsed = urlparse(installer)
        trusted = urlparse(MANIFEST_HOST)
        if (parsed.scheme != 'https' or parsed.hostname != trusted.hostname
                or parsed.port not in (None, 443) or parsed.username or parsed.password
                or parsed.fragment or os.path.basename(parsed.path) != expected_name):
            return
        out['remote'] = remote
        out['local'] = local
        out['page'] = installer
        out['pub'] = str(manifest.get('pub_date') or '')
    except Exception:
        pass


def show_update_box(root, info):
    remote = info.get('remote')
    local = info.get('local')
    lines = [
        '官方已发布新版本 %s。' % remote,
        '当前本地基底是 %s（全图鉴补丁适配于它）。' % local,
        '',
    ]
    tool = os.path.join(os.path.dirname(root), 'tools', 'update-watch.ps1')
    if os.path.exists(tool):
        lines += [
            '想跟进就运行下面这个脚本：',
            tool,
            '',
            '它只做「下载 + 试注入到 staging」，不会动现在能用的版本。',
            '锚点对不上会明确报错并保留旧版，重建和替换仍然要人工确认。',
        ]
    else:
        lines += [
            '程序不会自动升级：改过的版本过不了官方签名校验。',
            '如需升级请联系维护者重新适配。',
        ]
    lines += ['', '（版本 %s 只提示这一次）' % remote]
    try:
        ctypes.windll.user32.MessageBoxW(
            None, '\n'.join(lines), 'WuwaTappo 全图鉴版 · 官方有新版本',
            MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND)
    except Exception:
        pass


def notify_update(root):
    """主程序已经起来之后才调用，不拖慢启动；最坏情况静默放弃。"""
    hint = read_hint(root)
    now = time.time()
    try:
        last = float(hint.get('last_check') or 0)
    except Exception:
        last = 0.0
    if now - last < CHECK_THROTTLE:
        return

    out = {}
    worker = threading.Thread(target=probe_update, args=(root, out), daemon=True)
    worker.start()
    worker.join(CHECK_BUDGET)

    hint['last_check'] = now
    remote = out.get('remote')
    if not remote:
        write_hint(root, hint)
        return
    if hint.get('notified_version') == remote:
        write_hint(root, hint)
        return
    hint['notified_version'] = remote
    write_hint(root, hint)
    show_update_box(root, out)


# -------------------------------------------------------------------- 主流程

def start_app(app, root):
    """用 shell 的提权通道启动主程序。

    主程序的清单是 requireAdministrator：从非提权进程直接 CreateProcess 会拿到
    [WinError 740]。ShellExecuteW("runas") 交给系统按目标程序的清单处理，
    用户拒绝时返回 <= 32（SE_ERR_ACCESSDENIED = 5）。
    """
    shell32 = ctypes.windll.shell32
    shell32.ShellExecuteW.restype = ctypes.c_void_p
    code = shell32.ShellExecuteW(None, 'runas', app, None, root, 1) or 0
    if code <= 32:
        raise OSError(code, '系统没有授予管理员权限。主程序需要管理员权限才能启动，'
                            '请重试并在 Windows 的提示框里点「是」。')

def main():
    root = app_dir()
    host = os.path.join(root, HOST_EXE)
    app = os.path.join(root, APP_EXE)

    if not os.path.exists(app):
        fail_box('找不到主程序：\n%s' % app)
        return 1

    if not port_open() and os.path.exists(host):
        out = open(os.path.join(root, 'host.stdout.log'), 'ab')
        err = open(os.path.join(root, 'host.stderr.log'), 'ab')
        try:
            subprocess.Popen([host], cwd=root, creationflags=CREATE_NO_WINDOW,
                             stdout=out, stderr=err, close_fds=True)
        except OSError as exc:
            say('启动本地服务失败: %s' % exc)
        finally:
            out.close()
            err.close()

        deadline = time.time() + WAIT_SECONDS
        while time.time() < deadline:
            if port_open():
                break
            time.sleep(0.4)

    try:
        start_app(app, root)
    except OSError as exc:
        fail_box('启动主程序失败：\n%s' % exc)
        return 1

    # 主窗口先出来，再决定要不要提示更新
    time.sleep(1.5)
    notify_update(root)
    return 0


if __name__ == '__main__':
    sys.exit(main())
