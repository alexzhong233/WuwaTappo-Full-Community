"""社区版本检查：semver 比较与 /community/update-check 路由（含 prerelease 规则）。"""
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))


class FakeResponse:
    def __init__(self, payload):
        self.payload = json.dumps(payload).encode('utf-8')

    def read(self, size=-1):
        return self.payload

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


RELEASES = [
    {'tag_name': 'v9.9.9', 'draft': True, 'prerelease': False,
     'html_url': 'https://github.com/x/releases/tag/v9.9.9',
     'published_at': '2026-11-08T00:00:00Z'},
    {'tag_name': 'v1.3.0-alpha.2', 'draft': False, 'prerelease': True,
     'html_url': 'https://github.com/x/releases/tag/v1.3.0-alpha.2',
     'published_at': '2026-11-07T00:00:00Z'},
    {'tag_name': 'v1.2.4', 'draft': False, 'prerelease': False,
     'html_url': 'https://github.com/x/releases/tag/v1.2.4',
     'published_at': '2026-11-01T00:00:00Z'},
]


class CommunityCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        config = {'port': 0, 'token': 'fixture-token', 'region': 'cn',
                  'community_version': '1.3.0-alpha.1'}
        (self.root / 'local_config.json').write_text(json.dumps(config))
        (self.root / 'cache').mkdir()
        # 与 test_custom_resources_host.py 相同的加载方式：真实执行 local_host.py，
        # 但屏蔽输入/防火墙等 Windows 专属副作用。显式 utf-8 避免 CI cp1252 编码问题。
        source = (Path(__file__).resolve().parents[1] / 'src/local_host.py').read_text(encoding='utf-8')
        self.host = {'__file__': str(self.root / 'local_host.py'), '__name__': 'community_test'}
        with patch('desktop_assistant.PhysicalInput'), patch('threading.Thread.start'), \
                patch.dict(os.environ, {'SystemRoot': str(self.root)}):
            exec(compile(source, self.host['__file__'], 'exec'), self.host)
        self.host['set_firewall'] = lambda enabled: None
        self.server = self.host['ThreadingHTTPServer'](('127.0.0.1', 0), self.host['Handler'])
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5)
        self.temp.cleanup()

    def post(self, route, body=None):
        connection = HTTPConnection('127.0.0.1', self.server.server_port, timeout=5)
        payload = json.dumps(body or {}).encode('utf-8')
        connection.request('POST', route, body=payload, headers={
            'Origin': 'http://tauri.localhost', 'X-Local-Token': 'fixture-token',
            'Content-Type': 'application/json'})
        response = connection.getresponse()
        data = json.loads(response.read().decode('utf-8'))
        connection.close()
        return response.status, data

    def get(self, route):
        connection = HTTPConnection('127.0.0.1', self.server.server_port, timeout=5)
        connection.request('GET', route, headers={
            'Origin': 'http://tauri.localhost', 'X-Local-Token': 'fixture-token'})
        response = connection.getresponse()
        data = json.loads(response.read().decode('utf-8'))
        connection.close()
        return response.status, data

    def test_prerelease_ordering_follows_semver(self):
        compare = self.host['_compare_semver']
        self.assertGreater(compare('1.3.0-alpha.10', '1.3.0-alpha.9'), 0)
        self.assertGreater(compare('1.3.0-alpha.2', '1.3.0-alpha.1'), 0)
        self.assertGreater(compare('1.3.0', '1.3.0-alpha.2'), 0)
        self.assertLess(compare('1.3.0-alpha.1', '1.3.0'), 0)
        self.assertGreater(compare('1.4.0-alpha.1', '1.3.0'), 0)
        self.assertEqual(compare('1.3.0', '1.3.0'), 0)
        self.assertGreater(compare('v1.3.1', '1.3.0'), 0)

    def test_update_check_skips_prerelease_by_default(self):
        with patch('urllib.request.urlopen', return_value=FakeResponse(RELEASES)):
            status, data = self.post('/community/update-check')
        self.assertEqual(status, 200)
        # 默认只提醒正式版：alpha.2 被跳过，当前 alpha.1 相对 1.2.4 不算落后。
        self.assertFalse(data['updateAvailable'])
        self.assertEqual(data['latest'], '1.2.4')
        self.assertEqual(data['skippedPrerelease'], 1)
        self.assertFalse(data['prereleaseIncluded'])
        # 草稿（v9.9.9）不能成为最新版本。
        self.assertNotEqual(data['latest'], '9.9.9')

    def test_update_check_reports_prerelease_when_enabled(self):
        self.host['ASSISTANT'].state['settings']['prerelease_updates'] = True
        with patch('urllib.request.urlopen', return_value=FakeResponse(RELEASES)):
            status, data = self.post('/community/update-check')
        self.assertEqual(status, 200)
        self.assertTrue(data['updateAvailable'])
        self.assertEqual(data['current'], '1.3.0-alpha.1')
        self.assertEqual(data['latest'], '1.3.0-alpha.2')
        self.assertTrue(data['prereleaseIncluded'])
        self.assertEqual(data['skippedPrerelease'], 0)
        self.assertEqual(data['releaseUrl'], 'https://github.com/x/releases/tag/v1.3.0-alpha.2')

    def test_update_check_with_only_prerelease_says_so(self):
        only_alpha = [release for release in RELEASES if release['prerelease']]
        with patch('urllib.request.urlopen', return_value=FakeResponse(only_alpha)):
            status, data = self.post('/community/update-check')
        self.assertEqual(status, 200)
        self.assertFalse(data['updateAvailable'])
        self.assertEqual(data['latest'], '')
        self.assertIn('只发布了预发布版本', data['message'])

    def test_update_check_reports_up_to_date(self):
        releases = [dict(release) for release in RELEASES]
        releases[1]['tag_name'] = 'v1.2.4'
        with patch('urllib.request.urlopen', return_value=FakeResponse(releases)):
            status, data = self.post('/community/update-check')
        self.assertEqual(status, 200)
        self.assertFalse(data['updateAvailable'])
        self.assertEqual(data['latest'], '1.2.4')

    def test_asset_base_matches_the_local_asset_route(self):
        # 前端用 base + 'custom-assets/...' 拼图地址，必须能直接落到本机路由上。
        status, data = self.get('/asset-base')
        self.assertEqual(status, 200)
        self.assertTrue(data['base'].startswith('http://127.0.0.1:'))
        self.assertTrue(data['base'].endswith('/'))

    def test_pet_site_points_at_the_region_download_page(self):
        # 拦截真正的浏览器启动：这里只验证地址，不弹窗。
        with patch('webbrowser.open', return_value=True) as opener:
            status, data = self.post('/official/pet-site')
        self.assertEqual(status, 200)
        self.assertEqual(opener.call_args[0][0], data['url'])
        self.assertTrue(data['url'].startswith('https://download-mc-wuwatappo.'))
        self.assertTrue(data['url'].endswith('/download/'))

    def test_pet_site_uses_the_global_domain_for_global_builds(self):
        self.host['CONFIG']['region'] = 'global'
        with patch('webbrowser.open', return_value=True):
            status, data = self.post('/official/pet-site')
        self.assertEqual(status, 200)
        self.assertIn('download-wutheringwaves-wuwatappo.', data['url'])
        self.assertTrue(data['url'].endswith('/download/zh-tw/'))

    def test_update_check_survives_github_errors(self):
        with patch('urllib.request.urlopen', side_effect=OSError('network down')):
            status, data = self.post('/community/update-check')
        self.assertEqual(status, 200)
        self.assertFalse(data['updateAvailable'])
        self.assertIn('检查更新失败', data['message'])

    def test_version_route_reads_build_config(self):
        status, data = self.post('/community/version')
        self.assertEqual(status, 200)
        self.assertEqual(data, {'current': '1.3.0-alpha.1'})


