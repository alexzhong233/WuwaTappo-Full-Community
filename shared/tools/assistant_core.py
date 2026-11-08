"""桌宠助手注入：两个平台共用的主流程。

`macos/tools/integrate_assistant.py` 与 `windows/src/tools/integrate_assistant.py`
原本是同一份脚本的两份拷贝，差异全在「Windows 额外接了什么」：桌宠右键原生菜单、
控制台页签、开机自启托管。这里放共用主流程，平台差异用 `Hooks` 子类注入。

钩子拿到的 `ctx` 是主流程已经解析好的绑定（`once` / `mi` / `pi` / `source` /
`structure` / `main_name` / `open_tab` / `store` / `fields`），平台侧直接拿来用，
不重复解析一遍 —— 重复解析正是两份拷贝最容易漂的地方。
"""
import re


class Hooks:
    """平台差异的默认实现：macOS 什么都不加，Windows 覆盖需要的几个。"""

    # 助手前端源码目录（相对 platform project）
    facade_dir = 'fullatlas-source'
    # 要拼进 shared 的前端文件，按顺序拼接
    facade_files = ('assistant_dashboard.js', 'desktop_assistant.js')

    def facade_locals(self):
        """`__WUWA_LOCAL_ATLAS__` 赋值之后，还要挂到控制器上的东西。"""
        return ('\nif(localAtlasController)localAtlasController.assistant=createDesktopAssistant('
                '{ref,window,atlas:localAtlasController,displayName:resolveAssetDisplayName});')

    def before_main_code(self, main, ctx):
        """在插入 `startMain` 之前改 main（右键菜单、页签白名单之类）。"""
        return main

    def code_prefix(self, ctx):
        """`startMain` 那段代码前面要加的函数或监听。"""
        return ''

    def blocked(self, ctx):
        """`startMain` 的 blocked 判定：为真时助手不响应按键。"""
        return '__lg.assistant.updateRequired.value'

    def after_monitor(self, monitor, ctx):
        """显示器模块的额外补丁。"""
        return monitor


