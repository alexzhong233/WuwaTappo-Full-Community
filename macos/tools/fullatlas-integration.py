"""Apply the local-mode integration to the versioned official snapshot.

Bindings and component locations come from parsed JavaScript, not guessed
minified variable names. Unknown layouts fail before an installed file changes.
The shared pipeline lives in ``shared/tools/fullatlas_core.py``.
"""
from pathlib import Path
import re
import sys

PROJECT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT.parent / 'shared' / 'tools'))
from fullatlas_core import FullAtlasHooks, integrate_fullatlas
from inject_core import replace
from integrate_assistant import integrate


class MacHooks(FullAtlasHooks):
    controller_path = 'fullatlas-source/local_atlas_controller.js'
    strict_invoke = True
    startup_error_prefix = 'Mac 全图鉴启动失败：'

    def prepare_shared(self, shared, project, config):
        # Preserve Tauri asset URLs for local Mod images in the official renderers.
        for name in ('resolveAppearanceAssetUrl', 'resolveAppearanceAssetDisplayUrl', 'resolveAppearanceAtlasUrl'):
            pattern = r'function ' + name + r'\([^\n]*\) \{[\s\S]{0,3000}?(?=\n(?:async )?function )'
            match = re.search(pattern, shared)
            assert match, name
            original = match.group()
            assert '\nfunction ' not in original[1:], f'{name} regex matched past function boundary'
            updated = original.replace(r'/^https?:\/\//i', r'/^(?:https?|asset):\/\//i')
            assert updated != original, name
            shared = replace(shared, original, updated)
        return shared

    def transport_options(self, config):
        return (
            "origin: window.__WUWATAPPO_PROFILE__.assetHost, "
            "request: (route,payload) => window.__TAURI_INTERNALS__.invoke('fullatlas_request',{action:route.replace(/^\\//,''),payload:payload||{}}), "
            "token: null, customAssetUrl: (reference, file) => window.__TAURI_INTERNALS__.convertFileSrc(file)"
        )


integrate_fullatlas(PROJECT, MacHooks(), integrate)
