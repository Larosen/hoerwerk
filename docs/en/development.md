# Development

[← README](../../README.md) · **English** · [Deutsch](../de/development.md)

The server is a single Rust program (`rust/hoerwerk`), plus `hw-run` (the jail for ffmpeg) and `hw-tags` (M4B
tags). The UI is TypeScript (`frontend/src`), compiled to `static/`.

```bash
(cd rust && cargo build --release)              # server, hw-run, hw-tags -> rust/target/release/
npm ci && npm run build                         # TypeScript (frontend/src) -> static/*.js
(cd rust && cargo test)                         # unit tests, also against the golden files in contract/golden
pip install -r requirements-dev.txt             # test tools (mutagen, ruff)
python3 -m unittest discover -s contract        # API contract against the server (needs ffmpeg, no network)
python3 -m unittest discover -s tests           # every text translated
npm run check                                   # type check
npx playwright install chromium && npx playwright test   # click-through tests against a real server
INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 rust/target/release/hoerwerk
```

**Translations:** German is the source language. Every text is wrapped in `t("…")` (Rust and TypeScript);
`static/i18n/en.json` holds the English text. `python3 tests/test_i18n.py --missing` lists texts that still
need a translation – the tests fail while one is missing.

CI checks every pull request: format, Clippy and unit tests, licenses and advisories of every crate, the API
contract (with the system ffmpeg and with the own ffmpeg 9 in the mandatory jail), type check, click-through tests
against the source and against the finished Docker image.

| Part | Files (`rust/hoerwerk/src/`) |
|---|---|
| Start & wiring | `main.rs`, `config.rs`, `app.rs`, `settings.rs` |
| Web | `web/mod.rs` (checks, pages, live updates), `web/api.rs` (routes), `web/*.rs`, `web/hosts.rs` |
| Data | `db.rs` (SQLite), `migrations/` |
| Jobs | `engine.rs`, `pipeline.rs`, `uploads.rs` |
| Audio | `encoder.rs`, `chapters.rs`, `quality.rs`, `media.rs`; `rust/hw-run`, `docker/build-ffmpeg.sh` |
| Metadata | `audible.rs`, `links.rs`, `tagger.rs`, `rust/hw-tags` |
| Library & downloads | `filing.rs`, `library.rs`, `usenet.rs`, `releases.rs`, `packs.rs`, `requests.rs` |
| Accounts & security | `accounts.rs`, `invites.rs`, `totp.rs`, `authlog.rs` |
| Notifications & log | `notify.rs`, `text.rs` |
| Media servers & listening | `mediaservers.rs`, `listening.rs`, `stats.rs` |
| Discover | `discover.rs`, `pdftext.rs`, `trusted_authors.txt`, `frontend/src/discover.ts` |
| Interface | `frontend/src/*.ts`, `static/index.html`, `static/style.css`, `static/i18n/en.json`; E2E tests in `e2e/` |
| API contract | `contract/` (routes, schema, settings, golden files, tests) |
