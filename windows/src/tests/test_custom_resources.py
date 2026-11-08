import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
import custom_resources as resources


PNG = b'\x89PNG\r\n\x1a\nfixture'
KINDS = ('character_skin', 'character_skin_atlas', 'accessory',
         'accessory_atlas', 'emote', 'emote_atlas')


def fixture_items():
    return [
        {'type': 'character', 'name': '自制角色', 'assets': {
            'icon': 'assets/character/icon.png', 'body1': 'assets/character/body.png',
            'head1': 'assets/character/head.png'}},
        {'type': 'accessory', 'name': '猫耳挂饰', 'assets': {
            'icon': 'assets/accessory/icon.png', 'image': 'assets/accessory/image.png'}},
        {'type': 'emote', 'name': '开心表情', 'assets': {
            'icon': 'assets/emote/icon.png', 'image': 'assets/emote/image.png'}},
    ]


def write_pack(root, pack_id, items=None, region='ANY'):
    root.mkdir(parents=True, exist_ok=True)
    (root / 'manifest.json').write_text(json.dumps({
        'schemaVersion': 1, 'name': pack_id, 'region': region,
    }), encoding='utf-8')
    items = fixture_items() if items is None else items
    (root / 'items.json').write_text(json.dumps({'items': items}), encoding='utf-8')
    for item in items:
        for value in item['assets'].values():
            asset = root / value
            asset.parent.mkdir(parents=True, exist_ok=True)
            asset.write_bytes(PNG)


def empty_official():
    catalog = {kind: {} for kind in KINDS}
    catalog['character_skin']['42'] = {'url': 'assets/official.png'}
    catalog['character_skin_atlas']['43'] = {'height_light_id': 42}
    return {'atlas': [{'asset_id': 43, 'asset_type': 'character_skin_atlas',
                       'asset_name': 'official.png', 'display_name': 'Official'}],
            'catalog': catalog, 'assets_host': 'official-host', 'asset_count': 1}


class CustomResourcesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_multiple_mods_overlay_without_mutating_official_catalog(self):
        official = empty_official()
        before = json.dumps(official, sort_keys=True)
        first = self.root / 'first'
        second = self.root / 'second'
        write_pack(first, 'creator-one')
        write_pack(second, 'creator-two', [fixture_items()[1]])

        merged = resources.merge_official(official, [
            resources.inspect_bundle(first, 'CN'), resources.inspect_bundle(second, 'CN')])

        self.assertEqual(json.dumps(official, sort_keys=True), before)
        self.assertEqual(len(merged['atlas']), 5)
        self.assertEqual(merged['atlas'][0], official['atlas'][0])
        self.assertEqual(len(merged['catalog']['character_skin']), 2)
        self.assertEqual(len(merged['catalog']['accessory']), 2)
        self.assertEqual(len(merged['catalog']['emote']), 1)
        custom_ids = [row['asset_id'] for row in merged['atlas'][1:]]
        self.assertTrue(all(asset_id >= resources.MIN_GENERATED_ID for asset_id in custom_ids))
        self.assertTrue(set(custom_ids).isdisjoint({42, 43}))
        self.assertEqual(len({row['asset_code'] for row in merged['atlas'][1:]}), 4)
        # 背包用它当缩略图容器 class（atlas-thumb--mod），也是唯一的 Mod 视觉标记。
        self.assertEqual({row['rarity'] for row in merged['atlas'][1:]}, {'MOD'})

    def test_ids_are_stable_across_loads_and_pack_order(self):
        one, two = self.root / 'one', self.root / 'two'
        write_pack(one, 'z-pack')
        write_pack(two, 'a-pack')
        official = empty_official()
        packs = [resources.inspect_bundle(one, 'CN'), resources.inspect_bundle(two, 'CN')]
        a = resources.merge_official(official, packs)
        b = resources.merge_official(official, list(reversed(packs)))
        ids = lambda value: sorted((row['asset_code'], row['asset_id']) for row in value['atlas'] if 'asset_code' in row)
        self.assertEqual(ids(a), ids(b))

    def test_duplicate_names_and_wrong_region_are_rejected(self):
        duplicate = self.root / 'duplicate'
        item = fixture_items()[0]
        write_pack(duplicate, 'duplicate', [item, item])
        with self.assertRaisesRegex(ValueError, '同类型同名资源'):
            resources.inspect_bundle(duplicate, 'CN')
        other_region = self.root / 'global'
        write_pack(other_region, 'global-pack', [fixture_items()[0]], 'GLOBAL')
        with self.assertRaisesRegex(ValueError, '当前客户端'):
            resources.inspect_bundle(other_region, 'CN')

    def test_asset_path_traversal_and_non_image_are_rejected(self):
        with self.assertRaises(ValueError):
            resources._asset_path('assets/../../outside.png')
        for value in ('assets/.. /outside.png', 'assets/CON.png', 'assets/nul', 'assets/image.png.', 'assets/a\x00.png'):
            with self.assertRaises(ValueError):
                resources._safe_zip_name(value)
        archive = self.root / 'bad.zip'
        with zipfile.ZipFile(archive, 'w') as bundle:
            bundle.writestr('manifest.json', json.dumps({'schemaVersion': 1, 'id': 'bad', 'name': 'bad'}))
            bundle.writestr('items.json', json.dumps({'items': fixture_items()}))
            bundle.writestr('../escape.txt', b'bad')
        with self.assertRaisesRegex(ValueError, '不安全路径'):
            resources.install_archive(archive, self.root / 'installed', 'CN')

        bad_image = self.root / 'bad-image.zip'
        with zipfile.ZipFile(bad_image, 'w') as bundle:
            bundle.writestr('manifest.json', json.dumps({'schemaVersion': 1, 'id': 'bad-image', 'name': 'bad'}))
            bundle.writestr('items.json', json.dumps({'items': fixture_items()}))
            for item in fixture_items():
                for value in item['assets'].values():
                    bundle.writestr(value, b'not an image')
        with self.assertRaisesRegex(ValueError, '不是受支持的图片'):
            resources.install_archive(bad_image, self.root / 'installed', 'CN')

    def test_install_replace_and_remove_only_target_mod(self):
        source = self.root / 'source'
        write_pack(source, 'my-mod', [fixture_items()[1]])
        archive = self.root / 'my-mod.zip'
        with zipfile.ZipFile(archive, 'w') as bundle:
            for path in source.rglob('*'):
                if path.is_file():
                    bundle.write(path, path.relative_to(source).as_posix())
        destination = self.root / 'mods'
        result = resources.install_archive(archive, destination, 'CN')
        pack_id = result['id']
        self.assertTrue(pack_id.startswith('mod-'))
        self.assertEqual(resources.list_packs(destination, 'CN')[0]['id'], pack_id)
        second = self.root / 'another'
        write_pack(second, 'another-mod')
        other = resources.inspect_bundle(second, 'CN')
        second.rename(destination / other['id'])
        result2 = resources.install_archive(archive, destination, 'CN')
        self.assertEqual(result2['id'], pack_id)
        self.assertEqual(len(resources.list_packs(destination, 'CN')), 2)
        resources.remove_pack(destination, other['id'])
        resources.remove_pack(destination, pack_id)
        self.assertEqual(resources.list_packs(destination, 'CN'), [])

    def test_app_assigns_ids_and_keeps_them_after_image_or_order_changes(self):
        source = self.root / 'source'
        write_pack(source, '自制包')
        pack = resources.inspect_bundle(source, 'CN')
        original = resources.merge_official(empty_official(), [pack])
        write_pack(source, '自制包', list(reversed(fixture_items())))
        changed = resources.inspect_bundle(source, 'CN')
        self.assertEqual(pack['id'], changed['id'])
        after = resources.merge_official(empty_official(), [changed])
        self.assertEqual(original['atlas'], after['atlas'])
        self.assertEqual(resources._generated_id('pack', 'character', 'hero', set()), '1407742458454113')
        self.assertEqual(resources._hash('自制包'), 0x8A4775467A40FE51)
        self.assertEqual(resources._hash('hello'), 0xA430D84680AABD0B)

    def test_failed_import_preserves_existing_pack_and_official_catalog(self):
        source = self.root / 'source'
        write_pack(source, 'same-name')
        archive = self.root / 'good.zip'
        with zipfile.ZipFile(archive, 'w') as bundle:
            for path in source.rglob('*'):
                if path.is_file():
                    bundle.write(path, path.relative_to(source).as_posix())
            bundle.writestr('__MACOSX/._manifest.json', b'metadata')
            bundle.writestr('.DS_Store', b'metadata')
        destination = self.root / 'mods'
        result = resources.install_archive(archive, destination, 'CN')
        before = resources.merge_official(empty_official(), resources.list_packs(destination, 'CN'))
        bad = self.root / 'broken.zip'
        with zipfile.ZipFile(bad, 'w') as bundle:
            bundle.writestr('manifest.json', json.dumps({'schemaVersion': 1, 'name': 'same-name'}))
            bundle.writestr('items.json', json.dumps({'items': fixture_items()}))
        with self.assertRaisesRegex(ValueError, '缺少图片'):
            resources.install_archive(bad, destination, 'CN')
        self.assertTrue((destination / result['id']).is_dir())
        self.assertEqual(before, resources.merge_official(empty_official(), resources.list_packs(destination, 'CN')))

    def test_unsafe_removal_and_missing_official_mapping_are_rejected(self):
        for value in ('..', '.', '../other', 'a/b', '-bad'):
            with self.assertRaises(ValueError):
                resources.remove_pack(self.root, value)
        pack_root = self.root / 'source'
        write_pack(pack_root, 'example')
        official = empty_official()
        del official['catalog']['emote']
        with self.assertRaises(ValueError):
            resources.merge_official(official, [resources.inspect_bundle(pack_root, 'CN')])

    def test_no_mods_is_a_pure_passthrough_even_for_odd_official_data(self):
        # 没有装 Mod 时，Mod 层不该成为基础功能的失败点：官方目录缺分类也照原样返回。
        official = empty_official()
        del official['catalog']['emote']
        passthrough = resources.merge_official(official, [])
        self.assertEqual(passthrough['catalog'], official['catalog'])
        self.assertEqual(passthrough['atlas_count'], 1)
        self.assertNotIn('customAssetPaths', passthrough)
        self.assertNotIn('custom_resources', passthrough)

    def test_details_expose_icon_references_for_the_management_panel(self):
        write_pack(self.root / 'mod-id', 'creator')
        details = resources.list_details(self.root, 'CN')
        self.assertEqual(len(details), 1)
        pack = details[0]
        self.assertEqual(pack['item_count'], len(pack['items']))
        self.assertGreaterEqual(pack['asset_count'], pack['item_count'])
        for item in pack['items']:
            self.assertIn(item['type'], ('character', 'accessory', 'emote'))
            self.assertTrue(item['icon'].startswith('custom-assets/%s/' % pack['id']), item['icon'])
            self.assertNotIn('..', item['icon'])
        character = next(item for item in pack['items'] if item['type'] == 'character')
        self.assertTrue(character['icon'].endswith('.png'))

    def test_missing_action_frames_fall_back_to_the_base_layer(self):
        # 只给 body1/head1 的角色也要能切换动作帧：渲染器缺帧时会拿到空 URL，
        # 表现为每 5 秒眨眼时头部闪一下、拖动时身体消失。
        write_pack(self.root / 'minimal', 'minimal')
        merged = resources.merge_official(
            empty_official(), [resources.inspect_bundle(self.root / 'minimal', 'CN')])
        hero = merged['catalog']['character_skin'][str(next(
            int(key) for key in merged['catalog']['character_skin'] if int(key) >= resources.MIN_GENERATED_ID))]
        for action in ('body2', 'body3'):
            self.assertEqual(hero[action]['url'], hero['body1']['url'], action)
        self.assertEqual(hero['head2']['url'], hero['head1']['url'])

    def test_supplied_action_frames_are_kept(self):
        items = [{'type': 'character', 'name': '会动', 'assets': {
            'body1': 'assets/character/body.png', 'head1': 'assets/character/head.png',
            'body2': 'assets/character/body_2.png', 'head2': 'assets/character/head_2.png'}}]
        write_pack(self.root / 'animated', 'animated', items)
        merged = resources.merge_official(
            empty_official(), [resources.inspect_bundle(self.root / 'animated', 'CN')])
        hero = next(value for value in merged['catalog']['character_skin'].values()
                    if 'body1' in value)
        self.assertIn('body_2', hero['body2']['url'])
        self.assertIn('head_2', hero['head2']['url'])
        self.assertEqual(hero['body3']['url'], hero['body1']['url'], '没给 body3 时回退到 body1')


if __name__ == '__main__':
    unittest.main()
