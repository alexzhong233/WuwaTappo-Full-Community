# Development

Run commands from the repository root.

```sh
npm ci
npm test
npm run test:fullatlas
npm run test:rust
```

`npm test` runs the repository's Node test suite and shared frontend regression checks. `shared/tests/host-routes.cjs` cross-checks every local API path called by shared frontend modules and `shared/backend/src/injector.rs` against both `shared/backend/src/local_gallery_host.rs` and `macos/src-tauri/src/fullatlas.rs`, plus `shared/backend/src/service.rs`, so neither client can silently drop an endpoint. Run both native Rust suites with `cargo test --offline --locked --manifest-path shared/backend/Cargo.toml` and `cargo test --offline --manifest-path macos/src-tauri/Cargo.toml --lib`.

To change assistant UI or shared browser behavior, edit `shared/frontend/` (the single source of truth; no generated frontend copies are checked in). `shared/backend/src/injector.rs` renders platform placeholders directly during `assistant-core inject`, and `npm run sync:frontend -- --check` validates that legacy copy directories stay absent and all placeholders render cleanly. All injection anchors, PE repacking, `.lnk` generation, and local host logic live in `shared/backend/`.

Region IDs, hosts, package prefixes and application identities live in `shared/config/regions.json`. Do not copy them into platform scripts.

Use the root build dispatcher for platform builds:

```sh
npm run build:check
npm run build:mac
npm run build:win
```

Each build still needs its native toolchain and official upstream inputs. See the [macOS guide](../macos/README.md) and [Windows guide](../windows/README.md) for platform prerequisites and packaging details.
