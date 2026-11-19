# Cross-platform architecture

This repository shares feature behavior and keeps operating-system integration in platform adapters. A feature change starts in the shared layer whenever both clients can consume the same implementation. Windows no longer ships a Python runtime: its local bridge and launcher are Rust binaries built from `shared/backend`, while Python is limited to build, packaging, and upstream-injection tooling.

## Shared sources

| Area | Source of truth | Platform outputs or consumers |
| --- | --- | --- |
| Assistant UI and browser-side behavior | `shared/frontend/` | Generated into `macos/fullatlas-source/` and `windows/src/src/` by `npm run sync:frontend` |
| Assistant state, settings, scheduling, counters, rooms, config, diagnostics, and tokens | `shared/backend/src/lib.rs` | Linked by macOS Tauri and Windows LocalGalleryHost / WuwaTappoLauncher |
| Release selection, HTML fallback, version comparison, and official installer download | `shared/backend/src/release_updates.rs` | Linked by both native clients |
| Official atlas discovery, validation, and verified asset caching | `shared/backend/src/resource_cache.rs` | Linked by both native clients; Windows adds incremental disk asset sync & CLI `--prepare-cache` |
| Mod archive validation, identifiers, catalog merge, and safe asset resolution | `shared/backend/src/custom_resources.rs` | Linked by both native clients (`mod_assets.rs` on macOS, `local_gallery_host.rs` on Windows) |
| Upstream injection primitives | `shared/tools/inject_core.py`, `shared/tools/assistant_core.py` | Imported by both platform integration adapters (`activity-config` loading guard, taskbar default, console panel hooks, update-lock rules) |
| FullAtlas frontend injection | `shared/tools/fullatlas_core.py` | Shared by macOS and Windows (`controller_init`, `settings_ui`, embedded `"console"` panel tab, PE/Tauri hooks) |
| Region identity and upstream hosts | `shared/config/regions.json` | Read by Node/PowerShell build tools and Rust runtime/CLI profiles |
| Official upstream locks | `shared/upstream/cn.json`, `shared/upstream/global.json` | Canonical installer pins plus macOS/Windows-specific fingerprints |
| Windows update package paths and whitelist | `shared/contracts/windows-update.json` | Read by Rust `release_installer` (both update creation & apply) and PowerShell `payload.ps1`, including protected paths and Windows reserved names |
| Windows update staging, archive build, and file transaction | `shared/backend/src/release_installer.rs` | Linked by LocalGalleryHost (`--make-update` & `--apply-update`); OS process/file operations stay Windows-only |
| Published artifact names and checksum filename | `shared/contracts/release-assets.json` | Read by Node release tooling, PowerShell packaging, and Rust release/archive builders |
| Cross-platform upstream sync, NSIS extraction, and mirror tooling | `scripts/` (`extract-assets.mjs`, `sync-upstream.mjs`, `installer-*.mjs`, `upstream-*.mjs`, `regions.mjs`, `build-platform.mjs`, `bump-version.mjs`, `sync-frontend.mjs`) | Shared CLI entrypoints called by `package.json`, `.github/workflows/`, `macos/`, and `windows/` |
| Frontend regression fixtures | `shared/tests/` | Run as part of `npm test` (checks both Windows and macOS Rust hosts in `host-routes.cjs`) |
| Documentation | `docs/` | Shared documentation is flat; platform subdirectories are only for platform-specific instructions |

Region profiles are build and runtime inputs; do not restate CN/Global hosts or package prefixes in platform scripts. Generated frontend copies are build inputs for upstream integration tools; edit the shared source, then run `npm run sync:frontend`. `npm test` checks that both copies match the rendered source exactly.

## Platform adapters

The two clients do not share an application shell. macOS uses Rust/Tauri and system APIs for accessibility, input monitoring, app updates, and window management. Windows patches the official client and uses the shared Rust host plus Windows APIs. Python remains in the build and upstream-injection chain, not as the shipped local host runtime.

| Concern | macOS | Windows |
| --- | --- | --- |
| Runtime state and persistence | Shared `shared/backend` domain core plus `macos/src-tauri/src/` adapters | `shared/backend` plus Windows native adapters |
| Upstream asset injection | `macos/tools/` | `windows/src/tools/` |
| Packaging and installers | `macos/scripts/`, Tauri config | `windows/tools/`, `windows/src/installer/` |
| Platform-specific verification | `macos/tests/` | `windows/src/tests/` |

## Business-domain audit

| Domain | Shared today | Remaining platform copies | Recommended boundary |
| --- | --- | --- | --- |
| Assistant UI, typing-emote trigger, chest scheduling, reward outbox, monitor workflow | `shared/frontend/desktop_assistant.js`, `assistant_dashboard.js`, `shared/backend/src/lib.rs` | Native focus detection and monitor/input access | Keep decisions, state, validation and scheduling in shared sources; keep focus probes and native events behind adapters |
| Balance-threshold auto-click | Shared JS policy, defaults and boundary contract test | Persistence and click throttle only | Keep the policy single-sourced in shared JS |
| Local atlas, Mod selection and request restrictions | `shared/frontend/local_atlas_controller.js`, `shared/backend/src/resource_cache.rs` | Filesystem, image cache transfer, native IPC vs localhost transport | Keep catalog discovery/validation shared; keep storage and image delivery platform-owned |
| Assistant settings, daily counters, rooms, rewards and snapshots | `shared/backend/src/lib.rs` | Persistence locations, clocks and physical input are adapter-owned | Keep the Rust domain core authoritative; Python is build-time only |
| Mod ZIP validation, ID generation and catalog merge | `shared/backend/src/custom_resources.rs` | Filesystem paths, image transport and cache refresh lifecycle | Both clients call the shared Rust implementation |
| Update checks and release selection | `shared/backend/src/release_updates.rs` | Native UI aliases and installer download/install lifecycle | Share selection, comparison, URL validation and response data; retain platform-owned file operations |
| Input, permissions, windows, tray and startup | Shared user-facing behavior | Native OS APIs and lifecycle | Keep platform-owned; only share state/event contracts |
| Logon startup task and the pet's native context menu | Not applicable on macOS (Tauri owns both) | `shared/backend/src/windows_startup.rs`, `shared/backend/src/windows_menu.rs` | Windows keeps these inside the shared Rust host so the injected page talks to the same binary that owns `/startup` and `/assistant/context-menu` |
| Official client injection and packaging | Shared `inject_core.py` and `assistant_core.py` | Asset discovery, resource writing, PE vs Tauri bundle | Share transformation rules; retain separate input/output adapters |

The audit confirms that a single source for every line is not the right target: OS capabilities and package formats necessarily differ. Assistant state rules, Mod validation/merge, release selection and frontend behavior live in shared sources; platform code owns persistence locations, native input/focus probes, window lifecycle, and installer/update integration. The shipped Windows local host and updater are Rust. Python files that remain are build-time manifest and injection helpers covered by shared contracts, not a second runtime.

When the same rule must be implemented in Rust and Python, keep its defaults, boundary conditions, and expected transitions in a shared contract or fixture and verify both adapters against it. Do not duplicate platform-specific permission, storage, or event-loop code merely to make files look alike.

## Documentation ownership

All maintained documentation lives under this top-level `docs/` directory. Shared architecture, development, feature, release, upstream, and Mod documents live directly in `docs/`. Platform subdirectories are reserved for instructions and records that only apply to that platform, such as macOS input permissions and Windows build or installer procedures. Platform READMEs point to these canonical sources; packaging may copy Windows-specific references into its generated payload.
