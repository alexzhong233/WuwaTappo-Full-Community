"""Windows 侧的助手注入钩子。

主流程在 `shared/tools/assistant_core.py`，与 macOS 共用（包含面板「控制台」页签、
更新锁放行与页签白名单）。Windows 专属部分只有两项：

1. 桌宠右键原生菜单（`desktop_context_menu.js` + 原生 Win32 弹窗）
2. 开机自启交给本机服务托管（`/startup`）
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(next(p for p in Path(__file__).resolve().parents
                            if (p / 'shared' / 'tools').is_dir()) / 'shared' / 'tools'))
from assistant_core import Hooks, integrate as _integrate  # noqa: E402


class WindowsHooks(Hooks):
    facade_dir = 'src'
    facade_files = ('assistant_dashboard.js', 'desktop_assistant.js', 'desktop_context_menu.js')

    # 右键菜单的入口要留给 initialize 和 popup 两处，所以存成实例属性。
    context_actions = ''

    def facade_locals(self):
        return super().facade_locals() + '''
if(localAtlasController)localAtlasController.contextMenu=createDesktopContextMenu({invoke,request:(path,body)=>localAtlasController.request(path,body),settings:loadDesktopCatSettings,visible:()=>petDockVisibleForMenu,labels:async()=>{try{return await getNativeMenuLabels(formatShortcut((await getAppShortcuts()).togglePet));}catch(error){return {};}},report:details=>localAtlasController.request('/diagnostic',details)});'''

    def before_main_code(self, main, ctx):
        once, mi = ctx['once'], ctx['mi']
        source, structure, main_name = ctx['source'], ctx['structure'], ctx['main_name']
        open_tab = ctx['open_tab']
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
        self.context_actions = ('{openTab:' + open_tab + ',openConsole:async()=>{await __lg.assistant.openPanel();},'
                                'before:async()=>{await ' + cancel_gesture + '();await ' + native_invoke + "('set_main_swallow_mouse',{enabled:false});},"
                                'after:async()=>{await ' + native_invoke + "('set_main_swallow_mouse',{enabled:false});await " + cancel_gesture + '();}}')
        main = once(main, popup.group(0), f'''try{{await __lg.contextMenu.popup({popup.group(2)},{popup.group(3)},{self.context_actions});}}
            catch(error){{console.warn('[context-menu] native extension unavailable',error);{popup.group(0)};}}''')
        return main

    def code_prefix(self, ctx):
        mi = ctx['mi']
        return ('\n' + mi['onMounted'] + '(()=>{window.setTimeout(()=>__lg?.contextMenu?.initialize('
                + self.context_actions
                + ").catch(error=>console.warn('[context-menu] initialize',error)),1500);});")

    def blocked(self, ctx):
        return '__lg.assistant.updateRequired.value||Boolean(__lg.contextMenu?.isOpen())'

    def after_monitor(self, monitor, ctx):
        # 开机自启：官方在 Windows 上注册的是「直接拉桌宠 exe」的计划任务，开机后本地
        # 服务没起，图鉴和控制台会 Failed to fetch。这里改成问本机服务，由它注册一个
        # 先起宿主再起桌宠的任务，并顺手迁移官方旧任务。
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
        return monitor


def integrate(project, base, frontend, structure, shared, main, panel, main_name, panel_name):
    return _integrate(project, base, frontend, structure, shared, main, panel, main_name, panel_name,
                      hooks=WindowsHooks())
