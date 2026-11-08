"""Apply the local-mode integration to the versioned official snapshot.

Bindings and component locations come from parsed JavaScript, not guessed
minified variable names. Unknown layouts fail before an installed file changes.
"""
from pathlib import Path
import hashlib, json, re, shutil, os, sys

PROJECT = Path(__file__).resolve().parents[1]
ROOT = Path(os.environ['WUWA_BUILD_ROOT'])
BASE = Path((ROOT / 'current_base.txt').read_text(encoding='utf-8'))
structure = json.loads((BASE / 'js_structure.json').read_text(encoding='utf-8'))
config = json.loads((ROOT / 'local_config.json').read_text(encoding='utf-8'))
shared_name = structure['shared']
shared = (BASE / 'readable_js/assets' / shared_name).read_text(encoding='utf-8')
route_region = shared[shared.index('const router = createRouter'):]
routes = dict(re.findall(r'path:\s*"([^"]+)"[\s\S]*?import\("\./([^"]+)"\)', route_region)[:9])
main_name, panel_name = routes['/'], routes['/panel']
main = (BASE / 'readable_js/assets' / main_name).read_text(encoding='utf-8')
panel = (BASE / 'readable_js/assets' / panel_name).read_text(encoding='utf-8')
frontend = ROOT / 'frontend_v2'
shutil.copytree(BASE / 'frontend', frontend, dirs_exist_ok=True)
patch_log = []

def replace(text, old, new, count=1):
    actual = text.count(old)
    assert actual == count, (old[:100], actual, count)
    patch_log.append({'anchor': old[:100], 'matches': actual})
    return text.replace(old, new)

def apply_edits(text, edits):
    last = len(text) + 1
    for start, end, value in sorted(edits, key=lambda e: (e[0], e[1]), reverse=True):
        assert 0 <= start <= end <= last, (start, end, last)
        text = text[:start] + value + text[end:]
        last = start
    return text

def function_for(meta, text, needle):
    found = [f for f in meta['functions'] if needle in text[f['start']:f['end']]]
    assert found, needle
    return min(found, key=lambda f: f['end'] - f['start'])

def named(meta, name):
    found = [f for f in meta['functions'] if f['name'] == name]
    assert len(found) == 1, (name, found)
    return found[0]

# Shared public functions retain readable names between these releases.
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
shared = replace(shared, MOD_NAME_FALLBACK_OLD, MOD_NAME_FALLBACK_NEW)

# 任务栏默认隐藏。官方默认是显示（设置里「显示任务栏图标」勾着），这里只改默认值，
# 官方设置项本身保持可用；已经存过设置的用户迁移一次，之后手动勾回来就永久生效。
# 官方改版后锚点可能变，这时只跳过这一项，不让整条构建链红掉。
_TASKBAR_DEFAULT = 'showTaskbarIcon: !0'
if shared.count(_TASKBAR_DEFAULT) == 1 and shared.count('function clampPetScale(e) {') == 1:
    shared = replace(shared, _TASKBAR_DEFAULT, 'showTaskbarIcon: !1')
    shared = replace(shared, 'function clampPetScale(e) {', '''// 社区版：默认不占任务栏按钮；历史设置只迁移一次，避免每次启动覆盖用户选择。
(function () {
  try {
    const key = "wuwatappo-settings", marker = key + "-community-taskbar-v1";
    if (window.localStorage.getItem(marker)) return;
    const saved = JSON.parse(window.localStorage.getItem(key) || "null");
    if (saved && typeof saved === "object" && "showTaskbarIcon" in saved) {
      saved.showTaskbarIcon = false;
      window.localStorage.setItem(key, JSON.stringify(saved));
    }
    window.localStorage.setItem(marker, "1");
  } catch (error) {
    console.warn("[community] taskbar default skipped", error);
  }
})();

function clampPetScale(e) {''')
else:
    print('warn: 官方前端里找不到任务栏图标默认值，跳过「默认隐藏」这一项', file=sys.stderr)

