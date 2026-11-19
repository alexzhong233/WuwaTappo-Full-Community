"""Apply the local-mode integration to the versioned official snapshot.

The shared anchor pipeline lives in ``shared/tools/fullatlas_core.py``. This
file only supplies Windows-specific transport options and PE resource packing.
"""
from pathlib import Path
import base64
import hashlib
import json
import re
import struct
import sys

import brotli
import pefile

PROJECT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT.parent.parent / 'shared' / 'tools'))
from fullatlas_core import FullAtlasHooks, integrate_fullatlas
from inject_core import patch_log, replace
from integrate_assistant import integrate


class WindowsHooks(FullAtlasHooks):
    controller_path = 'src/local_atlas_controller.js'
    mod_name_first = True
    startup_error_prefix = '本地素材服务没有启动，请用桌面快捷方式重新打开；仍不行请查看安装目录下的 host.stderr.log。'

    def transport_options(self, config):
        origin = json.dumps(f'http://127.0.0.1:{config["port"]}')
        return (
            f'origin: {origin}, token: null, '
            f'customAssetUrl: reference => {origin} + "/" + reference.split("/").map(encodeURIComponent).join("/")'
        )

    def finalize(self, context):
        # Reuse the independently checked PE resource packer and private-runtime stub.
        project, root, base = context['project'], context['root'], context['base']
        packer = (project / 'tools/pe_packer.py').read_text(encoding='utf-8').split('# Add a read-only PE section', 1)[1]
        packer = '# Add a read-only PE section' + packer
        packer = packer.replace("source = Path(r'D:\\WuwaTappo\\WuwaTappo.exe')", "source = BASE / 'WuwaTappo.official.exe'")
        packer = packer.replace("ROOT / 'WuwaTappo.Local.exe'", "ROOT / 'WuwaTappo.Local.v2.exe'")
        packer = packer.replace("ROOT / 'build_report.json'", "ROOT / 'build_report_v2.json'")
        namespace = {**globals(), 'BASE': base, 'ROOT': root, 'ORIGINAL': base,
                     'frontend': context['frontend'], 'shared_name': context['shared_name'], 'main_name': context['main_name'],
                     'panel_name': context['panel_name'], 'extra_patched_assets': context['extra_patched_assets']}
        missing = [name for name in ('hashlib', 'json', 'base64', 'struct', 'patch_log', 'brotli', 'pefile', 'frontend', 'shared_name', 'main_name', 'panel_name') if name not in namespace]
        assert not missing, f'pe_packer 依赖的名字不在模块作用域里：{missing}'
        exec(compile(packer, 'checked_pe_packer', 'exec'), namespace)


integrate_fullatlas(PROJECT, WindowsHooks(), integrate)
