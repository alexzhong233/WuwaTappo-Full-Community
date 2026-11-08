"""Validate and manage user supplied appearance Mod packs.

The official offline catalog is never modified on disk. A Mod is a small
manifest/items/assets bundle; numeric IDs and catalog bridges are generated
only when the official catalog is loaded.
"""
from pathlib import Path
import copy
import json
import re
import shutil
import stat
import tempfile
import unicodedata
import zipfile

MAX_ARCHIVE = 512 * 1024 * 1024
MAX_EXPANDED = 1024 * 1024 * 1024
MAX_ENTRIES = 20000
MAX_JSON = 16 * 1024 * 1024
MAX_ITEMS = 5000
IMAGE_SIGNATURES = {
    '.png': (b'\x89PNG\r\n\x1a\n',),
    '.jpg': (b'\xff\xd8\xff',),
    '.jpeg': (b'\xff\xd8\xff',),
    '.webp': (b'RIFF',),
    '.gif': (b'GIF87a', b'GIF89a'),
}
PACK_ID = re.compile(r'^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')
MIN_GENERATED_ID = 1_000_000_000
MAX_GENERATED_ID = 9_007_199_254_740_991


def _json_bytes(value, name):
    if len(value) > MAX_JSON:
        raise ValueError('%s 超过 16 MiB' % name)
    try:
        return json.loads(value.decode('utf-8'))
    except (UnicodeDecodeError, ValueError) as error:
        raise ValueError('%s 不是有效 UTF-8 JSON' % name) from error


def _read_json(path, name):
    try:
        return _json_bytes(path.read_bytes(), name)
    except OSError as error:
        raise ValueError('自定义资源包缺少有效的 %s' % name) from error


def _asset_path(value):
    if not isinstance(value, str) or not value or '\\' in value or value.startswith('/') or any(c in value for c in ('?', '#', ':')) or any(ord(c) < 32 for c in value):
        raise ValueError('资源 URL 必须是包内相对路径')
    relative = value
    parts = relative.split('/')
    if not relative or any(part in ('', '.', '..') or ':' in part for part in parts):
        raise ValueError('资源路径包含不允许的目录段')
    if not relative.startswith('assets/'):
        relative = 'assets/' + relative
    return relative


def _text(value, label, pattern=None, max_length=160):
    if not isinstance(value, str):
        raise ValueError('%s 无效' % label)
    value = value.strip()
    if not value or len(value) > max_length or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ValueError('%s 无效' % label)
    if pattern and not pattern.fullmatch(value):
        raise ValueError('%s 只能使用字母、数字、点、下划线和短横线' % label)
    return value.strip()


def _hash(value):
    number = 14695981039346656037
    for byte in value.encode('utf-8'):
        number = ((number ^ byte) * 1099511628211) & 0xFFFFFFFFFFFFFFFF
    return number


def _image_paths(item):
    assets = item.get('assets')
    if not isinstance(assets, dict):
        raise ValueError('资源条目缺少 assets')
    kind = item['type']
    required = {'character': ('body1', 'head1'), 'accessory': ('image',), 'emote': ('image',)}[kind]
    for name in required:
        _asset_path(assets.get(name))
    for name, path in assets.items():
        if name not in ('icon', 'preview', 'image', 'body1', 'head1', 'body2', 'head2', 'body3'):
            raise ValueError('资源条目包含未知图片字段：%s' % name)
        _asset_path(path)
    return {_asset_path(path) for path in assets.values()}


