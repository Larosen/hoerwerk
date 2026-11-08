# Hörbuch-Jobs: Zustände, Übergänge, Neustart

Diese Datei ist die Spezifikation des Job-Lebenslaufs (umgesetzt in `rust/hoerwerk/src/pipeline.rs` und
`engine.rs`). Jeder Übergang, der hier nicht steht, ist ein Fehler.

## Begriffe

Ein **Job** ist ein Hörbuch auf dem Weg vom Eingang (oder aus der Bibliothek) in die Bibliothek. Er hat:

- **`status`**: das, was die Oberfläche zeigt.
- **`stage`**: die Stufe, die als Nächstes läuft oder gerade läuft.

| stage | Warteschlange | status während der Arbeit | Inhalt |
|---|---|---|---|
| `prepare` | `io` | `matching` | Dateien prüfen, Hinweise sammeln, Treffer suchen (Shop, Inhalt, Anfragen) |
| `convert` | `encode` | `converting` | M4B mit Kapiteln bauen (oder übernehmen) |
| `finish` | `io` | `tagging` | Duplikat prüfen, Tags und Cover schreiben, ablegen, Quelle wegräumen |

**Endzustände:**

| status | Bedeutung |
|---|---|
| `done` | Abgelegt. `dest` ist die M4B in der Bibliothek. |
| `review` | Wartet auf eine Entscheidung: kein oder unsicherer Treffer, oder ein Duplikat. |
| `failed` | Fehler, `error` sagt warum. Lässt sich erneut versuchen. |
| `cancelled` | Abgebrochen oder übersprungen. |

`queued` heißt: Die Stufe `stage` wartet auf einen freien Platz in ihrer Warteschlange.

## Übergänge

```text
                ┌──────────── retry / reopen / choose / manual ────────────┐
                ▼                                                          │
 (neu) ──► queued[prepare] ──► matching ──► queued[convert] ──► converting ─┤
                                  │                                  │      │
                                  │ Band schon vorhanden             │ kein Treffer
                                  ▼                                  ▼      │
                              cancelled                     review[finish] ─┘
                                                                     │ choose / manual
                                  queued[finish] ◄───────────────────┘
                                        │
                                        ▼
                                     tagging ──► done
                                        │
                                        ├─► review   (Duplikat, Einstellung „fragen“)
                                        └─► cancelled (Duplikat, Einstellung „überspringen“)

 jede Arbeitsstufe ──► failed      (Fehler)
 jede Arbeitsstufe ──► cancelled   (Abbruch durch den Nutzer)
 queued / review   ──► cancelled   (Abbruch, ohne dass gerade etwas läuft)
```

| von | nach | Auslöser | Bedingung / Wirkung |
|---|---|---|---|
| – | `queued` (prepare) | neuer Ordner im Eingang, Upload, „Umwandeln“ aus der Bibliothek | Ordner ist vollständig (keine Änderung seit `SETTLE_SECONDS`) |
| `queued` | `matching` / `converting` / `tagging` | ein Worker nimmt den Schritt | **Nur ein Worker je Job.** Der Wechsel geschieht unter Sperre und nur aus `queued`. |
| `matching` | `queued` (convert) | Treffer gefunden oder keiner nötig | – |
| `matching` | `cancelled` | dieser Band einer Reihe ist schon in der Bibliothek | – |
| `converting` | `queued` (finish) | M4B fertig, Treffer bekannt | – |
| `converting` | `review` (finish) | M4B fertig, kein Treffer | – |
| `tagging` | `done` | abgelegt | `dest` gesetzt, Quelle nach `_fertig` oder gelöscht, Arbeitsordner weg |
| `tagging` | `review` | Duplikat und `ab_duplicates=ask` | `duplicate` ist gesetzt |
| `tagging` | `cancelled` | Duplikat und `ab_duplicates=skip` | Quelle weggeräumt |
| `review` | `queued` (finish) | Treffer gewählt (`choose`) oder eigene Angaben (`manual`) | – |
| `review` | `queued` | Duplikat entschieden (`replace`/`keep`) | – |
| `failed`, `review` | `queued` | `retry` | Stufe `finish`, wenn die M4B noch existiert, sonst `prepare` |
| `done` | `review` (finish) | `reopen` (aus der Bibliothek zurückholen) | – |
| jede Arbeitsstufe | `failed` | Ausnahme | `error` gesetzt, Arbeitsordner bleibt bis zum Retry |
| jede Arbeitsstufe | `cancelled` | Nutzer bricht ab | ffmpeg wird sofort beendet, Arbeitsordner weg |
| `queued`, `review` | `cancelled` | Nutzer bricht ab | wartender Schritt aus der Warteschlange entfernt |
| `done`, `failed`, `cancelled` | (entfernt) | „Erledigte entfernen“ bzw. Löschen | – |