# 背包「我的预设」左边的预览走的是另一条链路：按名字拼 CDN 地址（character/<名字>.png），
# 不走 _atlas 目录查表，所以 Mod 条目在那里拼出一个必然 404 的官方地址，界面就剩一个「？」。
# 地址拼接处先问一句本机 Mod 表（local_atlas_controller 登记的名字 → 本机路由），
# 命中就返回本机地址，没命中照旧拼官方地址，官方素材的行为一字未改。
_MOD_ASSET_URL = re.search(r'function buildAssetUrl\(([^)]*)\)\s*\{', shared)
assert _MOD_ASSET_URL, 'official buildAssetUrl contract'
_MOD_ASSET_URL_PARAMS = [part.strip() for part in _MOD_ASSET_URL.group(1).split(',')]
assert len(_MOD_ASSET_URL_PARAMS) == 2, _MOD_ASSET_URL_PARAMS
shared = replace(shared, _MOD_ASSET_URL.group(0), _MOD_ASSET_URL.group(0) + '''
  const modAssetPath = (typeof window !== "undefined" && window.__WUWA_MOD_ASSET_PATH__) ? window.__WUWA_MOD_ASSET_PATH__(''' + _MOD_ASSET_URL_PARAMS[1] + ''') : "";
  if (modAssetPath) return withAssetUrlCacheBust(modAssetPath);''')

shared = replace(shared, 'loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();', '''let localOnlineServicesStarted = false;
function localStartOnlineServices() {
 if (localOnlineServicesStarted) return;
 localOnlineServicesStarted = true;
 loaderModule.init(kurodataConfig); window.kurodata = loaderModule;
 initTrackCommonProps(); initFrontendAlertReporting();
}''')
for name in ['invoke$1', 'invoke']:
    anchor = f'async function {name}(e, t = {{}}, n) {{'
    if anchor in shared:
        shared = replace(shared, anchor, anchor + '\n window.__WUWA_LOCAL_ATLAS__?.checkInvoke(e, t);')
for name in ['ensureAppearanceAssetCatalog(e)']:
    anchor = 'async function ' + name + ' {'
    shared = replace(shared, anchor, anchor + '\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return true;')
shared = replace(shared, 'function applyCatalog(e, t, n) {', '''function applyCatalog(e, t, n) {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && t !== window.__WUWA_LOCAL_ATLAS__.assetsHost) return;''')
shared = replace(shared, 'function atlas() {', '''function atlas() {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value) return Promise.resolve({code: 0, data: window.__WUWA_LOCAL_ATLAS__.offlineData.value.atlas});''')
shared = replace(shared, 'this.instance.interceptors.request.use(t => {', '''this.instance.interceptors.request.use(t => {
 if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && window.__WUWA_LOCAL_ATLAS__.restricted(t.url,t.method)) throw window.__WUWA_LOCAL_ATLAS__.offlineError();
 // 活动配置是后台刷新（挂机开箱、切页、重连都会触发），不该用全屏遮罩。
 if (t && typeof t.url === 'string' && t.url.indexOf('api/common/activity-config') !== -1) t.loading = !1;''')
controller = (PROJECT / 'src/local_atlas_controller.js').read_text(encoding='utf-8')
init = controller + '\nconst localAtlasController = createLocalAtlasController({\n' + \
    'ref, window, storage: localStorage, origin: ' + json.dumps(f'http://127.0.0.1:{config["port"]}') + ',\n' + \
    'token: null, customAssetUrl: reference => ' + json.dumps(f'http://127.0.0.1:{config["port"]}') + ' + "/" + reference.split("/").map(encodeURIComponent).join("/")' + ''', normalizeAtlasList, resolvePetAssetIdFromAppearanceId, getAppearanceAssetById,
snapshotCatalog: () => ({assets: appearanceAssetCatalog.value, atlases: appearanceAtlasCatalog.value, host: loadedAssetsHost, ts: loadedAssetsJsonTs}),
getAccountCumulative: () => globalStore.userInfo ? isFrozenClickCount(globalStore.serverClickTotal) ? globalStore.serverClickTotal : clampClickCountSum(globalStore.serverClickTotal+globalStore.localClickCount) : parseSafeClickCount(globalStore.localClickCount),
getAccountPending: () => parseSafeClickCount(globalStore.localClickCount),
catalogConsumer: ['/', '/panel'].includes((window.location.hash || '#/').slice(1).split('?')[0]),
applyOfflineCatalog: (payload, host) => applyCatalog(buildCatalogMaps(payload), host, Date.now()),
restoreCatalog: saved => applyCatalog({assets: saved.assets, atlases: saved.atlases}, saved.host, saved.ts),
onError: error => console.error('[local-gallery]', error)
});
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;
localAtlasController.ready.then(() => { localStartOnlineServices(); return bootstrap(); }).catch(error => {
 console.error('[local-gallery] startup failed', error);
 document.getElementById('app').textContent = '本地素材服务没有启动，请用桌面快捷方式重新打开；仍不行请查看安装目录下的 host.stderr.log。' + (error?.message || '');
});
watch(localAtlasController.enabled, on => { if (!on) localStartOnlineServices(); });
'''
shared = replace(shared, 'bootstrap();\nexport {', init + '\nexport { localAtlasController as zzLocalAtlas,\n')

