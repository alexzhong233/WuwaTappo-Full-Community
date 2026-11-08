"""Extend a structurally verified 1.0.2 frontend; fail on contract changes."""
import re
import sys


def integrate(project, base, frontend, structure, shared, main, panel, main_name, panel_name):
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
    factory = (project / 'src/assistant_dashboard.js').read_text(encoding='utf-8')+'\n'+(project / 'src/desktop_assistant.js').read_text(encoding='utf-8')+'\n'+(project / 'src/desktop_context_menu.js').read_text(encoding='utf-8')
    shared = once(shared, 'window.__WUWA_LOCAL_ATLAS__ = localAtlasController;', factory + '''
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;
localAtlasController.assistant=createDesktopAssistant({ref,window,atlas:localAtlasController,displayName:resolveAssetDisplayName});
localAtlasController.contextMenu=createDesktopContextMenu({invoke,request:(path,body)=>localAtlasController.request(path,body),settings:loadDesktopCatSettings,visible:()=>petDockVisibleForMenu,labels:async()=>{try{return await getNativeMenuLabels(formatShortcut((await getAppShortcuts()).togglePet));}catch(error){return {};}},report:details=>localAtlasController.request('/diagnostic',details)});
window.__WUWA_OPEN_PANEL_TAB__=tab=>invoke('open_panel_window',{payload:{tab}}).then(()=>true).catch(()=>false);''')
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
    # 桌宠上右键：官方原本调原生 popup_main_dock_context_menu。换成我们自己弹的原生菜单
    # （多了控制台入口，点外部或 Esc 会收起），并顺手释放拖拽状态，避免菜单卡住后拖不动。
    popup = re.search(r'await ([\w$]+)\("popup_main_dock_context_menu", \{\s*x: ([\w$]+),\s*y: ([\w$]+)\s*\}\)', main)
    assert popup, 'native dock menu anchor changed'
    popup_function = min((fn for fn in structure['files'][main_name]['functions']
                          if '"popup_main_dock_context_menu"' in source[fn['start']:fn['end']]),
                         key=lambda fn: fn['end'] - fn['start'])
    popup_source = source[popup_function['start']:popup_function['end']]
    cancel_gesture = re.search(r'finally\s*\{[\s\S]*?,\s*([\w$]+)\(\),\s*[\w$]+ && window\.setTimeout', popup_source)
    assert cancel_gesture, 'dock cleanup binding changed'
    cancel_gesture = cancel_gesture.group(1)
    assert cancel_gesture in re.findall(r'cancelDockPointerGesture:\s*([\w$]+)', source), 'dock cleanup binding changed'
    swallow_function = min((fn for fn in structure['files'][main_name]['functions']
                            if '"set_main_swallow_mouse"' in source[fn['start']:fn['end']]),
                           key=lambda fn: fn['end'] - fn['start'])
    swallow_header = re.search(r'function ([\w$]+)\(([\w$]+)\) \{',
                               source[swallow_function['start']:swallow_function['end']])
    assert swallow_header, 'mouse interception binding changed'
    # 菜单开着时不要把鼠标事件交给桌宠，否则点菜单等于点桌宠。
    main = once(main, swallow_header.group(0),
                swallow_header.group(0) + '\n if(__lg.contextMenu?.isOpen()) ' + swallow_header.group(2) + '=false;')
    native_invoke = mi.get('invoke$1', mi.get('invoke'))
    context_actions = ('{openTab:' + open_tab + ',openConsole:async()=>{await __lg.assistant.openPanel();},'
                       'before:async()=>{await ' + cancel_gesture + '();await ' + native_invoke + "('set_main_swallow_mouse',{enabled:false});},"
                       'after:async()=>{await ' + native_invoke + "('set_main_swallow_mouse',{enabled:false});await " + cancel_gesture + '();}}')
    main = once(main, popup.group(0), f'''try{{await __lg.contextMenu.popup({popup.group(2)},{popup.group(3)},{context_actions});}}
            catch(error){{console.warn('[context-menu] native extension unavailable',error);{popup.group(0)};}}''')
    # 「控制台」页不受更新锁拦截：更新卡住时用户仍要能进来点更新。
    tab_guard = re.search(r'if \((' + re.escape(mi['globalStore']) + r'|[\w$]+)\(\)\) \{\s*' + re.escape(mi['showToast']) + r'\([\w$]+\(("update_force_lock_hint")\)\)', main)
    if not tab_guard:
        tab_guard = re.search(r'if \(([\w$]+)\(\)\) \{\s*' + re.escape(mi['showToast']) + r'\(', main)
    assert tab_guard, 'panel update-lock guard changed'
    main = main[:tab_guard.start()] + 'if (e !== "console" && ' + tab_guard.group(1) + '()) {' + main[tab_guard.start() + len('if (' + tab_guard.group(1) + '()) {'):]
    code = f'''
{mi['onMounted']}(()=>{{window.setTimeout(()=>__lg.contextMenu.initialize({context_actions}).catch(error=>console.warn('[context-menu] initialize',error)),1500);}});
{mi['onMounted']}(()=>{{window.setTimeout(()=>__lg.assistant.startMain({{
 total:()=>{store}.cumulativeClickCount,balance:()=>{store}.totalClickCount,
 account:()=>String({store}.userInfo?.cuid||{store}.userInfo?.user_id||{store}.userInfo?.uid||'guest'),
 loggedIn:()=>{store}.isLoggedIn,blocked:()=>__lg.assistant.updateRequired.value||__lg.contextMenu.isOpen(),
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
    # 面板页签白名单：官方的折叠/切换逻辑只认背包/图鉴/合成/联机/设置四个页。
    # 新加的「控制台」必须放行，否则点进去会被当成非法页签退回。
    _tab_guard = 'return e === "bag" || e === "atlas" || e === "exchange" || e === "room" || e === "setting"'
    if monitor.count(_tab_guard) == 1:
        monitor = monitor.replace(_tab_guard, _tab_guard + ' || e === "console"', 1)
    else:
        print('warn: 面板页签白名单锚点变了，「控制台」页可能被官方逻辑拒掉', file=sys.stderr)
    # 开机自启：官方在 Windows 上注册的是「直接拉桌宠 exe」的计划任务，开机后本地
    # 服务没起，图鉴和控制台会 Failed to fetch。这里改成问本机服务，由它注册一个
    # 跑 launch.ps1 的任务（先起宿主再起桌宠），并顺手迁移官方旧任务。
    _auto_status = re.search(r'return Fe\(\) \? await [\w$]+\("is_elevated_autostart_enabled"\) : await [\w$]+\(\)', monitor)
    if _auto_status:
        monitor = monitor[:_auto_status.start()] + \
            'return (await window.__WUWA_LOCAL_ATLAS__.request("/startup",{action:"status"})).enabled' + \
            monitor[_auto_status.end():]
    else:
        print('warn: 官方开机自启状态查询锚点变了，设置里的开关仍走官方实现', file=sys.stderr)
    _auto_set = re.search(r'(async function ([\w$]+)\(([\w$]+)\) \{)\s*try \{\s*if \(Fe\(\)\)', monitor)
    if _auto_set:
        # 只把一句 return 插到函数体开头，原实现留在后面当死代码：
        # 这样花括号仍然配平，官方改内部逻辑也不会把这个文件搞坏。
        monitor = monitor[:_auto_set.start(1)] + _auto_set.group(1) + \
            ('\n return window.__WUWA_LOCAL_ATLAS__.request("/startup",{action:%s?"enable":"disable"});'
             % _auto_set.group(3)) + \
            monitor[_auto_set.end(1):]
    else:
        print('warn: 官方开机自启开关锚点变了，设置里的开关仍走官方实现', file=sys.stderr)
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
