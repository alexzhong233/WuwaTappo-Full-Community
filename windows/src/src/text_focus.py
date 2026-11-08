"""Bounded read-only UIA focus probe for Chromium, chat apps and modern editors.

Never retrieves Name, Value, text ranges or keyboard content. One daemon worker
isolates slow accessibility providers from the assistant's tick/exit lifecycle.
"""
import threading


def open_automation():
    """把 UIA 自动化对象准备好，不读焦点。

    打字自动表情靠这套东西判断「焦点是不是落在输入框」。它依赖 comtypes 和
    comtypes 生成出来的类型库模块，这两样在打包时最容易漏：comtypes 没进包、
    或 comtypes.gen.UIAutomationClient 没被 hidden-import 收进去，表现都一样——
    探测恒返回 uia-unavailable，界面上只显示「焦点不在输入框」，看不出是依赖缺失。

    所以单独抽出来：构建期的 --self-check 只验这一步（依赖齐不齐），
    不要求「此刻真有焦点」——CI 的 runner 上没有用户焦点，那不该算构建失败。
    """
    import sys
    sys.coinit_flags = 0
    import comtypes
    from comtypes.client import CreateObject
    try:
        from comtypes.gen.UIAutomationClient import IUIAutomation
    except ImportError:
        # build.py 打包前会让 comtypes 预生成这个类型库模块，但那份生成物一旦没
        # 进包，这里就只剩 uia-unavailable。运行时再生成一次，代价是首次探测多
        # 几百毫秒，随后走缓存。
        from comtypes.client import GetModule
        GetModule('UIAutomationCore.dll')
        from comtypes.gen.UIAutomationClient import IUIAutomation
    comtypes.CoInitializeEx(0)
    return _Automation(comtypes, CreateObject('{FF48DBA4-60EF-4201-AA87-54103EEF594E}', interface=IUIAutomation))


class _Automation:
    """CoInitializeEx / CoUninitialize 的配对封装，避免每处调用各写一遍。"""

    def __init__(self, comtypes, automation):
        self._comtypes = comtypes
        self.automation = automation

    def close(self):
        self.automation = None
        try:
            self._comtypes.CoUninitialize()
        except Exception:
            pass


class FocusedTextProbe:
    def __init__(self):
        self.lock=threading.Lock()
        self.busy=False

    def probe(self, timeout=.3):
        with self.lock:
            if self.busy:return {'editable':False,'source':'uia-busy'}
            self.busy=True
        done=threading.Event();result={'editable':False,'source':'uia-timeout'}
        def work():
            link=element=None
            try:
                link=open_automation()
                element=link.automation.GetFocusedElement()
                typ=element.CurrentControlType
                readonly=element.GetCurrentPropertyValue(30046)
                value_pattern=element.GetCurrentPropertyValue(30043)
                text_edit=element.GetCurrentPropertyValue(30149)
                editable=(readonly is not True and (typ==50004 or typ==50003 and value_pattern is True or typ==50030 and text_edit is True))
                result.update(editable=bool(editable),source='uia',control_type=typ)
            except Exception as error:
                # 只留异常类型名：整条链路排查靠它区分「依赖没进包」和「取焦点失败」，
                # 又不会把路径之类的东西带到前端。
                result.update(editable=False,source='uia-unavailable',error=type(error).__name__)
            finally:
                element=None
                if link is not None:link.close()
                with self.lock:self.busy=False
                done.set()
        threading.Thread(target=work,name='text-focus-probe',daemon=True).start()
        return dict(result) if done.wait(timeout) else {'editable':False,'source':'uia-timeout'}
