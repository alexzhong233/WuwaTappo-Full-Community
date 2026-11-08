"""Create a data-preserving in-place update ZIP from the built public payload.

原地升级包只带程序本体：账号、缓存、导入的 Mod 和各项设置都在别的文件里，
更新流程一个都不碰。`package_config.json` 里的本机 token 会被剥掉——那是每台
机器自己的凭据，混进发布包等于把维护机的口令发给所有人。
"""
from pathlib import Path
import hashlib
import json
import sys
import zipfile
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'src'))
from desktop_updater import (DIRECTORIES, TOP_LEVEL, installed_version,
                             release_asset_name, validate_manifest)


def payload_bytes(payload, name, config):
    """包内文件内容；公开配置去掉 token 之后不再与安装目录逐字节相同。"""
    if name == 'package_config.json':
        public = {key: value for key, value in config.items() if key != 'token'}
        return json.dumps(public, ensure_ascii=False, indent=4).encode('utf-8')
    return (Path(payload)/name).read_bytes()


def collect(payload):
    """按 desktop_updater 的白名单收集文件：更新包与安装包共用同一套判断。"""
    payload, names = Path(payload), []
    for path in sorted(payload.rglob('*')):
        if not path.is_file():
            continue
        name = path.relative_to(payload).as_posix()
        if name not in TOP_LEVEL and name.split('/')[0] not in DIRECTORIES:
            continue
        names.append(name)
    return names


def make(payload, destination):
    payload, destination = Path(payload), Path(destination)
    config = json.loads((payload/'package_config.json').read_text(encoding='utf-8-sig'))
    files = []
    for name in collect(payload):
        data = payload_bytes(payload, name, config)
        files.append({'path': name, 'size': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    manifest = validate_manifest({'schema': 1, 'version': installed_version(config),
                                  'client_version': config.get('client_version'), 'files': files})
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, 'w', zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        archive.writestr('update_manifest.json', json.dumps(manifest, ensure_ascii=False, indent=2))
        for item in files:
            archive.writestr(item['path'], payload_bytes(payload, item['path'], config))
    return manifest


def main():
    # tools/make_update.py -> windows/src/tools，payload 由 deploy.ps1 装配在 windows/build 下
    project = Path(__file__).resolve().parents[2]
    region = (sys.argv[1] if len(sys.argv) > 1 else 'cn').lower()
    payload = project/('build-global/payload' if region == 'global' else 'build/payload')
    if not (payload/'package_config.json').is_file():
        raise SystemExit('缺少装配产物：%s（先跑 build.ps1 -Region %s）' % (payload, region))
    config = json.loads((payload/'package_config.json').read_text(encoding='utf-8-sig'))
    config.setdefault('region', region)
    version = installed_version(config)
    # 目标目录是仓库根的 dist：package-installer.ps1 的 $OutDir 就指这里，安装包 exe
    # 和升级 zip 必须同目录（CI 也只收 dist/*），否则打包脚本第 315 行的 Test-Path 会红。
    target = project.parent/'dist'/release_asset_name(config, version)
    target.parent.mkdir(parents=True, exist_ok=True)
    manifest = make(payload, target)
    print(json.dumps({'archive': target.name, 'version': manifest['version'],
                      'files': len(manifest['files']),
                      'size': target.stat().st_size}, ensure_ascii=False))


if __name__ == '__main__':
    main()
