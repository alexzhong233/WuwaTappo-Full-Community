# Cross-platform architecture

This repository shares feature behavior and keeps operating-system integration in platform adapters. A feature change starts in the shared layer whenever both clients can consume the same implementation. Both runtime hosts and build/injection tooling are unified in Rust (`shared/backend`) and Node (`scripts/`); the repository carries zero Python dependencies.

## Shared sources

| Area | Source of truth | Platform outputs or consumers |
| --- | --- | --- |
| Assistant UI and browser-side behavior | `shared/frontend/` | Rendered directly into build outputs by `shared/backend/src/injector.rs` (`sync-frontend.mjs` also validates/renders) |
| Assistant state, settings, scheduling, counters, rooms, config, diagnostics, and tokens | `shared/backend/src/lib.rs` | Linked by macOS Tauri (`FullAtlasState`) and Windows `LocalGalleryHost` |
| Shared `FullAtlasService` runtime domain logic | `shared/backend/src/service.rs` | Called by macOS `fullatlas.rs` (Tauri IPC) and Windows `local_gallery_host.rs` (`tiny_http`) |
| Official atlas discovery, validation, and verified asset caching | `shared/backend/src/resource_cache.rs` | Used by both clients (`mod_assets.rs` on macOS, `local_gallery_host.rs` on Windows) for runtime `cache/` PNG serving |
| Mod archive validation, identifiers, catalog merge, and safe asset resolution | `shared/backend/src/custom_resources.rs` | Linked by both native clients (`mod_assets.rs` on macOS, `local_gallery_host.rs` on Windows) |
| Release selection, HTML fallback, version comparison, and official installer download | `shared/backend/src/release_updates.rs` | Linked by both native clients |
| Unified upstream injection, PE repacking, `.lnk` writer, and demo Mod packs | `shared/backend/src/{injector,pe_repack,lnk,demo_packs}.rs` | Exposed via `assistant-core` CLI (`inject`, `extract-pe`, `make-shortcut`, `make-demo-packs`) |
| Region identity and upstream hosts | `shared/config/regions.json` | Read by Node/PowerShell build tools and Rust runtime/CLI profiles |
| Official upstream locks | `shared/upstream/cn.json`, `shared/upstream/global.json` | Canonical installer pins shared across macOS and Windows |
| Windows update package paths and whitelist | `shared/contracts/windows-update.json` | Read by Rust `release_installer` (both update creation & apply) and PowerShell `payload.ps1`, including protected paths and Windows reserved names |
| Windows update staging, archive build, and file transaction | `shared/backend/src/release_installer.rs` | Linked by LocalGalleryHost (`--make-update` & `--apply-update`); OS process/file operations stay Windows-only |
| Hybrid local/official appearance | `shared/frontend/local_atlas_controller.js` | Shared by both injected renderers; source labels, local overrides, cloud-preset eligibility and synthesis policy stay identical |
| Published artifact names and checksum filename | `shared/contracts/release-assets.json` | Read by Node release tooling, PowerShell packaging, and Rust release/archive builders |
| Cross-platform upstream sync, NSIS extraction, official Windows fetch, and mirror tooling | `scripts/` (`extract-assets.mjs`, `fetch-official-win.mjs`, `sync-upstream.mjs`, `installer-*.mjs`, `upstream-*.mjs`, `regions.mjs`, `build-platform.mjs`, `bump-version.mjs`, `sync-frontend.mjs`) | Shared CLI entrypoints called by `package.json`, `.github/workflows/`, `macos/`, and `windows/` |
| Frontend regression fixtures | `shared/tests/` | Run as part of `npm test` (checks both Windows and macOS Rust hosts in `host-routes.cjs`) |
| Documentation | `docs/` | Shared documentation is flat; platform subdirectories are only for platform-specific instructions |

Region profiles are build and runtime inputs; do not restate CN/Global hosts or package prefixes in platform scripts. Edit `shared/frontend/` directly; `injector.rs` renders platform placeholders on demand and `npm test` verifies rendered output, contracts, and layout cleanliness (no generated frontend copies or `.py` files).

