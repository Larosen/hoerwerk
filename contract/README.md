# API-Vertrag

Was jede Hörwerk-Implementierung (heute Python, später Rust) erfüllen muss. Geprüft wird nur von außen über HTTP
und die Ordner, ohne einen Import der Anwendung.

| Datei | Inhalt |
|---|---|
| `routes.json` | Alle 96 API-Routen mit Methode, Pfad und Zugriff (`public` / `user` / `admin`) |
| `schema.sql` | Das Datenbank-Schema (Tabellen, Indizes, `user_version`) |
| `fixtures/hoerwerk-7.13.db` | Eine Datenbank aus 7.13, die jede neue Version unverändert übernehmen muss |
| `golden/` | Ein Buch (`book.json`) und die M4B-Tags, die daraus entstehen müssen (`tags.json`), gelesen mit `read_tags.py`; dazu die Antworten der Python-Regeln, die Rust gleich geben muss: Namen und Titel (`words.json`), Releases und Indexer (`releases.json`), Shop-Daten und Treffer (`audible.json`), Rezensionen, Feeds und Bestenlisten-PDFs (`discover.json`) |
| `test_contract.py` | Die Tests: Zugriff, Sicherheit, Konten, Einladungen, Einstellungen, Anfragen, Eingang → Bibliothek, Upload, Live-Updates, Herunterfahren, Schema, alte Datenbank |
| `../docs/JOBS.md` | Job-Zustände, Übergänge und das Verhalten bei einem Neustart |

```sh
cd contract && python3 -m unittest discover -s .                                   # gegen python3 app/main.py
cd contract && HW_SERVER_CMD=../rust/target/release/hoerwerk python3 -m unittest discover -s .
```

Das Fixture und die Vergleichsdateien werden nur nach einer **bewussten** Änderung neu erzeugt:
`python3 contract/make_fixture.py`, `python3 contract/make_golden.py`, `make_words.py`, `make_releases.py`,
`make_audible.py`, `make_discover.py` (schreibt auch `rust/hoerwerk/src/trusted_authors.txt`). `tests/test_golden.py`
meldet, wenn eine davon nicht mehr zu den Python-Regeln passt.