def integrate(project, base, frontend, structure, shared, main, panel, main_name, panel_name, hooks=None):
    hooks = hooks or Hooks()
    def once(text, old, new):
        assert text.count(old) == 1, ('assistant anchor', old[:100], text.count(old))
        return text.replace(old, new)

    mi = structure['files'][main_name]['imports']
    pi = structure['files'][panel_name]['imports']

    def with_alias(imports, canonical):
        # 压缩产物的本地名带不带 $1 后缀取决于该次构建：国服 1.0.3 是 listen，
        # 国际服 1.0.3 是 listen$1。两种都认，避免为同一版本维护两份锚点。
        if canonical in imports:
            return imports
        alternate = canonical + '$1'
        if alternate in imports:
            return dict(imports, **{canonical: imports[alternate]})
        return imports

    for name in ('listen', 'invoke', 'watch', 'onMounted', 'onUnmounted'):
        mi = with_alias(mi, name)
        pi = with_alias(pi, name)
    # 1.0.2 shipped a local `invoke$1` wrapper; 1.0.3 imports the tauri `invoke` directly.
    if 'invoke$1' not in mi and 'invoke' in mi:
        mi = dict(mi, **{'invoke$1': mi['invoke']})
    fields = structure['files'][main_name]['snapshots'][0]['fields']
    source = (base / 'readable_js/assets' / main_name).read_text(encoding='utf-8')
    store = mi['globalStore']
    handlers = []
    for event in ('key-press', 'key-release'):
        match = re.search(re.escape(mi['listen']) + r'\("'+event+r'", (\w+) => \{\s*([\w$]+)\(\1.payload\)', source)
        assert match, event
        handlers.append(match[2])
    press_source = next(fn for fn in structure['files'][main_name]['functions'] if fn['name']==handlers[0])
    press_text = source[press_source['start']:press_source['end']]
    first_guard = re.search(r'if \(([^\n]+)\) return;', press_text)
    held_guard = re.findall(r'[\w$]+\.has\(e\)', first_guard[1])[-1]
    main = once(main, 'function '+handlers[0]+'(e) {', 'function '+handlers[0]+'(e,__assistantVirtual=false) {')
    main = once(main, first_guard[0], 'if (__assistantVirtual ? '+held_guard+' : ('+first_guard[1]+')) return;')
    chest = min((fn for fn in structure['files'][main_name]['functions'] if 'openChest failed' in source[fn['start']:fn['end']]), key=lambda fn:fn['end']-fn['start'])
    chest_text = source[chest['start']:chest['end']]
    cost = re.search(re.escape(store)+r'\.totalClickCount < ([\w$]+)', chest_text)[1]
    animation = re.search(r'const \w+ = ([\w$]+)\(e\);', chest_text)[1]
    # Chest readiness computeds drift every official rebuild (1.0.3: Wn/Gn,
    # 1.0.4: Vn/Wn), so resolve them structurally instead of hardcoding names.
    # Shape: <ready> = pe(() => <blocked>.value || !<store>.isLoggedIn ? !0 : <remaining>.value <= 0)
    ready_pairs = re.findall(
        r'([\w$]+) = pe\(\(\) => ([\w$]+)\.value \|\| ![\w$]+\.isLoggedIn \? !0 : ([\w$]+)\.value <= 0\)', source)
    assert len(ready_pairs) == 2, 'chest readiness contract (expected two ready computeds)'
    left_ready, left_rem = ready_pairs[0][0], ready_pairs[0][2]
    right_ready, right_rem = ready_pairs[1][0], ready_pairs[1][2]
    # Busy guard inside the chest entry: if (<in-flight>.value || <blocked>.value) return;
    busy_guard = re.search(r'if \(([\w$]+)\.value \|\| ([\w$]+)\.value\) return;', chest_text)
    assert busy_guard, 'chest busy guard contract'
    busy_open, blocked = busy_guard.group(1), busy_guard.group(2)
    # Counters, native persistence and logout remain byte-for-byte original.
    # 但官方把裸 403 也当成「凭证失效」：响应拦截器里 403 会走 handleApiCode ->
    # clearAuthTokens + logoutAccount，一次网关策略或版本门禁就能把人踹下线。
    # 上游 FullAtlas 1.3.7 收紧为只认业务码 10002/10003 与 HTTP 401，这里跟上。
    shared = once(shared, 'c === 10002 || c === 10003 || r === 401 || r === 403',
                  'c === 10002 || c === 10003 || r === 401')
    # The official installer would replace this executable and lose integrations.
    shared = once(shared, 'function isAppUpdaterEnabled() {', 'function isAppUpdaterEnabled() {\n return false;')
    shared = once(shared, 'function isForceUpdateLocked() {', 'function isForceUpdateLocked() {\n return window.__WUWA_LOCAL_ATLAS__?.assistant?.updateRequired.value === true;')
    shared = once(shared, 'function schedulePreloadAppUiImages() {', '''function schedulePreloadAppUiImages() {
 const route=(window.location.hash||'#/').slice(1).split('?')[0];
 if(!['/','/panel'].includes(route))return;''')
    # 打字自动表情要「替用户按一下」已装备的表情：官方手动发表情走的是同一个函数，
    # 直接复用，不另造一条渲染路径，联机房间里的表现和手点完全一致。
    emote_fn = min((fn for fn in structure['files'][main_name]['functions']
                    if mi['reportSendSticker'] + '({' in source[fn['start']:fn['end']]),
                   key=lambda fn: fn['end'] - fn['start'])
    emote_text = source[emote_fn['start']:emote_fn['end']]
    assert 'e.src' in emote_text and '.push({' in emote_text, 'manual emote rendering contract'
    emote_items = re.search(r'"emote-items": ([\w$]+)\.value,\s*onSelect: ' + re.escape(emote_fn['name']), source)
    assert emote_items, 'emote selection binding changed'
    emote_items = emote_items.group(1)
    emote_url = re.search(r'"asset-url": ([\w$]+),\s*"emote-items": ' + re.escape(emote_items), source)
    assert emote_url, 'emote asset url binding changed'
    emote_url = emote_url.group(1)
    sync_call = re.search(re.escape(store) + r'\.isLoggedIn && ([\w$]+)\(e\.id\);', emote_text)
    assert sync_call, 'original room emote send contract'
    # 本地图鉴里的表情不属于账号素材，别当成账号道具回传给联机房间。
    main = once(main, sync_call.group(0),
                store + '.isLoggedIn && !__lg.enabled.value && ' + sync_call.group(1) + '(e.id);')
    # 只监听「真实的」物理按键：自动点击用的虚拟 F24 走的是 handler 直调，不会到这里。
    main = once(main, handlers[0] + '(e.payload)',
                handlers[0] + '(e.payload); __lg.assistant.physicalKey(e.payload)')
    main = once(main, handlers[1] + '(e.payload)',
                handlers[1] + '(e.payload); __lg.assistant.physicalRelease(e.payload)')
    factory = '\n'.join((project / hooks.facade_dir / name).read_text(encoding='utf-8')
                        for name in hooks.facade_files)
    shared = once(shared, 'window.__WUWA_LOCAL_ATLAS__ = localAtlasController;', factory + '''
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;''' + hooks.facade_locals())
    shared = once(shared, 'localAtlasController.ready.then(() => { localStartOnlineServices(); return bootstrap(); })',
                  'Promise.all([localAtlasController.ready,localAtlasController.assistant.ready]).then(() => { localStartOnlineServices(); return bootstrap(); })')
    # Record only the original successful server result, before its popup. The
    # recorder catches its own errors and never retries a chest transaction.
    main = once(main, 'const f = '+animation+'(e);', 'const __assistantAttempt=__lg.assistant.beginChest(e);\n        const f = '+animation+'(e);')
    main = once(main, '          '+mi['reportRewardClaimSucc']+'({\n            itemId: M.data.asset_id,',
        '          __lg.assistant.chestSucceeded(__assistantAttempt,M.data);\n          '+mi['reportRewardClaimSucc']+'({\n            itemId: M.data.asset_id,')
    room = fields['currentRoomId']
    members = re.search(r'^([\w$]+)\.value', fields['roomMembers'])[1]+'.value'
    input_room = fields['roomIdInput']
    join_fn = structure['files'][main_name]['cases']['join-room']
    open_settings = re.search(r'class: "settings-btn"[\s\S]{0,300}?onClick: '+re.escape(mi['withModifiers'])+r'\(([\w$]+),',source)[1]
    open_tab = min((fn for fn in structure['files'][main_name]['functions'] if '[panel] open tab failed' in source[fn['start']:fn['end']]), key=lambda fn:fn['end']-fn['start'])['name']
    quit_fn = re.search(r'"tray-quit", \(\) => \{\s*([\w$]+)\(\)', source)[1]
    persist_original = re.search(re.escape(mi['listenPersistClicksBeforeExit'])+r'\(async \(\) => \{\s*([\s\S]+?)\n\s*\}\)',source)[1]
    main = once(main, 'async function '+quit_fn+'() {', 'async function '+quit_fn+'() {\n try{await __lg.assistant.flush();}catch{}')
    ctx = {'once': once, 'mi': mi, 'pi': pi, 'source': source, 'structure': structure,
           'main_name': main_name, 'open_tab': open_tab, 'store': store, 'fields': fields}
    main = hooks.before_main_code(main, ctx)
    code = hooks.code_prefix(ctx) + f'''
{mi['onMounted']}(()=>{{window.setTimeout(()=>__lg.assistant.startMain({{
 total:()=>{store}.cumulativeClickCount,balance:()=>{store}.totalClickCount,
 account:()=>String({store}.userInfo?.cuid||{store}.userInfo?.user_id||{store}.userInfo?.uid||'guest'),
 loggedIn:()=>{store}.isLoggedIn,blocked:()=>{hooks.blocked(ctx)},
 press:key=>{handlers[0]}(key,true),release:{handlers[1]},chestCost:{cost},
 chests:()=>({{left:{{remaining:Math.max(0,{left_rem}.value),ready:{left_ready}.value}},right:{{remaining:Math.max(0,{right_rem}.value),ready:{right_ready}.value}},cooldown:1800000,busy:{busy_open}.value||{blocked}.value}}),
 emotes:()=>({emote_items}.value||[]).map(item=>({{id:item.id,name:item.name,src:item.src||(window.__WUWA_MOD_ASSET_PATH__&&window.__WUWA_MOD_ASSET_PATH__('emote/'+item.name))||{emote_url}('emote/'+item.name)}})),
 sendEmote:item=>{emote_fn['name']}(item),
 monitors:__assistantMonitors,restoreMonitor:__assistantRestoreMonitor,
 currentMonitor:{mi['currentMonitor']},toast:{mi['showToast']},
 openSettings:{open_settings},
 openView:async view=>{{
   if(__lg.assistant.updateRequired.value)return;
   await {open_tab}(view==='atlas'?'atlas':'settings');
   const send=()=>{mi['invoke$1']}('plugin:event|emit',{{event:'local-assistant-view',payload:view}}).catch(()=>{{}});
   send();window.setTimeout(send,800);window.setTimeout(send,2200);
 }},
 quit:{quit_fn},
 persistOriginal:async()=>{{{persist_original}}},
 listenRoom:callback=>{mi['watch']}([()=>{room},()=>{members}.map(x=>[x.id,x.nickname,x.isOwner]).join('|')],()=>{{
   const owner={members}.find(x=>x.isOwner);if({room})callback(String({room}),owner?.nickname||'').catch(()=>{{}});
 }},{{immediate:true}}),
 listenJoin:callback=>{mi['listen']}('local-assistant-join',event=>callback(String(event.payload))),
 join:async code=>{{{input_room}=code;await {join_fn}();}}
}}).catch(error=>console.error('[assistant]',error)),3000);}});
'''
    main = once(main, 'let __localSaved=null;',code+'\nlet __localSaved=null;')
    monitor_name = next(p.name for p in (base/'readable_js/assets').glob('switchDisplayMonitor-*.js'))
    monitor = (base/'readable_js/assets'/monitor_name).read_text(encoding='utf-8')
    # 1.0.3 rebuilt this module, so the switch routine is resolved by its body shape
    # instead of 1.0.2's minified names (Nt/Ht).
    sw = re.search(r'async function ([\w$]+)\(\) \{\n  let e = !1;\n  const t = `\$\{Date\.now\(\)\}', monitor)
    assert sw, 'display switch contract'
    switch_fn = sw.group(1)
    monitor = once(monitor, 'async function ' + switch_fn + '() {', 'async function ' + switch_fn + '(targetName=null) {')
    monitor = once(monitor, 'if (d.length <= 1)', 'if (!targetName && d.length <= 1)')
    monitor = once(monitor, 'const g = (N + 1) % d.length,',
                   'if(targetName){const __t=d.find(x=>x.name===targetName);if(!__t)return false;'
                   'if(_&&p(_)===p(__t))return false;}\n'
                   '    const g = targetName ? d.findIndex(x=>x.name===targetName) : (N + 1) % d.length,')
    mon = re.search(r'async function ([\w$]+)\(\) \{\n  const e = await me\(\),\n    t = await H\(\);\n  return qe\(e, t\)', monitor)
    assert mon, 'display list contract'
    monitor += '\nexport{' + mon.group(1) + ' as __zzMonitors,' + switch_fn + ' as __zzSwitchMonitor};\n'
    monitor = hooks.after_monitor(monitor, ctx)
    (frontend/'assets'/monitor_name).write_text(monitor, encoding='utf-8')
    main = 'import {__zzMonitors as __assistantMonitors,__zzSwitchMonitor as __assistantRestoreMonitor} from "./'+monitor_name+'";\n'+main
    h=pi['createBaseVNode']
    row=f'''{h}('div',{{class:'setting-item','data-desktop-assistant':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'桌宠助手'),{h}('div',{{class:'setting-sub'}},'自动点击与开箱、每日统计、Mod 管理与版本更新')]),{h}('button',{{class:'setting-btn',type:'button',onClick:()=>__lg.assistant.openPanel()}},'打开控制台')]),'''
    panel = once(panel, h+"('div',{class:'setting-item','data-local-gallery-setting':'true'},[",row+'\n'+h+"('div',{class:'setting-item','data-local-gallery-setting':'true'},[")
    # 1.0.3 renames the save+teardown pair; assert the shape (same pair used twice)
    # instead of 1.0.2's literal `ds`/`xt` identifiers.
    _pairs = {m.group(0) for m in re.finditer(r'await [\w$]+\(\), await [\w$]+\(\)', panel)}
    assert any(panel.count(call) == 2 for call in _pairs) or panel.count('await ds(), await xt()') == 2, \
        'panel close lifecycle changed'
    panel = once(panel,'let __panelDisposed=false;',f'''let __assistantViewUnlisten=null;
{pi['onMounted']}(async()=>{{__assistantViewUnlisten=await {pi['listen']}('local-assistant-view',event=>{{
 const view=event.payload;
 if(['overview','rewards','daily'].includes(view))__lg.assistant.openPanel(view).catch(error=>console.error('[assistant-view]',error));
 else if(['atlas','settings'].includes(view))__lg.assistant.closePanel();
}});}});
{pi['onUnmounted']}(()=>{{__assistantViewUnlisten?.();}});
let __panelDisposed=false;''')
    return shared,main,panel
