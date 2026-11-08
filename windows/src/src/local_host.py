"""Local cache host and executable-scoped Windows Firewall switch."""
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, unquote
import sys
import argparse, base64, ctypes, hashlib, json, mimetypes, os, subprocess, threading, time, re
import urllib.request
from sync_cache import sync as sync_public_cache
from desktop_assistant import Assistant, PhysicalInput
from desktop_updater import Updater, apply_update, notes_for_release, release_asset_name
from custom_resources import install_archive, list_details, list_packs, merge_official, remove_pack
try:
    from native_context_menu import popup as popup_context_menu
except Exception:      # 非 Windows 或 Win32 结构不可用时不影响其他功能
    popup_context_menu = None

if '--apply-update' in sys.argv:
    pos = sys.argv.index('--apply-update')
    apply_update(sys.argv[pos+1], sys.argv[pos+2])
    sys.exit(0)
if '--self-check' in sys.argv:
    import ssl
    context = ssl.create_default_context()
    report = {'tls_ready':True,'hostname_verification':context.check_hostname,
              'certificate_verification':context.verify_mode == ssl.CERT_REQUIRED}
    # 打字自动表情靠 UIA 判断焦点。comtypes 或它生成的类型库模块没进包时，探测会
    # 恒返回 uia-unavailable——构建照样成功，用户只看到「焦点不在输入框」，查不出原因。
    # 所以打包后立刻验一次「依赖能不能初始化」，不要求真有焦点（runner 上没有）。
    try:
        from text_focus import open_automation
        open_automation().close()
        report['text_focus_ready'] = True
    except Exception as error:
        report['text_focus_ready'] = False
        report['text_focus_error'] = f'{type(error).__name__}: {error}'
    print(json.dumps(report),flush=True)
    if not report['text_focus_ready'] and os.environ.get('WUWA_SKIP_UIA_CHECK') == '1':
        print('warn: UIA 自检失败，按 WUWA_SKIP_UIA_CHECK=1 放行', file=sys.stderr, flush=True)
        sys.exit(0)
    sys.exit(0 if report['text_focus_ready'] else 1)

ROOT = Path(sys.executable if getattr(sys,'frozen',False) else __file__).resolve().parent
CONFIG = json.loads((ROOT / 'local_config.json').read_text(encoding='utf-8-sig'))
# 旧布局的 local_config.json 里没有 community_version（那是 1.3 才加的字段），
# 而它只在安装器跑 initialize.ps1 时才合并进来。直接从构建目录启动、或者装的是
# 覆盖式升级包时，这一项会一直缺着：底栏退回桌面助手的 mod_version（1.2），
# 更新页的「当前安装版本」也永远停在「检测中」。这里从构建产物补一次，不动其它字段
# （尤其 asset_count：package_config 里是 0，真实值在 local_config 与 cache 里）。
if not CONFIG.get('community_version'):
    try:
        CONFIG['community_version'] = json.loads(
            (ROOT / 'package_config.json').read_text(encoding='utf-8-sig')).get('community_version') or ''
    except (OSError, ValueError):
        pass
PORT = CONFIG['port']
TOKEN = CONFIG['token']
ASSISTANT = Assistant(ROOT, physical=PhysicalInput())
UPDATER = Updater(ROOT, CONFIG, allow_prerelease=ASSISTANT.prerelease_updates)
try:
    from startup_manager import StartupManager
    STARTUP = StartupManager(ROOT)
except Exception:      # 非 Windows 或计划任务模块不可用时，设置里会退回官方开关
    STARTUP = None

def monitor_releases():
    """盯官方 Kuro 版本，每 8 小时一次。

    注意：不再盯 Averionx 仓库的修改版 Release —— 那套更新会把本地适配的补丁
    整体覆盖掉，且会强制置 update_required 挡掉开箱功能。官方发新版只是提示，
    不阻断当前使用。
    """
    while True:
        try:
            check_official()
        except Exception:
            pass
        time.sleep(8 * 3600)

ASSISTANT.update_required = False
STATE = ROOT / 'offline_state.json'
CUSTOM_ROOT = ROOT / 'custom-atlas'
CUSTOM_LOCK = threading.RLock()
# 素材与更新清单的官方源由构建区域决定（build.ps1 写进 local_config.json）。
# 老配置没有这两个字段时退回国服，既有部署照常工作。
CDN_HOST = (CONFIG.get('cdn_host') or 'https://mc-wuwatappo.kurogames-global.com').rstrip('/')
API_BASE = (CONFIG.get('api_base') or 'https://api-mc-wuwatappo.kurogames-global.com').rstrip('/')
UPDATER_HOST_FILE = 'latest-global_host.json' if CONFIG.get('region') == 'global' else 'latest_host.json'
OFFICIAL_STATE = ROOT / 'official_update.json'
COMMUNITY_RELEASES_URL = 'https://github.com/alexzhong233/WuwaTappo-Full-Community/releases'
MAX_OFFICIAL_INSTALLER = 512 * 1024 * 1024
_NAME_INDEX = None