def inspect_bundle(root, region):
    root = Path(root)
    manifest = _read_json(root / 'manifest.json', 'manifest.json')
    items_doc = _read_json(root / 'items.json', 'items.json')
    if not isinstance(manifest, dict) or manifest.get('schemaVersion') != 1:
        raise ValueError('资源包 schemaVersion 必须是 1')
    name = _text(manifest.get('name'), '资源包名称', max_length=80)
    pack_id = 'mod-%016x' % _hash(name)
    pack_region = manifest.get('region', 'ANY')
    if pack_region not in ('ANY', 'CN', 'GLOBAL') or pack_region not in ('ANY', region):
        raise ValueError('资源包属于 %s，当前客户端是 %s' % (pack_region, region))
    if not isinstance(items_doc, dict) or not isinstance(items_doc.get('items'), list):
        raise ValueError('items.json 必须包含 items 数组')
    items = []
    seen = set()
    urls = set()
    for raw in items_doc['items']:
        if not isinstance(raw, dict):
            raise ValueError('资源条目必须是对象')
        kind = raw.get('type')
        if kind not in ('character', 'accessory', 'emote'):
            raise ValueError('资源条目 type 必须是 character、accessory 或 emote')
        item_name = _text(raw.get('name'), '资源名称', max_length=180)
        identity = (kind, item_name)
        if identity in seen:
            raise ValueError('同一资源包内存在同类型同名资源：%s' % item_name)
        seen.add(identity)
        item_id = 'item-%016x' % _hash(kind + '\0' + item_name)
        if not isinstance(raw.get('assets'), dict):
            raise ValueError('资源条目缺少 assets')
        item = {'id': item_id, 'type': kind, 'name': item_name, 'assets': dict(raw['assets'])}
        urls.update(_image_paths(item))
        items.append(item)
    if not items or len(items) > MAX_ITEMS:
        raise ValueError('资源包必须包含 1 到 %s 条资源' % MAX_ITEMS)
    asset_files = {file.relative_to(root).as_posix() for file in (root / 'assets').rglob('*') if file.is_file()}
    missing = urls - asset_files
    if missing:
        raise ValueError('资源包缺少图片：%s' % sorted(missing)[0])
    return {'id': pack_id, 'name': name, 'region': pack_region, 'items': items, '_root': str(root.resolve()),
            'asset_count': len(urls), 'asset_paths': sorted(urls), 'item_count': len(items)}


def _used_ids(official):
    used = set()
    catalog = (official or {}).get('catalog') or {}
    for values in catalog.values():
        if isinstance(values, dict):
            for key in values:
                try:
                    used.add(int(key))
                except (TypeError, ValueError):
                    pass
    for row in (official or {}).get('atlas', []) or []:
        try:
            used.add(int(row.get('asset_id')))
        except (TypeError, ValueError, AttributeError):
            pass
    return used


def _generated_id(pack_id, kind, item_id, used):
    seed = '%s\0%s\0%s' % (pack_id, kind, item_id)
    number = _hash(seed)
    candidate = MIN_GENERATED_ID + (number % (MAX_GENERATED_ID - MIN_GENERATED_ID + 1))
    while candidate in used:
        candidate += 1
        if candidate > MAX_GENERATED_ID:
            candidate = MIN_GENERATED_ID
    used.add(candidate)
    return str(candidate)


def _asset_ref(pack, relative):
    return 'custom-assets/%s/%s' % (pack['id'], relative.removeprefix('assets/'))


