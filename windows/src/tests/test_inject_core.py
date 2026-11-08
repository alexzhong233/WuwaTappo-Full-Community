"""注入器公共核的契约：锚点没了要炸，补丁要按预期改写。

两个平台的 `build_integration.py` / `integrate_assistant.py` 现在都从这里取逻辑，
所以这里错了是两个平台一起错 —— 这些用例就是那道闸。
"""
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / 'shared' / 'tools'))

import assistant_core
import inject_core


class AnchorToolTests(unittest.TestCase):
    """replace / apply_edits / function_for / named 的行为。"""

    def test_replace_asserts_the_anchor_count(self):
        self.assertEqual(inject_core.replace('a b a', 'a', 'x', 2), 'x b x')
        with self.assertRaises(AssertionError):
            inject_core.replace('a b a', 'a', 'x')      # 默认只接受 1 处
        with self.assertRaises(AssertionError):
            inject_core.replace('a b', 'zzz', 'x')      # 锚点根本不在

    def test_apply_edits_keeps_earlier_offsets_stable(self):
        # 从后往前改，前面区间的下标才不会被前面的插入顶开。
        self.assertEqual(inject_core.apply_edits('0123456789', [(8, 10, 'AB'), (0, 1, 'zero')]),
                         'zero1234567AB')

    def test_apply_edits_rejects_out_of_range(self):
        with self.assertRaises(AssertionError):
            inject_core.apply_edits('abc', [(0, 99, 'x')])

    def test_function_for_picks_the_innermost_match(self):
        meta = {'functions': [{'name': 'outer', 'start': 0, 'end': 20},
                              {'name': 'inner', 'start': 5, 'end': 15}]}
        self.assertEqual(inject_core.function_for(meta, 'x' * 30, 'x')['name'], 'inner')

    def test_function_for_fails_when_the_needle_is_gone(self):
        with self.assertRaises(AssertionError):
            inject_core.function_for({'functions': [{'name': 'f', 'start': 0, 'end': 3}]}, 'abc', 'zzz')

    def test_named_requires_exactly_one(self):
        one = [{'name': 'f', 'start': 0, 'end': 1}]
        self.assertEqual(inject_core.named({'functions': one}, 'f')['end'], 1)
        with self.assertRaises(AssertionError):
            inject_core.named({'functions': one * 2}, 'f')


class SharedPatchTests(unittest.TestCase):
    """六段公共补丁。锚点变了要抛，改出来的内容要能认出来。"""

    def test_mod_asset_url_uses_the_second_parameter(self):
        out = inject_core.patch_mod_asset_url('function buildAssetUrl(e, t) {\n  return 1\n}')
        self.assertIn('__WUWA_MOD_ASSET_PATH__(t)', out)
        self.assertIn('withAssetUrlCacheBust(modAssetPath)', out)

    def test_mod_asset_url_requires_the_known_signature(self):
        with self.assertRaises(AssertionError):
            inject_core.patch_mod_asset_url('function buildAssetUrl(e) {\n}')

    def test_mod_display_name_falls_back_to_the_entry_name(self):
        source = ('function resolveAssetDisplayName(e, t) {\n'
                  '  const n = t?.trim() ?? "";\n'
                  '  if (!n) return "";\n'
                  '  const s = String(e ?? "").trim().toLowerCase();\n'
                  '  return s === "character_skin" || s === "character_skin_atlas" ? '
                  'resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? '
                  'resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? '
                  'resolveEmoteDisplayName(n) : ""\n'
                  '}')
        out = inject_core.patch_mod_display_name(source)
        self.assertIn('__WUWA_MOD_NAMES__', out)
        self.assertNotIn('return s === "character_skin"', out)

    def test_online_services_becomes_repeatable(self):
        source = ('loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\n'
                  'initTrackCommonProps();\ninitFrontendAlertReporting();')
        out = inject_core.patch_online_services(source)
        self.assertIn('function localStartOnlineServices() {', out)
        self.assertIn('if (localOnlineServicesStarted) return;', out)

    def test_invoke_guard_patches_every_wrapper_that_exists(self):
        out = inject_core.patch_invoke_guard('async function invoke$1(e, t = {}, n) {\n}\n')
        self.assertIn('checkInvoke(e, t)', out)

    def test_invoke_guard_strict_vs_lenient(self):
        # macOS 要求必须插上（保险丝），Windows 允许历史构建里没有。
        with self.assertRaises(AssertionError):
            inject_core.patch_invoke_guard('no wrapper here', strict=True)
        self.assertEqual(inject_core.patch_invoke_guard('no wrapper here'), 'no wrapper here')

    def test_atlas_guards_append_the_platform_interceptor(self):
        source = ('async function ensureAppearanceAssetCatalog(e) {\n}\n'
                  'function applyCatalog(e, t, n) {\n}\n'
                  'function atlas() {\n}\n'
                  'this.instance.interceptors.request.use(t => {\n')
        out = inject_core.patch_atlas_guards(source, '\n PLATFORM_EXTRA')
        self.assertIn('PLATFORM_EXTRA', out)
        self.assertIn('offlineError()', out)
        self.assertEqual(out.count('this.instance.interceptors.request.use'), 1)


class HooksTests(unittest.TestCase):
    """默认钩子就是 macOS 的行为；平台文件不许把主流程抄回来。"""

    def test_default_hooks_are_the_macos_behaviour(self):
        hooks = assistant_core.Hooks()
        self.assertIn('createDesktopAssistant', hooks.facade_locals())
        self.assertEqual(hooks.blocked({}), '__lg.assistant.updateRequired.value')
        self.assertEqual(hooks.before_main_code('main', {}), 'main')
        self.assertEqual(hooks.code_prefix({}), '')
        self.assertEqual(hooks.after_monitor('monitor', {}), 'monitor')
        self.assertEqual(hooks.facade_files, ('assistant_dashboard.js', 'desktop_assistant.js'))

    def test_platform_files_do_not_copy_the_main_flow(self):
        # 主流程的指纹：记录开箱、启动助手。出现在平台文件里就说明又抄了一份。
        markers = ('beginChest', 'chestSucceeded', 'startMain(', 'localStartOnlineServices')
        for relative in ('macos/tools/integrate_assistant.py',
                         'windows/src/tools/integrate_assistant.py'):
            text = (REPO / relative).read_text(encoding='utf-8')
            for marker in markers:
                self.assertNotIn(marker, text, f'{relative} 又抄了一份主流程（{marker}）')
            self.assertIn('integrate', text, f'{relative} 必须导出 integrate')


if __name__ == '__main__':
    unittest.main()