# Main renderer: derive names from imports and the existing snapshot contract.
m = structure['files'][main_name]
mi = m['imports']
component = next(c for c in m['components'] if c['name'] == 'index')
m = {**m, 'functions': [fn for fn in m['functions'] if component['body'] < fn['start'] < component['bodyEnd']]}
assert len(m['snapshots']) == 1
snapshot = m['snapshots'][0]
f = snapshot['fields']
store, watch, mounted = mi['globalStore'], mi['watch'], mi['onMounted']
role, accessory, host, bag, atlas = [f[k] for k in ['selectedRoleId', 'selectedPartId', 'assetsCdnHost', 'bagList', 'atlasList']]
getchar, getaccess, getemotes = [mi[k] for k in ['getUserCharacterAssetName', 'getUserAccessoryAssetName', 'getUserEmoteAssets']]
emote_pattern = re.escape(mi['computed']) + r'\(\(\) => ' + re.escape(store) + r'\.isLoggedIn \? ' + re.escape(getemotes) + r'\(' + re.escape(store) + r'\.userInfo\)[\s\S]{0,250}?\}\)\) : ([\w$]+)\.value\)'
emotes = re.search(emote_pattern, main)
assert emotes, 'Emote selection computed layout changed'
emotes = emotes.group(1) + '.value'
edits = [(0, 0, f'import {{zzLocalAtlas as __lg}} from "./{shared_name}";\n')]
def insert_function(name, code):
    pos = named(m, name)['body'] + 1
    edits.append((pos, pos, '\n' + code + '\n'))
bag_loader = function_for(m, main, 'await ' + mi['getBagList'] + '()')
atlas_loader = function_for(m, main, 'await ' + mi['atlas'] + '()')
activity_loader = function_for(m, main, 'await ' + mi['activityInfo'] + '()')
insert_function(atlas_loader['name'], f'if(__lg.enabled.value){{{atlas}=__lg.offlineData.value.atlas;{f["atlasListLoading"]}=false;return;}}')
insert_function(activity_loader['name'], 'try {')
edits.append((activity_loader['bodyEnd']-1,activity_loader['bodyEnd']-1,'}finally{if(__lg.enabled.value)__localRefresh();}'))
for action in ['create-room', 'join-room']:
    insert_function(m['cases'][action], f"if(__lg.enabled.value){{{mi['showToast']}('本地全图鉴已开启，无法进入联机。请先关闭开关。');return;}}")
select_name = m['cases']['select-appearance']
param = re.search(r'function\s+' + re.escape(select_name) + r'\((\w+)', main).group(1)
select_function = named(m, select_name)
select_source = main[select_function['start']:select_function['end']]
max_emotes = re.search(r'\.length >= ([\w$]+) \|\|', select_source)
assert max_emotes, 'Emote capacity guard changed'
insert_function(select_name, f'''if(__lg.enabled.value){{try{{__lg.select({param},{store}.userInfo,{max_emotes[1]});__localRefresh();}}catch(error){{{mi['showToast']}(error.message);}}return;}}''')
reorder_name = m['cases']['reorder-emotes']
param = re.search(r'function\s+' + re.escape(reorder_name) + r'\((\w+)', main).group(1)
insert_function(reorder_name, f'if(__lg.enabled.value){{__lg.reorder({param},{store}.userInfo);__localRefresh();return;}}')
favorite_name = m['cases']['collect-change']
param = re.search(r'function\s+' + re.escape(favorite_name) + r'\((\w+)', main).group(1)
insert_function(favorite_name, f'if(__lg.enabled.value){{__lg.favorite({param}.assetId,{param}.is_collect===1);return;}}')
# 官方预设存在账号上，本地模式不能走网络，所以「我的预设」改由控制器落本机。
# 列表归一化函数与容量上限都从产物里取，官方换名字也不影响。
preset_loader = function_for(m, main, 'await ' + mi['getPresetList'] + '()')
preset_source = main[preset_loader['start']:preset_loader['end']]
preset_normalize = re.search(re.escape(f['presetList']) + r' = [^\n]+\.map\(([\w$]+)\)', preset_source)
assert preset_normalize, 'Preset list normalization changed'
preset_limit = re.search(re.escape(f['presetList']) + r'\.length >= ([\w$]+)',
                         main[named(m, m['cases']['create-preset'])['start']:named(m, m['cases']['create-preset'])['end']])
