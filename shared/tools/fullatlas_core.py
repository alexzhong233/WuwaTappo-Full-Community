"""Shared FullAtlas injection pipeline for macOS and Windows.

Platform scripts provide ``FullAtlasHooks`` for the parts that genuinely differ:
the asset URL scheme, taskbar defaults, controller transport, settings UI,
Windows console tab, and PE packing. Every anchor shared by both builds lives
here so an official frontend update only changes one implementation.
"""
from pathlib import Path
import json
import re
import sys

from inject_core import (apply_edits, function_for, load_build, named,
                         patch_atlas_guards, patch_display_version, patch_invoke_guard,
                         patch_mod_asset_url, patch_mod_display_name, patch_online_services,
                         patch_taskbar_default, replace)


class FullAtlasHooks:
    controller_path = None
    strict_invoke = False
    mod_name_first = False
    atlas_guard_extra = ''
    startup_error_prefix = '全图鉴启动失败：'

    def prepare_shared(self, shared, project, config):
        return shared

    def transport_options(self, config):
        raise NotImplementedError

    def controller_init(self, project, config, controller):
        return (
            controller
            + "\nconst localAtlasRoute = ['/', '/panel'].includes((window.location.hash || '#/').slice(1).split('?')[0]);\n"
            + "const localAtlasController = localAtlasRoute ? createLocalAtlasController({\n"
            + "ref, window, storage: localStorage, "
            + self.transport_options(config)
            + ''', normalizeAtlasList, resolvePetAssetIdFromAppearanceId, getAppearanceAssetById,
snapshotCatalog: () => ({assets: appearanceAssetCatalog.value, atlases: appearanceAtlasCatalog.value, host: loadedAssetsHost, ts: loadedAssetsJsonTs}),
getAccountCumulative: () => globalStore.userInfo ? isFrozenClickCount(globalStore.serverClickTotal) ? globalStore.serverClickTotal : clampClickCountSum(globalStore.serverClickTotal+globalStore.localClickCount) : parseSafeClickCount(globalStore.localClickCount),
getAccountPending: () => parseSafeClickCount(globalStore.localClickCount),
catalogConsumer: localAtlasRoute,
applyOfflineCatalog: (payload, host) => applyCatalog(buildCatalogMaps(payload), localAtlasController.offlineData.value?.assets_host || host, Date.now()),
restoreCatalog: saved => applyCatalog({assets: saved.assets, atlases: saved.atlases}, saved.host, saved.ts),
onError: error => console.error('[local-gallery]', error)
}) : null;
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;
if (localAtlasController) {
localAtlasController.ready.then(() => { localStartOnlineServices(); return bootstrap(); }).catch(error => {
 console.error('[local-gallery] startup failed', error);
 document.getElementById('app').textContent = ''' + json.dumps(self.startup_error_prefix) + ''' + (error?.message || '');
});
watch(localAtlasController.enabled, on => { if (!on) localStartOnlineServices(); });
} else { localStartOnlineServices(); bootstrap(); }
'''
        )

    def settings_ui(self, pi, h, with_modifiers):
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
        return setup.rstrip() + '\n', row.rstrip() + '\n'

    def extend_panel(self, panel, p, pi, edits, settings, atlas_component, container):
        # 官方把面板导航写成固定数组（背包/图鉴/合成/联机/设置/收起）。这里在「设置」前
        # 插一个「控制台」页（联机下方、设置上方），两端共用同一份原生内嵌页签。
        module_component = next(c for c in p['components'] if c['name'] == 'MainModulePanel')
        module_text = panel[module_component['start']:module_component['end']]
        nav_iter = re.search(r'([\w$]+)\(([\w$]+)\(([\w$]+)\), ([\w$]+) =>', module_text)
        assert nav_iter, 'navigation iterator changed'
        nav_render, nav_unref, nav_items, nav_item = nav_iter.groups()
        nav_state = re.search(r'([\w$]+)\.activeTab === "bag"', module_text)[1]
        nav_label = re.search(r'return ([\w$]+)\.key === "toggle"', module_text)[1]
        h = pi['createBaseVNode']
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
        return edits

    def finalize(self, context):
        pass


