# Development

Run commands from the repository root.

```sh
npm ci
npm test
npm run test:fullatlas
npm run test:rust
npm run sync:frontend
```

`npm test` runs the repository's Node test suite and shared frontend regression checks. Run both native Rust suites with `cargo test --offline --manifest-path shared/backend/Cargo.toml` and `cargo test --offline --manifest-path macos/src-tauri/Cargo.toml --lib`. Windows Python tests currently cover legacy host behavior and build-time injection tools; the shipped Windows local host and launcher are Rust binaries.

To change assistant UI or shared browser behavior, edit `shared/frontend/`, run `npm run sync:frontend`, then run `npm test`. The generated copies under each platform are consumed by their respective injection and packaging chains. Shared Python injection logic belongs in `shared/tools/`; do not add business rules to the legacy Windows Python host. Port required behavior to `shared/backend` and keep the Rust host as the shipped runtime.

Use the root build dispatcher for platform builds:

```sh
npm run build:check
npm run build:mac
npm run build:win
```

Each build still needs its native toolchain and official upstream inputs. See the [macOS guide](../macos/README.md) and [Windows guide](../windows/README.md) for platform prerequisites and packaging details.
