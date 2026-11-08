"""Build the public package from a separately supplied official executable."""
from pathlib import Path
import argparse, hashlib, json, os, shutil, subprocess, sys

PROJECT=Path(__file__).resolve().parent
SUPPORTED_BASES={
    'dd340c92a7320d1ecf3697ba65b4ad23d8f196acb547d2baf3f75ba09d696d97':'1.0.1',
    '099b62be14efe2cc99bb0ad3d37860488c42fb0f4e6282b5687e6335ab178573':'1.0.2',
}
MOD_VERSION='1.2'

def copy_if_changed(source, destination):
    source, destination = Path(source), Path(destination)
    if destination.is_file() and source.stat().st_size == destination.stat().st_size:
        with source.open('rb') as first, destination.open('rb') as second:
            if hashlib.file_digest(first, 'sha256').digest() == hashlib.file_digest(second, 'sha256').digest():
                return str(destination)
    return shutil.copy2(source,destination)

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--official',type=Path,required=True)
    ap.add_argument('--cache',type=Path,required=True)
    ap.add_argument('--runtime',type=Path,required=True,help='Extracted official Microsoft Fixed Version x64 runtime')
    ap.add_argument('--node',default='node')
    ap.add_argument('--iscc',type=Path)
    args=ap.parse_args()
    digest=hashlib.sha256(args.official.read_bytes()).hexdigest()
    assert digest in SUPPORTED_BASES,'Unknown official binary: adapter validation is required before replacement'
    build=PROJECT/'build';base=build/('base-'+digest[:16]);payload=build/'payload'
    base.mkdir(parents=True,exist_ok=True);payload.mkdir(exist_ok=True)
    if args.official.resolve() != (base/'WuwaTappo.official.exe').resolve():
        shutil.copy2(args.official,base/'WuwaTappo.official.exe')
    env=os.environ.copy();env['WUWA_EXTRACT_ROOT']=str(base);env['WUWA_BUILD_ROOT']=str(build)
    run=lambda cmd:subprocess.run([str(x) for x in cmd],env=env,check=True,cwd=PROJECT)
    if not (base/'js_structure.json').exists():
        run([sys.executable,PROJECT/'tools/extract_static.py',base/'WuwaTappo.official.exe'])
        shared=next(p.name for p in (base/'readable_js/assets').glob('*.js') if 'function applyCatalog(' in p.read_text(encoding='utf-8'))
        run([args.node,PROJECT.parent.parent/'shared/tools/inspect_js.cjs',base,shared])
    (build/'current_base.txt').write_text(str(base),encoding='utf-8')
    manifest=json.loads((args.cache/'asset_manifest.json').read_text(encoding='utf-8'))
    atlas=json.loads((args.cache/'atlas.json').read_text(encoding='utf-8'))['data']
    assert not manifest['errors']
    for record in manifest['records']:
        assert hashlib.sha256((args.cache/'assets'/record['file']).read_bytes()).hexdigest()==record['sha256']
    config={'port':43229,'asset_count':len(manifest['records']),'atlas_count':len(atlas),'client_version':SUPPORTED_BASES[digest],'mod_version':MOD_VERSION}
    (build/'local_config.json').write_text(json.dumps(config),encoding='utf-8')
    run([sys.executable,PROJECT/'tools/build_integration.py'])
    shutil.copy2(build/'WuwaTappo.Local.v2.exe',payload/'WuwaTappo.Local.exe')
    shutil.copytree(args.cache,payload/'cache',dirs_exist_ok=True)
    assert (args.runtime/'msedgewebview2.exe').is_file()
    shutil.copytree(args.runtime,payload/'runtime',dirs_exist_ok=True,copy_function=copy_if_changed)
    for name in ['README.md','LICENSE','THIRD_PARTY_NOTICES.md']:
        shutil.copy2(PROJECT/name,payload/name)
    shutil.copytree(PROJECT/'docs',payload/'docs',dirs_exist_ok=True)
    licenses=payload/'licenses';licenses.mkdir(exist_ok=True)
    shutil.copy2(Path(sys.base_prefix)/'LICENSE.txt',licenses/'PYTHON_LICENSE.txt')
    import importlib.metadata
    pyinstaller_license=next(p for p in importlib.metadata.distribution('PyInstaller').files if p.name=='COPYING.txt')
    shutil.copy2(importlib.metadata.distribution('PyInstaller').locate_file(pyinstaller_license),licenses/'PYINSTALLER_COPYING.txt')
    psutil_dist=importlib.metadata.distribution('psutil')
    psutil_license=next(p for p in psutil_dist.files if p.name=='LICENSE')
    shutil.copy2(psutil_dist.locate_file(psutil_license),licenses/'PSUTIL_LICENSE.txt')
    # 打字自动表情要读「当前焦点是不是输入框」，用的是 UIAutomation 的只读焦点查询。
    # comtypes 的类型库模块不会被打包器自动收，先让 comtypes 生成 UIAutomationClient
    # 再用 --hidden-import 钉进去。
    # 以前这里是「装不上就 warn 继续打包」：产物照样构建成功，但打字自动表情永远不触发，
    # 界面上只显示「焦点不在输入框」，排查起来像功能没写对。缺依赖必须在构建期炸。
    try:
        from comtypes.client import GetModule
        GetModule('UIAutomationCore.dll')
        comtypes_dist=importlib.metadata.distribution('comtypes')
        comtypes_license=next(p for p in comtypes_dist.files if p.name=='LICENSE.txt')
        shutil.copy2(comtypes_dist.locate_file(comtypes_license),licenses/'COMTYPES_LICENSE.txt')
    except Exception as error:
        sys.exit('打包缺少 comtypes，打字自动表情会静默失效。'
                 '先执行 tools\\setup-env.ps1（或 pip install -r src\\requirements-build.txt）再构建：%s' % error)
    uia=['--hidden-import','comtypes.gen.UIAutomationClient']
    for script in (PROJECT/'installer').glob('*.ps1'):
        # Windows PowerShell 5.1 requires a BOM for Chinese labels.
        (payload/script.name).write_text(script.read_text(encoding='utf-8-sig'),encoding='utf-8-sig')
    shutil.copy2(build/'local_config.json',payload/'package_config.json')
    ssl_data=[]
    import ssl
    ssl_policy=Path(ssl.__file__).with_name('codex-rejected-cas.pem')
    if ssl_policy.is_file():ssl_data=['--add-data',str(ssl_policy)+os.pathsep+'.']
    # 这三个模块是在 try/except 或函数体里导入的（非 Windows、装不上、或只在
    # typing-context 分支用到）。打包器的静态分析虽然能扫到 IMPORT_NAME，但一旦被
    # 优化掉就会变成「右键没菜单、设置里退回官方自启开关、焦点探测恒不可用」，
    # 所以显式钉住，宁可多几 KB。
    runtime=['--hidden-import','native_context_menu','--hidden-import','startup_manager',
             '--hidden-import','text_focus']
    run([sys.executable,'-m','PyInstaller','--noconfirm','--clean','--onedir','--console',*ssl_data,*uia,*runtime,
         '--name','LocalGalleryHost','--distpath',build/'frozen','--workpath',build/'pyinstaller',
         '--specpath',build,'--paths',PROJECT/'src',PROJECT/'src/local_host.py'])
    # 这条不只验 TLS：local_host 的 --self-check 还会确认 UIA 依赖能初始化，
    # 失败即以非零退出（check=True 会让构建中断），见 src/local_host.py。
    run([build/'frozen/LocalGalleryHost/LocalGalleryHost.exe','--self-check'])
    shutil.copytree(build/'frozen/LocalGalleryHost',payload/'host',dirs_exist_ok=True)
    # Keep the frozen service adjacent to the app; its Python DLLs stay in _internal.
    shutil.copy2(payload/'host/LocalGalleryHost.exe',payload/'LocalGalleryHost.exe')
    shutil.copytree(payload/'host/_internal',payload/'_internal',dirs_exist_ok=True)
    if args.iscc:
        run([args.iscc,PROJECT/'installer/setup.iss'])

if __name__=='__main__':main()
