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


if __name__ == '__main__':
    unittest.main()