def _http_json(url, timeout=12):
    """直连官方站点，不经过系统代理（与 check-update.ps1 保持同一链路）。"""
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    request = urllib.request.Request(
        url, headers={'User-Agent': 'WuwaTappo-FullAtlas/1.0'})
    with opener.open(request, timeout=timeout) as response:
        _official_url(response.geturl())
        return json.loads(response.read().decode('utf-8'))


def _vtup(value):
    nums = re.findall(r'\d+', str(value or '')[:16])
    return tuple(int(x) for x in nums[:3]) or (0,)


def _official_url(value, expected_name=None):
    """只接受当前区域官方 HTTPS 资源，拒绝清单跳转到任意下载站。"""
    parsed = urlparse(str(value or ''))
    expected_host = urlparse(CDN_HOST).hostname
    if (parsed.scheme != 'https' or parsed.hostname != expected_host
            or parsed.port not in (None, 443) or parsed.username or parsed.password
            or parsed.fragment):
        raise ValueError('官方资源地址不是当前区域的 HTTPS 官方地址')
    name = Path(parsed.path).name
    if expected_name and name != expected_name:
        raise ValueError('官方原包文件名与版本或服务器区域不匹配')
    return parsed.geturl()


def _official_filename(version):
    region = 'global' if CONFIG.get('region') == 'global' else 'cn'
    return 'WuwaTappo-%s_%s_x64-setup.exe' % (region, version)


def _local_adaptation_capability():
    for project in (ROOT.parent, ROOT.parent.parent, ROOT.parent.parent.parent):
        required = [project / 'tools' / 'update-watch.ps1', project / 'tools' / 'apply-update.ps1']
        if all(path.exists() for path in required):
            return True, ''
    return False, '发行包没有完整源码和适配脚本；请由维护环境或 GitHub Actions 构建新的全图鉴包。'


def check_official():
    """查询 Kuro 官方桌宠最新版本。失败时回退上次成功结果，不抛异常。"""
    local = str(CONFIG.get('client_version') or '')
    can_adapt, adapt_reason = _local_adaptation_capability()
    result = {'local': local, 'remote': '', 'newer': False, 'remoteOlder': False, 'url': '', 'pub': '',
              'currentVersion': local, 'latestVersion': '', 'updateAvailable': False,
              'checked_at': int(time.time()), 'error': '', 'message': '',
              'community_releases_url': COMMUNITY_RELEASES_URL,
              'communityReleasesUrl': COMMUNITY_RELEASES_URL,
              'officialInstallerUrl': '', 'officialInstallerFileName': '',
              'officialInstallerPlatform': 'windows-x86_64',
              'download_available': False,
              'canDownloadOfficial': False,
              'local_adaptation_supported': can_adapt,
              'supportsLocalAdaptation': can_adapt,
              'local_adaptation_reason': adapt_reason}
    result['localAdaptationReason'] = adapt_reason
    try:
        # 两个地址都带一次性时间戳：CDN 的 max-age 只有两分钟，但边缘节点仍可能
        # 拿旧对象，表现就是官方清单长期停在上一版（例如我们已跟进 1.0.4 却读到 1.0.3）。
        stamp = str(int(time.time() * 1000))
        entry = _http_json(CDN_HOST + '/app/json-host/' + UPDATER_HOST_FILE + '?_=' + stamp)
        # 必须用清单里带 ?t= 的地址，否则命中 CDN 旧缓存
        manifest_url = _official_url(entry.get('url'))
        manifest_url += ('&' if '?' in manifest_url else '?') + '_=' + stamp
        manifest = _http_json(manifest_url)
        remote = str(manifest.get('version') or '')
        if not remote:
            raise ValueError('官方清单缺少 version')
        platforms = manifest.get('platforms')
        platform = platforms.get('windows-x86_64') if isinstance(platforms, dict) else None
        expected_name = _official_filename(remote)
        installer_url = _official_url((platform or {}).get('url'), expected_name)
        result['remote'] = remote
        result['latestVersion'] = remote
        result['url'] = installer_url
        result['file_name'] = expected_name
        result['officialInstallerUrl'] = installer_url
        result['officialInstallerFileName'] = expected_name
        result['officialInstallerPlatform'] = 'windows-x86_64'
        result['pub'] = str(manifest.get('pub_date') or '')
        result['newer'] = _vtup(remote) > _vtup(local)
        # 官方清单落后于我们已整合的基底时不能说「已是最新（旧号）」
        result['remoteOlder'] = _vtup(local) > _vtup(remote)
        result['updateAvailable'] = result['newer']
        result['download_available'] = bool(result['newer'])
        result['canDownloadOfficial'] = result['download_available']
        if result['newer']:
            result['message'] = '官方已发布 %s，当前基底 %s —— %s' % (
                remote, local, '可尝试本地全图鉴适配。' if can_adapt
                else '可下载官方原包；全图鉴包需由维护环境重新构建。')
        elif result['remoteOlder']:
            result['message'] = '官方清单当前是 %s，本包基底已跟进到 %s。' % (remote, local)
        else:
            result['message'] = '官方最新 %s，当前基底 %s —— 已是最新。' % (remote, local)
        try:
            OFFICIAL_STATE.write_text(json.dumps(result, ensure_ascii=False, indent=2),
                                      encoding='utf-8')
        except OSError:
            pass
    except Exception as error:
        result['error'] = str(error)
        cached = {}
        try:
            cached = json.loads(OFFICIAL_STATE.read_text(encoding='utf-8'))
        except (OSError, ValueError):
            pass
        for key in ('remote', 'newer', 'url', 'pub'):
            if key in cached:
                result[key] = cached[key]
        if result.get('remote'):
            result['file_name'] = _official_filename(result['remote'])
            result['latestVersion'] = result['remote']
            result['updateAvailable'] = result['newer']
            result['download_available'] = bool(result['newer'])
            result['officialInstallerFileName'] = result['file_name']
            result['officialInstallerUrl'] = result['url']
            result['officialInstallerPlatform'] = 'windows-x86_64'
            result['canDownloadOfficial'] = result['download_available']
        result['message'] = ('检查失败（%s）。%s'
                             % (error, ('显示上次结果：官方 %s。' % cached['remote'])
                                if cached.get('remote') else '当前无法联网。'))
    return result


