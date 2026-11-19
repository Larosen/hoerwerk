# Hörbücher: vom Eingang in die Bibliothek

[← README](../../README.de.md) · [English](../en/guide.md) · **Deutsch**

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
   **Qualität** (`AUDIO_QUALITY`): Hörspiele (erkannt an Titel, Genre, Release-Name oder vielen Sprechern) bekommen
   mehr Bitrate als Lesungen – `standard` 128k/192k, `high` 160k/256k (Standard), `original` bis 320k und behält
   verlustfreies ALAC. Nie mehr, als die Quelle hat. Dolby Atmos / E-AC-3 wird immer unverändert übernommen, nie
   heruntergemischt (der Web-Player von Audiobookshelf spielt es evtl. nicht, die meisten Apps schon). Kapitel hinter
   dem Dateiende werden verworfen.
   Eigene Kapitel: `chapters.txt` (`00:01:02.500 Titel`) in den Buchordner legen.
3. **Bei Audible erkennen.** Gesucht wird nach Ordnername bzw. Album/Interpret-Tags im gewählten Shop
   (`AUDIBLE_REGION`); eine ASIN im Namen (`… [B0ABCDE123]`) wird direkt verwendet. Nur eindeutige Treffer werden
   automatisch abgelegt. Alles andere wartet unter **Zu prüfen**, dort kannst du
   * einen Treffer übernehmen oder vorher **bearbeiten**,
   * neu suchen, auch in einem anderen Audible-Shop,
   * einen **Link von Audible, BookBeat oder Thalia** einfügen – die Angaben werden automatisch geholt,
   * oder alles selbst eintragen.

   Bücher, die es nirgends gibt: eine `metadata.yml` (Format von beets-audible) in den Ordner legen.
4. **Tags schreiben & ablegen.** Kapitelnamen kommen von Audible, wenn die eigenen nur „Track 01“ heißen und die Laufzeit
   passt. Der Pfad in der Bibliothek folgt `FOLDER_TEMPLATE`, Standard `{author}/[{series}/][{series_position:2} - ]{title}` –
   `[ … ]` entfällt, wenn leer, `:2` macht aus Band 3 „03“. Vorhandene Ordner werden auch bei anderer Schreibweise
   wiederverwendet (`E. T. A. Hoffmann` = `E.T.A. Hoffmann`, `Storm, Theodor` = `Theodor Storm`, `Mueller` = `Müller`).
   Originale wandern nach `_fertig` (oder werden gelöscht), Fehlschläge nach `_failed` mit Beschreibung und
   **Erneut versuchen**-Knopf.

Falsch einsortiert? **Korrigieren** holt das Buch aus der Bibliothek zurück in die Prüf-Liste.

### Hochladen (z. B. von CD)

Hörbücher, die du selbst hast – etwa von CD eingelesen –, lädst du direkt im Browser hoch: *Hörbücher → Eingang →
**Hochladen***. Einen Ordner pro Hörbuch hineinziehen oder auswählen, am besten „Autor – Titel“, gern mit
Unterordnern `CD1`, `CD2` …; einzelne Dateien gehen auch (dann den Namen eintragen). Hörwerk übernimmt Audio-Dateien,
Cover, Cue-Sheets und `chapters.txt`/`metadata.yml`; alles andere wird übersprungen. Geprüft wird auch der
**Inhalt**: Eine in `.mp3` umbenannte `.exe`, ein PDF, ein Archiv oder ein Skript wird abgelehnt.

Die Dateien landen zuerst in einem versteckten Ordner und werden erst nach dem letzten in einem Schritt in den Eingang
verschoben – ohne Wartezeit beginnt dann der gewohnte Weg: Ordnername und Tags → Audible → M4B mit Kapiteln → Bibliothek.
Bis 4 GB pro Datei; abgebrochene Uploads räumt Hörwerk nach einem Tag weg. Nur für Admins.

### Bibliotheks-Ansicht

**Hörbücher → Bibliothek** zeigt alle Bücher der Bibliothek, mit Suche über Titel, Autor, Serie und Ordner. Jede Zeile
zeigt, wie das Buch gelesen wurde – Autor, weitere Namen aus Tags und Ordnern, Ordner und Format. Genau das nutzen auch
die Doppelt-Prüfung und die Suche. Cover kommen aus `cover.jpg`/`folder.jpg` im Buchordner oder aus dem Bild in der
Audiodatei; Bücher ohne Cover bekommen eine erzeugte Kachel.

