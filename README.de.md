# Better Renamer

[English](README.md) · **Deutsch**

Eine selbst gehostete Web-App für dein NAS (Unraid oder jeder andere Docker-Host) mit zwei Aufgaben:

* **Hörbuch-Automatik** – Hörbücher in einen Eingangsordner legen; sie werden zu **einer M4B mit Kapiteln**,
  bei Audible erkannt, **getaggt** und sauber in die Bibliothek für Audiobookshelf, Plex & Co. **einsortiert**.
* **Stapel-Umbenenner** – Hunderte Dateien und Ordner im Browser umbenennen, mit Live-Vorschau, 16 kombinierbaren
  Regeln und Metadaten-Platzhaltern (ID3, Video, EXIF).

Downloads laufen über **SABnzbd**, Benachrichtigungen über **ntfy** – du bestimmst, bei welchen Ereignissen.
Die Oberfläche gibt es auf **Deutsch und Englisch**.

> Better Renamer ist ein Werkzeug zum Ordnen von Audiodateien, die du nutzen darfst – eigene Käufe, eigene
> Aufnahmen oder gemeinfreie Werke (z. B. von [LibriVox](https://librivox.org)). Es enthält keine Medien,
> umgeht keinen Kopierschutz und steht in keiner Verbindung zu Audible, Amazon, Apple oder anderen hier genannten Diensten.

---

## Inhalt

1. [Funktionen](#funktionen)
2. [Schnellstart](#schnellstart)
3. [Hörbücher](#hörbücher)
4. [Downloads mit SABnzbd](#downloads-mit-sabnzbd)
5. [Benachrichtigungen (ntfy)](#benachrichtigungen-ntfy)
6. [Stapel-Umbenenner](#stapel-umbenenner)
7. [Konfiguration](#konfiguration)
8. [Sicherheit](#sicherheit)
9. [Entwicklung](#entwicklung)

---

## Funktionen

| Bereich | Was du bekommst |
|---|---|
| Hörbuch-Eingang | Wartet, bis ein Ordner fertig kopiert ist, fügt MP3/M4A/FLAC/OGG/Opus/WMA (auch `CD1`/`CD2`-Unterordner) zu einer M4B mit sample-genauen Kapiteln zusammen |
| Erkennung | Audible-Suche in 10 Shops, bewertet nach Titel, Autor **und Laufzeit** (Lesung ≠ gekürzt ≠ Hörspiel); unsichere Treffer warten auf dich |
| Tags & Dateien | Titel, Autor, Sprecher, Serie + Band, Genre, Verlag, Datum, Beschreibung, ASIN, Cover in voller Größe; `metadata.json` für Audiobookshelf |
| Bibliothek | Ordner-Vorlage, nutzt vorhandene Autoren-/Serienordner, erkennt Doppelte, macht alte MP3-Bücher zu M4B |
| Downloads | Schickt NZBs an SABnzbd, beobachtet die Kategorie, nach dem Download übernimmt der Eingang; Newznab-Suche optional |
| Benachrichtigungen | ntfy-Push-Nachrichten, zehn Ereignisse, jedes einzeln an oder aus |
| Umbenenner | Live-Vorschau, Konfliktprüfung, Rückgängig mit Verlauf, Voreinstellungen, Unterordner, Begleitdateien wandern mit |
| Oberfläche | Deutsch / Englisch, helles / dunkles Design, Live-Updates ohne Neuladen |
| Sicherheit | Eigenes schlankes ffmpeg 9 im Landlock-Käfig, CSRF-Schutz, Zugangsdaten erreichen nie den Browser ([SECURITY.de.md](SECURITY.de.md)) |

---

## Schnellstart

### Unraid

1. **Docker → Add Container**, bei *Template* eintragen:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/better-renamer.xml`
2. Pfade setzen:
   * **Medien** (`/data`) – die Shares, die der Umbenenner anfassen darf, z. B. `/mnt/user/media/`
   * **Hörbuch-Eingang** (`/inbox`) – hier kommen neue Hörbücher an
   * **Hörbuch-Bibliothek** (`/audiobooks`) – die fertige Bibliothek (in Audiobookshelf/Plex einbinden)
3. **Language / Sprache** wählen (`APP_LANGUAGE`: `de` oder `en`). PUID `99`, PGID `100`, UMASK `000` passen für Unraid schon.
4. `http://<unraid-ip>:8080` öffnen.

Image: `ghcr.io/larosen/rename_merge:latest` (amd64 + arm64, wöchentlich für Sicherheitsupdates neu gebaut).

### Andere Docker-Hosts

```bash
# in docker-compose.yml Pfade, PUID/PGID (siehe `id`) und APP_LANGUAGE anpassen
docker compose up -d
```

---

## Hörbücher

### Vom Eingang in die Bibliothek

```
/inbox                                              /audiobooks
├── Storm - Der Schimmelreiter/               ──►   ├── Theodor Storm/
│   ├── CD1/01.mp3 …                                │   └── Der Schimmelreiter/
│   └── CD2/01.mp3 …                                │       ├── Der Schimmelreiter.m4b   (Kapitel, Tags, Cover)
├── Doyle - The Hound of the Baskervilles.m4b ──►   │       ├── cover.jpg · desc.txt · reader.txt
├── _fertig/   verarbeitete Originale               │       └── metadata.json
└── _failed/   mit FEHLER.txt                       └── Arthur Conan Doyle/Sherlock Holmes/03 - The Hound of the Baskervilles/…
```

1. **Abwarten.** Jeder Ordner bzw. jede Datei im Eingang ist ein Buch. Verarbeitet wird erst, wenn sich eine Weile
   nichts mehr ändert (`SETTLE_SECONDS`, Standard 2 min) und keine `.part`/`.crdownload`-Dateien mehr da sind.
   Ein Ordner mit mehreren Buch-Ordnern (`Reihe/Band 1`, `Reihe/Band 2`) wird zu mehreren Büchern.
2. **M4B erzeugen.** Jede Datei wird parallel zu AAC kodiert und dann ohne Neukodierung zusammengefügt – ein Kapitel
   pro Datei (oder die Kapitel, die eine Datei schon hat). AAC-Quellen werden kopiert, eine einzelne M4B wird übernommen.
   Eigene Kapitel: `chapters.txt` (`00:01:02.500 Titel`) in den Buchordner legen.
3. **Bei Audible erkennen.** Gesucht wird nach Ordnername bzw. Album/Interpret-Tags im gewählten Shop
   (`AUDIBLE_REGION`); eine ASIN im Namen (`… [B0ABCDE123]`) wird direkt verwendet. Nur eindeutige Treffer werden
   automatisch abgelegt. Alles andere wartet unter **Zu prüfen**, dort kannst du
   * einen Treffer übernehmen oder vorher **bearbeiten**,
   * neu suchen, auch in einem anderen Audible-Shop,
   * einen **Link von Audible, BookBeat oder Thalia** einfügen – die Angaben werden automatisch geholt,
   * oder alles selbst eintragen.

   Bücher, die es nirgends gibt: eine `metadata.yml` (Format von beets-audible) in den Ordner legen.
4. **Taggen & ablegen.** Kapitelnamen kommen von Audible, wenn die eigenen nur „Track 01“ heißen und die Laufzeit
   passt. Der Pfad in der Bibliothek folgt `FOLDER_TEMPLATE`, Standard `{author}/[{series}/][{series_position:2} - ]{title}` –
   `[ … ]` entfällt, wenn leer, `:2` macht aus Band 3 „03“. Vorhandene Ordner werden auch bei anderer Schreibweise
   wiederverwendet (`E. T. A. Hoffmann` = `E.T.A. Hoffmann`, `Storm, Theodor` = `Theodor Storm`, `Mueller` = `Müller`).
   Originale wandern nach `_fertig` (oder werden gelöscht), Fehlschläge nach `_failed` mit Beschreibung und
   **Erneut versuchen**-Knopf.

Falsch einsortiert? **Korrigieren** holt das Buch aus der Bibliothek zurück in die Prüf-Liste.

### Doppelte vermeiden

Die Bibliothek wird beim Start und alle paar Stunden eingelesen (`LIBRARY_RESCAN_HOURS`) – auch ältere MP3-Ordner,
erkannt über ASIN, Tags oder die Ordnerstruktur. Ist ein neues Buch schon da (gleiche ASIN, gleicher Autor + Titel
oder gleiche Serie + Folge), entscheidest du: **Verwerfen**, **Altes ersetzen** (die alte Fassung wandert nach
`_ersetzt` im Eingang) oder **Trotzdem zusätzlich ablegen**. `DUPLICATES=skip` oder `keep` entscheidet automatisch.

### Alte MP3-Bücher zu M4B machen

**Hörbücher → Nicht als M4B** zeigt alle Bücher der Bibliothek, die noch aus Einzeldateien bestehen. Bücher auswählen,
**Zu M4B** klicken, vorher bei Bedarf die Kapitelnamen anpassen (**Kapitel & Namen …**, benennt auf Wunsch auch die
Originaldateien in `01 - Kapitel.mp3` um). Die alte Fassung wird erst weggeräumt, wenn die neue M4B getaggt ist.

### Weitere Metadaten-Quellen

Ist Audible nicht erreichbar und die ASIN bekannt, kommen dieselben Daten von **Audnexus**. Findet Audible nichts,
werden **Apple Books** und **Open Library** gefragt. Diese Treffer haben weder Laufzeit noch Sprecher und werden daher
nie automatisch abgelegt. Abschalten mit `METADATA_FALLBACK=false`.

### Audiobookshelf

* Neben jedem Buch liegt eine `metadata.json` im Format von Audiobookshelf (Titel, Autoren, Sprecher, Serie, Genres,
  Beschreibung, ASIN, Kapitel) – `WRITE_ABS_METADATA=false` schaltet sie ab.
* Optional unter **Suchen → Einrichten**: Adresse und API-Token von Audiobookshelf. Nach dem Ablegen neuer Bücher wird
  die Bibliothek dort neu eingelesen – hilfreich auf Netzwerk-Shares, wo die Ordnerüberwachung nichts mitbekommt.

---

## Downloads mit SABnzbd

Du brauchst nur SABnzbd; die Suche ist optional.

1. In SABnzbd eine Kategorie anlegen, z. B. `audiobooks`, deren Ordner der als **Hörbuch-Eingang** (`/inbox`) eingebundene ist.
2. In Better Renamer: **Suchen → Einrichten** → SABnzbd-Adresse, API-Key (SABnzbd → Config → General) und Kategorie.
3. Alles, was SABnzbd in dieser Kategorie fertig lädt, läuft automatisch durch M4B → Audible → Bibliothek. Die Liste
   **Downloads** zeigt Fortschritt und Verlauf der Kategorie.

**Optionale Suche:** Mit einem Newznab-Indexer (Adresse + API-Key) kannst du direkt im Reiter **Suchen** suchen und
Ergebnisse an SABnzbd schicken. Ergebnisse werden zusammengeführt, sortiert (bevorzugte Sprache, ungekürzt vor gekürzt,
M4B vor MP3) und mit deiner Bibliothek abgeglichen. Umlaute werden in allen Schreibweisen gesucht (`ä` / `ae` / `a`).
**Bei Audible nachschlagen** sucht vorher mit dem genauen Titel aus dem Shop und merkt sich die ASIN fürs Taggen.

Tipp: In SABnzbd unter *Config → Switches → Unerwünschte Endungen* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
auf **Abbrechen** stellen.

---

## Benachrichtigungen (ntfy)

Push-Nachrichten aufs Handy über [ntfy](https://ntfy.sh) – ntfy.sh oder ein eigener Server.

1. ntfy-App installieren und ein Topic abonnieren, z. B. `hoerbuecher-1234`.
2. **⚙︎ Einstellungen → Benachrichtigungen (ntfy)**: Server, Topic, optional Access-Token (oder Benutzer + Passwort)
   und Priorität. Mit *Adresse dieser Oberfläche* öffnet ein Tippen auf die Nachricht Better Renamer.
3. Gewünschte Ereignisse anhaken und **Testnachricht senden**.

| Ereignis | Standard |
|---|---|
| Hörbuch abgelegt | an |
| Hörbuch braucht Prüfung | an |
| Schon in der Bibliothek | an |
| Fehler bei einem Hörbuch | an |
| Download an SABnzbd geschickt | aus |
| Download in SABnzbd fertig | aus |
| Download in SABnzbd fehlgeschlagen | an |
| „Jetzt zu M4B“ fertig | aus |
| „Jetzt zu M4B“ fehlgeschlagen | an |
| Audiobookshelf-Scan fehlgeschlagen | an |

Token und Passwort bleiben auf dem Server und werden bei geänderter Adresse nicht an einen anderen Server geschickt.
Nachrichten kommen in der Sprache der Oberfläche.

---

## Stapel-Umbenenner

1. Links den Ordner wählen. Optionen: **Unterordner**, **Ordnernamen**, **Begleitdateien**, **„/“ = Ordner**,
   **Konflikte nummerieren**.
2. Regeln hinzufügen – sie wirken der Reihe nach; die Vorschau hebt jede Änderung hervor.
3. Auswahl per Checkbox (Shift-Klick für Bereiche), Typ-Chips (Audio/Video/Bilder …) oder Filter.
4. **Umbenennen** (⌘/Strg + Enter). **Rückgängig** (⌘/Strg + Z) oder über den **Verlauf** (die letzten 50 Vorgänge).

Beispiel mit einer gemeinfreien Aufnahme: `der_schimmelreiter_kapitel1.mp3` → *Aufräumen* (`_` → Leerzeichen),
*Groß-/Kleinschreibung* (Titel) und *Zahlen auffüllen* → `Der Schimmelreiter Kapitel01.mp3`. Mit der Vorlage-Regel
`{track:2} - {title}` kommt der Name direkt aus den ID3-Tags.

### Regeln

| Gruppe | Regel | Was sie tut |
|---|---|---|
| Text | Text ersetzen | Suchen/Ersetzen, alle/erstes/letztes Vorkommen, ganzes Wort, Regex mit `$1`/`\1` |
| | Text einfügen | Am Anfang, Ende, an Position oder vor/nach einem Text; Platzhalter erlaubt |
| | Bereich entfernen | N Zeichen ab Position, oder alles vor/nach einem Text |
| | Groß-/Kleinschreibung | Titel, Jedes Wort, Satz, GROSS, klein, umkehren; Ausnahmen wie „MP3, DJ“ |
| | Kürzen | Auf N Zeichen, wahlweise an Wortgrenze oder vom Ende |
| Zeichen | Aufräumen | `_ . %20` → Leerzeichen, Klammern/führende Ziffern weg, Umlaute → ae/oe/ue, Windows/SMB-sicher, trimmen |
| | Zeichen/Wörter entfernen | Ziffern, Buchstaben, Satzzeichen, Nicht-ASCII, eigene Zeichen, Wortliste |
| Position | Text verschieben | Treffer (auch Regex) an den Anfang/das Ende |
| | Teile tauschen | „Künstler - Titel“ → „Titel - Künstler“ |
| Nummern | Nummerieren | 1/a/A/I/i, Start, Schritt, Stellen, Präfix/Suffix, Position, pro Ordner neu |
| | Zahlen auffüllen | „Track 1“ → „Track 01“ |
| | Zahlen verschieben | Vorhandene Zahlen um ±N ändern |
| Datum & Metadaten | Datum einfügen | Aufnahme (EXIF/Video/ID3), Änderung, Erstellung, heute – mit frei wählbarem Format |
| | Vorlage / Metadaten | Name aus Platzhaltern bauen, z. B. `{track:2} - {title}` oder `{show} S{season:2}E{episode:2}` |
| | Aus Liste | Eingefügte Liste: Zeile 1 für die erste Datei, Zeile 2 für die zweite … |
| Endung | Dateiendung | klein/GROSS, vereinheitlichen (jpeg → jpg), ersetzen, anhängen, entfernen |

Jede Regel kann eine **Bedingung** haben: nur anwenden, wenn der Name etwas enthält, auf eine Regex passt oder eine
bestimmte Endung hat (auch umkehrbar).

### Platzhalter

| Platzhalter | Bedeutung |
|---|---|
| `{name}` `{original}` `{ext}` | aktueller Name, Originalname, Endung |
| `{n}` `{n:3}` `{folder_n}` | laufende Nummer (aufgefüllt), Nummer innerhalb des Ordners |
| `{folder}` `{parent}` | Ordner der Datei, Ordner darüber |
| `{modified:%Y-%m-%d}` `{created:…}` `{taken:…}` `{now:…}` | Datumswerte mit beliebigem Format |
| `{size}` `{bytes}` | Dateigröße |
| `{title}` `{artist}` `{album}` `{album_artist}` `{track:2}` `{disc}` `{year}` `{genre}` `{composer}` `{duration}` `{bitrate}` | Audio |
| `{resolution}` `{width}` `{height}` `{codec}` `{fps}` `{show}` `{season}` `{episode}` | Video |
| `{camera}` `{make}` `{model}` `{lens}` `{iso}` `{fnumber}` `{exposure}` `{focal}` `{dimensions}` | Foto (JPEG, HEIC, RAW) |

Formate hinter dem Doppelpunkt: Zahl = mit Nullen auffüllen, `upper` `lower` `title` `ascii`, `0-20` = Ausschnitt,
bei Datumswerten ein strftime-Format. Das ⓘ neben einer Datei zeigt alle Platzhalter mit ihren Werten.

**Jetzt zu M4B** im Umbenenner fügt die MP3s des geöffneten Ordners dort zu `<Ordner>.m4b` zusammen;
**→ Bibliothek** schickt den Ordner durch die Hörbuch-Automatik und lässt die Originale unangetastet.

---

## Konfiguration

Fast alles lässt sich unter **⚙︎ Einstellungen** ändern; die Umgebungsvariablen liefern nur die Startwerte.
Einstellungen, Voreinstellungen, Verlauf, Hörbuch-Liste, Bibliotheks-Index und alle Zugangsdaten liegen in der
SQLite-Datenbank `/config/better-renamer.db` (Rechte 600).

| Variable | Standard | Bedeutung |
|---|---|---|
| `APP_LANGUAGE` | `de` | Oberfläche und Benachrichtigungen: `de` oder `en` |
| `PUID` / `PGID` | `99` / `100` | Benutzer/Gruppe für alle Dateizugriffe |
| `UMASK` | `000` | Rechte neuer Dateien und Ordner |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Hörbuch-Eingang und -Bibliothek |
| `AUDIBLE_REGION` | `de` | Audible-Shop: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (eindeutige automatisch), `review` (immer prüfen), `off` (nur konvertieren) |
| `MATCH_THRESHOLD` | `0.8` | Mindest-Sicherheit für automatisches Ablegen |
| `FOLDER_TEMPLATE` | `{author}/[{series}/][{series_position:2} - ]{title}` | Ordner in der Bibliothek |
| `FILE_TEMPLATE` | `{title}` | Dateiname der M4B |
| `CHAPTERS` | `auto` | Kapitelnamen von Audible: `auto`, `audible`, `files` |
| `COVER_SOURCE` | `audible` | `audible` (aus dem Shop, volle Auflösung) oder `local` |
| `INBOX_ORIGINALS` | `archive` | Originale danach: `archive` (nach `_fertig`) oder `delete` |
| `SETTLE_SECONDS` | `120` | So lange muss ein Eingang-Ordner unverändert sein |
| `SCAN_INTERVAL` | `60` | Sekunden zwischen zwei Prüfungen des Eingangs |
| `METADATA_FALLBACK` | `true` | Audnexus, Apple Books und Open Library fragen, wenn Audible nichts findet |
| `WRITE_ABS_METADATA` | `true` | `metadata.json` für Audiobookshelf schreiben |
| `DUPLICATES` | `ask` | Buch schon in der Bibliothek: `ask`, `skip`, `keep` |
| `LIBRARY_RESCAN_HOURS` | `6` | Abstand, in dem die Bibliothek neu eingelesen wird |
| `LIBRARY_ORIGINALS` | `archive` | Alte Einzeldateien nach „Zu M4B“: `archive` (nach `_ersetzt`) oder `delete` |
| `NARRATOR_IN_ARTIST` | `false` | Sprecher zusätzlich im Interpreten-Tag |
| `KEEP_SERIES_IN_TITLE` | `true` | „…, Band 3“ im Titel lassen |
| `WRITE_DESC` / `WRITE_READER` / `WRITE_COVER` | `true` | `desc.txt`, `reader.txt`, `cover.jpg` schreiben |
| `CPU_CORES` | halbe Kernzahl | Parallele ffmpeg-Prozesse pro Buch |
| `ENCODE_WORKERS` | `1` | Bücher, die gleichzeitig kodiert werden |
| `IO_WORKERS` | `2` | Gleichzeitige Erkennungs-, Tag- und Ablageschritte |
| `NICE` | `10` | CPU-Priorität von ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg im Landlock-Käfig: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Speichergrenze je ffmpeg-Prozess |
| `AUTO_CONVERT` | `true` | „Danach MP3 → M4B“ im Umbenenner standardmäßig an |
| `AAC_BITRATE` | `auto` | `auto` (wie die Quelle, höchstens 128k) oder fester Wert |
| `FORCE_MONO` | `false` | Mono ausgeben |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` oder `tag` |
| `TITLE_SOURCE` | `folder` | `folder` oder `album` |
| `OUTPUT_NAME` | `{folder}` | Name der M4B im Umbenenner, Platzhalter `{folder} {album} {artist}` |
| `ORIGINALS` / `ORIGINALS_FOLDER` | `keep` / `_mp3` | MP3s nach „Jetzt zu M4B“: `keep`, `move`, `delete` |
| `SIDECARS` / `SIDECAR_EXTS` | `true` / `xmp,srt,…` | Begleitdateien wandern mit ihrer Datei |
| `CONFLICT` | `error` | `error` oder `number` („(2)“ anhängen) |
| `ALLOW_SUBDIRS` | `false` | „/“ legt standardmäßig Ordner an |
| `MAX_ITEMS` | `20000` | Obergrenze für Einträge in der Vorschau |
| `SHOW_HIDDEN` | `false` | Versteckte Dateien anzeigen |
| `AUTH_USER` / `AUTH_PASS` | – | Passwortschutz (Basic Auth) – empfohlen |
| `ALLOW_PUBLIC` | `false` | Ohne Passwort nur Antworten an private Netze; `true` hebt das auf |
| `ALLOWED_HOSTS` | – | Nur unter diesen Hostnamen/IPs antworten, z. B. `tower,192.168.1.10` |

SABnzbd, der optionale Indexer, Audiobookshelf und ntfy werden in der Oberfläche eingerichtet, nicht über Variablen.

---

## Sicherheit

Kurz gefasst – Details, Tests und Komponenten-Versionen in [SECURITY.de.md](SECURITY.de.md):

* `AUTH_USER`/`AUTH_PASS` setzen und die Oberfläche nie ohne Reverse-Proxy mit HTTPS ins Internet stellen.
  Ohne Passwort antwortet der Server nur privaten Netzen.
* Nur nötige Ordner einbinden; der Container läuft ohne Linux-Capabilities und mit `no-new-privileges`.
* Heruntergeladene Dateien werden nie ausgeführt. ffmpeg ist ein eigener schlanker Build (nur Audio, kein
  Netzwerk-Code) in einem Landlock-Käfig, der nur die Dateien des Buchs lesen darf.
* API-Keys, Token und Passwörter (SABnzbd, Indexer, Audiobookshelf, ntfy) bleiben auf dem Server und gehen nie an den Browser.

---

## Entwicklung

```bash
pip install -r requirements.txt                 # mutagen + PyYAML
npm ci && npm run build                         # TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests           # Python-Tests (brauchen ffmpeg, kein Netz)
npm run check                                   # Typprüfung
npx playwright install chromium && npx playwright test   # Durchklick-Tests gegen einen echten Server
DATA_DIR=/tmp/data INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
```

**Übersetzungen:** Deutsch ist die Ausgangssprache. Jeder Text der Oberfläche steht in `t("…")` (Python und
TypeScript); `app/static/i18n/en.json` enthält den englischen Text. `python3 tests/test_i18n.py --missing` listet
Texte ohne Übersetzung – die Tests schlagen fehl, solange einer fehlt.

Die CI prüft jeden Pull Request: Python-Tests (mit System-ffmpeg und mit dem eigenen ffmpeg 9 im Pflicht-Käfig),
Typprüfung, Durchklick-Tests gegen den Quelltext und gegen das fertig gebaute Docker-Image.

| Teil | Dateien |
|---|---|
| Start & Verdrahtung | `app/main.py`, `app/config.py`, `app/services.py` |
| Web | `app/web/server.py`, `app/web/router.py`, `app/web/routes/*` |
| Daten | `app/db.py` (SQLite), `app/stores.py` |
| Abläufe | `app/engine.py`, `app/pipeline.py`, `app/converter.py`, `app/events.py` |
| Audio | `app/encoder.py`, `app/sandbox.py`, `app/meta.py`, `docker/build-ffmpeg.sh` |
| Metadaten | `app/audible.py`, `app/providers.py`, `app/links.py`, `app/tagger.py`, `app/abs.py` |
| Bibliothek & Downloads | `app/library.py`, `app/library_index.py`, `app/usenet.py` |
| Benachrichtigungen | `app/notify.py` |
| Sprache | `app/i18n.py`, `app/static/i18n/en.json` |
| Umbenennen | `app/rules.py`, `app/planner.py` |
| Oberfläche | `frontend/src/*.ts`, `app/static/index.html`, `style.css`; Durchklick-Tests in `e2e/` |

Inspiriert von den Open-Source-Projekten [auto-m4b](https://github.com/seanap/auto-m4b) und
[beets-audible](https://github.com/Neurrone/beets-audible); Kapiteldaten über [Audnexus](https://github.com/laxamentumtech/audnexus).