assert preset_limit, 'Preset capacity guard changed'
for action, operation in [('create-preset', f'__lg.createLocalPreset({store}.userInfo,{preset_limit[1]})'),
                          ('delete-preset', '__lg.deleteLocalPreset({0})'),
                          ('apply-preset', '__lg.applyLocalPreset({0}.preset_id)')]:
    action_name = m['cases'][action]
    action_param = re.search(r'function\s+' + re.escape(action_name) + r'\((\w+)', main)
    insert_function(action_name, 'if(__lg.enabled.value){try{' + operation.format(action_param[1] if action_param else 'null')
                   + f';__localRefresh();}}catch(error){{{mi["showToast"]}(error.message);}}return;}}')
insert_function(preset_loader['name'], f'if(__lg.enabled.value){{{f["presetListLoading"]}=false;return;}}')
snap_text = main[snapshot['start']:snapshot['end']]
snap_text = snap_text.replace('bagList: ' + bag, 'bagList: __lg.inventory(' + atlas + ',' + bag + ')')
snap_text = snap_text.replace('atlasList: ' + atlas, 'atlasList: __lg.enabled.value ? __lg.offlineData.value.atlas : ' + atlas)
snap_text = snap_text.replace('presetList: ' + f['presetList'],
                              'presetList: __lg.enabled.value ? __lg.presetList().map(' + preset_normalize[1] + ') : ' + f['presetList'])
snap_text = snap_text.replace('presetListLoading: ' + f['presetListLoading'],
                              'presetListLoading: !__lg.enabled.value && ' + f['presetListLoading'])
snap_text = snap_text.replace(f['userInfo'], f['userInfo'].replace(store + '.userInfo', '__lg.viewUser(' + store + '.userInfo)'))
snap_text = snap_text.replace('selectedEmoteAssetIds: ' + f['selectedEmoteAssetIds'], 'selectedEmoteAssetIds: ' + getemotes + '(__lg.viewUser(' + store + '.userInfo)).map(item=>item.id)')
edits.append((snapshot['start'], snapshot['end'], snap_text))
for meaning in ['getUserCharacterAssetId', 'getUserCharacterAssetName', 'getUserAccessoryAssetId', 'getUserAccessoryAssetName', 'getUserEmoteAssets', 'getUserAccessoryOffset', 'getUserCharacterMyOffset']:
    alias = mi[meaning]
    for match in re.finditer(re.escape(alias) + r'\(' + re.escape(store) + r'\.userInfo\)', main):
        if not (snapshot['start'] <= match.start() < snapshot['end']):
            edits.append((match.start(), match.end(), alias + '(__lg.viewUser(' + store + '.userInfo))'))
local_code = f'''
let __localSaved=null;
function __localRefresh(){{
 if(!__lg.enabled.value)return;
 const user=__lg.viewUser({store}.userInfo);
 {host}=__lg.assetsHost;
 const name={getchar}(user);if(name){role}=name.replace(/\\.png$/i,'');
 {accessory}={getaccess}(user)||null;
 {emotes}={getemotes}(user).map(item=>({{id:item.id,name:item.name}}));
 {atlas}=__lg.offlineData.value.atlas;
}}
{watch}(__lg.enabled,on=>{{
 if(on){{__localSaved={{role:{role},accessory:{accessory},emotes:[...{emotes}],host:{host},atlas:{atlas}}};__localRefresh();}}
 else if(__localSaved){{{role}=__localSaved.role;{accessory}=__localSaved.accessory;{emotes}=__localSaved.emotes;{host}=__localSaved.host;{atlas}=__localSaved.atlas;__localSaved=null;{activity_loader['name']}({{force:true}}).then(()=>{bag_loader['name']}()).catch(()=>{{}});}}
}},{{immediate:true}});
{watch}([__lg.selection,__lg.revision,{mi['appearanceAssetCatalog']}],()=>{{if(__lg.enabled.value)__localRefresh();}});
{watch}([()=>{role},()=>{accessory},()=>{host}],()=>{{if(__lg.enabled.value)__localRefresh();}});
'''
edits.append((component['returnStart'], component['returnStart'], local_code))
main = apply_edits(main, edits)
for key in ['canCreateRoom', 'canJoinRoom']:
    main = main.replace(key + ': ' + f[key], key + ': !__lg.enabled.value && ' + f[key])