**Aufräumen** (im Kasten *Bibliothek*) löscht `desc.txt` und `reader.txt` aus allen Buchordnern, in denen eine
`metadata.json` liegt – vorher zeigt es, wie viele es sind. Alles darin steht auch in `metadata.json` und in den Tags
der M4B; Plex liest die Dateien gar nicht, Audiobookshelf nur, wenn `metadata.json` fehlt. Hörbücher, `cover.jpg`
und Ordner ohne `metadata.json` bleiben unberührt, Symlinks werden nicht verfolgt. Neue Bücher bekommen die beiden
Dateien danach nicht mehr.

**Bibliothek prüfen** (Admins) schaut jedes Buch einmal an, rein lokal und ohne etwas zu verändern, und gibt ihm einen
Zustand von 0 bis 100 mit Befunden: Kapitelnamen, die den Titel wiederholen oder nichts sagen, keine Kapitel, Cover,
Jahr oder Sprecher fehlen, keine Audible-Verknüpfung, noch keine M4B. Beim ersten Mal startet die Prüfung von selbst,
danach per Knopf; sie läuft im Hintergrund, ein Buch nach dem anderen, und weicht neuen Büchern. Ein Antippen eines
Befunds zeigt nur die betroffenen Bücher, **Sortieren → Zustand** die schlechtesten zuerst. Behoben wird mit den
vorhandenen Knöpfen (*Kapitel* mit Vorschau, *Besser*, *Korrigieren*).

**Auswahl:** das Häkchen auf dem Cover wählt ein Buch, *Alle (N)* alle Treffer von Suche und Filter. Für die Auswahl gibt
es *Kapitel* (Vorschau), *Ergänzen* und *Löschen*.

**Cover** (Knopf an jedem M4B-Buch): ein eigenes Bild (JPEG/PNG) für Bücher, zu denen es öffentlich keins gibt. Es wird
`cover.jpg`/`cover.png` im Buchordner und das Bild in der M4B; alle anderen Tags bleiben.

**Daten ergänzen** (Auswahlleiste): zeigt je Buch, was Hörwerk ergänzen kann, wenn es sich bei der Fassung sicher ist
(ASIN oder enge Übereinstimmung von Titel, Autor und Länge): Jahr, Sprecher, Verlag, Serie, Beschreibung, Cover. Ein
Klick ergänzt alle; nur Leeres wird gefüllt, Vorhandenes bleibt.

Das Umschreiben von Tags, Kapiteln und Cover verschiebt oder benennt ein Buch nie um; die Zwischendatei ist beim
Schreiben versteckt, und *Neueste* behält das Datum, an dem ein Buch zuerst gesehen wurde.

**Audible-Daten** (unabhängig vom Zustand): *Audible-Daten laden* holt Hörer-Bewertung und Laufzeit der Bücher mit ASIN,
langsam im Hintergrund, und merkt sie einen Monat. Danach lässt sich die Bibliothek nach **Audible-Bewertung**
sortieren (schlechteste oder beste zuerst) – genau nach der angezeigten Zahl; Bewertungen mit weniger als 5 Stimmen
folgen den übrigen, Bücher ohne Bewertung stehen am Ende.
Weicht die Länge der Datei stark von Audibles Laufzeit ab (mehr als 8 % und 5 Minuten), erscheint der Befund
*Laufzeit weicht stark von Audible ab* – meist eine andere Fassung.

**Echte Kapitelnamen:** Würden beim Umbenennen nur „Kapitel 7“ übrig bleiben, sucht Hörwerk zuerst online: die Kapitel
von Audible (gleiche Länge ±2 %, gleiche Kapitelzahl, aussagekräftige Namen) oder die Titel der CD bei Apple Music
(gleiche Titelzahl, jedes Kapitel beginnt dort, wo der Titel davor endet, Gesamtlänge ±3 %). Erst danach wird nummeriert.
Die Vorschau zeigt, woher ein Name kommt.

### Downloads mit mehreren Bänden

Ein Download wie `Autor-Reihe 01 - 07-…` ist ein **Paket**: Jeder Band-Ordner wird ein eigenes Buch – einzeln
zusammengefügt, erkannt und getaggt, mit Autor und Band-Nummer aus dem Release-Namen. Ordner wie `01 - Titel`,
`02 - Titel` sind Bände; `CD1`/`CD2`, `Disc 1`, `Teil 1` oder nur `01`/`02` bleiben Teile eines Buchs.

* In der **Suche** zeigt ein Paket seine Bände und welche davon du schon hast. **Download** fragt, welche Bände du
  willst – vorhandene sind abgewählt. Nur die gewählten werden konvertiert, die anderen unkonvertiert weggeräumt.
* Pakete, die nicht über die Suche kamen: Bände, die schon in der Bibliothek sind, werden automatisch übersprungen –
  über Reihe und Band-Nummer, spätestens wenn Audible das Buch erkennt, immer vor dem Konvertieren.

### Doppelte vermeiden

