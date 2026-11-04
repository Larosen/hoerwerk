# Rückmeldung in der Oberfläche – Bestandsaufnahme und System (7.13)

## Bestandsaufnahme vorher

| Datei | Schreibende Aufrufe | Lesende Aufrufe | Klick-Handler | Formulare | Fehlte |
|---|---|---|---|---|---|
| `account.ts` | 16 | 6 | 20 | – | Laden der Listen ohne Platzhalter; Fehler nur als kurzer Toast; Doppelklick möglich |
| `books.ts` | 16 | 8 | 26 | 1 | Erneut / Abbrechen / Entfernen ohne jede Rückmeldung; das Ende der Arbeit wurde nicht gemeldet |
| `requests.ts` | 9 | 1 | 19 | – | Liste beim ersten Laden leer (sah aus wie „keine Anfragen“); Fehler unsichtbar |
| `search.ts` | 8 | 1 | 9 | 3 | Ältere Suche konnte die neuere überschreiben; Tests der Quellen ohne Fortschritt |
| `discover.ts` | 1 | 3 | 4 | – | Wechsel des Bereichs zeigte kurz die alten Karten |
| `upload.ts` | 3 (+ XHR) | – | 3 | – | Start-Knopf ohne Zustand |
| `settings.ts`, `app.ts`, `notify.ts`, `listening.ts`, `downloads.ts`, `stats.ts`, `logs.ts`, `login.ts` | 1–4 je Datei | 1–3 | 0–3 | 0–4 | Unterschiedlich: Statistik ohne Platzhalter, Speichern ohne Laufzustand |
| `core.ts` | `api()` ohne Timeout, Netzfehler als „Failed to fetch“, `toast()` mit nur einem Platz (überschrieb sich) | | | | |
| `live.ts` | Nach einer Abweisung (503/401) blieb die Live-Verbindung dauerhaft weg | | | | |

Native `alert()`-Fenster gab es keine.

## Das System jetzt (`frontend/src/core.ts`)

- **`api()`** ist der einzige Weg zum Server.
  - Timeouts: 30 s beim Lesen, 90 s bei Änderungen, einstellbar je Aufruf.
  - Lesbare Meldungen statt Technik: kein Netz, Zeitüberschreitung, 503 = ausgelastet, 502/504 = nicht erreichbar.
  - Stacktraces und HTML aus Antworten werden herausgefiltert.
  - Dieselbe Änderung zweimal gleichzeitig ergibt **eine** Anfrage.
  - Änderungen werden **nie automatisch wiederholt**.
  - Suchen mit `key` verwerfen die Antwort einer überholten Suche.
  - CSRF-Header und Anmeldung bleiben wie bisher.
- **Knöpfe** zeigen ihren Zustand automatisch.
  - Ein Klick wird gemerkt; die schreibende Anfrage, die er auslöst, gehört zu diesem Knopf.
  - Während der Anfrage: Spinner und die laufende Aktion („Speichern“ → „Speichert …“), gleiche Größe, `aria-busy`, weitere Klicks werden verschluckt.
  - Danach kurz grün (Erfolg) oder rot (Fehler), dann wieder bedienbar.
  - Nach einer Rückfrage (`confirmBox`) zeigt der ursprüngliche Knopf den Fortschritt.
  - Änderungen über Auswahlfelder erzeugen keinen falschen Spinner.
  - Für Arbeit außerhalb von `api()` gibt es `trackButton()`, z. B. für den Upload.
- **Meldungen** (`toast(text, "success" | "info" | "warning" | "error")`):
  - Sie stapeln sich, höchstens vier sichtbar, gleiche Meldungen werden nicht verdoppelt.
  - Fehler stehen 12 s und haben die Rolle `alert`; alle Meldungen lassen sich mit der Maus oder der Tastatur schließen.
  - Unter der Maus oder beim Fokus bleiben sie stehen.
  - `toast(text, true)` funktioniert wie bisher.
- **Listen** (`loadList()`):
  - Beim ersten Laden Platzhalter in der Form des Inhalts.
  - Bei Aktualisierungen bleibt der bisherige Inhalt stehen.
  - Ein Fehler ohne Inhalt zeigt „Erneut versuchen“; bei Hintergrund-Abfragen bleibt der letzte Stand ohne Meldung.
  - Leer und „lädt noch“ sind unterscheidbar (`aria-busy`, Platzhalter).
- **Hintergrundarbeit**:
  - Eine angenommene Anfrage meldet „angenommen / eingereiht“ (blau, info).
  - Ist die Arbeit selbst beendet (abgelegt, fehlgeschlagen, braucht eine Entscheidung), kommt eine eigene Meldung. Die Seite verfolgt dafür die gestarteten Jobs über die Live-Daten.
- **Live-Verbindung**:
  - Nach einer Abweisung baut sie sich selbst neu auf, mit wachsendem Abstand bis höchstens 30 s.
  - Ab 10 s Unterbrechung erscheint ein Hinweis, und es wird mitgeteilt, wenn sie wieder da ist.
- **Tokens** (`style.css`): Statusfarben (ok, warn, err, info), Abstände, Radien, Knopfhöhen, Schriftgrößen, Dauer, Platzhalterfarben.
  - Mit `prefers-reduced-motion` gibt es keine Animationen; nur der Spinner dreht sich langsam weiter.

## Bewusst nicht umgebaut

- **Einzelne Handler mit eigener `disabled`-Logik** (z. B. „Anfragen“ auf Karten): Sie funktionieren weiter und bekommen den Spinner zusätzlich.
- **Protokoll und Downloads**: Sie hatten schon eigene Fehleranzeigen im Inhalt und behalten sie.
- **Cover-Bilder**: Sie laden mit `loading="lazy"` und fallen bei Fehlern auf die Ersatzkachel zurück. Das ist bereits ein eigener Ladezustand.

## Tests

`e2e/feedback.spec.ts` (Playwright) prüft mit simulierten langsamen, fehlerhaften und abgebrochenen Antworten:

- Knopfzustand und Größe
- Doppelklick und Enter halten ergeben nur eine Anfrage
- Erfolg und Fehler am Knopf
- Fehlermeldung bleibt stehen und lässt sich schließen
- Netzabbruch ohne automatische Wiederholung
- Platzhalter, „Erneut versuchen“ und das Ergebnis danach
- Stapel und Ablauf der Meldungen
- Neuverbindung der Live-Verbindung nach 503
- Handy-Breite und Tastatur

Die bisherigen Klick-Tests laufen unverändert weiter.