# Settings/atlas UI retains this release's own bindings and components.
p = structure['files'][panel_name]
pi = p['imports']
settings = next(c for c in p['components'] if c['name'] == 'SettingsModule')
atlas_component = next(c for c in p['components'] if c['name'] == 'MainPanelAtlasTab')
container = next(c for c in p['containers'] if settings['body'] < c['start'] < settings['bodyEnd'])
h = pi['createBaseVNode']
with_modifiers = pi['withModifiers']
setup = f'''
const __localEnabled=__lg.enabled,__localBusy=__lg.busy,__localSync=__lg.syncStatus,__localCache=__lg.cacheReady;
async function __toggleLocal(event){{try{{await __lg.setEnabled(event.target.checked);}}catch(error){{{pi['showToast']}(error.message);}}event.target.checked=__localEnabled.value;}}
async function __syncLocal(){{try{{const status=await __lg.syncCache(true);{pi['showToast']}('本地图鉴已同步：'+(status.result?.atlas_count||'')+'项');}}catch(error){{{pi['showToast']}(error.message);}}}}
function __openCommunityUpdate(event){{try{{event&&event.stopPropagation&&event.stopPropagation();__lg.assistant.openPanel('update');}}catch(error){{console.error('[assistant]',error);}}}}
'''
row = f'''{h}('div',{{class:'setting-item','data-local-gallery-setting':'true'}},[
 {h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'外观模式'),{h}('div',{{class:'setting-sub'}},__localBusy.value?'正在切换本地显示模式…':!__localCache.value?(__localSync.value.status==='running'?'正在下载本地素材 '+__localSync.value.done+'/'+__localSync.value.total+'，完成后自动可用':__localSync.value.status==='failed'?'本地素材下载失败，点下方「检查并同步」重试':'本地素材还没准备好，点下方「检查并同步」开始下载'):__localEnabled.value?'当前：全图鉴 + Mod · 联机不可用，其他网络功能正常':'当前：官方 · 开启后使用全图鉴和已导入 Mod',1)]),
 {h}('div',{{class:'setting-content'}},[{h}('label',{{class:'setting-switch'}},[{h}('input',{{type:'checkbox',role:'switch','aria-label':'全图鉴 + Mod',style:'display:block;position:absolute;width:1px;height:1px;opacity:0',checked:__localEnabled.value,disabled:__localBusy.value||!__localCache.value||__localSync.value.status==='running',onChange:__toggleLocal,onMousedown:{with_modifiers}(()=>{{}},['stop'])}},null,40,['checked','disabled']),{h}('span',{{class:'setting-slider'}})]),{h}('button',{{class:'setting-btn',type:'button',style:'margin-left:10px;white-space:nowrap',onMousedown:{with_modifiers}(()=>{{}},['stop']),onClick:()=>__lg.assistant.openPanel('mods')}},'Mod 管理')])]),
 {h}('div',{{class:'setting-item','data-local-gallery-sync':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'更新本地图鉴'),{h}('div',{{class:'setting-sub'}},__localSync.value.status==='running'?'正在同步 '+__localSync.value.done+'/'+__localSync.value.total:!__localCache.value?'首次启动要联网补齐本地素材（后台进行，不耽误玩）':'联网检查新增图鉴，本地全图鉴开启时也可同步',1)]),{h}('button',{{class:'setting-btn',type:'button',disabled:__localSync.value.status==='running',onClick:__syncLocal}},'检查并同步',8,['disabled'])]),
'''
edits = [(0, 0, f'import {{zzLocalAtlas as __lg}} from "./{shared_name}";\n'),
         (settings['body'] + 1, settings['body'] + 1, setup),
         (container['array'] + 1, container['array'] + 1, row)]
