# Development

Run commands from the repository root.

```sh
npm ci
npm test
npm run test:fullatlas
npm run test:rust
npm run sync:frontend
```

`npm test` runs the repository's Node test suite and shared frontend regression checks. `shared/tests/host-routes.cjs` cross-checks every local API path called by shared frontend modules and injectors against both `shared/backend/src/local_gallery_host.rs` and `macos/src-tauri/src/fullatlas.rs`, so neither client can silently drop an endpoint. Run both native Rust suites with `cargo test --offline --locked --manifest-path shared/backend/Cargo.toml` and `cargo test --offline --manifest-path macos/src-tauri/Cargo.toml --lib`. Python tests (`test_inject*.py`) cover shared injection anchors (`shared/tools/`); asset cache sync and in-place update packaging run in Rust (`shared/backend`).

To change assistant UI or shared browser behavior, edit `shared/frontend/`, run `npm run sync:frontend`, then run `npm test`. The generated copies under each platform are consumed by their respective injection and packaging chains. Shared Python injection logic belongs in `shared/tools/`; do not add business rules to the legacy Windows Python host. The FullAtlas anchor pipeline is `shared/tools/fullatlas_core.py`; macOS and Windows provide platform hooks only. Port required runtime behavior to `shared/backend` and keep the Rust host as the shipped runtime.

Region IDs, hosts, package prefixes and application identities live in `shared/config/regions.json`. Do not copy them into platform scripts.

Use the root build dispatcher for platform builds:

```sh
npm run build:check
npm run build:mac
npm run build:win
```

Each build still needs its native toolchain and official upstream inputs. See the [macOS guide](../macos/README.md) and [Windows guide](../windows/README.md) for platform prerequisites and packaging details.
