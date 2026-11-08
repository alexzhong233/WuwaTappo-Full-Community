"""Validated mod-release updates with a reversible file transaction."""
from pathlib import Path, PurePosixPath
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
import uuid
import zipfile
from desktop_assistant import atomic_json

try:
    import psutil
except ImportError:      # 只有升级助手和进程枚举需要它；缺了也不该让导入失败
    psutil = None

REPO = 'alexzhong233/WuwaTappo-Full-Community'
# 发行包顶层允许被原地更新覆盖的文件。必须与 windows/tools/payload.ps1 的判断一致：
# 多写一个就会把运行期状态带进更新包，少写一个则用户升级后拿到旧程序。
TOP_LEVEL = {'WuwaTappo.Local.exe', 'LocalGalleryHost.exe', 'WuwaTappoLauncher.exe',
             'package_config.json', 'app.ico', 'initialize.ps1',
             'README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md'}
# host\ 是早期布局里冻结服务的副本，运行期只读根目录那份；更新包不带它，
# 免得每个用户白下二十多兆。
DIRECTORIES = {'_internal', 'licenses', 'docs'}
PROTECTED = {'local_config.json', 'offline_state.json', 'assistant_state.json', 'update_policy.json',
             'update_hint.json', 'official_update.json', 'profile', 'cache', 'custom-atlas', 'runtime'}


def release_prefix(config):
    """发布产物前缀，跟 tools/region.ps1 的 ArtifactPrefix 一致。"""
    return 'WuwaTappo-Full-Global-Community' if config.get('region') == 'global' else 'WuwaTappo-Full-CN-Community'


def release_asset_name(config, version):
    return release_prefix(config) + '-' + version + '-win-x64-update.zip'


def installed_version(config):
    """本机整合包版本。community_version 才是本仓库的发布号，mod_version 是上游 MOD 号。"""
    return str(config.get('community_version') or config.get('mod_version') or '')


def safe_name(name):
    path = PurePosixPath(name)
    if not name or '\\' in name or ':' in name or path.is_absolute() or any(x in ('', '.', '..') for x in name.split('/')):
        raise ValueError('Unsafe update path')
    for part in path.parts:
        if part.endswith((' ', '.')) or any(ord(c)<32 for c in part) or re.fullmatch(r'(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?',part,re.I):
            raise ValueError('Unsafe Windows path')
    if path.parts[0] in PROTECTED or (len(path.parts) == 1 and name not in TOP_LEVEL) or (len(path.parts)>1 and path.parts[0] not in DIRECTORIES):
        raise ValueError('Update tried to replace data or an unknown component')
    return path


def digest(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024*1024), b''):
            result.update(chunk)
    return result.hexdigest()


def _prerelease_key(value):
    """预发布段按 semver 排序：数字段比字母段小，alpha.10 大于 alpha.9。"""
    parts = []
    for token in (value or '').split('.'):
        parts.append((0, int(token), '') if token.isdigit() else (1, 0, token))
    return tuple(parts)


def version_tuple(value):
    """版本比较键：支持 1.3.0-alpha.4 这类我们自己发出去的预发布号。"""
    match = re.fullmatch(r'(\d+(?:\.\d+){1,3})(?:-([0-9A-Za-z.-]+))?', str(value or ''))
    if not match:
        raise ValueError('Invalid release version')
    numbers = tuple(int(x) for x in match.group(1).split('.'))
    numbers += (0,) * (4 - len(numbers))
    pre = match.group(2) or ''
    return (numbers, 0 if pre else 1, _prerelease_key(pre))


def is_prerelease(value):
    return bool(re.fullmatch(r'\d+(?:\.\d+){1,3}-[0-9A-Za-z.-]+', str(value or '')))