# 官方把面板导航写成固定数组（背包/图鉴/合成/联机/设置/收起）。这里在「设置」前
# 插一个「控制台」页，位置与上游一致：联机下面、设置上面；图标用三段滑杆，沿用暖色。
module_component = next(c for c in p['components'] if c['name'] == 'MainModulePanel')
module_text = panel[module_component['start']:module_component['end']]
nav_iter = re.search(r'([\w$]+)\(([\w$]+)\(([\w$]+)\), ([\w$]+) =>', module_text)
assert nav_iter, 'navigation iterator changed'
nav_render, nav_unref, nav_items, nav_item = nav_iter.groups()
nav_state = re.search(r'([\w$]+)\.activeTab === "bag"', module_text)[1]
nav_label = re.search(r'return ([\w$]+)\.key === "toggle"', module_text)[1]
console_component = f'''
const __ConsolePage={pi['defineComponent']}({{
 __name:'WuwaNativeConsole',setup(){{
  const target={pi['ref']}(null);
  {pi['onMounted']}(()=>__lg.assistant.mountConsole(target.value));
  {pi['onUnmounted']}(()=>__lg.assistant.closePanel());
  return ()=>{h}('div',{{class:'module-block',style:{{height:'100%',minHeight:0,position:'relative',overflow:'hidden',paddingTop:'6px'}}}},[{h}('div',{{ref:target,style:{{height:'100%',minHeight:0,overflow:'auto'}}}})]);
 }}
}});
const __consoleTabs={pi['computed']}(()=>{pi['unref']}({nav_items}).flatMap(item=>item.key==='setting'?[{{key:'console',labelKey:'console'}},item]:[item]));
'''
edits.append((module_component['body'] + 1, module_component['body'] + 1, console_component))
icon_match = re.search(re.escape(h) + r'\("span", \{\s*class: [\w$]+\(\["tab-btn__icon", `tab-btn__icon--\$\{' + re.escape(nav_item) + r'.key\}`\]\)\s*\}, null, 2\)', module_text)
assert icon_match, 'native navigation icon layout changed'
console_icon = f'''{nav_item}.key==='console'?({pi['openBlock']}(),{pi['createElementBlock']}('svg',{{key:'console-icon',class:'tab-btn__icon tab-btn__icon--console',viewBox:'0 0 40 54','aria-hidden':'true','data-console-icon':'sliders',style:{{color:{nav_state}.activeTab==='console'?'#9c6541':'#f4d5a5'}}}},[
 {h}('rect',{{x:7,y:15,width:26,height:25,rx:5,fill:'none',stroke:'currentColor','stroke-width':2.5}}),
 {h}('path',{{d:'M14 21v13M20 21v13M26 21v13',fill:'none',stroke:'currentColor','stroke-width':2.2,'stroke-linecap':'round'}}),
 {h}('circle',{{cx:14,cy:25,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:20,cy:30,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:26,cy:24,r:2.6,fill:'currentColor'}})
],4)):{icon_match[0]}'''
_icon_pos = module_component['start'] + icon_match.start()
edits.append((_icon_pos, module_component['start'] + icon_match.end(), console_icon))
for _old, _new in [
        (f'return {nav_label}.key === "toggle"', f'return {nav_label}.key === "console" ? "控制台" : {nav_label}.key === "toggle"'),
        (f'{nav_render}({nav_unref}({nav_items}),', f'{nav_render}(__consoleTabs.value,'),
        (f'{nav_state}.activeTab === "bag" ?',
         f'{nav_state}.activeTab === "console" ? ({pi["openBlock"]}(),{pi["createBlock"]}(__ConsolePage,{{key:"console"}})) : {nav_state}.activeTab === "bag" ?')]:
    assert module_text.count(_old) == 1, ('native console anchor', _old)
    _pos = module_component['start'] + module_text.index(_old)
    edits.append((_pos, _pos + len(_old), _new))
panel_root = next(c for c in p['components'] if c['name'] == 'index')
root_source = panel[panel_root['body']:panel_root['bodyEnd']]
state_ref = re.search(r'const ([\w$]+) = ' + re.escape(pi['ref']) + r'\(null\)', root_source)
if not state_ref:
    # 1.0.3 delivers the snapshot through a subscription callback instead of a null ref.
    sub = re.search(r'=\s*await\s+[\w$]+\(\s*([\w$]+)\s*=>\s*\{(.{0,800}?)\n\s*\}\)', root_source, re.S)
    assert sub, 'Panel snapshot subscription changed'
    _param, _body = sub.group(1), sub.group(2)
    _got = re.search(r'([\w$]+)\.value = ' + re.escape(_param) + r'\b', _body)
    assert _got, 'Panel snapshot state assignment changed'
    state_ref = (_got.group(0), _got.group(1))