def open_community_releases():
    """打开固定的本项目 Releases 页面，不接受前端传入地址。"""
    try:
        import webbrowser
        return {'ok': bool(webbrowser.open(COMMUNITY_RELEASES_URL, new=2)),
                'url': COMMUNITY_RELEASES_URL}
    except Exception as error:
        return {'ok': False, 'error': str(error), 'url': COMMUNITY_RELEASES_URL}


def pet_site_url():
    """官方桌宠（Wuwa Tappo）下载页：国服 / 国际服各一个入口。

    官方只发 Windows 版，Mac 整合版也指向这里，让用户能拿到纯净原版对照。
    """
    if str(CONFIG.get('region') or '').lower() == 'global':
        return 'https://download-wutheringwaves-wuwatappo.kurogames-global.com/download/zh-tw/'
    return 'https://download-mc-wuwatappo.kurogames-global.com/download/'


def open_pet_site():
    """打开官方桌宠下载页；地址由构建时的区域决定，不接受前端传参。"""
    url = pet_site_url()
    try:
        import webbrowser
        return {'ok': bool(webbrowser.open(url, new=2)), 'url': url}
    except Exception as error:
        return {'ok': False, 'error': str(error), 'url': url}


def _semver_parts(value):
    text = str(value or '').strip().lstrip('vV')
    core, _, pre = text.partition('-')
    numbers = []
    for part in core.split('.'):
        digits = ''.join(ch for ch in part if ch.isdigit())
        numbers.append(int(digits) if digits else 0)
    while len(numbers) < 3:
        numbers.append(0)
    return numbers[:3], pre


def _compare_prerelease(left, right):
    a, b = left.split('.'), right.split('.')
    for x, y in zip(a, b):
        if x.isdigit() and y.isdigit():
            if int(x) != int(y):
                return -1 if int(x) < int(y) else 1
        elif x != y:
            return -1 if x < y else 1
    return (len(a) > len(b)) - (len(a) < len(b))


def _compare_semver(left, right):
    """semver 比较：正式版高于同号预发布，alpha.10 高于 alpha.9。"""
    left_numbers, left_pre = _semver_parts(left)
    right_numbers, right_pre = _semver_parts(right)
    if left_numbers != right_numbers:
        return -1 if left_numbers < right_numbers else 1
    if not left_pre and not right_pre:
        return 0
    if not left_pre:
        return 1
    if not right_pre:
        return -1
    return _compare_prerelease(left_pre, right_pre)


