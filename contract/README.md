# API-Vertrag

Was der Hörwerk-Server (`rust/hoerwerk`) erfüllen muss. Geprüft wird nur von außen über HTTP und die Ordner, ohne
einen Import der Anwendung. Entstanden mit Hörwerk 7.13 (damals Python): Was 7.13 tat, ist der Maßstab.

| Datei | Inhalt |
|---|---|
| `routes.json` | Alle 96 API-Routen mit Methode, Pfad und Zugriff (`public` / `user` / `admin`) – der Server liest sie beim Bauen |
| `settings.json` | Einstellungen: Standardwerte, erlaubte Werte, Umgebungsvariablen, Version – der Server liest sie beim Bauen |
| `schema.sql` | Das Datenbank-Schema (Tabellen, Indizes, `user_version`) |
| `fixtures/hoerwerk-7.13.db` | Eine Datenbank aus 7.13, die jede neue Version unverändert übernehmen muss |
| `golden/` | Die Antworten von 7.13 auf knifflige Eingaben – die Rust-Unit-Tests prüfen dagegen: M4B-Tags eines Buchs (`book.json` → `tags.json`, gelesen mit `read_tags.py`; zurückgelesen `read.json`), Namen und Titel (`words.json`), Releases und Indexer (`releases.json`), Shop-Daten und Treffer (`audible.json`), Rezensionen, Feeds und Bestenlisten-PDFs (`discover.json`) |
| `test_contract.py` | Die Tests: Zugriff, Sicherheit, Konten, Einladungen, Einstellungen, Anfragen, Eingang → Bibliothek, Upload, Live-Updates, Herunterfahren und Neustart, Schema, alte Datenbank |
| `test_tags.py` | Der Tag-Schreiber `hw-tags` gegen `golden/` |
| `../docs/JOBS.md` | Job-Zustände, Übergänge und das Verhalten bei einem Neustart |

```sh
(cd rust && cargo build --release)
pip install -r requirements-dev.txt
cd contract && python3 -m unittest discover -s .          # gegen rust/target/release/hoerwerk
HW_SERVER_CMD=/pfad/zu/hoerwerk python3 -m unittest discover -s .
```

Die Golden Files und das Fixture sind eingefroren. Wer eine Regel bewusst ändert, passt die Datei von Hand an und
schreibt im Commit, warum.
