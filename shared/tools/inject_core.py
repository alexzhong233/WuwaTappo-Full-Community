"""两个平台共用的注入工具与公共补丁。

`macos/tools/fullatlas-integration.py` 与 `windows/src/tools/build_integration.py`
是同一类脚本：按 `js_structure.json` 里的位置索引找锚点、断言、然后做文本替换，
把本地全图鉴/Mod 的钩子织进官方前端产物。

两边真正一字不差的部分收在这里：锚点工具、构建根加载、六段公共补丁。
平台专属的补丁留在各自文件里 —— macOS 的 Tauri asset URL 改写、Windows 的
任务栏默认值和 PE 资源打包，锚点完全不同，硬抽只会变成满屏 if 分支。

改这个文件等于同时改两个平台的构建产物：动完一定要跑
`npm run test:fullatlas`，并在两边的构建根上各重跑一次注入器比对产物哈希。
"""
from pathlib import Path
import json
import os
import re
import shutil


def replace(text, old, new, count=1):
    """断言锚点出现次数后再替换；锚点漂了直接抛，别让构建带着半套补丁过。"""
    actual = text.count(old)
    assert actual == count, (old[:100], actual, count)
    return text.replace(old, new)


def apply_edits(text, edits):
    """按位置区间批量改写。从后往前应用，前面区间的偏移量才不会被影响。"""
    last = len(text) + 1
    for start, end, value in sorted(edits, key=lambda e: (e[0], e[1]), reverse=True):
        assert 0 <= start <= end <= last, (start, end, last)
        text = text[:start] + value + text[end:]
        last = start
    return text


def function_for(meta, text, needle):
    """定位包含某个特征的函数，多个命中取最短（最内层）那个。"""
    found = [f for f in meta['functions'] if needle in text[f['start']:f['end']]]
    assert found, needle
    return min(found, key=lambda f: f['end'] - f['start'])


def named(meta, name):
    found = [f for f in meta['functions'] if f['name'] == name]
    assert len(found) == 1, (name, found)
    return found[0]


def load_build(project):
    """读构建根，返回结构索引、区域配置、三份可读 JS 和待改的 frontend 副本。

    `WUWA_BUILD_ROOT` 由构建脚本传入，里面必须有 `current_base.txt`（指向官方
    基底目录）、`local_config.json`（区域与端口），基底目录里要有 `js_structure.json`
    和 `readable_js/assets/*.js`。缺任何一项都在这里就炸，不会等到装完之后才发现。
    """
    root = Path(os.environ['WUWA_BUILD_ROOT'])
    base = Path((root / 'current_base.txt').read_text(encoding='utf-8'))
    structure = json.loads((base / 'js_structure.json').read_text(encoding='utf-8'))
    config = json.loads((root / 'local_config.json').read_text(encoding='utf-8'))
    shared_name = structure['shared']
    shared = (base / 'readable_js/assets' / shared_name).read_text(encoding='utf-8')
    # 路由表里前九条就是我们关心的页面；/ 是桌宠窗口，/panel 是官方面板。
    route_region = shared[shared.index('const router = createRouter'):]
    routes = dict(re.findall(r'path:\s*"([^"]+)"[\s\S]*?import\("\./([^"]+)"\)', route_region)[:9])
    main_name, panel_name = routes['/'], routes['/panel']
    main = (base / 'readable_js/assets' / main_name).read_text(encoding='utf-8')
    panel = (base / 'readable_js/assets' / panel_name).read_text(encoding='utf-8')
    frontend = root / 'frontend_v2'
    shutil.copytree(base / 'frontend', frontend, dirs_exist_ok=True)
    return {'project': Path(project), 'root': root, 'base': base,
            'structure': structure, 'config': config, 'frontend': frontend,
            'shared_name': shared_name, 'main_name': main_name, 'panel_name': panel_name,
            'shared': shared, 'main': main, 'panel': panel}


# 本机 Mod 条目的名字要能显示：官方按 asset_name 查远端字典，Mod 名字不在字典里，
# 这里在查不到时回退到条目自带的名称（名字集合由 local_atlas_controller 登记）。
MOD_NAME_FALLBACK_OLD = '''function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  return s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : ""
}'''
MOD_NAME_FALLBACK_NEW = '''function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  const resolved = s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : "";
  // 本机 Mod 的名字不在官方远端字典里：查不到就直接用条目名，否则图鉴/背包只剩空白或占位符。
  const mods = typeof window !== "undefined" ? window.__WUWA_MOD_NAMES__ : null;
  return resolved || (mods && mods.has(n) ? n : "")
}'''