def materialize(pack, used):
    catalog = {kind: {} for kind in ('character_skin', 'character_skin_atlas', 'accessory', 'accessory_atlas', 'emote', 'emote_atlas')}
    atlas = []
    paths = {}
    for item in sorted(pack['items'], key=lambda row: row['id']):
        kind = item['type']
        appearance_id = _generated_id(pack['id'], kind, item['id'], used)
        atlas_kind = 'character_skin_atlas' if kind == 'character' else kind + '_atlas'
        atlas_id = _generated_id(pack['id'], atlas_kind, item['id'], used)
        assets = item['assets']
        for value in assets.values():
            relative = _asset_path(value)
            paths[_asset_ref(pack, relative)] = str(Path(pack['_root']) / relative)
        catalog_kind = 'character_skin' if kind == 'character' else kind
        if kind == 'character':
            primary = assets.get('preview') or assets.get('icon') or assets['body1']
            entry = {'game_role_id': 0, 'url': _asset_ref(pack, _asset_path(primary)),
                     'offset_x': 0, 'offset_y': 0, 'my_offset_x': 0, 'my_offset_y': 0}
            for layer in ('body1', 'head1', 'body2', 'head2', 'body3'):
                if layer in assets:
                    entry[layer] = {'url': _asset_ref(pack, _asset_path(assets[layer])), 'offset_x': 0, 'offset_y': 0, 'my_offset_x': 0, 'my_offset_y': 0}
            # 渲染器切换动作帧时不会回退：缺帧会拿到空 URL，那一瞬间整层消失
            # （表现为每 5 秒眨眼时头部闪一下）。用基础帧补齐，等价于动作时保持静止。
            for action, base in (('body2', 'body1'), ('body3', 'body1'), ('head2', 'head1')):
                if action not in entry and base in entry:
                    entry[action] = dict(entry[base])
            atlas_url = _asset_ref(pack, _asset_path(assets.get('icon') or primary))
        else:
            primary = assets['image']
            entry = {'game_role_id': 0, 'url': _asset_ref(pack, _asset_path(primary)),
                     'offset_x': 0, 'offset_y': 0, 'my_offset_x': 0, 'my_offset_y': 0}
            atlas_url = _asset_ref(pack, _asset_path(assets.get('icon') or primary))
        catalog[catalog_kind][appearance_id] = entry
        atlas_kind = catalog_kind + '_atlas'
        catalog[atlas_kind][atlas_id] = {'height_light_id': int(appearance_id), 'game_role_id': 0,
                                         'url': atlas_url, 'offset_x': 0, 'offset_y': 0,
                                         'my_offset_x': 0, 'my_offset_y': 0}
        atlas.append({'asset_id': int(atlas_id), 'asset_name': item['name'], 'display_name': item['name'],
                      'asset_code': 'mod:%s:%s' % (pack['id'], item['id']),
                      'asset_type': atlas_kind, 'star_level': 0, 'region_code': '',
                      'series_name': pack['name'], 'rarity': 'MOD',
                      'custom_pack_id': pack['id'], 'custom_item_id': item['id']})
    return {'atlas': atlas, 'catalog': catalog, 'customAssetPaths': paths,
            'custom_resources': {'id': pack['id'], 'name': pack['name'], 'region': pack['region']}}


def merge_official(official, packs):
    if not isinstance(official, dict) or not isinstance(official.get('atlas'), list) or not isinstance(official.get('catalog'), dict):
        raise ValueError('官方图鉴缓存不完整，无法叠加 Mod')
    merged = copy.deepcopy(official)
    pack_meta = []
    all_paths = {}
    if not packs:
        # 没有 Mod 时原样透传：Mod 层不该成为基础功能的失败点。
        merged['atlas_count'] = len(merged['atlas'])
        return merged
    used = _used_ids(official)
    for pack in sorted(packs, key=lambda row: row['id']):
        addon = materialize(pack, used)
        merged['atlas'].extend(addon['atlas'])
        for kind, values in addon['catalog'].items():
            if not isinstance(merged['catalog'].get(kind), dict):
                raise ValueError('官方目录缺少 %s' % kind)
            merged['catalog'][kind].update(values)
        all_paths.update(addon['customAssetPaths'])
        pack_meta.append({'id': pack['id'], 'name': pack['name'], 'region': pack['region'],
                          'item_count': pack['item_count'], 'asset_count': pack['asset_count']})
    merged['customAssetPaths'] = all_paths
    merged['custom_resources'] = {'packs': pack_meta}
    merged['atlas_count'] = len(merged['atlas'])
    return merged


def list_packs(root, region):
    root = Path(root)
    packs = []
    if not root.exists():
        return packs
    for path in sorted(root.iterdir(), key=lambda item: item.name):
        if not path.is_dir() or path.name.startswith('.'):
            continue
        try:
            packs.append(inspect_bundle(path, region))
        except (OSError, ValueError):
            continue
    return packs


def list_details(root, region):
    """管理面板用：包内条目明细 + 可直接渲染的缩略图引用。"""
    packs = []
    for pack in list_packs(root, region):
        items = []
        for item in pack['items']:
            assets = item['assets']
            primary = (assets.get('icon') or assets.get('preview') or assets.get('image')
                       or assets.get('body1') or 'assets/missing.png')
            items.append({'id': item['id'], 'name': item['name'], 'type': item['type'],
                          'icon': _asset_ref(pack, _asset_path(primary))})
        packs.append({'id': pack['id'], 'name': pack['name'], 'region': pack['region'],
                      'item_count': pack['item_count'], 'asset_count': pack['asset_count'],
                      'items': items})
    return packs