def validate_manifest(manifest):
    if manifest.get('schema') != 1:
        raise ValueError('Unknown update format')
    version_tuple(manifest['version'])
    files = manifest['files']
    if not isinstance(files, list) or not 3 <= len(files) <= 5000:
        raise ValueError('Invalid update inventory')
    seen = set()
    total = 0
    for item in files:
        safe_name(item['path'])
        key = item['path'].casefold()
        if key in seen or not re.fullmatch('[0-9a-f]{64}', item['sha256']):
            raise ValueError('Duplicate path or invalid hash')
        seen.add(key)
        if type(item['size']) is not int or not 0 <= item['size'] <= 300*1024*1024:
            raise ValueError('Invalid update size')
        total += item['size']
    if total > 1024*1024*1024:
        raise ValueError('Update too large')
    if not {'wuwatappo.local.exe','localgalleryhost.exe','package_config.json'} <= seen:
        raise ValueError('Missing executable or public configuration')
    return manifest


def unpack(archive, stage):
    stage = Path(stage)
    stage.mkdir(parents=True, exist_ok=False)
    with zipfile.ZipFile(archive) as z:
        info = z.getinfo('update_manifest.json')
        if info.file_size > 2*1024*1024:
            raise ValueError('Manifest too large')
        manifest = validate_manifest(json.loads(z.read(info)))
        inventory = {x['path']: x for x in manifest['files']}
        if len(z.infolist()) != len(inventory)+1 or set(z.namelist()) != set(inventory)|{'update_manifest.json'}:
            raise ValueError('Archive inventory mismatch')
        for name, item in inventory.items():
            info = z.getinfo(name)
            if info.file_size != item['size'] or (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError('Invalid archive entry')
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with z.open(info) as source, target.open('wb') as dest:
                shutil.copyfileobj(source, dest)
            if digest(target) != item['sha256']:
                raise ValueError('Update hash mismatch')
    config = json.loads((stage/'package_config.json').read_text(encoding='utf-8-sig'))
    if 'token' in config or installed_version(config) != manifest['version']:
        raise ValueError('Invalid public configuration')
    atomic_json(stage/'update_manifest.json', manifest)
    return manifest


class FileTransaction:
    def __init__(self, root, stage):
        self.root, self.stage = Path(root).resolve(), Path(stage).resolve()
        self.manifest = validate_manifest(json.loads((self.stage/'update_manifest.json').read_text(encoding='utf-8')))
        self.backup = self.stage.parent/'backup'
        self.changed = []

    def apply(self):
        # Validate every staged byte before replacing even one installed file.
        for item in self.manifest['files']:
            source = self.stage/item['path']
            if not source.is_file() or source.stat().st_size != item['size'] or digest(source) != item['sha256']:
                raise ValueError('Staged content changed')
            target = self.root/item['path']
            if not target.resolve().is_relative_to(self.root) or target.is_symlink():
                raise ValueError('Target escapes installation')
        for item in self.manifest['files']:
            name = item['path']
            source, target, backup = self.stage/name, self.root/name, self.backup/name
            existed = target.exists()
            if existed:
                backup.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(target, backup)
            self.changed.append({'path': name, 'existed': existed})
            atomic_json(self.stage.parent/'transaction.json', self.changed)
            target.parent.mkdir(parents=True, exist_ok=True)
            temp = target.with_name(target.name+'.update-tmp')
            shutil.copy2(source, temp)
            os.replace(temp, target)

    def rollback(self):
        for item in reversed(self.changed):
            target = self.root/item['path']
            if item['existed']:
                shutil.copy2(self.backup/item['path'], target)
            elif target.exists():
                target.unlink()


class Updater:
    """本仓库 GitHub 发布的可选原地升级。

    与上游的区别：这里始终是可选更新（required 恒为 False），因为一旦强制，
    用户被挡在旧版本里而升级包又出错时就没有别的出口；不发或发不出升级包时
    前端改走「因系统原因跳 GitHub 下载安装包」。
    """

    def __init__(self, root, config, allow_prerelease=None):
        self.root, self.config = Path(root), config
        self.version = installed_version(config)
        self.allow_prerelease = allow_prerelease or (lambda: False)
        self.available = None
        self.lock = threading.Lock()
        self.check_lock = threading.Lock()
        self.policy_file = self.root/'update_policy.json'
        self.last_error = None
        self.checked_at = 0
        self.latest = None
        self.release_url = None
        if self.policy_file.exists():
            try:
                saved = json.loads(self.policy_file.read_text(encoding='utf-8'))
                candidate = saved.get('release')
                if candidate and self.valid_candidate(candidate) and version_tuple(candidate['version']) > version_tuple(self.version):
                    self.available = candidate
            except (ValueError, KeyError, TypeError):
                pass

    def valid_candidate(self, value):
        version = value['version']
        version_tuple(version)
        prefix = 'https://github.com/'+REPO+'/releases/download/v'+version+'/'
        other = 'https://github.com/'+REPO+'/releases/download/'+version+'/'
        return (isinstance(value['url'], str) and value['url'].startswith((prefix, other)) and
                value['url'].endswith('/'+release_asset_name(self.config, version)) and
                re.fullmatch('[0-9a-f]{64}', value['sha256']) and
                type(value['size']) is int and 0 < value['size'] <= 500*1024*1024)

    def fetch(self, url, max_size=2*1024*1024):
        req = urllib.request.Request(url, headers={'User-Agent':'WuwaTappo-FullAtlas-Updater','Accept':'application/vnd.github+json'})
        with urllib.request.urlopen(req, timeout=45) as response:
            data = response.read(max_size+1)
        if len(data)>max_size:
            raise ValueError('Download exceeds limit')
        return data

    def check(self):
        with self.check_lock:
            try:
                result = self._check()
                self.checked_at = int(time.time())
                self.last_error = None
                return {**self.status(), **result}
            except Exception as error:
                self.last_error = str(error)
                if self.available:
                    return self.status()
                raise

    def allow_pre(self):
        """是否把预发布算进候选——只看用户在控制台里打开的开关，和版本检查保持同一套规则。"""
        return bool(self.allow_prerelease())

    def _release_list(self):
        # /releases/latest 会跳过 prerelease，而本项目的 alpha 正以 prerelease 发布，
        # 用那个接口永远查不到自己的新版本。自己扫列表，规则与前端版本检查一致。
        return json.loads(self.fetch('https://api.github.com/repos/'+REPO+'/releases?per_page=30', 512*1024))

    def _check(self):
        try:
            releases = self._release_list()
        except Exception:
            # 共享出口 IP 打满匿名配额时，公开发布页仍然可用。
            release = self.public_release()
            releases = [release] if release.get('assets') else []
        include_pre = self.allow_pre()
        best = None
        for release in releases if isinstance(releases, list) else []:
            if not isinstance(release, dict) or release.get('draft'):
                continue
            tag = str(release.get('tag_name') or '')
            version = tag[1:] if tag[:1] in 'vV' else tag
            try:
                version_tuple(version)
            except ValueError:
                continue
            if release.get('prerelease') and not include_pre:
                continue
            if version_tuple(version) <= version_tuple(self.version):
                continue
            if best is None or version_tuple(version) > version_tuple(best[0]):
                best = (version, release)
        if not best:
            if self.available:
                return self.status()
            return {'available':False,'latest':self.latest,'installable':False,
                    'message':'当前整合版 '+self.version+'，已是最新发布版本。'}
        version, release = best
        self.latest = version
        self.release_url = str(release.get('html_url') or ('https://github.com/'+REPO+'/releases'))
        asset = next((a for a in release.get('assets') or [] if a.get('name') == release_asset_name(self.config, version)), None)
        sums = next((a for a in release.get('assets') or [] if a.get('name') == 'SHA256SUMS.txt'), None)
        prefix = 'https://github.com/'+REPO+'/releases/download/'
        if not asset or not sums or not str(asset.get('browser_download_url','')).startswith(prefix) or not str(sums.get('browser_download_url','')).startswith(prefix):
            # 有新版但没有可用的原地升级包（比如手建的空 Release）：不改动任何文件，
            # 前端据此把按钮降级成「跳 GitHub 下载」。
            return {'available':False,'latest':version,'installable':False,'release_url':self.release_url,
                    'message':'发现新版本 '+version+'，但这个发布没有程序内升级包；请到 GitHub 下载安装包。'}
        entries = self.fetch(sums['browser_download_url'],65536).decode('utf-8-sig')
        match = re.search(r'^([0-9a-fA-F]{64})\s+\*?'+re.escape(asset['name'])+r'\s*$', entries, re.M)
        if not match or not 0 < int(asset.get('size') or 0) <= 500*1024*1024:
            raise ValueError('升级包缺少有效 SHA256 校验')
        candidate = {'version':version,'url':asset['browser_download_url'],'sha256':match[1].lower(),'size':int(asset['size'])}
        if not self.valid_candidate(candidate):
            raise ValueError('Invalid release download destination')
        if self.available and version_tuple(self.available['version']) > version_tuple(version):
            return self.status()
        self.available = candidate
        atomic_json(self.policy_file, {'repository':REPO,'release':self.available})
        return {**self.status(), 'latest':version, 'installable':True, 'release_url':self.release_url}

    def public_release(self):
        request = urllib.request.Request('https://github.com/'+REPO+'/releases/latest',headers={'User-Agent':'WuwaTappo-Full-Community-Updater'})
        with urllib.request.urlopen(request,timeout=45) as response:
            final = response.geturl()
        prefix = 'https://github.com/'+REPO+'/releases/tag/'
        if not final.startswith(prefix):
            raise ValueError('无法确定整合版发布版本')
        tag = final.removeprefix(prefix)
        version = tag.removeprefix('v')
        version_tuple(version)
        result = {'tag_name':tag,'assets':[],'html_url':final}
        if version_tuple(version) <= version_tuple(self.version):
            return result
        if is_prerelease(version) and not self.allow_pre():
            return result
        base = 'https://github.com/'+REPO+'/releases/download/'+tag+'/'
        name = release_asset_name(self.config, version)
        try:
            with urllib.request.urlopen(urllib.request.Request(base+name,method='HEAD',headers={'User-Agent':'WuwaTappo-Full-Community-Updater'}),timeout=45) as response:
                size = int(response.headers.get('Content-Length','0'))
            result['assets'] = [{'name':name,'browser_download_url':base+name,'size':size},
                                {'name':'SHA256SUMS.txt','browser_download_url':base+'SHA256SUMS.txt'}]
        except Exception:
            pass
        return result

    def _spawn_helper(self, helper, folder):
        try:
            subprocess.Popen([str(helper/'LocalGalleryHost.exe'),'--apply-update',str(self.root),str(folder/'stage')],
                             cwd=helper,creationflags=0x08000000)
        except OSError as error:
            self.last_error = '无法启动升级助手：%s' % error

    def prepare(self):
        if not self.lock.acquire(blocking=False):
            raise ValueError('更新已在准备中')
        try:
            if not self.available:
                self.check()
            if not self.available:
                raise ValueError('没有可用的程序内升级包；请到 GitHub 下载安装包')
            updates = self.root/'.updates'
            updates.mkdir(exist_ok=True)
            folder = updates/uuid.uuid4().hex
            folder.mkdir()
            archive = folder/'download.zip'
            data = self.fetch(self.available['url'],self.available['size'])
            if len(data)!=self.available['size'] or hashlib.sha256(data).hexdigest()!=self.available['sha256']:
                raise ValueError('升级包校验失败，原程序未变动')
            archive.write_bytes(data)
            manifest = unpack(archive,folder/'stage')
            if manifest['version'] != self.available['version']:
                raise ValueError('Release version mismatch')
            helper = folder/'helper'
            helper.mkdir()
            shutil.copy2(self.root/'LocalGalleryHost.exe',helper/'LocalGalleryHost.exe')
            shutil.copytree(self.root/'_internal',helper/'_internal')
            atomic_json(folder/'pending.json',{'version':manifest['version'],'id':folder.name})
            # 先把响应交回前端，再拉起助手：助手第一步就是结束本宿主，
            # 同一轮里 Popen 会让「正在升级」这句提示永远发不出去。
            timer = threading.Timer(1.2, self._spawn_helper, (helper, folder))
            timer.daemon = True
            timer.start()
            return {'restarting':True,'message':'升级包已校验。正在退出桌宠、备份并原地升级…'}
        finally:
            self.lock.release()

    def status(self):
        if self.available:
            return {'available':True,'required':False,'installable':True,'version':self.available['version'],
                    'current':self.version,'repository':REPO,'checked_at':self.checked_at,'error':self.last_error,
                    'release_url':'https://github.com/'+REPO+'/releases',
                    'message':'可更新至 '+self.available['version']+'。升级保留账号、图鉴缓存与设置。'}
        return {'available':False,'required':False,'installable':False,'current':self.version,
                'repository':REPO,'checked_at':self.checked_at,'error':self.last_error,
                'release_url':'https://github.com/'+REPO+'/releases',
                'message':'当前整合版 '+self.version}

    def healthy(self, version=None):
        # 前端上报的是上游 MOD 号（1.2 之类），本机发布号在 package_config 里，
        # 两者对不上是常态；这里只按 pending 回执，不拿版本号当门禁。
        for pending in (self.root/'.updates').glob('*/pending.json'):
            try:
                value = json.loads(pending.read_text(encoding='utf-8'))
            except (OSError, ValueError):
                continue
            if value.get('version') == self.version:
                (pending.parent/'healthy').write_text(str(value['version']),encoding='utf-8')
        return {'ok':True,'version':self.version}


def processes(root):
    if psutil is None:
        return []
    root = Path(root).resolve()
    result = []
    for proc in psutil.process_iter(['pid','exe']):
        try:
            exe = Path(proc.info['exe'] or '')
            if exe.is_file() and exe.resolve().is_relative_to(root) and '.updates' not in exe.parts and exe.name.lower() in ('wuwatappo.local.exe','wuwatappolauncher.exe','localgalleryhost.exe','msedgewebview2.exe'):
                result.append(proc)
        except (OSError, psutil.Error):
            pass
    return result


def launch(root):
    """升级完把桌宠重新拉起来：优先走无控制台的启动器，老安装退回 launch.ps1。"""
    launcher = Path(root)/'WuwaTappoLauncher.exe'
    if launcher.is_file():
        subprocess.Popen([str(launcher)], cwd=root, creationflags=0x08000000)
        return
    powershell = Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'
    subprocess.Popen([str(powershell),'-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',str(root/'launch.ps1')],
                     cwd=root,creationflags=0x08000000)


def apply_update(root_arg, stage_arg):
    root, stage = Path(root_arg).resolve(), Path(stage_arg).resolve()
    if not stage.is_relative_to(root/'.updates') or stage.name!='stage':
        raise ValueError('Invalid update stage')
    folder = stage.parent
    transaction = FileTransaction(root, stage)
    try:
        # Original exit flow persists counters. Never force-close the old app.
        deadline = time.monotonic()+60
        while processes(root):
            running = processes(root)
            if not any(Path(p.exe()).name.lower()=='wuwatappo.local.exe' for p in running):
                # v1.0's host has no quit endpoint. Once its app has exited, it
                # holds no unsaved renderer counters; stop only that exact host.
                for proc in running:
                    if Path(proc.exe()).resolve() == root/'LocalGalleryHost.exe':
                        proc.terminate()
            if time.monotonic()>deadline:
                raise TimeoutError('旧桌宠仍在运行，升级取消；原文件未替换。')
            time.sleep(.5)
        transaction.apply()
        launch(root)
        deadline = time.monotonic()+90
        while not (folder/'healthy').exists():
            if time.monotonic()>deadline:
                raise TimeoutError('新版本未通过启动检查，正在恢复旧版本。')
            time.sleep(.5)
        # Only delete this verified update's own backup, after renderer acknowledgment.
        if transaction.backup.resolve().is_relative_to(root/'.updates') and transaction.backup.exists():
            shutil.rmtree(transaction.backup)
        if stage.resolve().is_relative_to(root/'.updates'):
            shutil.rmtree(stage)
        (folder/'download.zip').unlink(missing_ok=True)
        (folder/'pending.json').unlink(missing_ok=True)
        atomic_json(folder/'result.json',{'ok':True,'version':transaction.manifest['version']})
    except Exception as error:
        if transaction.changed:
            for proc in processes(root):
                proc.terminate()
            if psutil is not None:
                psutil.wait_procs(processes(root),timeout=10)
            transaction.rollback()
            launch(root)
        else:
            # The old application may still be open, but its host already exited.
            launch(root)
        atomic_json(folder/'result.json',{'ok':False,'error':str(error)})
