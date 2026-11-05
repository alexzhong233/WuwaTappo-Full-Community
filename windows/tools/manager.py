"""WuwaTappo 全图鉴版 更新管理器（Tkinter）。

功能：
  - 显示本地基底版本 / 官方最新版本 / 上次检测时间
  - 立即检测按钮（手动查官方清单）
  - 更新并打补丁按钮（检测到新版才可用）：跑 update-watch.ps1 试注入，
    成功后询问是否 apply-update.ps1 正式替换（带备份回滚）
  - 每 8 小时自动检测一次（窗口开着时；另可配合计划任务在后台查）
  - 启动游戏按钮（用 runas 拉起提权启动器，管理器自身无需提权）

命令行：
  WuwaTappoManager.exe                 打开管理器窗口
  WuwaTappoManager.exe --check-only    静默查一次，有新版本弹提示后退出（供计划任务用）
  WuwaTappoManager.exe --play          直接启动游戏（等价于旧启动器）

编译：tools\\build-manager.ps1
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

import tkinter as tk
from tkinter import ttk, messagebox
from tkinter.scrolledtext import ScrolledText

MANIFEST_HOST = 'https://mc-wuwatappo.kurogames-global.com'
STATE_FILE = 'update_state.json'
CHECK_INTERVAL = 8 * 3600
HTTP_TIMEOUT = 8
CREATE_NO_WINDOW = 0x08000000

APP_EXE = 'WuwaTappo.Local.exe'
HOST_EXE = 'LocalGalleryHost.exe'
LAUNCHER_EXE = 'WuwaTappoLauncher.exe'

MB_ICONINFORMATION = 0x40
MB_ICONWARNING = 0x30
MB_TOPMOST = 0x40000
MB_SETFOREGROUND = 0x10000


# ------------------------------------------------------------------ 路径与配置

def app_dir():
    if getattr(sys, 'frozen', False):
        # 打包后 exe 就放在 app\ 里
        return os.path.dirname(os.path.abspath(sys.executable))
    # 开发模式：脚本在 tools\ 下，运行目录是同级的 app\
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    app = os.path.join(root, 'app')
    return app if os.path.isdir(app) else root


def project_dir():
    """管理器在 app\\ 下时，项目根是上一级；独立放时退回 app 自己。"""
    root = app_dir()
    parent = os.path.dirname(root)
    if os.path.isdir(os.path.join(parent, 'tools')):
        return parent
    return root


# ------------------------------------------------------------------ 版本与网络

def vtup(text):
    nums = re.findall(r'\d+', str(text or '')[:16])
    return tuple(int(x) for x in nums[:3]) or (0,)


def local_version():
    try:
        with open(os.path.join(app_dir(), 'local_config.json'), 'r', encoding='utf-8') as fh:
            return str(json.load(fh).get('client_version') or '')
    except Exception:
        return ''


def _json_url(url, timeout=HTTP_TIMEOUT):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    req = urllib.request.Request(
        url, headers={'User-Agent': 'WuwaTappo-FullAtlas-Manager/1.0'})
    with opener.open(req, timeout=timeout) as resp:
        data = json.loads(resp.read().decode('utf-8'))
    return data if isinstance(data, dict) else {}


def _official_url(value, expected_name=None):
    parsed = urlparse(str(value or ''))
    host = urlparse(MANIFEST_HOST).hostname
    if (parsed.scheme != 'https' or parsed.hostname != host
            or parsed.port not in (None, 443) or parsed.username or parsed.password
            or parsed.fragment):
        raise RuntimeError('官方资源地址不是当前区域的 HTTPS 官方地址')
    name = os.path.basename(parsed.path)
    if expected_name and name != expected_name:
        raise RuntimeError('官方原包文件名与版本不匹配')
    return parsed.geturl()


def fetch_remote():
    """返回 dict：version / url / pub；失败抛异常。"""
    entry = _json_url(MANIFEST_HOST + '/app/json-host/latest_host.json')
    url = _official_url(entry.get('url'))
    if not url:
        raise RuntimeError('latest_host.json 未返回 url')
    # 必须用带 ?t= 的地址，否则命中 CDN 旧清单
    manifest = _json_url(url)
    version = str(manifest.get('version') or '')
    if not version:
        raise RuntimeError('latest.json 未返回 version')
    platforms = manifest.get('platforms')
    plat = platforms.get('windows-x86_64') if isinstance(platforms, dict) else None
    expected = 'WuwaTappo-cn_%s_x64-setup.exe' % version
    return {
        'version': version,
        'url': _official_url((plat or {}).get('url'), expected),
        'file_name': expected,
        'pub': str(manifest.get('pub_date') or ''),
    }


# ------------------------------------------------------------------ 状态存取

def read_state():
    try:
        with open(os.path.join(app_dir(), STATE_FILE), 'r', encoding='utf-8') as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def write_state(state):
    try:
        with open(os.path.join(app_dir(), STATE_FILE), 'w', encoding='utf-8') as fh:
            json.dump(state, fh, ensure_ascii=False, indent=2)
    except Exception:
        pass


# ------------------------------------------------------------------ 进程与弹窗

def app_running():
    """程序是否在跑（不含管理器自己）。用 tasklist 避免依赖 psutil。"""
    try:
        out = subprocess.run(
            ['tasklist', '/FI', 'IMAGENAME eq ' + APP_EXE, '/NH'],
            capture_output=True, text=True, encoding='gbk', errors='replace',
            creationflags=CREATE_NO_WINDOW, timeout=10).stdout
        return APP_EXE.lower() in (out or '').lower()
    except Exception:
        return False


def box(text, title='WuwaTappo 全图鉴版', flags=MB_ICONINFORMATION):
    try:
        ctypes.windll.user32.MessageBoxW(
            None, text, title, flags | MB_TOPMOST | MB_SETFOREGROUND)
    except Exception:
        pass


def launch_game():
    root = app_dir()
    exe = os.path.join(root, LAUNCHER_EXE)
    if not os.path.exists(exe):
        exe = os.path.join(root, APP_EXE)
    if not os.path.exists(exe):
        box('找不到主程序：\n%s' % exe, flags=MB_ICONWARNING)
        return False
    # 主程序清单是 requireAdministrator，必须提权拉起
    rc = ctypes.windll.shell32.ShellExecuteW(None, 'runas', exe, None, root, 1)
    if rc <= 32:
        box('启动失败（ShellExecute 返回 %d）。\n请直接双击桌面快捷方式。' % rc,
            flags=MB_ICONWARNING)
        return False
    return True


# ------------------------------------------------------------------ 静默检查模式

def check_only():
    """供计划任务调用：查一次，有新版就弹提示，然后退出。"""
    state = read_state()
    local = local_version()
    try:
        info = fetch_remote()
    except Exception:
        return 1
    state['last_check'] = time.time()
    state['last_remote'] = info['version']
    write_state(state)

    if not local or vtup(info['version']) <= vtup(local):
        return 0
    if state.get('notified_version') == info['version']:
        return 0
    state['notified_version'] = info['version']
    write_state(state)

    text = ('官方已发布新版本 %s。\n当前本地基底是 %s。\n\n'
            '打开「WuwaTappo 更新管理」可以一键下载并打补丁。\n'
            '（版本 %s 只提示这一次）' % (info['version'], local, info['version']))
    box(text, title='WuwaTappo 全图鉴版 · 官方有新版本')
    return 0


# ------------------------------------------------------------------ GUI

class Manager:
    def __init__(self, root):
        self.root = root
        self.remote = None          # 官方最新版本
        self.local = local_version()
        self.state = read_state()
        self.busy = False
        self.root_dir = project_dir()

        root.title('WuwaTappo 全图鉴版 · 更新管理')
        root.geometry('620x460')
        root.minsize(560, 400)
        try:
            root.iconbitmap(default='')
        except Exception:
            pass

        self._build_ui()
        self._refresh_labels()
        self.root.after(600, self.auto_kick)
        self.root.after(30000, self.tick)

    # -------------------------------------------------------------- UI 构建
    def _build_ui(self):
        style = ttk.Style()
        try:
            style.theme_use('vista')
        except Exception:
            pass
        style.configure('Title.TLabel', font=('Microsoft YaHei UI', 13, 'bold'))
        style.configure('Key.TLabel', font=('Microsoft YaHei UI', 9), foreground='#666')
        style.configure('Val.TLabel', font=('Consolas', 11, 'bold'))
        style.configure('State.TLabel', font=('Microsoft YaHei UI', 10))
        style.configure('Go.TButton', font=('Microsoft YaHei UI', 10, 'bold'))

        pad = {'padx': 14, 'pady': 8}
        frm = ttk.Frame(self.root)
        frm.pack(fill='both', expand=True, **pad)

        ttk.Label(frm, text='WuwaTappo 全图鉴版', style='Title.TLabel').grid(
            row=0, column=0, columnspan=4, sticky='w', pady=(0, 10))

        ttk.Label(frm, text='本地基底', style='Key.TLabel').grid(row=1, column=0, sticky='w')
        self.lbl_local = ttk.Label(frm, text='-', style='Val.TLabel')
        self.lbl_local.grid(row=2, column=0, sticky='w', padx=(0, 30))

        ttk.Label(frm, text='官方最新', style='Key.TLabel').grid(row=1, column=1, sticky='w')
        self.lbl_remote = ttk.Label(frm, text='-', style='Val.TLabel')
        self.lbl_remote.grid(row=2, column=1, sticky='w', padx=(0, 30))

        ttk.Label(frm, text='上次检测', style='Key.TLabel').grid(row=1, column=2, sticky='w')
        self.lbl_checked = ttk.Label(frm, text='未检测', style='Val.TLabel')
        self.lbl_checked.grid(row=2, column=2, sticky='w')

        self.lbl_state = ttk.Label(frm, text='就绪', style='State.TLabel')
        self.lbl_state.grid(row=3, column=0, columnspan=4, sticky='w', pady=(10, 2))

        self.lbl_next = ttk.Label(frm, text='', style='Key.TLabel')
        self.lbl_next.grid(row=4, column=0, columnspan=4, sticky='w')

        bar = ttk.Frame(frm)
        bar.grid(row=5, column=0, columnspan=4, sticky='ew', pady=(12, 10))
        self.btn_play = ttk.Button(bar, text='启动游戏', command=self.play, width=12)
        self.btn_play.pack(side='left')
        self.btn_check = ttk.Button(bar, text='立即检测', command=self.check, width=12)
        self.btn_check.pack(side='left', padx=8)
        self.btn_update = ttk.Button(bar, text='更新并打补丁', command=self.update,
                                     width=20, style='Go.TButton', state='disabled')
        self.btn_update.pack(side='left')

        ttk.Label(frm, text='日志', style='Key.TLabel').grid(row=6, column=0, sticky='w')
        self.log_box = ScrolledText(
            frm, height=10, font=('Consolas', 9), wrap='word', state='disabled',
            relief='solid', borderwidth=1)
        self.log_box.grid(row=7, column=0, columnspan=4, sticky='nsew', pady=(4, 0))

        frm.columnconfigure(0, weight=1)
        frm.rowconfigure(7, weight=1)

    # -------------------------------------------------------------- 小工具
    def log(self, msg):
        def _do():
            self.log_box.configure(state='normal')
            self.log_box.insert('end', msg.rstrip() + '\n')
            self.log_box.see('end')
            self.log_box.configure(state='disabled')
        self.root.after(0, _do)

    def set_state(self, text):
        self.root.after(0, lambda: self.lbl_state.configure(text=text))

    def set_busy(self, busy):
        self.busy = busy
        st = 'disabled' if busy else 'normal'
        self.root.after(0, lambda: self.btn_check.configure(state=st))
        self.root.after(0, lambda: self.btn_play.configure(state=st))
        self.root.after(0, self._sync_update_btn)

    def _sync_update_btn(self):
        if self.busy:
            self.btn_update.configure(state='disabled')
            return
        if self.remote and self.local and vtup(self.remote['version']) > vtup(self.local):
            self.btn_update.configure(
                state='normal', text='更新到 %s 并打补丁' % self.remote['version'])
        else:
            self.btn_update.configure(state='disabled', text='更新并打补丁')

    def _refresh_labels(self):
        self.lbl_local.configure(text=self.local or '未知')
        remote = self.remote['version'] if self.remote else '-'
        self.lbl_remote.configure(text=remote)
        ts = self.state.get('last_check')
        if ts:
            self.lbl_checked.configure(text=time.strftime('%m-%d %H:%M', time.localtime(float(ts))))
        self._sync_update_btn()

    def _update_next_label(self):
        ts = float(self.state.get('last_check') or 0)
        left = ts + CHECK_INTERVAL - time.time()
        if left <= 0:
            self.lbl_next.configure(text='即将自动检测…')
        else:
            h = int(left // 3600)
            m = int((left % 3600) // 60)
            self.lbl_next.configure(text='下次自动检测：%d 小时 %d 分后（每 8 小时一次）' % (h, m))

    # -------------------------------------------------------------- 自动检测
    def auto_kick(self):
        # 用户主动打开管理器，先查一次；8 小时节流只约束后台定时检测
        self.check(silent=True)

    def tick(self):
        self._update_next_label()
        if not self.busy:
            ts = float(self.state.get('last_check') or 0)
            if time.time() - ts >= CHECK_INTERVAL:
                self.check(silent=True)
        self.root.after(30000, self.tick)

    # -------------------------------------------------------------- 检测
    def check(self, silent=False):
        if self.busy:
            return
        self.set_busy(True)
        self.set_state('正在查询官方版本清单…')
        if not silent:
            self.log('— 开始检测 —')
        threading.Thread(target=self._check_worker, args=(silent,), daemon=True).start()

    def _check_worker(self, silent):
        info, err = None, None
        try:
            info = fetch_remote()
        except Exception as exc:
            err = str(exc)
        self.root.after(0, lambda: self._check_done(info, err, silent))

    def _check_done(self, info, err, silent):
        self.set_busy(False)
        now = time.time()
        if err:
            self.set_state('检测失败：%s' % err)
            self.log('检测失败：%s' % err)
            self._refresh_labels()
            return

        self.remote = info
        self.state['last_check'] = now
        self.state['last_remote'] = info['version']
        self.local = local_version()
        self._refresh_labels()
        self._update_next_label()

        newer = self.local and vtup(info['version']) > vtup(self.local)
        if newer:
            self.set_state('发现新版本 %s（当前 %s）' % (info['version'], self.local))
            self.log('官方最新 %s，本地基底 %s —— 有更新' % (info['version'], self.local))
            if self.state.get('notified_version') != info['version']:
                self.state['notified_version'] = info['version']
        else:
            self.set_state('已是最新（%s）' % self.local if self.local else '已是最新')
            self.log('官方最新 %s，本地基底 %s —— 无需更新' % (info['version'], self.local))
        write_state(self.state)

    # -------------------------------------------------------------- 运行脚本
    def _run_ps(self, script_name, extra_args, on_done):
        script = os.path.join(self.root_dir, 'tools', script_name)
        if not os.path.exists(script):
            self.log('找不到脚本：%s' % script)
            self.set_busy(False)
            return
        argline = ''.join(" '%s'" % a for a in extra_args)
        cmd = [
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-Command',
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; & '%s'%s" % (script, argline),
        ]
        self.log('$ powershell -File tools\\%s %s' % (script_name, ' '.join(extra_args)))

        def worker():
            try:
                proc = subprocess.Popen(
                    cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    text=True, encoding='utf-8', errors='replace',
                    creationflags=CREATE_NO_WINDOW, cwd=self.root_dir)
                for line in proc.stdout:
                    self.log(line.rstrip())
                proc.wait()
                code = proc.returncode
            except Exception as exc:
                self.log('执行异常：%s' % exc)
                code = -1
            self.root.after(0, lambda: on_done(code))

        threading.Thread(target=worker, daemon=True).start()

    # -------------------------------------------------------------- 更新
    def update(self):
        if not self.remote:
            return
        ver = self.remote['version']
        if app_running():
            box('游戏正在运行。\n\n请先完全退出游戏（包括右下角/后台的素材服务）再更新，\n'
                '否则文件被占用会导致更新失败。', flags=MB_ICONWARNING)
            return
        if not messagebox.askyesno(
                '开始更新到 %s' % ver,
                '将执行：\n\n'
                '1. 下载官方 %s 安装包\n'
                '2. 静默解包，提取主程序\n'
                '3. 试注入全图鉴补丁到 staging（不动现有版本）\n\n'
                '全程需要几分钟，界面会实时输出日志。\n'
                '注入失败会明确报错并保留现用版本。\n\n继续吗？' % ver):
            return

        self.set_busy(True)
        self.set_state('步骤 1/2：下载并试注入（可能需要几分钟）…')
        self.log('— 开始更新到 %s —' % ver)
        self._run_ps('update-watch.ps1', [], lambda code: self._stage_done(code, ver))

    def _stage_done(self, code, ver):
        if code != 0:
            self.set_state('试注入失败，现用版本未改动')
            self.log('试注入失败（退出码 %s）。把上面的报错发给维护者。' % code)
            self.set_busy(False)
            box('试注入失败。\n\n现用版本没有任何改动，可以继续用。\n'
                '详细原因见日志最后几行。', flags=MB_ICONWARNING)
            return

        self.set_state('步骤 2/2：等待确认是否替换')
        self.log('试注入成功。询问是否正式应用…')
        ask = messagebox.askyesno(
            '补丁注入成功',
            '全图鉴补丁已成功注入到新版本 %s。\n\n'
            '是否立刻替换正在使用的版本？\n\n'
            '· 当前主程序会备份到 build\\rollback\\，随时可回滚\n'
            '· 替换过程约需几分钟（重新打包 + 复制）\n'
            '· 替换后建议先启动一次确认图鉴正常' % ver)
        if not ask:
            self.log('用户选择暂不替换。staging 保留在 build\\updates\\%s' % ver)
            self.set_state('已准备好 %s，暂未替换' % ver)
            self.set_busy(False)
            return

        self.set_state('正在应用 %s（重新打包并替换）…' % ver)
        self._run_ps('apply-update.ps1', ['-Version', ver],
                     lambda c: self._apply_done(c, ver))

    def _apply_done(self, code, ver):
        self.set_busy(False)
        self.local = local_version()
        self._refresh_labels()
        if code == 0:
            self.set_state('已更新到 %s，建议启动一次确认' % ver)
            self.log('应用完成，本地基底 = %s' % self.local)
            box('更新完成：现在运行的是 %s。\n\n'
                '建议点「启动游戏」确认图鉴页正常。\n'
                '旧版备份在 build\\rollback\\，确认没问题后可删。' % ver)
        else:
            self.set_state('应用失败，已自动还原旧版')
            self.log('应用失败（退出码 %s），已尝试还原备份' % code)
            box('应用失败，已自动还原旧版。\n\n现用版本应仍可正常启动。\n'
                '详细原因见日志。', flags=MB_ICONWARNING)

    # -------------------------------------------------------------- 启动游戏
    def play(self):
        launch_game()


# ------------------------------------------------------------------ 入口

def main():
    args = sys.argv[1:]
    if '--check-only' in args:
        return check_only()
    if '--play' in args:
        launch_game()
        return 0

    root = tk.Tk()
    Manager(root)
    root.mainloop()
    return 0


if __name__ == '__main__':
    sys.exit(main())
