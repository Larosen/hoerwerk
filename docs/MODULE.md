# Große Module – Abhängigkeiten und Entscheidung (Stand 7.13)

## Abhängigkeiten

| Modul | Zeilen | importiert (aus `app/`) | wird benutzt von |
|---|---|---|---|
| `pipeline.py` | 1459 | abs, applog, audible, bookrequests (nur `book_key`, lokal), db, encoder, i18n, library, links, meta, natsort, packs, providers, quality, releases, tagger | services |
| `discover.py` | 691 | applog, audible, db, i18n, pdftext, trusted_authors | services, Route `discover` |
| `audible.py` | 655 | i18n | bookrequests, discover, library_index, links, pipeline, providers; Routen `audiobooks`, `search`, `system` |
| `bookrequests.py` | 652 | applog, audible, db, i18n, releases, usenet | pipeline, services; Routen `discover`, `requests`, `search` |
| `usenet.py` | 611 | db, i18n | bookrequests, library_index, releases, services; Route `search` |

Zwischen den Modulen in `app/` gibt es **keine zyklischen Imports**. Die Richtung ist durchgängig:
`audible`/`usenet` (Netzwerk, ohne App-Zustand) ← `releases`/`bookrequests`/`discover` ← `pipeline` ← `services` ← `web`.

## `pipeline.py` aufteilen?

Geprüft wurde eine Aufteilung in ein Paket `pipeline/` mit diesen Teilen:

| Teil | Inhalt | Zeilen |
|---|---|---|
| `jobs.py` | Job und Zustände | 115 |
| `scanner.py` | Eingang beobachten | 326 |
| `matching.py` | Treffer finden und auswählen | 339 |
| `processing.py` | Stufen, Umwandlung, Kapitel | 250 |
| `filing.py` | Ablegen in die Bibliothek | 265 |
| `__init__.py` | Zustand, Speichern, öffentliche Aktionen | 263 |

Ein Prototyp lief durch; die Methoden wurden dabei nur verschoben.

**Ergebnis: Die Aufteilung unterbleibt.** Gründe:

* **Nicht unabhängig testbar.** Die Teile wären Mixins einer Klasse und teilen deren Zustand: Jobs, Locks, Engine, Index und Einstellungen. Keiner der neuen Teile ließe sich ohne die ganze Pipeline testen.
* **Bestehende Tests müssten geändert werden.** Die Tests verschieben die Zeit über `pipeline._now` und lesen `pipeline.GRAB_SETTLE`. Nach der Aufteilung wirkt das nicht mehr, weil `_now` dann in `scanner.py` gebunden ist. Diese Tests müssten also angepasst werden. Das Abnahmekriterium „Pipeline-Tests unverändert“ wäre verletzt.
* **Kein messbarer Gewinn.** Die Datei ist bereits nach Zuständigkeiten in Abschnitte gegliedert (`# -- persistence`, `# -- public API`, `# -- steps`). Abhängigkeiten nach außen gibt es nur zu `services`. Die Aufteilung hätte Navigation gegen zusätzliche Kopplung über Modulgrenzen getauscht.

Sinnvoll wird eine Aufteilung erst, wenn ein Teil eigenständig wird. Ein Beispiel wäre das Ablegen in die Bibliothek als eigene Klasse mit eigener Schnittstelle (Ziel, alte Fassung, Journal). Dann lohnt sie sich, und zwar mit eigenen Tests.

## Die anderen großen Module

* **`audible.py`** – reiner Netzwerk-Client (Katalog, Kapitel, Bilder) plus Abgleich von Namen und Titeln. Bei weiterem Wachstum lassen sich die Abgleich-Funktionen (`_ratio`, `surname_of`, `split_volume`, `content_queries`) als eigenes Modul ohne Netzwerk abtrennen. Sie sind schon heute reine Funktionen.
* **`usenet.py`** – Newznab, SABnzbd und Ratenbegrenzung in getrennten Klassen. Bereits sauber geschnitten.
* **`bookrequests.py`** – eine Klasse für den Lebenszyklus einer Anfrage. Gut gegliedert, keine Aufteilung nötig.
* **`discover.py`** – Quellen (Feeds, Listen, PDF) und Auswahl. Die Parser sind schon reine Funktionen und haben eigene Tests. Der PDF-Leser liegt bereits in `pdftext.py`.