def remove_pack(root, pack_id):
    pack_id = _text(pack_id, 'Mod ID', PACK_ID, 64)
    target = Path(root).resolve() / pack_id
    if not target.is_dir() or target.parent != Path(root).resolve():
        raise ValueError('未找到该 Mod')
    shutil.rmtree(target)


def _safe_zip_name(name):
    if not name or name.startswith('/') or '\\' in name:
        raise ValueError('ZIP 中包含不安全路径')
    parts = name[:-1].split('/') if name.endswith('/') else name.split('/')
    if any(part in ('', '.', '..') or part.endswith((' ', '.'))
           or any(c in part for c in '<>:"|?*') or any(ord(c) < 32 for c in part)
           or re.fullmatch(r'(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?', part, re.I)
           for part in parts):
        raise ValueError('ZIP 中包含不安全路径')
    return name[:-1] if name.endswith('/') else name


def install_archive(archive_path, destination, region):
    archive_path = Path(archive_path)
    if not archive_path.is_absolute() or archive_path.suffix.lower() != '.zip' or not archive_path.is_file():
        raise ValueError('请选择有效的 ZIP 资源包')
    if archive_path.stat().st_size > MAX_ARCHIVE:
        raise ValueError('ZIP 资源包不能超过 512 MiB')
    destination = Path(destination).resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix='.custom-mod-', dir=str(destination)))
    expanded = 0
    seen = set()
    try:
        with zipfile.ZipFile(str(archive_path), 'r') as archive:
            if len(archive.infolist()) > MAX_ENTRIES:
                raise ValueError('ZIP 文件数量超过限制')
            for info in archive.infolist():
                name = _safe_zip_name(info.filename)
                key = unicodedata.normalize('NFC', name).casefold()
                if key in seen:
                    raise ValueError('ZIP 中存在重复文件路径')
                seen.add(key)
                mode = info.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise ValueError('ZIP 不允许包含符号链接')
                if info.is_dir():
                    continue
                if name.startswith('__MACOSX/') or name.rsplit('/', 1)[-1] == '.DS_Store':
                    continue
                if name not in ('manifest.json', 'items.json') and not name.startswith('assets/'):
                    raise ValueError('ZIP 包含未知文件：%s' % name)
                if name in ('manifest.json', 'items.json') and info.file_size > MAX_JSON:
                    raise ValueError('%s 超过 16 MiB' % name)
                expanded += info.file_size
                if expanded > MAX_EXPANDED:
                    raise ValueError('ZIP 解压后不能超过 1 GiB')
                target = stage.joinpath(*name.split('/'))
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(info, 'r') as source:
                    first = source.read(16)
                    if name.startswith('assets/'):
                        signature = IMAGE_SIGNATURES.get(target.suffix.lower())
                        valid = bool(signature and any(first.startswith(item) for item in signature))
                        if target.suffix.lower() == '.webp':
                            valid = valid and first[8:12] == b'WEBP'
                        if not valid:
                            raise ValueError('资源文件不是受支持的图片格式：%s' % name)
                    with target.open('wb') as output:
                        output.write(first)
                        shutil.copyfileobj(source, output, 1024 * 1024)
        pack = inspect_bundle(stage, region)
        target = destination / pack['id']
        backup = target.with_name('.%s.previous' % target.name)
        if backup.exists():
            shutil.rmtree(backup)
        if target.exists():
            target.rename(backup)
        try:
            stage.rename(target)
        except Exception:
            if backup.exists():
                backup.rename(target)
            raise
        if backup.exists():
            shutil.rmtree(backup)
        return {key: pack[key] for key in ('id', 'name', 'item_count', 'asset_count')}
    finally:
        if stage.exists():
            shutil.rmtree(stage, ignore_errors=True)
