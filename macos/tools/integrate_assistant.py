"""Extend a structurally checked official frontend; fail on contract changes."""
# 主流程在 shared/tools/assistant_core.py，与 Windows 侧共用；macOS 不额外加钩子，
# 所以这里只做一次转发，调用方（fullatlas-integration.py）的 import 路径保持不变。
from pathlib import Path
import sys

sys.path.insert(0, str(next(p for p in Path(__file__).resolve().parents
                            if (p / 'shared' / 'tools').is_dir()) / 'shared' / 'tools'))
from assistant_core import integrate  # noqa: E402,F401