class PageResponse:
    """发布页 HTML 的假响应：read() 返回字节。"""

    def __init__(self, page):
        self.page = page.encode('utf-8') if isinstance(page, str) else page

    def read(self, size=-1):
        return self.page

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


class ReleaseNotesTests(unittest.TestCase):
    """更新说明的两种来源：Release 正文（Markdown）与发布页 HTML。"""

    def setUp(self):
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
        from desktop_updater import notes_for_release, release_notes_from_html, release_notes_text
        self.text = release_notes_text
        self.html = release_notes_from_html
        self.notes = notes_for_release

    def test_markdown_body_drops_tables_images_and_urls(self):
        body = '\n'.join([
            '## v1.3.1',
            '| 文件 | 大小 |',
            '| --- | --- |',
            '![preview](https://example.invalid/a.png)',
            '- 控制台改成页签，见 [说明](https://example.invalid/doc)',
            'https://example.invalid/raw',
            '**修复** 右键菜单不响应',
        ])
        self.assertEqual(self.text(body), 'v1.3.1\n控制台改成页签，见 说明\n修复 右键菜单不响应')

    def test_html_parser_reads_only_the_markdown_body(self):
        page = ('<html><body><nav>导航</nav>'
                '<div class="markdown-body"><h1>v1.3.1</h1>'
                '<p>修复了<strong>控制台</strong>页签</p>'
                '<ul><li>右键菜单</li><li>自启托管</li></ul>'
                '<table><tr><td>升级包</td><td>86MB</td></tr></table>'
                '<p>末尾一句</p></div>'
                '<footer>页脚不该出现</footer></body></html>')
        out = self.html(page)
        self.assertEqual(out, 'v1.3.1\n修复了控制台页签\n• 右键菜单\n• 自启托管\n末尾一句')
        self.assertNotIn('导航', out)
        self.assertNotIn('页脚', out)
        self.assertNotIn('86MB', out)

    def test_truncated_page_is_treated_as_no_notes(self):
        # 页面被 CDN 截断时 markdown-body 没有闭合，宁可显示"读不到"也不要贴半段。
        self.assertEqual(self.html('<div class="markdown-body"><p>半截'), '')

    def test_page_without_markdown_body_returns_nothing(self):
        self.assertEqual(self.html('<html><body><div class="release-header">v1.3.1</div></body></html>'), '')

    def test_release_body_wins_without_touching_the_network(self):
        with patch('urllib.request.urlopen') as opened:
            out = self.notes({'body': '- 直接来自 API', 'html_url': 'https://github.com/x/releases/tag/v1'})
        self.assertEqual(out, '直接来自 API')
        opened.assert_not_called()

    def test_empty_body_falls_back_to_the_release_page(self):
        page = '<div class="markdown-body"><p>来自页面</p></div>'
        with patch('urllib.request.urlopen', return_value=PageResponse(page)) as opened:
            out = self.notes({'body': '', 'html_url': 'https://github.com/x/releases/tag/v1'})
        self.assertEqual(out, '来自页面')
        self.assertEqual(opened.call_count, 1)

    def test_non_github_page_is_refused(self):
        with patch('urllib.request.urlopen') as opened:
            out = self.notes({'body': '', 'html_url': 'https://evil.invalid/releases/tag/v1'})
        self.assertEqual(out, '')
        opened.assert_not_called()

    def test_page_fetch_failure_keeps_the_check_alive(self):
        with patch('urllib.request.urlopen', side_effect=OSError('offline')):
            self.assertEqual(self.notes({'body': '', 'html_url': 'https://github.com/x/releases/tag/v1'}), '')


if __name__ == '__main__':
    unittest.main()