## Platform adapters

The two clients do not share an application shell. macOS uses Rust/Tauri and system APIs for accessibility, input monitoring, app updates, window management, and loopback PNG transport (`mod_assets.rs`). Windows patches the official client PE binary using `assistant-core` (`inject --pack-pe`) and uses the shared Rust HTTP host plus Windows APIs.

| Concern | macOS | Windows |
| --- | --- | --- |
| Runtime state, FullAtlas service, and persistence | Shared `shared/backend` `FullAtlasService` (single-file persistence) + `macos/src-tauri/src/` IPC & `mod_assets.rs` | Shared `shared/backend` `FullAtlasService` (split-file persistence) + `local_gallery_host.rs` HTTP & Windows native adapters |
| Upstream asset injection & PE handling | `macos/scripts/fullatlas-frontend.mjs` -> `assistant-core inject --platform mac` | `windows/tools/build.ps1` -> `extract-assets.mjs` + `assistant-core {extract-pe,inject --pack-pe}` |
| Packaging and installers | `macos/scripts/`, Tauri config, `package-macos.sh` | `windows/tools/`, `windows/src/installer/`, NSIS + `LocalGalleryHost.exe --make-update` |
| Platform-specific verification | `macos/tests/` | `windows/src/tests/` |

## Business-domain audit

| Domain | Shared today | Platform shell boundary | Recommended boundary |
| --- | --- | --- | --- |
| Assistant UI, context menu, typing-emote trigger, chest scheduling, reward outbox, OBS backdrop / update checks | `shared/frontend/{desktop_assistant,assistant_dashboard,desktop_context_menu}.js`, `shared/backend/src/lib.rs` | Native focus detection and monitor/input access | Keep decisions, state, validation and scheduling in shared sources; keep focus probes and native events behind adapters |
| Balance-threshold auto-click | Shared JS policy, defaults and boundary contract test | Persistence and click throttle only | Keep the policy single-sourced in shared JS |
| Local atlas, Mod selection and official cache sync | `shared/frontend/local_atlas_controller.js`, `shared/backend/src/{service,resource_cache}.rs` | Tauri IPC + `mod_assets.rs` vs `tiny_http` in `local_gallery_host.rs` | Keep `FullAtlasService` & cache validation shared; keep transport and filesystem paths platform-owned |
| Assistant settings, daily counters, rooms, rewards and snapshots | `shared/backend/src/lib.rs` | Persistence locations, clocks and physical input are adapter-owned | Keep the Rust domain core authoritative |
| Mod ZIP validation, ID generation and catalog merge | `shared/backend/src/custom_resources.rs` | Filesystem paths and image serving | Both clients call the shared Rust implementation |
| Update checks and release selection | `shared/backend/src/release_updates.rs` | Native UI aliases and installer download/install lifecycle | Share selection, comparison, URL validation and response data; retain platform-owned file operations |
| Input, permissions, windows, tray and startup | Shared user-facing behavior | Native OS APIs and lifecycle | Keep platform-owned; only share state/event contracts |
| Logon startup task and the pet's native context menu | Not applicable on macOS (Tauri owns both) | `shared/backend/src/windows_startup.rs`, `shared/backend/src/windows_menu.rs` | Windows keeps these inside the shared Rust host so `/startup` and `/assistant/context-menu` stay unified |
| Official client injection, PE extraction/repack, `.lnk` creation | `shared/backend/src/{injector,pe_repack,lnk}.rs` | Target directory / PE resource table I/O | Share transformation rules and `assistant-core` CLI across both builds |

This layout keeps shared behavior, contracts, injection anchors, and runtime services in `shared/` (Rust + frontend JS) while platform directories own OS bindings, build scripts, and packaging.

## Documentation ownership

All maintained documentation lives under this top-level `docs/` directory. Shared architecture, development, feature, release, upstream, and Mod documents live directly in `docs/`. Platform subdirectories are reserved for instructions and records that only apply to that platform, such as macOS input permissions and Windows build or installer procedures. Platform READMEs point to these canonical sources; packaging may copy Windows-specific references into its generated payload.
