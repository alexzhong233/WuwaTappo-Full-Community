from pathlib import Path
import hashlib
import json
import sys
import tempfile
import unittest
import zipfile
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
from desktop_updater import (FileTransaction, is_prerelease, release_asset_name, safe_name,
                             unpack, validate_manifest, version_tuple, apply_update, Updater)

CONFIG={'community_version':'1.1','region':'cn'}
SHA='SHA256SUMS.txt'
ZIP='WuwaTappo-Full-CN-Community-1.2-win-x64-update.zip'
BASE='https://github.com/alexzhong233/WuwaTappo-Full-Community/releases/download/v1.2/'


class VersionTests(unittest.TestCase):
    def test_prerelease_ordering_matches_semver(self):
        self.assertGreater(version_tuple('1.3.0'),version_tuple('1.3.0-alpha.4'))
        self.assertGreater(version_tuple('1.3.0-alpha.10'),version_tuple('1.3.0-alpha.9'))
        self.assertGreater(version_tuple('1.3.0-alpha.4'),version_tuple('1.2.9'))
        self.assertEqual(version_tuple('1.3'),version_tuple('1.3.0.0'))
        self.assertTrue(is_prerelease('1.3.0-alpha.4'))
        self.assertFalse(is_prerelease('1.3.0'))
    def test_release_names_follow_region(self):
        self.assertEqual(release_asset_name(CONFIG,'1.2'),ZIP)
        self.assertEqual(release_asset_name({'region':'global'},'1.2'),
                         'WuwaTappo-Full-Global-Community-1.2-win-x64-update.zip')
    def test_bad_version_rejected(self):
        for value in ['1','v1.2','1.2.3.4.5','1.2 beta']:
            with self.subTest(value=value),self.assertRaises(ValueError):version_tuple(value)


class UpdaterTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.root=Path(self.temp.name)
        self.contents={'WuwaTappo.Local.exe':b'new-pet','LocalGalleryHost.exe':b'new-host',
                       'package_config.json':b'{"community_version":"1.1","client_version":"1.0.2"}'}
        self.manifest={'schema':1,'version':'1.1','files':[{'path':n,'size':len(v),'sha256':hashlib.sha256(v).hexdigest()} for n,v in self.contents.items()]}
    def tearDown(self):self.temp.cleanup()
    def archive(self,extra=None):
        path=self.root/'update.zip'
        with zipfile.ZipFile(path,'w') as z:
            z.writestr('update_manifest.json',json.dumps(self.manifest))
            for n,v in self.contents.items():z.writestr(n,v)
            if extra:z.writestr(*extra)
        return path
    def test_transaction_preserves_data_and_rolls_back(self):
        target=self.root/'install';target.mkdir()
        for name in ['local_config.json','assistant_state.json','offline_state.json']:(target/name).write_text('private')
        (target/'profile').mkdir();(target/'profile/account').write_text('private-account')
        (target/'WuwaTappo.Local.exe').write_bytes(b'old-pet')
        stage=self.root/'job/stage';unpack(self.archive(),stage)
        t=FileTransaction(target,stage);t.apply()
        self.assertEqual((target/'WuwaTappo.Local.exe').read_bytes(),b'new-pet')
        self.assertEqual((target/'profile/account').read_text(),'private-account')
        self.assertEqual((target/'local_config.json').read_text(),'private')
        t.rollback()
        self.assertEqual((target/'WuwaTappo.Local.exe').read_bytes(),b'old-pet')
        self.assertFalse((target/'LocalGalleryHost.exe').exists())
        self.assertEqual((target/'assistant_state.json').read_text(),'private')
    def test_tampered_stage_never_changes_installation(self):
        target=self.root/'install';target.mkdir();(target/'WuwaTappo.Local.exe').write_bytes(b'old')
        stage=self.root/'job/stage';unpack(self.archive(),stage)
        (stage/'package_config.json').write_text('tamper')
        with self.assertRaises(ValueError):FileTransaction(target,stage).apply()
        self.assertEqual((target/'WuwaTappo.Local.exe').read_bytes(),b'old')
    def test_invalid_zip_hash_and_extra_entries(self):
        with self.assertRaises(ValueError):unpack(self.archive(('profile/password',b'invalid')),self.root/'extra')
        self.contents['WuwaTappo.Local.exe']=b'fake'
        with self.assertRaises(ValueError):unpack(self.archive(),self.root/'hash')
    def test_data_paths_and_traversal_denied(self):
        for name in ['../a','/a','C:/a','_internal/../../x','profile/account','cache/atlas.json','local_config.json','assistant_state.json','custom-atlas/item.json','runtime/msedgewebview2.exe','_internal/a:stream','_internal\\a','launch.ps1']:
            with self.subTest(name=name),self.assertRaises(ValueError):safe_name(name)
    def test_duplicate_case_collision_denied(self):
        self.manifest['files'].append({**self.manifest['files'][0],'path':'WUWATAPPO.LOCAL.EXE'})
        with self.assertRaises(ValueError):validate_manifest(self.manifest)
    def test_api_quota_uses_public_release(self):
        updater=Updater(self.root,CONFIG)
        with patch.object(updater,'fetch',side_effect=OSError('API quota')),patch.object(updater,'public_release',return_value={'tag_name':'v1.0','assets':[]}) as fallback:
            self.assertFalse(updater.check()['available'])
            fallback.assert_called_once()
    def release(self, version='1.2', draft=False, prerelease=False, complete=True):
        base=BASE.replace('v1.2','v'+version)
        name=ZIP.replace('1.2',version)
        return {'tag_name':'v'+version,'draft':draft,'prerelease':prerelease,
                'html_url':base.replace('/releases/download/','/releases/tag/'),
                'assets':[{'name':name,'browser_download_url':base+name,'size':1234},
                          {'name':SHA,'browser_download_url':base+SHA}] if complete else []}
    def check_release(self,updater,release):
        name=next((a['name'] for a in release['assets'] if a['name']!=SHA),ZIP)
        sums=('a'*64+'  '+name+'\n').encode()
        with patch.object(updater,'fetch',side_effect=[json.dumps([release]).encode(),sums]):
            return updater.check()
    def test_formal_release_is_an_optional_update(self):
        updater=Updater(self.root,CONFIG)
        result=self.check_release(updater,self.release())
        self.assertTrue(result['available'])
        self.assertTrue(result['installable'])
        # 可选更新：前端不会拿它挡住开箱等原有功能
        self.assertFalse(result['required'])
        self.assertTrue(Updater(self.root,CONFIG).status()['installable'])
        self.assertFalse(Updater(self.root,{'community_version':'1.2','region':'cn'}).status()['available'])
    def test_draft_and_older_release_ignored(self):
        for release in [self.release(draft=True),self.release(version='1.1')]:
            self.assertFalse(self.check_release(Updater(self.root,CONFIG),release)['available'])
    def test_prerelease_needs_opt_in(self):
        release=self.release(version='1.1.5-alpha.1',prerelease=True)
        self.assertFalse(self.check_release(Updater(self.root,CONFIG),release)['available'])
        allowed=Updater(self.root,CONFIG,allow_prerelease=lambda:True)
        self.assertTrue(self.check_release(allowed,release)['available'])
    def test_release_without_zip_falls_back_to_github(self):
        updater=Updater(self.root,CONFIG)
        result=self.check_release(updater,self.release(complete=False))
        self.assertFalse(result['available'])
        self.assertFalse(result['installable'])
        self.assertIn('1.2',result['message'])
        self.assertFalse(updater.status()['available'])
    def test_bad_checksum_never_creates_candidate(self):
        updater=Updater(self.root,CONFIG)
        with patch.object(updater,'fetch',side_effect=[json.dumps([self.release()]).encode(),b'invalid']):
            with self.assertRaises(ValueError):updater.check()
        self.assertFalse(updater.status()['available'])
    def test_health_ack_uses_installed_version(self):
        updater=Updater(self.root,CONFIG)
        folder=self.root/'.updates/job';folder.mkdir(parents=True)
        (folder/'pending.json').write_text('{"version":"1.1","id":"job"}')
        self.assertTrue(updater.healthy('1.2')['ok'])
        self.assertTrue((folder/'healthy').exists())
    def helper_fixture(self):
        target=self.root/'install';target.mkdir()
        (target/'WuwaTappo.Local.exe').write_bytes(b'old-pet')
        (target/'local_config.json').write_text('private')
        folder=target/'.updates/job';stage=folder/'stage'
        unpack(self.archive(),stage)
        (folder/'pending.json').write_text('{"version":"1.1"}')
        return target,folder,stage
    def test_helper_health_ack_cleans_backup(self):
        target,folder,stage=self.helper_fixture()
        with patch('desktop_updater.processes',return_value=[]),patch('desktop_updater.launch',side_effect=lambda root:(folder/'healthy').write_text('1.1')):
            apply_update(target,stage)
        self.assertTrue(json.loads((folder/'result.json').read_text())['ok'])
        self.assertFalse((folder/'backup').exists())
        self.assertFalse(stage.exists())
        self.assertEqual((target/'local_config.json').read_text(),'private')
    def test_helper_startup_timeout_restores_old_executable(self):
        target,folder,stage=self.helper_fixture()
        from unittest.mock import MagicMock
        # 前五个 0 给「写进度/等退出/替换/启动」这几步，之后跳到 91 让启动检查超时。
        with patch('desktop_updater.processes',return_value=[]),patch('desktop_updater.launch') as launch,patch('desktop_updater.psutil',MagicMock()),patch('desktop_updater.time.monotonic',side_effect=[0,0,0,0,0,91,91,91,91]):
            apply_update(target,stage)
        self.assertFalse(json.loads((folder/'result.json').read_text())['ok'])
        self.assertEqual((target/'WuwaTappo.Local.exe').read_bytes(),b'old-pet')
        self.assertEqual(launch.call_count,2)
        self.assertEqual((target/'local_config.json').read_text(),'private')


if __name__=='__main__':unittest.main()
