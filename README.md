# Better Renamer

Stapel-Umbenennen im Browser, angelehnt an *A Better Finder Rename* – plus eine **Hörbuch-Automatik**,
die [auto-m4b](https://github.com/seanap/auto-m4b) und [beets-audible](https://github.com/Neurrone/beets-audible)
in einem Container ersetzt: Hörbücher in den Eingang legen → **M4B mit Kapiteln** → **Audible-Tags** →
sauber einsortiert in die Bibliothek für Audiobookshelf, Plex & Co. Als Docker-Container für Unraid und
jeden anderen Docker-Host.

Zwei Reiter in der Oberfläche:

* **Umbenennen** – der Batch-Umbenenner (siehe unten)
* **Hörbücher** – Eingang, laufende Konvertierungen und die Prüf-Liste für unsichere Audible-Treffer
* **Suchen** – Newznab-Indexer durchsuchen und Downloads an SABnzbd schicken (wie Readarr)

## Hörbücher: Eingang → M4B → Audible → Bibliothek

```
/inbox                                  /audiobooks
├── Fitzek - Der Insasse/          ──►  ├── Sebastian Fitzek/
│   ├── CD1/01.mp3 …                    │   └── Der Insasse/
│   └── CD2/01.mp3 …                    │       ├── Der Insasse.m4b   (Kapitel, Tags, Cover)
├── Tolkien - Die Gefährten.m4b    ──►  │       ├── cover.jpg
├── _fertig/   (verarbeitete Originale) │       ├── desc.txt   (Beschreibung)
└── _failed/   (mit FEHLER.txt)         │       └── reader.txt (Sprecher)
                                        └── J.R.R. Tolkien/Herr der Ringe/1 - Die Gefährten/…
```

1. **Eingang überwachen.** Jeder Ordner bzw. jede Datei im Eingang ist ein Buch. Verarbeitet wird erst, wenn
   sich nichts mehr ändert (Standard: 2 Minuten) und keine `.part`/`.!qB`/`.crdownload`-Dateien mehr da sind –
   halb kopierte Downloads werden also nicht angefasst. Ein Ordner mit `CD1`/`CD2`/`Teil 1`-Unterordnern wird
   zu einem Buch, ein Ordner mit mehreren Buch-Ordnern (`Reihe/Band 1`, `Reihe/Band 2`) zu mehreren.
2. **M4B erzeugen.** Jede Datei wird parallel einzeln zu AAC kodiert, dann ohne Neukodierung zusammengefügt.
   Ein Kapitel pro Datei (oder die Kapitel, die eine Datei schon hat), Positionen sample-genau.
   MP3, M4A, M4B, FLAC, OGG, Opus, WMA … gemischt, mit unterschiedlichen Abtastraten und Mono/Stereo.
   AAC-Quellen werden kopiert statt neu kodiert, eine einzelne M4B wird nur übernommen.
   Eigene Kapitel: `chapters.txt` (Format `00:01:02.500 Titel`) in den Buchordner legen.
3. **Bei Audible erkennen.** Suche im gewählten Audible-Shop (Standard: audible.de) nach Ordnername bzw.
   Album/Interpret-Tags, eine ASIN im Namen (`[B0054JTDOG]`) wird direkt verwendet. Die Treffer werden nach
   Titel, Autor und **Laufzeit** bewertet – so wird zwischen Lesung, gekürzter Fassung und Hörspiel
   unterschieden. Nur eindeutige Treffer werden automatisch übernommen, alles andere landet unter
   **Zu prüfen**:
   * Treffer **übernehmen** oder vorher **bearbeiten** (z. B. falsche Serie entfernen),
   * neu suchen – auch in einem anderen Audible-Shop,
   * einen **Link von Audible, BookBeat oder Thalia** ins Suchfeld oder in *Selbst eintragen* einfügen:
     Titel, Autor, Sprecher, Serie/Band, Verlag, Laufzeit, Beschreibung und Cover werden automatisch geholt,
   * oder alles **selbst eintragen**.

   Thalia blockiert automatische Abrufe teilweise (von Rechenzentren aus immer); dann sagt die Oberfläche
   das, und die Daten lassen sich trotzdem von Hand eintragen.
   Ein falsch einsortiertes Buch holt **Korrigieren** aus der Bibliothek zurück in die Prüf-Liste.
   Bücher, die es nirgends gibt: `metadata.yml` wie bei beets-audible in den Ordner legen.
4. **Taggen & ablegen.** Titel, Autor, Sprecher, Serie + Band, Genre, Verlag, Erscheinungsdatum,
   Beschreibung, ASIN, Cover in voller Auflösung, Medientyp *Hörbuch*. Haben die eigenen Kapitel nur
   Namen wie „Track 01“ und passt die Laufzeit, kommen die Kapitelnamen von Audible
   (über [Audnexus](https://github.com/laxamentumtech/audnexus)). Dann ab in die Bibliothek nach
   Vorlage `{author}/[{series}/][{series_position:2} - ]{title}` – `[ … ]` entfällt, wenn leer, `:2` macht
   aus Band 1 „01“, damit die Bände richtig sortiert sind. Vorhandene Autoren- und Serienordner werden
   wiederverwendet, auch wenn sie etwas anders geschrieben sind (`J. R. R. Tolkien` = `J.R.R. Tolkien`,
   `Fitzek, Sebastian` = `Sebastian Fitzek`, `Mueller` = `Müller`, Groß/Klein egal).
   Die Originale wandern nach `_fertig` (oder werden gelöscht), bei Fehlern nach `_failed` mit `FEHLER.txt`;
   *Erneut versuchen* holt sie zurück.

Im Umbenenner gibt es zusätzlich **→ Bibliothek**: Der gerade geöffnete Ordner wird genauso verarbeitet,
die Originale bleiben dabei unangetastet.

### Doppelte vermeiden: Abgleich mit der Bibliothek

Die Bibliothek wird beim Start und danach alle paar Stunden eingelesen (*Hörbücher → Bibliothek → Einlesen*
geht jederzeit) – auch ältere Bestände als MP3-Ordner: erkannt über ASIN, Autor und Titel aus den Tags oder,
wenn die fehlen, über die Ordnerstruktur `Autor/Titel` bzw. `Autor/Serie/NN - Titel`. Schreibweisen wie
„Fitzek, Sebastian“ / „Sebastian Fitzek“ oder „Gefährten“ / „Gefaehrten“ gelten als gleich.

* **Beim Suchen:** Releases und Audible-Treffer, die schon in der Bibliothek sind, tragen „✓ vorhanden“;
  vor dem Download kommt dann eine Rückfrage.
* **Beim Ablegen:** Ist das Buch schon da (gleiche ASIN, gleicher Autor + Titel oder gleiche Serie + Folge),
  wird nicht still ein „(2)“ angelegt. Stattdessen landet es unter *Zu prüfen* mit drei Möglichkeiten:
  **Verwerfen**, **Altes ersetzen** (die alte Fassung wandert nach `_ersetzt` im Eingang, wird nicht
  gelöscht) oder **Trotzdem zusätzlich ablegen**. Mit `DUPLICATES=skip` werden Doppelte automatisch
  übersprungen, mit `keep` immer zusätzlich abgelegt.

### Alte Bestände nachträglich zu M4B machen

Unter *Hörbücher → Nicht als M4B* stehen alle Bücher der Bibliothek, die noch aus einzelnen Dateien
(MP3, M4A, FLAC …) bestehen – Ordner wie `CD1`, `CD2`, `Teil 1` zählen dabei zum selben Buch.

* **Auswählen** (einzeln oder *Alle*) und **Zu M4B**: Tracks werden zu einer M4B mit Kapiteln
  zusammengefügt, das Buch bei Audible gesucht, getaggt (Titel, Autor, Sprecher, Serie, Cover,
  Beschreibung) und wie neue Bücher abgelegt. Unsichere Treffer kommen wie gewohnt unter *Zu prüfen*.
* **Kapitel & Namen …** zeigt vorher alle Tracks mit Länge. Kapitelnamen lassen sich einzeln ändern oder
  per Vorlage setzen: *automatisch*, *aus Tags*, *Dateinamen*, *ohne Nummer*, *Kapitel 1, 2, 3 …*.
  Mit **Dateien vorher auch so umbenennen** heißen die Originaldateien danach `01 - Kapitelname.mp3`.
* Die alte Fassung wandert nach `_ersetzt` im Eingang (oder wird mit `LIBRARY_ORIGINALS=delete` gelöscht),
  erst nachdem die neue M4B fertig getaggt ist.

### Wenn Audible nichts findet: weitere Quellen

Audible ist die erste Wahl (Laufzeit, Sprecher, Serie, Kapitel). Ist Audible nicht erreichbar und die ASIN bekannt,
kommen dieselben Daten von **Audnexus**. Findet Audible gar nichts, wird bei **Apple Books** (Hörbuch-Shop des
eingestellten Landes, also mit deutschen Titeln) und **Open Library** gesucht. Diese Treffer haben weder Laufzeit noch
Sprecher und werden deshalb nie automatisch übernommen – sie landen unter *Zu prüfen*, mit der Quelle am Treffer.
Abschaltbar unter ⚙︎ Einstellungen oder mit `METADATA_FALLBACK=false`.

### Audiobookshelf

* Neben jedem Buch liegt eine `metadata.json` im Format von Audiobookshelf (Titel, Autoren, Sprecher, Serie mit Folge,
  Genres, Beschreibung, ASIN, Kapitel) – zusätzlich zu `desc.txt`, `reader.txt` und `cover.jpg`. Abschaltbar
  (`WRITE_ABS_METADATA=false`). Liegt eine solche Datei schon in einem Buchordner, wird das Buch daraus erkannt.
* Optional unter *Suchen → Einrichten*: Adresse und API-Token von Audiobookshelf. Nach dem Ablegen neuer Bücher
  (gesammelt, 30 s nach dem letzten) wird die Bibliothek in Audiobookshelf neu eingelesen – wichtig auf Unraid-Shares,
  wo die Ordnerüberwachung von Audiobookshelf neue Dateien oft nicht bemerkt. Gibt es nur eine Hörbuch-Bibliothek,
  wird sie beim Testen automatisch gewählt.

### Suchen & Download über SABnzbd (wie Readarr)

Im Reiter **Suchen** werden Newznab-Indexer durchsucht (direkt, z. B. NZBgeek/NZBFinder/DrunkenSlug,
oder über **Prowlarr** – dort pro Indexer die Newznab-URL `http://prowlarr:9696/<id>/` mit dem Prowlarr-API-Key).
Standard-Kategorie ist `3030` (Audio › Hörbuch), pro Indexer änderbar.

* Ergebnisse aller Indexer zusammengeführt, Dubletten entfernt, sortiert nach Treffergenauigkeit,
  bevorzugter Sprache, ungekürzt vor gekürzt, M4B vor MP3, Grabs. Format, Sprache, Bitrate, „gekürzt“
  und „Hörspiel“ werden aus dem Release-Namen erkannt.
* **Deutsche Titel:** Anders als Readarr (englische Goodreads-Daten) wird mit genau dem gesucht, was du
  eingibst – und jeder Begriff mit Umlauten zusätzlich als `ae/oe/ue/ss` und ohne Umlaut-Punkte
  („Gefährten“, „Gefaehrten“, „Gefahrten“). Der Abgleich der Ergebnisse ignoriert diese Schreibweisen ebenfalls.
* **Bei Audible nachschlagen:** Buch erst im eingestellten Audible-Shop (Standard audible.de) suchen und
  auswählen – dann wird mit dem deutschen Titel und Autor gesucht, und die ASIN wird beim Download gemerkt,
  sodass das Buch nach dem Download ohne Rückfrage richtig getaggt wird.
* **Download** schickt die NZB an SABnzbd in die eingestellte Kategorie; Fortschritt und Verlauf dieser
  Kategorie stehen darunter.
* **Einrichtung:** In SABnzbd eine Kategorie (z. B. `audiobooks`) anlegen, deren Zielordner der Ordner ist,
  der in Better Renamer als **Hörbuch-Eingang** (`/inbox`) eingebunden ist. Fertige Downloads laufen dann
  automatisch durch M4B → Audible → Bibliothek. Der Suchbegriff wird gemerkt und hilft bei kryptischen
  Release-Namen der Audible-Erkennung.
* API-Keys liegen nur in der Datenbank `/config/better-renamer.db` (Rechte 600) und werden nie an den Browser geschickt,
  auch die NZB-Links (die den Indexer-Key enthalten) nicht.

### Was gegenüber auto-m4b und beets-audible besser ist

| | auto-m4b / beets-audible | Better Renamer |
|---|---|---|
| Container | zwei Container, m4b-tool (PHP) + selbst gebautes ffmpeg, beets + Plugin | einer, Python 3.15 + eigenes schlankes ffmpeg 9 im Landlock-Käfig |
| Halb kopierte Ordner | werden sofort verarbeitet | erst wenn alles fertig kopiert ist |
| Warnungen | „Non-monotonic DTS“, „Estimating duration from bitrate“, ungenaue Kapitel | keine – AAC-Segmente werden roh aneinandergehängt, Kapitel per Paketzählung |
| Gemischte Quellen | Probleme bei anderen Abtastraten/Formaten | jede Datei wird einzeln gewandelt |
| Taggen | `beet import` in der Konsole, interaktiv | automatisch; Rückfragen in der Weboberfläche |
| Falscher Treffer | Titel/Autor-Abgleich | zusätzlich Laufzeit-Abgleich (Lesung ≠ Hörspiel ≠ gekürzt) |
| Fehlschläge | bleiben liegen und werden immer wieder probiert | `_failed` mit Fehlerbeschreibung, Knopf zum Wiederholen |

* Live-Vorschau mit hervorgehobenen Änderungen, Konfliktprüfung vor dem Ausführen
* 16 Regeltypen, beliebig kombinierbar, per Drag & Drop sortierbar, jede Regel optional mit Bedingung
* Metadaten als Platzhalter: ID3-Tags, Video-Infos (Auflösung, Codec, Staffel/Folge), Foto-EXIF (Kamera, Aufnahmedatum)
* Unterordner rekursiv, Ordnernamen selbst, Begleitdateien (`.srt`, `.xmp`, `.nfo` …) wandern automatisch mit
* „/“ im neuen Namen legt Ordner an – z. B. Fotos nach `{taken:%Y}/{taken:%m}/…` einsortieren
* Mehrstufiges Rückgängig mit Verlauf, Voreinstellungen inkl. Export/Import

## Unraid

**Image:** `ghcr.io/larosen/rename_merge:latest` (wird bei jedem Push auf `main` von GitHub Actions gebaut, amd64 + arm64).

1. In Unraid unter **Docker → Add Container** bei *Template* die URL eintragen:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/better-renamer.xml`
   – oder die Datei `unraid/better-renamer.xml` nach `/boot/config/plugins/dockerMan/templates-user/` kopieren.
2. **Medien** auf die Shares setzen, die umbenannt werden dürfen (z. B. `/mnt/user/media/`),
   **Hörbuch-Eingang** und **Hörbuch-Bibliothek** auf die Hörbuch-Ordner (die Bibliothek in Audiobookshelf/Plex einbinden).
3. PUID `99`, PGID `100`, UMASK `000` stehen schon richtig – so bleiben neue Dateien über SMB beschreibbar.
4. WebUI öffnen: `http://<unraid-ip>:8080`

> **Privates Repository:** Solange das Repo privat ist, kann Unraid weder das Template noch das Icon
> per URL laden, und das Image auf ghcr.io ist ebenfalls privat. Entweder das Package auf GitHub unter
> *Packages → rename_merge → Package settings* auf **Public** stellen, oder auf Unraid einmal
> `docker login ghcr.io` mit einem Token (Recht `read:packages`) ausführen und das Template manuell kopieren.

## Andere Docker-Hosts

```bash
# in docker-compose.yml Medienpfad, PUID/PGID (siehe `id`) anpassen
docker compose up -d --build
```

## Bedienung

1. Links den Ordner wählen. Optionen über der Vorschau: **Unterordner** (rekursiv), **Ordnernamen**
   (Ordner selbst umbenennen), **Begleitdateien**, **„/“ = Ordner**, **Konflikte nummerieren**.
2. Regeln hinzufügen – sie wirken der Reihe nach. Die Vorschau zeigt Änderungen gelb markiert.
3. Auswahl per Checkbox (Shift-Klick für Bereiche), Typ-Chips (Audio/Video/Bilder …) oder Filter.
4. **Umbenennen** (⌘/Strg + Enter). Mit „Danach MP3 → M4B“ startet für jeden betroffenen Ordner die Konvertierung.
5. **Rückgängig** (⌘/Strg + Z) oder über den **Verlauf** (die letzten 50 Vorgänge).

Das ⓘ neben einer Datei zeigt alle Platzhalter mit ihren Werten für genau diese Datei.

**Live:** Fortschritt, neue Bücher, Warteschlangen und Bibliothek kommen per Server-Sent Events sofort in den Browser –
ohne ständiges Nachfragen. Reißt die Verbindung ab, fragt die Seite übergangsweise wieder regelmäßig nach.

**Hell/Dunkel:** Der Knopf ◐ oben rechts wechselt zwischen *Automatisch* (folgt dem System), *Hell* und *Dunkel*; die Wahl merkt sich der Browser.

### Regeln

| Gruppe | Regel | Was sie tut |
|---|---|---|
| Text | Text ersetzen | Suchen/Ersetzen, alle/erstes/letztes Vorkommen, ganzes Wort, Regex mit `$1`/`\1` |
| | Text einfügen | Am Anfang, Ende, an Position oder vor/nach einem Text; Platzhalter erlaubt |
| | Bereich entfernen | N Zeichen ab Position, oder alles vor/nach einem Text |
| | Groß-/Kleinschreibung | Titel (Füllwörter de/en klein), Jedes Wort, Satz, GROSS, klein, umkehren; Ausnahmen wie „MP3, DJ“ |
| | Kürzen | Auf N Zeichen, wahlweise an Wortgrenze oder vom Ende |
| Zeichen | Aufräumen | `_ . %20` → Leerzeichen, Klammern/führende Ziffern weg, Umlaute → ae/oe/ue, Windows/SMB-sicher, trimmen |
| | Zeichen/Wörter entfernen | Ziffern, Buchstaben, Satzzeichen, Nicht-ASCII, eigene Zeichen, Wortliste („1080p, x264“) |
| Position | Text verschieben | Treffer (auch Regex) an den Anfang/das Ende |
| | Teile tauschen | „Künstler - Titel“ → „Titel - Künstler“ |
| Nummern | Nummerieren | 1/a/A/I/i, Start, Schritt, Stellen, Präfix/Suffix, Position, pro Ordner neu |
| | Zahlen auffüllen | „Track 1“ → „Track 01“ |
| | Zahlen verschieben | Vorhandene Zahlen um ±N ändern (Folgen umnummerieren) |
| Datum & Metadaten | Datum einfügen | Aufnahme (EXIF/Video/ID3), Änderung, Erstellung, heute – mit frei wählbarem Format |
| | Vorlage / Metadaten | Name komplett aus Platzhaltern bauen, z. B. `{track:2} - {title}` oder `{show} S{season:2}E{episode:2}` |
| | Aus Liste | Eingefügte Liste: Zeile 1 für die erste Datei, Zeile 2 für die zweite … |
| Endung | Dateiendung | klein/GROSS, vereinheitlichen (jpeg → jpg), ersetzen, anhängen, entfernen |

Jede Regel kann eine **Bedingung** haben: nur anwenden, wenn der Name etwas enthält, auf eine Regex passt
oder eine bestimmte Endung hat (auch umkehrbar).

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

Formate hinter dem Doppelpunkt: Zahl = mit Nullen auffüllen, `upper` `lower` `title` `ascii`,
`0-20` = Ausschnitt, bei Datumswerten ein strftime-Format (`%Y` `%m` `%d` `%H` `%M` `%S` `%B`).
Dateien, denen die verwendeten Tags fehlen, bleiben bei der Vorlage-Regel unverändert.

## „Jetzt zu M4B“ im Umbenenner

* Alle `.mp3` direkt im Ordner, natürlich sortiert (`2` vor `10`), Kapitelname = Dateiname (oder ID3-Titel).
* Gleicher Encoder wie bei den Hörbüchern, Ergebnis `<Ordnername>.m4b` im selben Ordner, erst nach Erfolg sichtbar.
* Cover aus `cover.jpg`/`folder.jpg`/`front.jpg`, sonst das eingebettete Bild der ersten MP3.
* ffmpeg läuft mit niedriger CPU-Priorität (`NICE`), damit Plex & Co. nicht ausgebremst werden.
* ffmpeg 9 ist ein eigener schlanker Build (nur Audio, kein Netzwerk-Code) und läuft in einem Landlock-Käfig:
  es kommt nur an die Dateien des jeweiligen Buchs und das Arbeitsverzeichnis (siehe [SECURITY.md](SECURITY.md)).
  MPEG-PS-Videos (`.mpg`, `.vob`) liefern im Umbenenner deshalb keine Auflösung.
* Läuft über dieselbe Warteschlange wie die Hörbücher: es wird immer nur ein Buch gleichzeitig kodiert
  (`ENCODE_WORKERS`), und wartende Aufträge überstehen einen Neustart.

## Umgebungsvariablen

| Variable | Standard | Bedeutung |
|---|---|---|
| `PUID` / `PGID` | `99` / `100` | Benutzer/Gruppe für alle Dateizugriffe |
| `UMASK` | `000` | Rechte neuer Dateien und Ordner |
| `INBOX_DIR` / `LIBRARY_DIR` | `/inbox` / `/audiobooks` | Hörbuch-Eingang und -Bibliothek |
| `AUDIBLE_REGION` | `de` | Audible-Shop: `de us uk fr it es ca au in jp` |
| `MATCH_MODE` | `auto` | `auto` (eindeutige automatisch), `review` (immer prüfen), `off` (nur konvertieren) |
| `MATCH_THRESHOLD` | `0.8` | Mindest-Übereinstimmung für automatische Übernahme |
| `FOLDER_TEMPLATE` | `{author}/[{series}/][{series_position:2} - ]{title}` | Ordner in der Bibliothek |
| `FILE_TEMPLATE` | `{title}` | Dateiname der M4B |
| `CHAPTERS` | `auto` | Kapitel von Audible: `auto` (wenn eigene nur „Track 1“), `audible`, `files` |
| `COVER_SOURCE` | `audible` | `audible` (aus dem Shop – Audible/BookBeat, volle Auflösung) oder `local` |
| `INBOX_ORIGINALS` | `archive` | Originale danach: `archive` (nach `_fertig`) oder `delete` |
| `SETTLE_SECONDS` | `120` | So lange muss ein Eingang-Ordner unverändert sein |
| `METADATA_FALLBACK` | `true` | Audnexus, Apple Books und Open Library fragen, wenn Audible nichts liefert |
| `WRITE_ABS_METADATA` | `true` | `metadata.json` für Audiobookshelf neben jedes Buch schreiben |
| `DUPLICATES` | `ask` | Buch schon in der Bibliothek: `ask` (nachfragen), `skip`, `keep` |
| `LIBRARY_RESCAN_HOURS` | `6` | Abstand, in dem die Bibliothek neu eingelesen wird |
| `LIBRARY_ORIGINALS` | `archive` | Alte Einzeldateien nach „Zu M4B“ aus der Bibliothek: `archive` (nach `_ersetzt` im Eingang) oder `delete` |
| `SCAN_INTERVAL` | `60` | Sekunden zwischen zwei Prüfungen des Eingangs |
| `CPU_CORES` | halbe Kernzahl | Parallele ffmpeg-Prozesse |
| `ENCODE_WORKERS` | `1` | Bücher, die gleichzeitig kodiert werden (jedes mit `CPU_CORES` Prozessen) |
| `IO_WORKERS` | `2` | Gleichzeitige Erkennungs-, Tag- und Ablageschritte (laufen neben dem Kodieren) |
| `NARRATOR_IN_ARTIST` | `false` | Sprecher zusätzlich im Interpreten-Tag |
| `KEEP_SERIES_IN_TITLE` | `true` | „…, Band 3“ im Titel lassen |
| `WRITE_DESC` / `WRITE_READER` / `WRITE_COVER` | `true` | `desc.txt`, `reader.txt`, `cover.jpg` schreiben |
| `AUTO_CONVERT` | `true` | „Danach MP3 → M4B“ standardmäßig an |
| `AAC_BITRATE` | `auto` | `auto` = wie die Quelle (höchstens 128k, Mono 64k) oder fester Wert |
| `FORCE_MONO` | `false` | Mono ausgeben |
| `CHAPTER_TITLES` | `auto` | `auto` (ID3-Titel, sonst Dateiname ohne Nummer), `filename` oder `tag` |
| `TITLE_SOURCE` | `folder` | `folder` oder `album` |
| `OUTPUT_NAME` | `{folder}` | Name der M4B, Platzhalter `{folder} {album} {artist}` |
| `ORIGINALS` | `keep` | MP3s danach: `keep`, `move`, `delete` |
| `ORIGINALS_FOLDER` | `_mp3` | Unterordner für `move` |
| `NICE` | `10` | CPU-Priorität von ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg im Landlock-Käfig: `auto`, `require` (nur mit Käfig), `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Speichergrenze je ffmpeg-Prozess |
| `SIDECARS` | `true` | Begleitdateien standardmäßig mitnehmen |
| `SIDECAR_EXTS` | `xmp,srt,ass,…` | Endungen von Begleitdateien |
| `CONFLICT` | `error` | `error` oder `number` (automatisch „(2)“ anhängen) |
| `ALLOW_SUBDIRS` | `false` | „/“ legt standardmäßig Ordner an |
| `MAX_ITEMS` | `20000` | Obergrenze für Einträge in der Vorschau |
| `AUTH_USER` / `AUTH_PASS` | – | Passwortschutz (Basic Auth) – empfohlen |
| `ALLOW_PUBLIC` | `false` | Ohne Passwort nur Zugriffe aus privaten Netzen; `true` hebt das auf |
| `ALLOWED_HOSTS` | – | Optional: nur unter diesen Hostnamen/IPs antworten, z. B. `tower,192.168.1.10` |
| `SHOW_HIDDEN` | `false` | Versteckte Dateien anzeigen |

Einstellungen, Voreinstellungen, Verlauf, Hörbuch-Liste, Bibliotheks-Index und Quellen liegen in einer SQLite-Datenbank
`/config/better-renamer.db`. JSON-Dateien älterer Versionen werden beim ersten Start übernommen und als `*.json.migrated`
aufgehoben. Fast alles lässt sich auch in der Oberfläche unter ⚙︎ Einstellungen ändern.

> Ohne `AUTH_USER`/`AUTH_PASS` hat die Oberfläche keinen Login und antwortet nur Geräten aus dem Heimnetz.
> Was das Programm gegen bösartige Downloads, SSRF, CSRF & Co. tut und welche Einstellungen empfohlen sind,
> steht in [SECURITY.md](SECURITY.md).

## Entwicklung

```bash
pip install -r requirements.txt                # mutagen (M4B-Tags) + PyYAML (metadata.yml)
npm ci && npm run build                        # Oberfläche: TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests          # Python-Tests (brauchen ffmpeg, kein Netz)
npm run check                                  # Typprüfung (strict, ohne implizites any)
npx playwright install chromium && npx playwright test   # Durchklick-Tests gegen einen echten Server
DATA_DIR=/irgendein/ordner INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
```

Die CI prüft bei jedem PR: Python-Tests (einmal mit System-ffmpeg, einmal mit dem eigenen ffmpeg 9 im
Pflicht-Käfig), TypeScript-Typprüfung, Durchklick-Tests gegen den Server aus dem Quelltext und gegen das
fertig gebaute Docker-Image, gestartet mit den Rechten des Unraid-Templates.

**Aufbau**

| Teil | Dateien |
|---|---|
| Start & Verdrahtung | `app/main.py` (Einstieg), `app/config.py` (Umgebungsvariablen, Standard-Einstellungen), `app/services.py` (Dienste zusammenstecken) |
| Web | `app/web/server.py` (HTTP-Server, Sicherheitsprüfungen, Server-Sent Events), `app/web/router.py`, `app/web/routes/*` (API je Bereich) |
| Daten | `app/db.py` (SQLite, Schema-Versionen, Übernahme alter JSON-Dateien), `app/stores.py` (Einstellungen, Verlauf, Voreinstellungen) |
| Abläufe | `app/engine.py` (Warteschlangen encode/io), `app/pipeline.py` (Hörbuch: prepare → convert → finish), `app/converter.py` („Jetzt zu M4B“), `app/events.py` (Live-Updates) |
| Audio | `app/encoder.py` (M4B mit Kapiteln), `app/sandbox.py` (ffmpeg im Landlock-Käfig), `app/meta.py` (ffprobe & EXIF), `docker/build-ffmpeg.sh` (eigenes ffmpeg) |
| Metadaten | `app/audible.py` (Suche, Abgleich), `app/providers.py` (Audnexus, Apple Books, Open Library), `app/links.py` (Shop-Links), `app/tagger.py` (Tags, Begleitdateien), `app/abs.py` (Audiobookshelf) |
| Bibliothek & Download | `app/library.py` (Pfad-Vorlagen), `app/library_index.py` (Bestand), `app/usenet.py` (Newznab, SABnzbd) |
| Umbenennen | `app/rules.py` (Regeln & Platzhalter), `app/planner.py` (Plan, Konflikte, Ausführen, Rückgängig) |
| Oberfläche | `frontend/src/*.ts` (TypeScript), `app/static/index.html`, `style.css`; Tests in `e2e/` |