def check_community(include_prerelease=False):
    """对比本仓库 GitHub 发布（排除草稿）。

    GitHub 的 /releases/latest 会跳过 prerelease，而本项目的 alpha 正以 prerelease
    发布，用那个接口永远查不到新版本，所以自己扫列表。是否把 alpha/beta 算进候选由
    `include_prerelease` 决定——默认不算，用户在控制台里可以打开。
    """
    current = str(CONFIG.get('community_version') or '')
    result = {'current': current, 'latest': '', 'updateAvailable': False, 'installable': False,
              'releaseUrl': COMMUNITY_RELEASES_URL, 'pageUrl': COMMUNITY_RELEASES_URL,
              'publishedAt': '', 'prereleaseIncluded': include_prerelease,
              'skippedPrerelease': 0, 'message': ''}
    try:
        request = urllib.request.Request(
            'https://api.github.com/repos/alexzhong233/WuwaTappo-Full-Community/releases?per_page=20',
            headers={'User-Agent': 'WuwaTappo-Full-Community-Win',
                     'Accept': 'application/vnd.github+json'})
        with urllib.request.urlopen(request, timeout=15) as response:
            releases = json.loads(response.read(512 * 1024).decode('utf-8'))
        best = None
        skipped = 0
        for release in releases if isinstance(releases, list) else []:
            if not isinstance(release, dict) or release.get('draft'):
                continue
            if not include_prerelease and release.get('prerelease'):
                skipped += 1
                continue
            tag = str(release.get('tag_name') or '').strip().lstrip('vV')
            if not tag:
                continue
            if best is None or _compare_semver(tag, best[0]) > 0:
                best = (tag, str(release.get('html_url') or COMMUNITY_RELEASES_URL),
                        str(release.get('published_at') or ''), release)
        result['skippedPrerelease'] = skipped
        if best:
            result['latest'], result['releaseUrl'], result['publishedAt'], best_release = best
            result['updateAvailable'] = bool(current) and _compare_semver(best[0], current) > 0
            # 说明正文：API 有 body 就直接清洗；匿名配额被打满时退回抓发布页 HTML。
            result['release_notes'] = notes_for_release(best_release)
            # 有没有能直接装的原地升级包（zip）。没有这条路时前端只剩「跳 GitHub 下载」。
            wanted = release_asset_name(CONFIG, best[0])
            result['installable'] = bool(result['updateAvailable']) and any(
                str(asset.get('name')) == wanted for asset in best_release.get('assets') or [])
            if result['updateAvailable']:
                result['message'] = '发现新版本 v%s，当前 v%s。' % (best[0], current)
            elif current:
                result['message'] = '已是最新版本 v%s。' % current
            else:
                result['message'] = '已读取 GitHub 最新发布 v%s。' % best[0]
        elif skipped:
            result['message'] = '当前只发布了预发布版本；需要这类提醒可在设置里打开「包含预发布」。'
        else:
            result['message'] = 'GitHub 上还没有已发布的安装包。'
    except Exception as error:
        result['message'] = '检查更新失败（%s）。' % error
    return result


def download_official():
    """把当前官方新 Windows 原包下载到用户 Downloads，不执行安装或覆盖。"""
    info = check_official()
    if not info.get('newer') or not info.get('url'):
        return {'ok': False, 'error': info.get('message') or '没有可下载的官方新版本'}
    url = _official_url(info['url'], info.get('file_name'))
    downloads = Path(os.environ.get('USERPROFILE') or Path.home()) / 'Downloads'
    downloads.mkdir(parents=True, exist_ok=True)
    target = downloads / info['file_name']
    temporary = downloads / ('.' + info['file_name'] + '.download')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        request = urllib.request.Request(url, headers={'User-Agent': 'WuwaTappo-Full-Community/1.0'})
        with opener.open(request, timeout=300) as response:
            _official_url(response.geturl(), info.get('file_name'))
            length = int(response.headers.get('Content-Length') or 0)
            if length > MAX_OFFICIAL_INSTALLER:
                raise ValueError('官方原包超过 512 MiB，已停止下载')
            total = 0
            digest = hashlib.sha256()
            with temporary.open('wb') as stream:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > MAX_OFFICIAL_INSTALLER:
                        raise ValueError('官方原包超过 512 MiB，已停止下载')
                    stream.write(chunk)
                    digest.update(chunk)
        os.replace(temporary, target)
    except Exception as error:
        try:
            temporary.unlink()
        except OSError:
            pass
        return {'ok': False, 'error': '下载官方原包失败：%s' % error}
    return {'ok': True, 'version': info['remote'], 'path': str(target),
            'sha256': digest.hexdigest(), 'message':
            '官方 %s 原包已下载到 %s。它只保存在本机，不会覆盖当前全图鉴版本。' %
            (info['remote'], target)}


# Start only after the update helpers and trusted URL checks have been defined.
threading.Thread(target=monitor_releases, daemon=True).start()


def open_manager():
    """从面板唤起更新管理器（真正的下载/构建/替换在那边做）。"""
    supported, reason = _local_adaptation_capability()
    if not supported:
        return {'ok': False, 'error': reason}
    exe = ROOT / 'WuwaTappoManager.exe'
    try:
        if exe.exists():
            command = [str(exe)]
        else:
            project = next((candidate for candidate in (ROOT.parent, ROOT.parent.parent, ROOT.parent.parent.parent)
                            if (candidate / 'tools' / 'manager.py').exists()), None)
            if not project or getattr(sys, 'frozen', False):
                return {'ok': False, 'error': '找不到 WuwaTappoManager.exe'}
            command = [sys.executable, str(project / 'tools' / 'manager.py')]
        subprocess.Popen(command, cwd=str(ROOT),
                         creationflags=0x00000008 | 0x00000200)
        return {'ok': True}
    except Exception as error:
        return {'ok': False, 'error': str(error)}


def asset_name_index():
    """Map an official remote file name to its content-addressed cache file.

    1.0.3 asks the asset host for role-based preview names
    (``role_1501_1001_head_1.png``) instead of the content hash alone.
    """
    global _NAME_INDEX
    if _NAME_INDEX is None:
        _NAME_INDEX = {}
        try:
            manifest = json.loads((ROOT / 'cache/asset_manifest.json').read_text(encoding='utf-8'))
            for record in manifest.get('records', []):
                base = urlparse(record['source']).path.rsplit('/', 1)[-1]
                if base and record.get('file'):
                    _NAME_INDEX[base] = record['file']
        except Exception:
            pass
    return _NAME_INDEX