assert state_ref, 'Panel snapshot state declaration changed'
ready_retry = f'''
let __panelDisposed=false;
{pi['onUnmounted']}(()=>{{__panelDisposed=true;}});
{pi['onMounted']}(()=>{{
 for(const delay of [500,1500,3000,6000,10000])window.setTimeout(()=>{{
  if(!__panelDisposed&&!{state_ref[1]}.value)window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{{event:'cat-panel-action',payload:{{type:'ready'}}}}).catch(()=>{{}});
 }},delay);
}});
'''
edits.append((panel_root['returnStart'], panel_root['returnStart'], ready_retry))
for condition in p['roomConditions']:
    text = condition['test']
    if 'activeTab' in text:
        message = pi['openBlock'] + '(), ' + pi['createElementBlock'] + "('div',{class:'module-block',style:'padding:24px'},'本地全图鉴已开启，无法进入联机。请先关闭开关。')"
        edits.append((condition['start'], condition['start'], '__lg.enabled.value && ' + text + ' ? (' + message + ') : '))
# 图鉴顶部的分类收集进度（上游 1.3.4）：本地模式下所有条目都算「已拥有」，
# 进度恒 100% 没有意义，所以这块只在官方模式显示真实账号的分类收集情况。
atlas_text = panel[atlas_component['start']:atlas_component['end']]
assert re.search(r'[\w$]+ = ' + re.escape(pi['computed']) + r'\(\(\) => [\w$]+\([\w$]+\([\w$]+\.atlasList, [\w$]+\.bagList\)\)\)', atlas_text), \
    'atlas row merge computed changed'
category = re.search(r'([\w$]+) = ' + re.escape(pi['computed']) + r'\(\(\) => \{\s*const [\w$]+ = [\w$]+\[([\w$]+)\.value\];', atlas_text)
assert category, 'atlas category computed changed'
category_rows, category_index = category.groups()
atlas_loading = re.search(r'([\w$]+) = ' + re.escape(pi['computed']) + r'\(\(\) => [\w$]+\.bagListLoading \|\| [\w$]+\.atlasListLoading\)', atlas_text)
assert atlas_loading, 'atlas loading computed changed'
atlas_loading = atlas_loading.group(1)
edits.append((atlas_component['returnStart'], atlas_component['returnStart'],
              f"const __atlasProgress={pi['computed']}(()=>__lg.collectionProgress({category_rows}.value,{atlas_loading}.value)),"
              f"__atlasProgressTitle={pi['computed']}(()=>['全图鉴收集进度','全角色收集进度','全配件收集进度','全表情收集进度'][{category_index}.value]||'全图鉴收集进度');\n"))
tab_anchor = 'null, 8, ["modelValue", "tabs"]),'
assert atlas_text.count(tab_anchor) == 1, 'atlas tab header changed'
progress = f''' !__lg.enabled.value ? ({pi['openBlock']}(true),{pi['createElementBlock']}('section',{{key:'account-atlas-progress','data-atlas-progress':'online',style:{{padding:'12px 4px 14px',width:'100%',boxSizing:'border-box'}}}},[
 {h}('div',{{style:{{display:'flex',justifyContent:'space-between',alignItems:'center',gap:'12px',fontSize:'14px',color:'#927b65',marginBottom:'8px'}}}},[
 {h}('span',null,__atlasProgressTitle.value),{h}('strong',{{style:{{fontVariantNumeric:'tabular-nums',color:'#996644'}}}},
 !{pi['globalStore']}.isLoggedIn?'登录后查看':{atlas_loading}.value?'正在读取…':__atlasProgress.value.collected+'/'+__atlasProgress.value.total+' · '+__atlasProgress.value.percent.toFixed(1)+'%')]),
 {h}('div',{{role:'progressbar','aria-label':'真实账号'+__atlasProgressTitle.value,'aria-valuemin':0,'aria-valuemax':__atlasProgress.value.total,'aria-valuenow':__atlasProgress.value.collected,'aria-busy':{atlas_loading}.value,style:{{height:'9px',width:'100%',background:'#ecddc4',borderRadius:'20px',overflow:'hidden'}}}},[
 {h}('span',{{style:{{display:'block',height:'100%',width:({pi['globalStore']}.isLoggedIn?__atlasProgress.value.percent:0)+'%',background:'linear-gradient(90deg,#e4b56f,#c58b51)',borderRadius:'inherit',transition:'width .25s ease'}}}})])],-2)) : {pi['createCommentVNode']}('local-atlas-progress-hidden',true),'''
progress_at = atlas_component['start'] + atlas_text.index(tab_anchor) + len(tab_anchor)
edits.append((progress_at, progress_at, progress))
# 本地模式下预设不存账号，登录守卫也要放行，否则未登录时「我的预设」整块只显示空槽。
bag_component = next(c for c in p['components'] if c['name'] == 'MainPanelBagTab')
bag_text = panel[bag_component['start']:bag_component['end']]
bag_guard = 'if (!' + pi['globalStore'] + '.isLoggedIn)'
assert bag_text.count(bag_guard) == 2, 'preset login guard layout changed'
for match in re.finditer(re.escape(bag_guard), bag_text):
    edits.append((bag_component['start'] + match.start(), bag_component['start'] + match.end(),
                  'if (!' + pi['globalStore'] + '.isLoggedIn && !__lg.enabled.value)'))
