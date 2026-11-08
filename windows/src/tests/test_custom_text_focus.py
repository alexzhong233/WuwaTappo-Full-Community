"""焦点探测的契约测试。

这条链路（comtypes + comtypes.gen.UIAutomationClient）在打包时最容易静默缺失：
探测恒返回 uia-unavailable，界面上只显示「焦点不在输入框」，看不出是依赖没进包。
构建期由 LocalGalleryHost --self-check 兜住；这里锁两端契约：

* probe 永远返回带 editable/source 的 dict，不向上抛；
* open_automation 遇到缺依赖必须把异常抛出来，不能自己吞掉——self-check 正是
  靠它来判断依赖去留。
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))

from text_focus import FocusedTextProbe, open_automation


class ProbeContractTests(unittest.TestCase):
    def test_probe_returns_shape_without_raising(self):
        # 没装 comtypes 的机器上会退化成 uia-unavailable，这同样算通过：
        # 这条测的是「不崩、字段齐」，依赖在不在由 self-check 负责。
        result = FocusedTextProbe().probe(timeout=5)
        self.assertIn('editable', result)
        self.assertIn('source', result)
        self.assertIsInstance(result['editable'], bool)

    def test_missing_comtypes_raises(self):
        saved = sys.modules.get('comtypes')
        sys.modules['comtypes'] = None      # 让 `import comtypes` 直接抛 ImportError
        try:
            with self.assertRaises(Exception):
                open_automation()
        finally:
            if saved is None:
                del sys.modules['comtypes']
            else:
                sys.modules['comtypes'] = saved


if __name__ == '__main__':
    unittest.main()