## Neustart und Herunterfahren

| Zustand beim Ende | Nach dem Start |
|---|---|
| `queued`, `matching`, `converting`, `tagging` | `queued` in derselben `stage`: der Schritt läuft neu. Eine fertige M4B im Arbeitsordner wird wiederverwendet. |
| Ablegen unterbrochen (Ablage-Journal `filing` gesetzt) | Liegt die M4B vollständig in der Bibliothek (Name und Größe stimmen), ist der Job `done` und die alte Fassung wird archiviert bzw. gelöscht. Sonst kommt die zur Seite gestellte alte Fassung zurück, und der Job ist `queued[finish]`. Halbe Kopien (`.hw-part-*`) werden entfernt. |
| `docker stop` (SIGTERM) | Laufendes ffmpeg endet sofort. Der Job ist `queued` (nicht `cancelled`) mit Meldung „Beim Beenden unterbrochen“. |
| Endzustände | unverändert |

**Start:** Kein Job wird bearbeitet, bevor die Bibliothek einmal vollständig eingelesen ist (Duplikat-Prüfung).
Ist sie nicht lesbar, wartet Hörwerk und versucht es jede Minute neu.

## Garantien (getestet)

- Ein Job läuft nie in zwei Workern zugleich, auch wenn er mehrfach eingereiht wird.
  Test: `engine.rs` `one_step_per_job_at_a_time`.
- Eine M4B erscheint in der Bibliothek nur vollständig (atomares Verschieben, auch über Plattengrenzen).
  Test: `filing.rs` `moves_whole_files_also_across_disks`.
- Ein Neustart verliert keinen Job und legt keinen doppelt ab; Herunterfahren stellt laufende Jobs zurück.
  Tests: `contract/test_contract.py` `Lifecycle.test_interrupted_job_is_finished_after_a_restart` und
  `test_sigterm_ends_in_order`.
- Ein Abbruch erreicht ffmpeg in jedem Moment, auch beim Start, samt Kindprozessen.
  Test: `encoder.rs` `runner_kills_a_cancelled_process`.

## Persistenz

Ein Job ist eine Zeile in der Tabelle `jobs` (siehe `contract/schema.sql`):

- Spalten `id`, `kind='audiobook'`, `status`, `source`, `created`, `updated`;
- Spalte `data` mit allen Feldern als JSON.

Bekannte Felder in `data` (`struct Job` in `rust/hoerwerk/src/pipeline.rs`):

| Gruppe | Felder |
|---|---|
| Herkunft | `id`, `source`, `rel`, `name`, `origin` |
| Zustand | `status`, `stage`, `message`, `progress`, `log`, `error`, `created`, `updated` |
| Dateien | `files`, `duration`, `m4b`, `cover`, `dest`, `converted_info` |
| Erkennung | `hints`, `candidates`, `match`, `region` |
| Duplikat | `duplicate`, `dup_decision` |
| Kapitel | `chapter_names`, `rename_files` |
| Sonstiges | `pack`, `filing` |

Unbekannte Felder werden beim Lesen ignoriert. So bleiben ältere und neuere Datenbanken lesbar.