def patch_mod_display_name(text):
    """Mod 条目的显示名回退：官方字典查不到时用条目自带的名字。"""
    return replace(text, MOD_NAME_FALLBACK_OLD, MOD_NAME_FALLBACK_NEW)


def patch_mod_asset_url(text):
    """背包「我的预设」左侧预览走 buildAssetUrl 拼 CDN 地址，Mod 条目会拼出 404。

    地址拼接处先问一句本机 Mod 表，命中就返回本机路由，没命中照旧拼官方地址，
    官方素材的行为一字未改。参数名从产物里取，官方改名也不影响。
    """
    match = re.search(r'function buildAssetUrl\(([^)]*)\)\s*\{', text)
    assert match, 'official buildAssetUrl contract'
    params = [part.strip() for part in match.group(1).split(',')]
    assert len(params) == 2, params
    return replace(text, match.group(0), match.group(0) + '''
  const modAssetPath = (typeof window !== "undefined" && window.__WUWA_MOD_ASSET_PATH__) ? window.__WUWA_MOD_ASSET_PATH__(''' + params[1] + ''') : "";
  if (modAssetPath) return withAssetUrlCacheBust(modAssetPath);''')


def patch_online_services(text):
    """把官方启动时的联网初始化包成可重复调用的函数。

    全图鉴关掉时要补跑一次（用户可能在本地模式下挂机很久），所以不能留在启动
    那一刻的直线代码里。
    """
    return replace(text, 'loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();', '''let localOnlineServicesStarted = false;
function localStartOnlineServices() {
 if (localOnlineServicesStarted) return;
 localOnlineServicesStarted = true;
 loaderModule.init(kurodataConfig); window.kurodata = loaderModule;
 initTrackCommonProps(); initFrontendAlertReporting();
}''')


def patch_invoke_guard(text, strict=False):
    """在 tauri invoke 包装函数入口插一句 checkInvoke。

    包装函数名字随官方构建漂移（1.0.2 是 `invoke$1`，1.0.3/1.0.4 是 `invoke`），
    所以按实际存在的名字找，不看版本号。`strict` 时一个都找不到就抛（macOS 侧要求
    必须插上，否则 Restrictions 形同虚设）；否则跳过（Windows 侧历史构建里两个都
    没有，不该为此红掉整条构建）。
    """
    names = [name for name in ('invoke$1', 'invoke')
             if f'async function {name}(e, t = {{}}, n) {{' in text]
    if strict:
        assert names, 'tauri invoke wrapper contract'
    for name in names:
        anchor = f'async function {name}(e, t = {{}}, n) {{'
        text = replace(text, anchor, anchor + '\n window.__WUWA_LOCAL_ATLAS__?.checkInvoke(e, t);')
    return text


def patch_atlas_guards(text, extra_interceptor=''):
    """四处「本地模式短路」：素材目录、图鉴列表、请求拦截器。

    `extra_interceptor` 给平台追加自己的拦截器语句（Windows 用它压掉活动配置
    请求的全屏遮罩），插在同一条 use() 里，不额外增加锚点。
    """
    text = replace(text, 'async function ensureAppearanceAssetCatalog(e) {',
                   'async function ensureAppearanceAssetCatalog(e) {\n'
                   ' if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return true;')
    text = replace(text, 'function applyCatalog(e, t, n) {', '''function applyCatalog(e, t, n) {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && t !== window.__WUWA_LOCAL_ATLAS__.assetsHost) return;''')
    text = replace(text, 'function atlas() {', '''function atlas() {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return Promise.resolve({code: 0, data: window.__WUWA_LOCAL_ATLAS__.offlineData.value.atlas});''')
    text = replace(text, 'this.instance.interceptors.request.use(t => {', '''this.instance.interceptors.request.use(t => {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && window.__WUWA_LOCAL_ATLAS__.restricted(t.url,t.method)) throw window.__WUWA_LOCAL_ATLAS__.offlineError();''' + extra_interceptor)
    return text