def integrate_fullatlas(project, hooks, integrate_assistant):
    project = Path(project)
    build = load_build(project)
    root, base, frontend = build['root'], build['base'], build['frontend']
    structure, config = build['structure'], build['config']
    shared_name, main_name, panel_name = build['shared_name'], build['main_name'], build['panel_name']
    shared, main, panel = build['shared'], build['main'], build['panel']

    if hooks.mod_name_first:
        shared = patch_mod_display_name(shared)
        shared = hooks.prepare_shared(shared, project, config)
    else:
        shared = hooks.prepare_shared(shared, project, config)
        shared = patch_mod_display_name(shared)
    shared = patch_taskbar_default(shared)
    shared = patch_mod_asset_url(shared)
    shared = patch_online_services(shared)
    repo_pkg = project.parent / 'package.json'
    if not repo_pkg.exists():
        repo_pkg = project.parent.parent / 'package.json'
    community_version = str(
        config.get('community_version')
        or config.get('mod_version')
        or json.loads(repo_pkg.read_text(encoding='utf-8'))['version']
    ).strip()
    official_version = str(config.get('client_version') or '').strip()
    shared = patch_display_version(shared, community_version, official_version)
    shared = patch_invoke_guard(shared, strict=hooks.strict_invoke)
    shared = patch_atlas_guards(shared, hooks.atlas_guard_extra)

    controller = (project / hooks.controller_path).read_text(encoding='utf-8')
    init = hooks.controller_init(project, config, controller)
    shared = replace(shared, 'bootstrap();\nexport {', init + '\nexport { localAtlasController as zzLocalAtlas,\n')

    # Main renderer: derive names from imports and the existing snapshot contract.
    m = structure['files'][main_name]
    mi = m['imports']
    component = next(c for c in m['components'] if c['name'] == 'index')
    m = {**m, 'functions': [fn for fn in m['functions'] if component['body'] < fn['start'] < component['bodyEnd']]}
    assert len(m['snapshots']) == 1
    snapshot = m['snapshots'][0]
    f = snapshot['fields']
    store, watch = mi['globalStore'], mi['watch']
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
                              ('apply-preset', '__lg.applyPreset({0}.preset_id)')]:
        action_name = m['cases'][action]
        action_param = re.search(r'function\s+' + re.escape(action_name) + r'\((\w+)', main)
        insert_function(action_name, 'if(__lg.enabled.value){try{' + operation.format(action_param[1] if action_param else 'null')
                       + f';__localRefresh();}}catch(error){{{mi["showToast"]}(error.message);}}return;}}')
    # 账号预设在本地模式下照样加载：它与本地预设并存，点账号那条等于把它复制成本地外观，
    # 不会写账号。之前这里直接 return，等于把官方那份藏起来，用户就没法分别设置了。
    snap_text = main[snapshot['start']:snapshot['end']]
    snap_text = snap_text.replace('bagList: ' + bag, 'bagList: __lg.inventory(' + atlas + ',' + bag + ')')
    snap_text = snap_text.replace('atlasList: ' + atlas, 'atlasList: __lg.enabled.value ? __lg.offlineData.value.atlas : ' + atlas)
    snap_text = snap_text.replace('presetList: ' + f['presetList'],
                                  'presetList: __lg.enabled.value ? __lg.mergePresets(__lg.presetList().map('
                                  + preset_normalize[1] + '), ' + f['presetList'] + ') : ' + f['presetList'])
    snap_text = snap_text.replace('presetListLoading: ' + f['presetListLoading'],
                                  'presetListLoading: ' + f['presetListLoading'])
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
    setup, row = hooks.settings_ui(pi, h, with_modifiers)
    panel_edits = [(0, 0, f'import {{zzLocalAtlas as __lg}} from "./{shared_name}";\n'),
             (settings['body'] + 1, settings['body'] + 1, setup),
             (container['array'] + 1, container['array'] + 1, row)]
    panel_edits = hooks.extend_panel(panel, p, pi, panel_edits, settings, atlas_component, container)

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
    panel_edits.append((panel_root['returnStart'], panel_root['returnStart'], ready_retry))
    for condition in p['roomConditions']:
        text = condition['test']
        if 'activeTab' in text:
            message = pi['openBlock'] + '(), ' + pi['createElementBlock'] + "('div',{class:'module-block',style:'padding:24px'},'本地全图鉴已开启，无法进入联机。请先关闭开关。')"
            panel_edits.append((condition['start'], condition['start'], '__lg.enabled.value && ' + text + ' ? (' + message + ') : '))
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
    panel_edits.append((atlas_component['returnStart'], atlas_component['returnStart'],
                  f"const __atlasProgress={pi['computed']}(()=>__lg.collectionProgress({category_rows}.value,{atlas_loading}.value)),"
                  f"__atlasProgressTitle={pi['computed']}(()=>['全图鉴收集进度','全角色收集进度','全配件收集进度','全表情收集进度'][{category_index}.value]||'全图鉴收集进度');\n"))
    tab_anchor = 'null, 8, ["modelValue", "tabs"]),'
    assert atlas_text.count(tab_anchor) == 1, 'atlas tab header changed'
    # 收集进度在两种模式下都显示：本地模式的图鉴列表现在带真实 owned，
    # 这个数字就是「账号里到底收了多少」，不再是假 100%。
    progress = f''' ({pi['openBlock']}(true),{pi['createElementBlock']}('section',{{key:'account-atlas-progress','data-atlas-progress':'online',style:{{padding:'12px 4px 14px',width:'100%',boxSizing:'border-box'}}}},[
 {h}('div',{{style:{{display:'flex',justifyContent:'space-between',alignItems:'center',gap:'12px',fontSize:'14px',color:'#927b65',marginBottom:'8px'}}}},[
 {h}('span',null,__atlasProgressTitle.value),{h}('strong',{{style:{{fontVariantNumeric:'tabular-nums',color:'#996644'}}}},
 !{pi['globalStore']}.isLoggedIn?'登录后查看':{atlas_loading}.value?'正在读取…':__atlasProgress.value.collected+'/'+__atlasProgress.value.total+' · '+__atlasProgress.value.percent.toFixed(1)+'%')]),
 {h}('div',{{role:'progressbar','aria-label':'真实账号'+__atlasProgressTitle.value,'aria-valuemin':0,'aria-valuemax':__atlasProgress.value.total,'aria-valuenow':__atlasProgress.value.collected,'aria-busy':{atlas_loading}.value,style:{{height:'9px',width:'100%',background:'#ecddc4',borderRadius:'20px',overflow:'hidden'}}}},[
 {h}('span',{{style:{{display:'block',height:'100%',width:({pi['globalStore']}.isLoggedIn?__atlasProgress.value.percent:0)+'%',background:'linear-gradient(90deg,#e4b56f,#c58b51)',borderRadius:'inherit',transition:'width .25s ease'}}}})])],-2)),'''
    progress_at = atlas_component['start'] + atlas_text.index(tab_anchor) + len(tab_anchor)
    panel_edits.append((progress_at, progress_at, progress))
    # 图鉴页本来就会做「图鉴 ∩ 背包 → owned」，官方 CSS 自带 .atlas-item--unowned。
    # 本地模式把索引换成真实账号背包，于是账号没有的条目照样列出来、但带未拥有角标。
    merge = re.search(r'\(\s*([\w$]+)\(\s*([\w$]+)\.atlasList\s*,\s*([\w$]+)\.bagList\s*\)\s*\)', panel)
    if merge:
        panel_edits.append((merge.start(), merge.end(),
                            '(' + merge.group(1) + '(' + merge.group(2) + '.atlasList, '
                            'window.__WUWA_LOCAL_ATLAS__?.ownedBag?.() || ' + merge.group(3) + '.bagList))'))
    else:
        # 官方换掉图鉴/背包的合并写法时不要红掉整条构建：这一项失效的后果只是
        # 本地模式下所有条目都显示成「已拥有」，图鉴本身仍可用。
        print('warn: 找不到图鉴与背包的合并点，跳过「未拥有角标」', file=sys.stderr)
    # 官方把未拥有判成不可点击（r 里第一句就是 if(!item.owned) return false）。
    # 本地模式要能直接试穿，所以只放开点击，角标和排序保持官方行为。
    for match in list(re.finditer(r'"is-item-clickable":\s*([\w$]+)(?![\w$.])', panel)):
        name = match.group(1)
        panel_edits.append((match.start(), match.end(),
                            '"is-item-clickable": (__c => window.__WUWA_LOCAL_ATLAS__?.enabled.value ? !0 : '
                            + name + '(__c))'))
    # 「我的预设」里两类卡片并排：账号那份 id>0，本地 id<=0。角标样式由控制器注入，
    # 这里只补 class，模板其余部分不动。
    for match in list(re.finditer(r'\{\s*"preset-item--add":\s*([\w$]+)\.kind\s*===\s*"add"\s*\}', panel)):
        item = match.group(1)
        panel_edits.append((match.start(), match.end(),
                            '{"preset-item--add": ' + item + '.kind === "add", "preset-item--account": '
                            + item + '.kind === "preset" && (' + item + '.preset?.preset_id ?? 0) > 0, '
                            '"preset-item--local": ' + item + '.kind === "preset" && ('
                            + item + '.preset?.preset_id ?? 0) <= 0}'))
    # 本地模式下预设不存账号，登录守卫也要放行，否则未登录时「我的预设」整块只显示空槽。
    bag_component = next(c for c in p['components'] if c['name'] == 'MainPanelBagTab')
    bag_text = panel[bag_component['start']:bag_component['end']]
    bag_guard = 'if (!' + pi['globalStore'] + '.isLoggedIn)'
    assert bag_text.count(bag_guard) == 2, 'preset login guard layout changed'
    for match in re.finditer(re.escape(bag_guard), bag_text):
        panel_edits.append((bag_component['start'] + match.start(), bag_component['start'] + match.end(),
                      'if (!' + pi['globalStore'] + '.isLoggedIn && !__lg.enabled.value)'))
    delete_guard = re.search(r'if \([\w$]+\.stopPropagation\(\), !' + re.escape(pi['globalStore']) + r'\.isLoggedIn\)', bag_text)
    assert delete_guard, 'preset delete login guard changed'
    delete_at = bag_component['start'] + bag_text.index(delete_guard[0])
    panel_edits.append((delete_at, delete_at + len(delete_guard[0]), delete_guard[0][:-1] + ' && !__lg.enabled.value)'))
    panel = apply_edits(panel, panel_edits)
    # 官方的「检查更新」改成打开本机控制台的「更新」页：官方自更新会把整合包替换回原版，
    # 而且它读的清单与我们自己的版本检查不是同一个来源。按钮的处理函数名随官方构建漂移，
    # 所以按「类名 → 紧随其后的 onClick」定位，而不是匹配函数名。
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
    pattern = r'([\w$]+)\.value \? await ([\w$]+)\(await ' + re.escape(pi['unfavoriteAsset']) + r'\(([\w$]+)\.item.asset_id\)\) : await \2\(await ' + re.escape(pi['favoriteAsset']) + r'\(\3\.item\.asset_id\)\)'
    match = re.search(pattern, panel)
    assert match, 'Favorite toggle layout changed'
    panel = panel[:match.start()] + f'__lg.enabled.value ? __lg.favorite({match[3]}.item.asset_id,!{match[1]}.value) : ' + panel[match.start():]

    # 版本号只做记录，不做准入：官方每次发版都会换号，白名单会让"锚点一个都没漂"的
    # 新版也无谓失败（1.0.4 就是这样，白名单得人工加一行才肯编译）。
    # 真正的兼容性判断交给上面的结构断言和 integrate() 里的锚点解析——匹配不上就直接抛，
    # 那才是"需要人工改适配"的信号；版本号不是。
    assert config.get('client_version'), 'Official client version missing from app config'
    shared, main, panel = integrate_assistant(project, base, frontend, structure, shared, main, panel, main_name, panel_name)
    extra_patched_assets = [p.name for p in (base/'frontend/assets').glob('switchDisplayMonitor-*.js')
                            if (frontend/'assets'/p.name).read_bytes() != p.read_bytes()]
    for name, text in [(shared_name, shared), (main_name, main), (panel_name, panel)]:
        (frontend / 'assets' / name).write_text(text, encoding='utf-8')
    manifest = {'shared': shared_name, 'main': main_name, 'panel': panel_name,
                'settingsSymbol': settings['variable'], 'atlasSymbol': atlas_component['variable'],
                'base': str(base), 'version': config['client_version']}
    (root / 'build_manifest_v2.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    hooks.finalize({'project': project, 'root': root, 'base': base, 'frontend': frontend,
                    'structure': structure, 'config': config, 'manifest': manifest,
                    'shared_name': shared_name, 'main_name': main_name, 'panel_name': panel_name,
                    'extra_patched_assets': extra_patched_assets})