Die Bibliothek wird beim Start und alle paar Stunden eingelesen (*Einstellungen → Eingang & Bibliothek*, dort auch
„Jetzt einlesen“ und das einmalige Aufräumen von `desc.txt`/`reader.txt`) – auch ältere MP3-Ordner,
erkannt über ASIN, Tags oder die Ordnerstruktur. Ist ein neues Buch schon da (gleiche ASIN, gleicher Autor + Titel
oder gleiche Serie + Folge), entscheidest du: **Verwerfen**, **Altes ersetzen** (die alte Fassung wandert nach
`_ersetzt` im Eingang) oder **Trotzdem zusätzlich ablegen**. `DUPLICATES=skip` oder `keep` entscheidet automatisch.

Der Abgleich ist bewusst tolerant: Sprecher im Interpreten-Tag, Album-Tags wie „Autor - Titel“, Ordner wie
„Autor - Titel“ und Release-Namen ohne Autor (bei Titeln ab zwei Wörtern) werden trotzdem erkannt. In der **Suche**
erscheint „Schon in deiner Bibliothek“ über den Ergebnissen, wenn der Suchbegriff zu einem vorhandenen Buch passt.

### Alte MP3-Bücher zu M4B machen

**Hörbücher → Nicht als M4B** zeigt alle Bücher der Bibliothek, die noch aus Einzeldateien bestehen. Bücher auswählen,
**Zu M4B** klicken, vorher bei Bedarf die Kapitelnamen anpassen (**Kapitel & Namen …**, benennt auf Wunsch auch die
Originaldateien in `01 - Kapitel.mp3` um).

**Alte Dateien danach** wählst du direkt neben dem Knopf: *nach `_ersetzt` im Eingang verschieben* (Standard) oder
*löschen*. In beiden Fällen wird nichts angefasst, bevor die neue M4B liegt: Der alte Ordner wird beiseitegelegt
(umbenannt, versteckt) und erst gelöscht bzw. verschoben, wenn das neue Buch abgelegt ist. Klappt das Ablegen nicht,
kommt er unverändert zurück.

Kapitelnamen unter *automatisch* werden für das ganze Buch aufgeräumt: Text, den jeder Track wiederholt (Buchtitel,
„(2012)“, „Titel - “), fällt weg, und Tracks, die nur nummeriert sind, werden zu „Kapitel 1, 2, 3 …“. Echte
Kapitelnamen bleiben.

### Weitere Metadaten-Quellen

Ist Audible nicht erreichbar und die ASIN bekannt, kommen dieselben Daten von **Audnexus**. Findet Audible nichts,
werden **Apple Books** und **Open Library** gefragt; deren Treffer haben weder Laufzeit noch Sprecher und werden daher
nie automatisch abgelegt. Findet Audible nichts oder nur schwache Treffer (Werke, die es nur als CD gibt), fragt
Hörwerk auch die **Deutsche Nationalbibliothek** (offene SRU-Schnittstelle, nur Tonträger) und **Apple Music**
(Alben, mit Cover und der Summe der Titellängen als Laufzeit). Ihre Treffer haben eine Laufzeit; sie werden nur dann automatisch übernommen, wenn Titel, Autor und Laufzeit sehr genau passen, sonst
stehen sie zur Auswahl. Abschalten mit `METADATA_FALLBACK=false`.

### Audiobookshelf

* Neben jedem Buch liegt eine `metadata.json` im Format von Audiobookshelf (Titel, Autoren, Sprecher, Serie, Genres,
  Beschreibung, ASIN, Kapitel) – `WRITE_ABS_METADATA=false` schaltet sie ab.
* Optional unter **Suchen → Einrichten**: Adresse und API-Token von Audiobookshelf. Nach dem Ablegen neuer Bücher wird
  die Bibliothek dort neu eingelesen – hilfreich auf Netzwerk-Shares, wo die Ordnerüberwachung nichts mitbekommt.

### Plex

Optional unter **Suchen → Einrichten**: Plex-Adresse (z. B. `http://192.168.1.10:32400`), Token und die
Hörbuch-Mediathek (wird beim Testen automatisch gewählt, wenn eindeutig). Nach dem Ablegen oder Konvertieren liest Plex
diese Mediathek neu ein.

**Konvertierte Bücher doppelt** (M4B und die alten MP3s): Plex behält Einträge gelöschter Dateien als „nicht verfügbar“,
bis sein Papierkorb geleert wird. Mit **Danach Papierkorb der Mediathek leeren** leert Hörwerk nach dem Scan den
Papierkorb genau dieser Mediathek, und die alten MP3-Einträge verschwinden.

Token: in Plex Web bei einem beliebigen Titel **…** → **Informationen abrufen** → **XML anzeigen**; der Wert hinter
`X-Plex-Token=` in der Adresse.