LOCK = threading.RLock()
PROGRAMS = [ROOT / 'WuwaTappo.Local.exe', ROOT / 'runtime/msedgewebview2.exe']
PREFIX = 'WuwaTappo.LocalGallery.' + hashlib.sha256(str(ROOT).lower().encode()).hexdigest()[:12]
PS = str(Path(__import__('os').environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe')

def powershell(source):
    source = "$ProgressPreference='SilentlyContinue';" + source
    encoded = base64.b64encode(source.encode('utf-16le')).decode('ascii')
    proc = subprocess.run([PS, '-NoProfile', '-NonInteractive', '-EncodedCommand', encoded],
                          capture_output=True, encoding='utf-8', errors='replace',
                          creationflags=0x08000000, timeout=60)
    if proc.returncode:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or 'Firewall operation failed')
    return proc.stdout.strip()

def quote(value):
    return "'" + str(value).replace("'", "''") + "'"

def desired_offline():
    try:
        return json.loads(STATE.read_text(encoding='utf-8')).get('enabled') is True
    except (OSError, ValueError):
        return False

def set_firewall(enabled):
    """Remove only legacy whole-process blocking rules; never create new ones."""
    if enabled:
        raise ValueError('Full-process blocking was removed; local mode only blocks rooms.')
    if not ctypes.windll.shell32.IsUserAnAdmin():
        result = powershell("$ErrorActionPreference='Stop';$r=@(Get-NetFirewallRule -PolicyStore ActiveStore -Name " + quote(PREFIX+'.*') + " -ErrorAction SilentlyContinue);if($r.Count -eq 0){'no_rules'}else{'rules_exist'}")
        if result.splitlines()[-1:] == ['no_rules']:
            return
        raise PermissionError('请通过启动器授予管理员权限，以清理旧版本的断网规则。')
    powershell("$ErrorActionPreference='Stop';Get-NetFirewallRule -Name " + quote(PREFIX+'.*') + " -ErrorAction SilentlyContinue|Remove-NetFirewallRule;exit 0;")

def cache_ready():
    try:
        manifest = json.loads((ROOT / 'cache/asset_manifest.json').read_text(encoding='utf-8'))
        return not manifest['errors'] and len(manifest['records']) == CONFIG['asset_count'] and all(
            (ROOT / 'cache/assets' / r['file']).stat().st_size == r['size'] for r in manifest['records'])
    except (OSError, ValueError, KeyError):
        return False

def cache_summary():
    """给前端显示「已下载 / 应有」用。发行包不带 cache，首次启动要从 0 拉到 3000+。"""
    try:
        manifest = json.loads((ROOT / 'cache/asset_manifest.json').read_text(encoding='utf-8'))
        assets = len(manifest.get('records') or [])
    except (OSError, ValueError):
        assets = 0
    return {'assets': assets, 'expected': CONFIG.get('asset_count') or 0}

READY = cache_ready()
ACTUAL_OFFLINE = False
SYNC = {'status': 'idle', 'done': 0, 'total': 0, 'checked_at': 0}
CUSTOM_ERROR = None


def _region_code():
    return 'GLOBAL' if CONFIG.get('region') == 'global' else 'CN'


def official_bundle():
    catalog = json.loads((ROOT / 'cache/offline_catalog.json').read_text(encoding='utf-8'))
    atlas = json.loads((ROOT / 'cache/atlas.json').read_text(encoding='utf-8'))['data']
    return {'catalog': catalog, 'atlas': atlas, 'atlas_count': len(atlas),
            'asset_count': CONFIG.get('asset_count', 0)}


def custom_bundles():
    with CUSTOM_LOCK:
        return list_packs(CUSTOM_ROOT, _region_code())


def custom_details():
    with CUSTOM_LOCK:
        return list_details(CUSTOM_ROOT, _region_code())


def merged_bundle():
    """Mod 叠加失败时降级返回官方数据，不让本地图鉴整体不可用。"""
    global CUSTOM_ERROR
    official = official_bundle()
    try:
        with CUSTOM_LOCK:
            data = merge_official(official, custom_bundles())
        CUSTOM_ERROR = None
        return data
    except Exception as error:
        CUSTOM_ERROR = str(error)
        official['atlas_count'] = len(official['atlas'])
        return official


def resources_ready():
    return READY

def start_sync(force=False):
    global READY
    with LOCK:
        if SYNC['status'] == 'running':
            return dict(SYNC)
        if not force and time.time() - SYNC['checked_at'] < 900:
            return dict(SYNC)
        SYNC.update(status='running', done=0, total=0, error=None)
    def worker():
        global READY
        try:
            def progress(done, total):
                SYNC.update(done=done, total=total)
            result = sync_public_cache(ROOT, progress, api=API_BASE, allowed_hosts=[CDN_HOST])
            with LOCK:
                CONFIG.update(asset_count=result['asset_count'], atlas_count=result['atlas_count'])
                (ROOT / 'local_config.json').write_text(json.dumps(CONFIG, indent=2), encoding='utf-8')
                READY = cache_ready()
                SYNC.update(status='complete', result=result, checked_at=time.time())
        except Exception as error:
            with LOCK:
                SYNC.update(status='failed', error=str(error), checked_at=time.time())
    threading.Thread(target=worker, daemon=True).start()
    return dict(SYNC)

class Handler(BaseHTTPRequestHandler):
    def cached_asset(self, name):
        candidates=[ROOT/'cache/assets'/name]
        for previous in sorted(ROOT.glob('cache_previous_*'),reverse=True)[:2]:
            if previous.is_dir() and previous.resolve().is_relative_to(ROOT):
                candidates.append(previous/'assets'/name)
        for candidate in candidates:
            if candidate.is_file() and candidate.resolve().is_relative_to(ROOT):
                return candidate
        return None

    def log_message(self, *args):
        pass

    def headers_common(self):
        origin = self.headers.get('Origin', '')
        if origin in ['http://tauri.localhost', 'https://tauri.localhost', 'tauri://localhost']:
            self.send_header('Access-Control-Allow-Origin', origin)
            self.send_header('Vary', 'Origin')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type, X-Local-Token')
        self.send_header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
        self.send_header('Access-Control-Allow-Private-Network', 'true')
        self.send_header('Cross-Origin-Resource-Policy', 'cross-origin')
        self.send_header('Cache-Control', 'no-store')

    def redirect(self, location):
        self.send_response(302)
        self.headers_common()
        self.send_header('Location', location)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def respond(self, code, data, content_type='application/json; charset=utf-8'):
        if not isinstance(data, bytes):
            data = json.dumps(data, ensure_ascii=False).encode('utf-8')
        self.send_response(code)
        self.headers_common()
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_OPTIONS(self):
        self.respond(200, {})

    def do_GET(self):
        path = urlparse(self.path).path
        if path == '/bootstrap':
            if self.headers.get('Origin') not in ['http://tauri.localhost','https://tauri.localhost','tauri://localhost']:
                return self.respond(403, {'error':'App origin required'})
            return self.respond(200, {'token':TOKEN})
        if path == '/status':
            return self.respond(200, {'ready': resources_ready(), 'enabled': ACTUAL_OFFLINE,
                                      'mod_version': CONFIG.get('mod_version'), 'client_version': CONFIG.get('client_version'),
                                      'firewall_verified': False, 'mode_verified': True, 'network_policy': 'rooms-only',
                                      'asset_count': CONFIG['asset_count'], 'atlas_count': CONFIG['atlas_count'],
                                      'cache': cache_summary(), 'sync': dict(SYNC)})
        if path == '/sync/status':
            return self.respond(200, dict(SYNC))
        if path == '/custom/status':
            packs = custom_details()
            info = {'installed': bool(packs), 'enabled': ACTUAL_OFFLINE, 'name': '、'.join(pack['name'] for pack in packs),
                    'atlas_count': sum(pack['item_count'] for pack in packs),
                    'asset_count': sum(pack['asset_count'] for pack in packs),
                    'packs': packs, 'region': _region_code(), 'error': CUSTOM_ERROR}
            return self.respond(200, info)
        if path == '/offline-data':
            if not resources_ready():
                return self.respond(409, {'error': '本地素材还在准备中，请等同步完成后再开启本地图鉴'})
            return self.respond(200, merged_bundle())
        if path == '/asset-base':
            # 与 macOS 的 mod_assets 路由同义：前端拿这个前缀拼 custom-assets/... 键。
            return self.respond(200, {'base': 'http://127.0.0.1:%d/' % CONFIG['port']})
        if path.startswith('/custom-assets/'):
            relative = unquote(path.removeprefix('/custom-assets/'))
            parts = relative.split('/')
            if len(parts) < 2 or any(part in ('', '.', '..') or '\\' in part or ':' in part for part in parts):
                return self.respond(400, {'error': 'Invalid path'})
            asset = (CUSTOM_ROOT / parts[0] / 'assets' / Path(*parts[1:])).resolve()
            if not asset.is_relative_to(CUSTOM_ROOT.resolve()) or not asset.is_file():
                return self.respond(404, {'error': 'Not found'})
            return self.respond(200, asset.read_bytes(), mimetypes.guess_type(asset.name)[0] or 'application/octet-stream')
        # Several official preview paths rebuild a type folder from the file
        # key. Cached file keys are content-addressed, so serve those aliases
        # from the same verified cache rather than falling back to the CDN.
        if path.startswith(('/character/', '/accessory/', '/emote/', '/emote_atlas/', '/character_skin_atlas/', '/accessory_atlas/')):
            name = path.rsplit('/', 1)[-1]
            cached = self.cached_asset(name)
            if not cached:
                mapped = asset_name_index().get(name)
                cached = self.cached_asset(mapped) if mapped else None
            if cached:
                return self.respond(200, cached.read_bytes(), 'image/png')
            # Unknown to the local cache: fall back to the public CDN instead of
            # breaking the image in either gallery mode.
            return self.redirect(CDN_HOST + path)
        if path.startswith('/assets/'):
            name = path.removeprefix('/assets/')
            if '/' in name or '\\' in name or '..' in name:
                return self.respond(400, {'error': 'Invalid path'})
            p = self.cached_asset(name)
            if p:
                return self.respond(200, p.read_bytes(), mimetypes.guess_type(p.name)[0] or 'application/octet-stream')
            with LOCK, (ROOT / 'client_diagnostics.jsonl').open('a', encoding='utf-8') as stream:
                stream.write(json.dumps({'kind': 'asset_not_found', 'path': path}) + '\n')
        return self.respond(404, {'error': 'Not found'})

    def read_body(self):
        # 未读完请求体就 respond 并关连接，Windows 会给客户端回 RST（WinError 10053）。
        # 分发前统一读一次并缓存，既保证连接干净，也避免各分支重复消费 socket。
        if not hasattr(self, '_request_body'):
            try:
                length = int(self.headers.get('Content-Length', '0') or 0)
            except ValueError:
                length = 0
            self._request_body = self.rfile.read(length) if 0 < length <= 65536 else b''
        return self._request_body

    def do_POST(self):
        global ACTUAL_OFFLINE
        self.read_body()
        if self.headers.get('Origin') not in [None,'http://tauri.localhost','https://tauri.localhost','tauri://localhost']:
            return self.respond(403, {'error':'App origin required'})
        if self.headers.get('X-Local-Token') != TOKEN:
            return self.respond(403, {'error': 'Invalid local control token'})
        path = urlparse(self.path).path
        if path in ('/official/check', '/official/update-check'):
            return self.respond(200, check_official())
        if path == '/official/releases':
            return self.respond(200, open_community_releases())
        if path == '/community/version':
            return self.respond(200, {'current': str(CONFIG.get('community_version') or '')})
        if path == '/community/update-check':
            return self.respond(200, check_community(ASSISTANT.prerelease_updates()))
        if path == '/official/pet-site':
            return self.respond(200, open_pet_site())
        if path == '/official/download':
            return self.respond(200, download_official())
        if path == '/official/manager':
            return self.respond(200, open_manager())
        if path in ('/custom/import', '/custom/remove'):
            try:
                length = int(self.headers.get('Content-Length', '0'))
                if not 0 <= length <= 16384:
                    raise ValueError('Request too large')
                body = json.loads(self.read_body() or b'{}')
                if not isinstance(body, dict):
                    raise ValueError('Object required')
                if path == '/custom/import':
                    with CUSTOM_LOCK:
                        info = install_archive(body.get('path', ''), CUSTOM_ROOT, _region_code())
                    return self.respond(200, dict(info, installed=True, enabled=ACTUAL_OFFLINE))
                if path == '/custom/remove':
                    with CUSTOM_LOCK:
                        remove_pack(CUSTOM_ROOT, body.get('id'))
                        packs = custom_bundles()
                    return self.respond(200, {'removed': body.get('id'), 'any_installed': bool(packs),
                                              'enabled': ACTUAL_OFFLINE})
            except Exception as error:
                return self.respond(409, {'error': str(error)})
        if path.startswith('/update/'):
            # 程序内更新（只做 Windows）：下载本仓库 Release 里的 update.zip，
            # 校验 SHA256 后由独立助手进程备份、替换、重启，失败自动回滚。
            # 没有可用升级包或安装失败时，前端退回「因系统原因跳 GitHub 下载」。
            action = path.rsplit('/', 1)[-1]
            try:
                body = json.loads(self.read_body() or b'{}')
            except Exception:
                body = {}
            if not isinstance(body, dict):
                body = {}
            if action == 'healthy':
                return self.respond(200, UPDATER.healthy(body.get('version')))
            if action == 'status':
                return self.respond(200, UPDATER.status())
            if action == 'check':
                try:
                    return self.respond(200, UPDATER.check())
                except Exception as error:
                    return self.respond(200, {**UPDATER.status(), 'installable': False,
                                              'message': '检查更新失败（%s）。' % error,
                                              'release_url': COMMUNITY_RELEASES_URL})
            if action == 'install':
                try:
                    return self.respond(200, UPDATER.prepare())
                except Exception as error:
                    return self.respond(409, {'error': str(error), 'installable': False,
                                              'release_url': COMMUNITY_RELEASES_URL})
            return self.respond(404, {'error': 'Unknown update action'})
        if path == '/startup':
            # 开机自启改由本机服务托管：计划任务跑 launch.ps1，先起本地服务再起桌宠。
            # 官方原来的自启项直接拉桌宠 exe，开机后图鉴和控制台会连不上服务。
            if STARTUP is None:
                return self.respond(501, {'error': 'Startup manager unavailable'})
            try:
                body = json.loads(self.read_body() or b'{}')
                action = body.get('action', 'status')
                if action not in ('status', 'enable', 'disable', 'migrate'):
                    raise ValueError('Unknown startup action')
                return self.respond(200, STARTUP.run(action))
            except Exception as error:
                return self.respond(409, {'error': str(error), 'enabled': False})
        if path == '/assistant/context-menu':
            # 原生菜单是阻塞调用（要等用户点完或收起），所以单开一条路由：
            # 它不该占着 Assistant 的锁，否则菜单开着时看板读不到状态。
            if popup_context_menu is None:
                return self.respond(501, {'action': None, 'error': 'Native menu unavailable'})
            try:
                if int(self.headers.get('Content-Length', '0')) > 16384:
                    raise ValueError('Request too large')
                body = json.loads(self.read_body() or b'{}')
                return self.respond(200, popup_context_menu(body.get('items')))
            except Exception as error:
                return self.respond(400, {'action': None, 'error': str(error)})
        if path.startswith('/assistant/'):
            try:
                length = int(self.headers.get('Content-Length', '0'))
                if not 0 <= length <= 16384:
                    raise ValueError('Request too large')
                body = json.loads(self.read_body() or b'{}')
                if not isinstance(body, dict):
                    raise ValueError('Object required')
                return self.respond(200, ASSISTANT.handle(path.rsplit('/', 1)[-1], body, ACTUAL_OFFLINE))
            except Exception as error:
                return self.respond(400, {'error': str(error)})
        if urlparse(self.path).path == '/sync':
            try:
                body = json.loads(self.read_body() or b'{}')
                return self.respond(200, start_sync(body.get('force') is True))
            except Exception as error:
                return self.respond(409, {'error': str(error)})
        if urlparse(self.path).path == '/diagnostic':
            try:
                length = int(self.headers.get('Content-Length', '0'))
                if length > 8192:
                    raise ValueError('Diagnostic too large')
                payload = json.loads(self.read_body())
                with LOCK, (ROOT / 'client_diagnostics.jsonl').open('a', encoding='utf-8') as stream:
                    stream.write(json.dumps(payload, ensure_ascii=False) + '\n')
                return self.respond(200, {'ok': True})
            except Exception as error:
                return self.respond(400, {'error': str(error)})
        if urlparse(self.path).path != '/mode':
            return self.respond(404, {'error': 'Not found'})
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if length > 1024:
                raise ValueError('Request too large')
            body = json.loads(self.read_body())
            enabled = body['enabled']
            if not isinstance(enabled, bool):
                raise ValueError('enabled must be boolean')
            with LOCK:
                if enabled and not resources_ready():
                    raise RuntimeError('离线素材缓存不完整，未开启。')
                set_firewall(False)
                ACTUAL_OFFLINE = enabled
                tmp = STATE.with_suffix('.tmp')
                tmp.write_text(json.dumps({'enabled': enabled}), encoding='utf-8')
                tmp.replace(STATE)
            return self.respond(200, {'enabled': enabled, 'firewall_verified': False, 'mode_verified': True, 'network_policy': 'rooms-only', 'ready': READY})
        except Exception as error:
            return self.respond(409, {'error': str(error)})

def main():
    global ACTUAL_OFFLINE
    parser = argparse.ArgumentParser()
    parser.add_argument('--restore-firewall', action='store_true')
    args = parser.parse_args()
    if args.restore_firewall:
        set_firewall(False)
        STATE.write_text('{"enabled":false}', encoding='utf-8')
        return
    if desired_offline():
        if resources_ready():
            set_firewall(False)
            ACTUAL_OFFLINE = True
        else:
            # 升级或手工清过缓存时素材可能还没补齐。这里不能像以前那样抛错退出：
            # 服务一退，前端连 /status 都拿不到，整块界面会被替换成一行
            # 「请通过本地版启动器运行」。退回联机模式并落盘，等后台同步补齐后
            # 用户重新打开开关即可。
            STATE.write_text(json.dumps({'enabled': False}), encoding='utf-8')
            print(json.dumps({'warning': 'offline mode requested but cache is incomplete; staying online'}), flush=True)
    else:
        set_firewall(False)
    server = ThreadingHTTPServer(('127.0.0.1', PORT), Handler)
    if not READY:
        # 发行包不带素材缓存（体积原因），首次启动就后台补齐，前端只显示进度。
        start_sync()
    print(json.dumps({'ready': READY, 'enabled': ACTUAL_OFFLINE, 'port': PORT}), flush=True)
    # 桌宠退出后释放后台服务（上游 1.3.4）：启动器是先把服务拉起来再开主程序、
    # 然后自己退出的，所以主程序关掉以后这个进程会一直挂着。这里盯一下主程序，
    # 它没了就收工，别在用户任务管理器里留一个常驻 python。
    retiring = threading.Event()
    try:
        from process_lifecycle import watch_native_exit
        threading.Thread(target=watch_native_exit, args=(ROOT, server.shutdown),
                         kwargs={'stop_event': retiring}, daemon=True).start()
    except Exception as error:
        print(json.dumps({'warning': 'watch_native_exit unavailable: %s' % error}), flush=True)
    server.serve_forever()
    server.server_close()
    ASSISTANT.save(True)

if __name__ == '__main__':
    main()