delete_guard = re.search(r'if \([\w$]+\.stopPropagation\(\), !' + re.escape(pi['globalStore']) + r'\.isLoggedIn\)', bag_text)
assert delete_guard, 'preset delete login guard changed'
delete_at = bag_component['start'] + bag_text.index(delete_guard[0])
edits.append((delete_at, delete_at + len(delete_guard[0]), delete_guard[0][:-1] + ' && !__lg.enabled.value)'))
panel = apply_edits(panel, edits)

# 官方的「检查更新」改成打开本机控制台的「更新」页：官方自更新会把整合包替换回原版，
# 而且它读的清单与我们自己的版本检查不是同一个来源。按钮的处理函数名随官方构建漂移，
# 所以按「类名 → 紧随其后的 onClick」定位，而不是匹配函数名。
# 只认类名本身，前后空白与引号风格交给 index 定位，避免锚在压缩差异上。
_update_marker = '"setting-btn btn-check-update"'
assert panel.count(_update_marker) == 1, 'official update button contract'
_call_at = panel.index('onClick:', panel.index(_update_marker))
_handler_at = panel.index('R(', _call_at) + len('R(')
_handler_end = panel.index(',', _handler_at)
_raw = panel[_handler_at:_handler_end]
_handler = _raw.strip()
assert re.fullmatch(r'[\w$]+', _handler), _handler
_handler_at += _raw.index(_handler)
panel = panel[:_handler_at] + '__openCommunityUpdate' + panel[_handler_at + len(_handler):]
pattern = r'([\w$]+)\.value \? await ([\w$]+)\(await ' + re.escape(pi['unfavoriteAsset']) + r'\(([\w$]+)\.item.asset_id\)\) : await \2\(await ' + re.escape(pi['favoriteAsset']) + r'\(\3\.item.asset_id\)\)'
match = re.search(pattern, panel)
assert match, 'Favorite toggle layout changed'
panel = panel[:match.start()] + f'__lg.enabled.value ? __lg.favorite({match[3]}.item.asset_id,!{match[1]}.value) : ' + panel[match.start():]

from integrate_assistant import integrate
# 版本号只做记录，不做准入：官方每次发版都会换号，白名单会让"锚点一个都没漂"的
# 新版也无谓失败（1.0.4 就是这样，白名单得人工加一行才肯编译）。
# 真正的兼容性判断交给上面的结构断言和 integrate() 里的锚点解析——匹配不上就直接抛，
# 那才是"需要人工改适配"的信号；版本号不是。
assert config.get('client_version'), 'Official client version missing from app config'
shared, main, panel = integrate(PROJECT, BASE, frontend, structure, shared, main, panel, main_name, panel_name)
extra_patched_assets = [p.name for p in (BASE/'frontend/assets').glob('switchDisplayMonitor-*.js')
                        if (frontend/'assets'/p.name).read_bytes() != p.read_bytes()]
for name, text in [(shared_name, shared), (main_name, main), (panel_name, panel)]:
    (frontend / 'assets' / name).write_text(text, encoding='utf-8')
manifest = {'shared': shared_name, 'main': main_name, 'panel': panel_name,
            'settingsSymbol': settings['variable'], 'atlasSymbol': atlas_component['variable'],
            'base': str(BASE), 'version': config['client_version']}
(ROOT / 'build_manifest_v2.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
# Reuse the independently checked PE resource packer and private-runtime stub.
packer = (PROJECT / 'tools/pe_packer.py').read_text(encoding='utf-8').split('# Add a read-only PE section', 1)[1]
packer = '# Add a read-only PE section' + packer
packer = packer.replace("source = Path(r'D:\\WuwaTappo\\WuwaTappo.exe')", "source = BASE / 'WuwaTappo.official.exe'")
packer = packer.replace("ROOT / 'WuwaTappo.Local.exe'", "ROOT / 'WuwaTappo.Local.v2.exe'")
packer = packer.replace("ROOT / 'build_report.json'", "ROOT / 'build_report_v2.json'")
ORIGINAL = BASE
import sys

import brotli, pefile, base64, struct
exec(compile(packer, 'checked_pe_packer', 'exec'))
