"""Exercise the real localhost Mod routes without Windows input/firewall APIs."""
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.parse import quote
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from test_custom_resources import empty_official, PNG


class CustomResourceHostTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        config = {'port': 0, 'token': 'fixture-token', 'region': 'cn', 'asset_count': 0, 'atlas_count': 1}
        (self.root / 'local_config.json').write_text(json.dumps(config))
        cache = self.root / 'cache'
        cache.mkdir()
        self.official = empty_official()
        (cache / 'offline_catalog.json').write_text(json.dumps(self.official['catalog']))
        (cache / 'atlas.json').write_text(json.dumps({'data': self.official['atlas']}))
        (cache / 'asset_manifest.json').write_text(json.dumps({'records': [], 'errors': []}))
        self.original_cache = {p.name: p.read_bytes() for p in cache.iterdir()}
        source = (Path(__file__).resolve().parents[1] / 'src/local_host.py').read_text()
        self.host = {'__file__': str(self.root / 'local_host.py'), '__name__': 'mod_host_test'}
        with patch('desktop_assistant.PhysicalInput'), patch('threading.Thread.start'), patch.dict(os.environ, {'SystemRoot': str(self.root)}):
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

    def request(self, route, body=None, token='fixture-token'):
        connection = HTTPConnection('127.0.0.1', self.server.server_port, timeout=5)
        headers = {'Origin': 'http://tauri.localhost', 'X-Local-Token': token, 'Content-Type': 'application/json'}
        try:
            connection.request('GET' if body is None else 'POST', route, None if body is None else json.dumps(body), headers)
            response = connection.getresponse()
            value = response.read()
            return response.status, json.loads(value) if 'application/json' in response.getheader('Content-Type', '') else value
        finally:
            connection.close()

    def test_import_images_remove_and_official_mode_preserve_cache_and_state(self):
        archive = self.root / 'mod.zip'
        image_path = 'assets/我的 图片.png'
        with zipfile.ZipFile(archive, 'w') as pack:
            pack.writestr('manifest.json', json.dumps({'schemaVersion': 1, 'name': '自制外观包'}))
            pack.writestr('items.json', json.dumps({'items': [
                {'name': '自制表情', 'type': 'emote', 'assets': {'image': image_path}}
            ]}))
            pack.writestr(image_path, PNG)
        status, _ = self.request('/custom/import', {'path': str(archive)}, token='wrong')
        self.assertEqual(status, 403)
        status, installed = self.request('/custom/import', {'path': str(archive)})
        self.assertEqual(status, 200)
        self.assertFalse(installed['enabled'])
        self.assertEqual(self.request('/custom/status')[1]['packs'][0]['name'], '自制外观包')
        self.assertFalse(self.host['desired_offline']())
        status, mode = self.request('/mode', {'enabled': True})
        self.assertEqual(status, 200)
        self.assertTrue(mode['enabled'])
        self.assertTrue(self.host['desired_offline']())
        status, data = self.request('/offline-data')
        self.assertEqual(status, 200)
        self.assertEqual(len(data['atlas']), len(self.official['atlas']) + 1)
        reference = next(iter(data['customAssetPaths']))
        self.assertEqual(self.request('/' + quote(reference))[1], PNG)
        self.assertEqual(self.request('/custom-assets/' + installed['id'] + '/%2e%2e/manifest.json')[0], 400)
        self.assertEqual(self.request('/custom/remove', {'id': '..'})[0], 409)
        self.assertEqual(self.request('/custom/remove', {'id': installed['id']})[0], 200)
        self.assertTrue(self.request('/status')[1]['enabled'], 'last Mod removal preserves full atlas')
        self.assertTrue(self.host['desired_offline']())
        data = self.request('/offline-data')[1]
        self.assertEqual(data['atlas'], self.official['atlas'])
        self.assertEqual(data['catalog'], self.official['catalog'])
        self.assertEqual(self.request('/mode', {'enabled': False})[0], 200)
        self.assertFalse(self.host['desired_offline']())
        self.assertEqual({p.name: p.read_bytes() for p in (self.root / 'cache').iterdir()}, self.original_cache)


if __name__ == '__main__':
    unittest.main()
