# Entwicklung

[← README](../../README.de.md) · [English](../en/development.md) · **Deutsch**

Der Server ist ein einziges Rust-Programm (`rust/hoerwerk`), dazu `hw-run` (der Käfig für ffmpeg) und `hw-tags`
(M4B-Tags). Die Oberfläche ist TypeScript (`frontend/src`), übersetzt nach `static/`.

```bash
(cd rust && cargo build --release)              # Server, hw-run, hw-tags -> rust/target/release/
npm ci && npm run build                         # TypeScript (frontend/src) -> static/*.js
(cd rust && cargo test)                         # Unit-Tests, auch gegen die Golden Files in contract/golden
pip install -r requirements-dev.txt             # Testwerkzeuge (mutagen, ruff)
python3 -m unittest discover -s contract        # API-Vertrag gegen den Server (braucht ffmpeg, kein Netz)
python3 -m unittest discover -s tests           # jeder Text übersetzt
npm run check                                   # Typprüfung
npx playwright install chromium && npx playwright test   # Durchklick-Tests gegen einen echten Server
INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 rust/target/release/hoerwerk
```

**Übersetzungen:** Deutsch ist die Ausgangssprache. Jeder Text steht in `t("…")` (Rust und TypeScript);
`static/i18n/en.json` enthält den englischen Text. `python3 tests/test_i18n.py --missing` listet Texte ohne
Übersetzung – die Tests schlagen fehl, solange einer fehlt.

Die CI prüft jeden Pull Request: Format, Clippy und Unit-Tests, Lizenzen und Sicherheitsmeldungen aller Crates, den
API-Vertrag (mit System-ffmpeg und mit dem eigenen ffmpeg 9 im Pflicht-Käfig), Typprüfung, Durchklick-Tests gegen
den Quelltext und gegen das fertig gebaute Docker-Image.

| Teil | Dateien (`rust/hoerwerk/src/`) |
|---|---|
| Start & Verdrahtung | `main.rs`, `config.rs`, `app.rs`, `settings.rs` |
| Web | `web/mod.rs` (Prüfungen, Seiten, Live-Updates), `web/api.rs` (Routen), `web/*.rs`, `web/hosts.rs` |
| Daten | `db.rs` (SQLite), `migrations/` |
| Abläufe | `engine.rs`, `pipeline.rs`, `uploads.rs` |
| Audio | `encoder.rs`, `chapters.rs`, `quality.rs`, `media.rs`; `rust/hw-run`, `docker/build-ffmpeg.sh` |
| Metadaten | `audible.rs`, `links.rs`, `tagger.rs`, `rust/hw-tags` |
| Bibliothek & Downloads | `filing.rs`, `library.rs`, `usenet.rs`, `releases.rs`, `packs.rs`, `requests.rs` |
| Konten & Sicherheit | `accounts.rs`, `invites.rs`, `totp.rs`, `authlog.rs` |
| Benachrichtigungen & Protokoll | `notify.rs`, `text.rs` |
| Medienserver & Hörverlauf | `mediaservers.rs`, `listening.rs`, `stats.rs` |
| Entdecken | `discover.rs`, `pdftext.rs`, `trusted_authors.txt`, `frontend/src/discover.ts` |
| Oberfläche | `frontend/src/*.ts`, `static/index.html`, `static/style.css`, `static/i18n/en.json`; Durchklick-Tests in `e2e/` |
| API-Vertrag | `contract/` (Routen, Schema, Einstellungen, Golden Files, Tests) |
