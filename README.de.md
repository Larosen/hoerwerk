# Hörwerk

*Die Hörbuch-Werkstatt für dein NAS.*

[English](README.md) · **Deutsch**

Eine selbst gehostete Hörbuch-Automatik für dein NAS (Unraid oder jeder andere Docker-Host): Hörbücher in einen
Eingangsordner legen; sie werden zu **einer M4B mit Kapiteln**, bei Audible erkannt, **getaggt** und sauber in die
Bibliothek für Audiobookshelf, Plex & Co. **einsortiert**. Ältere MP3-Bücher in der Bibliothek werden genauso
umgewandelt – Tracks und Kapitel bekommen dabei sinnvolle Namen.

Downloads laufen über **SABnzbd**, Benachrichtigungen über **ntfy** – du bestimmst, bei welchen Ereignissen.
Die Oberfläche gibt es auf **Deutsch und Englisch**.

> Hörwerk ist ein Werkzeug zum Ordnen von Audiodateien, die du nutzen darfst – eigene Käufe, eigene
> Aufnahmen oder gemeinfreie Werke (z. B. von [LibriVox](https://librivox.org)). Es enthält keine Medien,
> umgeht keinen Kopierschutz und steht in keiner Verbindung zu Audible, Amazon, Apple oder anderen hier genannten Diensten.

> Früher *Better Renamer*: Bis Version 4 hatte die App zusätzlich einen allgemeinen Stapel-Umbenenner; seit Version 5 heißt sie Hörwerk und kümmert sich nur noch um Hörbücher. Eine vorhandene Datenbank (`better-renamer.db`) wird automatisch übernommen.

---

## Inhalt

1. [Funktionen](#funktionen)
2. [Schnellstart](#schnellstart)
3. [Hörbücher](#hörbücher)
4. [Downloads mit SABnzbd](#downloads-mit-sabnzbd)
5. [Entdecken und Anfragen](#entdecken)
6. [Benachrichtigungen (ntfy)](#benachrichtigungen-ntfy)
7. [Protokoll](#protokoll)
8. [Konten und Zugriff von außen](#konten-und-zugriff-von-außen)
9. [Konfiguration](#konfiguration)
10. [Sicherheit](#sicherheit)
11. [Entwicklung](#entwicklung)

---

## Funktionen

| Bereich | Was du bekommst |
|---|---|
| Hörbuch-Eingang | Wartet, bis ein Ordner fertig kopiert ist, fügt MP3/M4A/FLAC/OGG/Opus/WMA (auch `CD1`/`CD2`-Unterordner) zu einer M4B mit sample-genauen, sauber benannten Kapiteln zusammen |
| Erkennung | Audible-Suche in 10 Shops, bewertet nach Titel, Autor **und Laufzeit** (Lesung ≠ gekürzt ≠ Hörspiel); unsichere Treffer warten auf dich |
| Tags & Dateien | Titel, Autor, Sprecher, Serie + Band, Genre, Verlag, Datum, Beschreibung, ASIN, Cover in voller Größe; `metadata.json` für Audiobookshelf |
| Bibliothek | Alle Bücher durchsuchen, Ordner-Vorlage, nutzt vorhandene Autoren-/Serienordner, erkennt Doppelte, macht alte MP3-Bücher zu M4B und räumt die MP3s sicher weg |
| Downloads | Schickt NZBs an SABnzbd, beobachtet die Kategorie, nach dem Download übernimmt der Eingang; Newznab-Suche optional |
| Entdecken | Vorschläge passend zur Bibliothek, nach Genre und aus der Buchkritik (Deutschlandfunk Kultur, Perlentaucher) – nur gut bewertet, ungekürzt, nichts, was du schon hast; **Anfragen** wie bei Overseerr: Familie wünscht, Hörwerk besorgt (mit Freigabe, Kontingent, interaktiver Suche) |
| Medienserver | Audiobookshelf und Plex lesen ihre Bibliothek nach dem Ablegen neu ein; Plex leert auf Wunsch den Papierkorb, damit ersetzte MP3s verschwinden |
| Benachrichtigungen | ntfy-Push-Nachrichten, elf Ereignisse, jedes einzeln an oder aus |
| Protokoll | Alles, was passiert ist – filterbar und als Datei herunterladbar, direkt in der App |
| Oberfläche | Deutsch / Englisch, helles / dunkles Design, Live-Updates ohne Neuladen |
| Sicherheit | Eigenes schlankes ffmpeg 9 im Landlock-Käfig, CSRF-Schutz, Zugangsdaten erreichen nie den Browser ([SECURITY.de.md](SECURITY.de.md)) |

---

## Schnellstart

### Unraid

1. **Docker → Add Container**, bei *Template* eintragen:
   `https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/hoerwerk.xml`
2. Pfade setzen:
   * **Hörbuch-Eingang** (`/inbox`) – hier kommen neue Hörbücher an
   * **Hörbuch-Bibliothek** (`/audiobooks`) – die fertige Bibliothek (in Audiobookshelf/Plex einbinden)
3. **Language / Sprache** wählen (`APP_LANGUAGE`: `de` oder `en`). PUID `99`, PGID `100`, UMASK `000` passen für Unraid schon.
4. `http://<unraid-ip>:8080` öffnen.

Icon: Das Template lädt es von GitHub. Ist dein Repository privat, kommt Unraid nicht daran – dann als
**Icon URL** `http://<unraid-ip>:8080/static/icon.png` eintragen; der Container liefert sein Logo selbst aus
(die einzige Datei, die er ohne Passwort herausgibt).

Image: `ghcr.io/larosen/hoerwerk:latest` (amd64 + arm64, wöchentlich für Sicherheitsupdates neu gebaut).

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

### Hochladen (z. B. von CD)

Hörbücher, die du selbst hast – etwa von CD eingelesen –, lädst du direkt im Browser hoch: *Hörbücher → Eingang →
**Hochladen***. Einen Ordner pro Hörbuch hineinziehen oder auswählen, am besten „Autor – Titel“, gern mit
Unterordnern `CD1`, `CD2` …; einzelne Dateien gehen auch (dann den Namen eintragen). Hörwerk übernimmt Audio-Dateien,
Cover, Cue-Sheets und `chapters.txt`/`metadata.yml`; alles andere wird übersprungen.

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

### Downloads mit mehreren Bänden

Ein Download wie `Autor-Reihe 01 - 07-…` ist ein **Paket**: Jeder Band-Ordner wird ein eigenes Buch – einzeln
zusammengefügt, erkannt und getaggt, mit Autor und Band-Nummer aus dem Release-Namen. Ordner wie `01 - Titel`,
`02 - Titel` sind Bände; `CD1`/`CD2`, `Disc 1`, `Teil 1` oder nur `01`/`02` bleiben Teile eines Buchs.

* In der **Suche** zeigt ein Paket seine Bände und welche davon du schon hast. **Download** fragt, welche Bände du
  willst – vorhandene sind abgewählt. Nur die gewählten werden konvertiert, die anderen unkonvertiert weggeräumt.
* Pakete, die nicht über die Suche kamen: Bände, die schon in der Bibliothek sind, werden automatisch übersprungen –
  über Reihe und Band-Nummer, spätestens wenn Audible das Buch erkennt, immer vor dem Konvertieren.

### Doppelte vermeiden

Die Bibliothek wird beim Start und alle paar Stunden eingelesen (`LIBRARY_RESCAN_HOURS`) – auch ältere MP3-Ordner,
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
werden **Apple Books** und **Open Library** gefragt. Diese Treffer haben weder Laufzeit noch Sprecher und werden daher
nie automatisch abgelegt. Abschalten mit `METADATA_FALLBACK=false`.

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

---

## Downloads mit SABnzbd

Du brauchst nur SABnzbd; die Suche ist optional.

1. In SABnzbd eine Kategorie anlegen, z. B. `audiobooks`, deren Ordner der als **Hörbuch-Eingang** (`/inbox`) eingebundene ist.
2. In Hörwerk: **Suchen → Einrichten** → SABnzbd-Adresse, API-Key (SABnzbd → Config → General) und Kategorie.
3. Alles, was SABnzbd in dieser Kategorie fertig lädt, läuft automatisch durch M4B → Audible → Bibliothek. Die Liste
   **Downloads** zeigt Fortschritt und Verlauf der Kategorie.

**Suchen** (mit einem Newznab-Indexer): Du suchst ein **Hörbuch**, nicht ein Release – Treffer kommen von Audible und
BookBeat. Erst wenn du eins auswählst, sucht Hörwerk bei den Indexern; den Suchbegriff dafür kannst du ändern, wenn
nichts kommt (z. B. nur Titel oder nur Nachname). Releases werden zusammengeführt, sortiert (bevorzugte Sprache,
ungekürzt vor gekürzt, M4B vor MP3) und mit deiner Bibliothek abgeglichen; Umlaute in allen Schreibweisen. Beim
Download gehen ASIN bzw. Titel und Autor mit – so erkennt der Eingang das Buch sicher, egal wie das Release heißt.
Für Bücher, die es in keinem Shop gibt: **Direkt bei den Indexern suchen**.

Fertige SABnzbd-Downloads aus der Suche übernimmt der Eingang nach wenigen Sekunden; bis dahin stehen sie als
„Im Eingang – startet in ca. … s“ oben in der Hörbuch-Liste und im Protokoll.

Tipp: In SABnzbd unter *Config → Switches → Unerwünschte Endungen* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
auf **Abbrechen** stellen.

---

## Entdecken

Der Reiter **Entdecken** schlägt neue Hörbücher vor:

* **Für dich** – Titel, die Audible-Hörer mit den (bis zu 30 neuesten) Büchern deiner Bibliothek verbinden. Je mehr
  deiner Bücher auf denselben Titel zeigen, desto weiter oben steht er; „Passt zu …“ nennt sie.
* **Nach Genre** – Bestseller und Neuheiten eines Audible-Genres; das gewählte Genre wird gemerkt.
* **Aus der Kritik** – besprochene Bücher, die es bei Audible als Hörbuch gibt (gleicher Titel, gleicher Autor), mit Link
  zur Besprechung. Quellen: der RSS-Feed „Buchkritik“ von Deutschlandfunk Kultur, die Perlentaucher-Liste
  [Hörbücher – Literatur aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html) (mit Anzahl
  der Pressekritiken) und die Bücherschau/Bücherbriefe aus dem Perlentaucher-Feed. Übernommen werden nur Autor und Titel;
  gesehene Besprechungen merkt sich Hörwerk 90 Tage, die Liste wächst also mit der Zeit.

**Suchen** auf einer Karte springt in den Reiter *Suchen*, mit dem Buch schon ausgewählt (Titel, Autor und ASIN helfen
später beim Taggen).

**Qualitätsregeln** (unter *Einstellungen → Entdecken*): Mindestbewertung (Standard 4,3 Sterne) und Mindestanzahl
Bewertungen (50), nur ungekürzt, nur deutschsprachig, keine Erotik. **Immer ausgeschlossen** ist, was du schon hast:
Bücher in der Bibliothek (auch in anderer Schreibweise), Downloads der letzten 30 Tage und Bücher, die gerade verarbeitet werden. Bei
Büchern aus der Kritik entfällt die Bewertungsregel – sie sind oft neu und noch kaum bewertet.

**Ehrlich gesagt:** Ob ein Sachbuch inhaltlich stimmt, kann keine Software zuverlässig prüfen – auch Bewertungen
messen nur Beliebtheit. Deshalb kommen Sachbücher standardmäßig **nur aus der redaktionellen Quelle** (Kritik),
nicht aus Bestsellerlisten; Sachbuch-Genres sind ausgeblendet, bis du sie in den Einstellungen erlaubst.
BookBeat ist nicht angebunden, weil es keine offene Schnittstelle hat.

Die Listen werden im Hintergrund zusammengestellt, höchstens einmal am Tag erneuert (oder mit **Aktualisieren**) und
in der Datenbank zwischengespeichert. Ausgehende Verbindungen: nur die Audible-API des gewählten Shops,
`www.deutschlandfunkkultur.de` und `www.perlentaucher.de` (HTTPS).

### Anfragen (wie bei Overseerr/Jellyseerr)

Familienmitglieder wünschen sich ein **Buch**, kein Release: in *Suchen* (Audible, BookBeat) oder *Entdecken* auf
**Anfragen** klicken. Den Rest erledigt Hörwerk – der Reiter **Anfragen** zeigt, wo es steht:

`Wartet auf Freigabe → Wird gesucht → Lädt → Wird verarbeitet → Verfügbar` (oder *Abgelehnt* mit Grund, *Fehlgeschlagen*)

* **Freigabe:** Anfragen von Familienkonten warten, bis ein Admin sie freigibt (ntfy „Neue Buchanfrage“, Zahl am
  Reiter). Admins und Konten mit dem Recht *Ohne Freigabe* starten sofort.
* **Kontingent:** Anfragen pro Kalendermonat (Standard 5, je Konto änderbar; abgelehnte zählen nicht).
* **Sichtbarkeit:** Jeder sieht seine eigenen Anfragen; mit dem Recht *Alle Anfragen sehen* auch die der anderen.
* **Automatisch besorgen (wie Radarr):** Hörwerk sucht bei allen Indexern und bewertet jedes Release – passt der
  Autor, passt der Titel, ist es ungekürzt, in der richtigen Sprache, passt die Größe zur Laufzeit (kbit/s), ist es
  ein Paket mit genau diesem Band? Das beste passende geht mit ASIN und Anfrage-Nummer an SABnzbd; danach verfolgt
  Hörwerk den Download bis in die Bibliothek. Scheitert der Download oder passt das Buch beim Import nicht, wird
  das Release gesperrt und das nächste genommen (höchstens 5 Versuche). Ohne Treffer wird alle 12 Stunden erneut
  gesucht.
* **Interaktive Suche (nur bei Bedarf):** Admins und Konten mit dem Recht *Interaktive Suche* öffnen bei einer
  Anfrage **Releases**: alle Treffer mit Urteil – auch die verworfenen samt Grund („gekürzte Fassung“, „zu klein
  für 9 h Laufzeit“ …). Ein Release von Hand laden, eins sperren oder mit anderen Wörtern neu suchen.
* **Benachrichtigung an den Anfragenden:** Wer unter *Konto* ein eigenes ntfy-Topic einträgt, erfährt, wenn sein
  Hörbuch da ist oder abgelehnt wurde.

Eine „Gesucht“-Liste aus Version 5.3 wird beim ersten Start als Anfragen des Admins übernommen.

---

## Benachrichtigungen (ntfy)

Push-Nachrichten aufs Handy über [ntfy](https://ntfy.sh) – ntfy.sh oder ein eigener Server.

1. ntfy-App installieren und ein Topic abonnieren, z. B. `hoerbuecher-1234`.
2. **⚙︎ Einstellungen → Benachrichtigungen (ntfy)**: Server, Topic, optional Access-Token (oder Benutzer + Passwort)
   und Priorität. Mit *Adresse dieser Oberfläche* öffnet ein Tippen auf die Nachricht Hörwerk.
3. Gewünschte Ereignisse anhaken und **Testnachricht senden**.

| Ereignis | Standard |
|---|---|
| Hörbuch abgelegt | an |
| Hörbuch braucht Prüfung | an |
| Schon in der Bibliothek | an |
| Fehler bei einem Hörbuch | an |
| Neue Buchanfrage | an |
| Buchanfrage fehlgeschlagen | an |
| Download an SABnzbd geschickt | aus |
| Download in SABnzbd fertig | aus |
| Download in SABnzbd fehlgeschlagen | an |
| Audiobookshelf-Scan fehlgeschlagen | an |
| Plex-Scan fehlgeschlagen | an |

Token und Passwort bleiben auf dem Server und werden bei geänderter Adresse nicht an einen anderen Server geschickt.
Nachrichten kommen in der Sprache der Oberfläche.

---

## Protokoll

**Protokoll** oben in der Leiste zeigt, was passiert ist: gefundene, konvertierte, erkannte und abgelegte Bücher,
Scans der Medienserver, Warnungen und Fehler. Nach Stufe oder Text filtern, live mitlesen, als Datei herunterladen.
Dieselben Zeilen stehen im Container-Log (`docker logs hoerwerk`).

---

## Konten und Zugriff von außen

Ohne Anmeldung zeigt Hörwerk nur die Login-Seite. Beim **ersten Aufruf** legst du das Admin-Konto an (aus dem
Heimnetz direkt; von außen braucht es den Einrichtungs-Code aus `docker logs hoerwerk`). Wer bisher
`AUTH_USER`/`AUTH_PASS` gesetzt hatte, bekommt daraus automatisch das Admin-Konto.

* **Konten** (oben rechts, nur für Admins): Familienkonten anlegen (Startpasswort erzeugen und weitergeben),
  Rolle, Rechte (*Alle Anfragen sehen*, *Ohne Freigabe*, *Interaktive Suche*), Kontingent pro Monat, sperren,
  Passwort neu setzen, Zweitfaktor zurücksetzen, löschen. Familienkonten sehen *Suchen*, *Entdecken* und *Anfragen*
  – Eingang, Bibliothek, Einstellungen und Protokoll bleiben Admins vorbehalten. Unten im Dialog: das
  **Admin-Protokoll** (wer hat wann was geändert, mit IP).
* **Konto** (Name oben rechts, für alle): Passwort ändern, **Zweitfaktor** (Authenticator-App wie Aegis, 2FAS,
  Google/Microsoft Authenticator) einrichten, eigenes ntfy-Topic, angemeldete Geräte sehen und abmelden.
  Mit *Admins brauchen einen Zweitfaktor* (Einstellungen) kommen Admins ohne Zweitfaktor nur noch an ihr Konto.
* „Angemeldet bleiben“ hält 30 Tage (verlängert sich bei Nutzung), sonst 12 Stunden.

**Über Pangolin ins Internet:**
1. In Pangolin eine Ressource auf `http://<unraid-ip>:8080` anlegen (HTTPS macht Pangolin).
   Pangolins eigene Anmeldung kann zusätzlich davor bleiben – sicherer, aber zweimal anmelden.
2. `TRUSTED_PROXIES` auf die Adresse setzen, von der Newt/Pangolin bei Hörwerk ankommt (z. B. `172.18.0.0/16`
   für das Docker-Netz oder die IP des Newt-Containers). Nur dann steht im Protokoll die echte IP der Besucher.
3. `ALLOWED_HOSTS` um den öffentlichen Namen ergänzen, z. B. `tower,192.168.1.10,hoerbuecher.example.de`.
4. **CrowdSec:** Jeder Fehlversuch steht mit IP in `/config/logs/auth.log`. Parser, Szenarien und Beispiele für
   Vector bzw. den CrowdSec-Agenten liegen in [contrib/crowdsec](contrib/crowdsec/README.md). Hörwerk selbst sperrt
   eine IP nach 5 Fehlversuchen für 15 Minuten; CrowdSec sperrt sie dann schon am VPS.

---

## Konfiguration

Fast alles lässt sich unter **⚙︎ Einstellungen** ändern; die Umgebungsvariablen liefern nur die Startwerte.
Einstellungen, Voreinstellungen, Verlauf, Hörbuch-Liste, Bibliotheks-Index und alle Zugangsdaten liegen in der
SQLite-Datenbank `/config/hoerwerk.db` (Rechte 600).

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
| `WRITE_DESC` / `WRITE_READER` | `false` | `desc.txt`, `reader.txt` schreiben (nur nötig ohne `metadata.json`) |
| `WRITE_COVER` | `true` | `cover.jpg` schreiben (Audiobookshelf und Plex nutzen es) |
| `DISCOVER_MIN_RATING` / `DISCOVER_MIN_COUNT` | `4.3` / `50` | Entdecken: Mindestbewertung und -anzahl Bewertungen |
| `DISCOVER_UNABRIDGED` / `DISCOVER_GERMAN` | `true` | Entdecken: nur ungekürzt / nur deutschsprachig |
| `DISCOVER_NONFICTION_STORE` | `false` | Entdecken: Sachbücher auch aus Audible-Listen |
| `WANTED_INTERVAL_HOURS` | `12` | Anfragen ohne Treffer: Abstand der erneuten Suche |
| `ADMIN_TOTP_REQUIRED` | `false` | Admins brauchen einen Zweitfaktor |
| `CPU_CORES` | halbe Kernzahl | Parallele ffmpeg-Prozesse pro Buch |
| `ENCODE_WORKERS` | `1` | Bücher, die gleichzeitig kodiert werden |
| `IO_WORKERS` | `2` | Gleichzeitige Erkennungs-, Tag- und Ablageschritte |
| `NICE` | `10` | CPU-Priorität von ffmpeg (0 = normal) |
| `FFMPEG_SANDBOX` | `auto` | ffmpeg im Landlock-Käfig: `auto`, `require`, `off` |
| `FFMPEG_MEMORY_MB` | `4096` | Speichergrenze je ffmpeg-Prozess |
| `AAC_BITRATE` | `auto` | `auto` (wie die Quelle, höchstens 128k) oder fester Wert |
| `FORCE_MONO` | `false` | Mono ausgeben |
| `CHAPTER_TITLES` | `auto` | `auto`, `filename` oder `tag` |
| `TRUSTED_PROXIES` | – | IPs/Netze des Reverse-Proxys (Pangolin/Newt, Traefik), z. B. `172.18.0.0/16` – nur von dort zählt `X-Forwarded-For` |
| `COOKIE_SECURE` | `auto` | Anmelde-Cookie nur über HTTPS: `auto` (wenn der Proxy HTTPS meldet), `true`, `false` |
| `REQUEST_QUOTA` | `5` | Buchanfragen pro Nutzer und Monat (je Konto änderbar) |
| `AUTH_USER` / `AUTH_PASS` | – | Nur noch für den Umstieg: wird beim ersten Start zum Admin-Konto |
| `ALLOWED_HOSTS` | – | Nur unter diesen Hostnamen/IPs antworten, z. B. `tower,192.168.1.10` |

SABnzbd, der optionale Indexer, Audiobookshelf, Plex und ntfy werden in der Oberfläche eingerichtet, nicht über Variablen.

---

## Sicherheit

Kurz gefasst – Details, Tests und Komponenten-Versionen in [SECURITY.de.md](SECURITY.de.md):

* Konten mit Login; ins Internet nur hinter einem Reverse-Proxy mit HTTPS (z. B. Pangolin) und mit
  `TRUSTED_PROXIES`. Fehlversuche stehen mit IP in `/config/logs/auth.log` – fertig für CrowdSec.
* Nur nötige Ordner einbinden; der Container läuft ohne Linux-Capabilities und mit `no-new-privileges`.
* Heruntergeladene Dateien werden nie ausgeführt. ffmpeg ist ein eigener schlanker Build (nur Audio, kein
  Netzwerk-Code) in einem Landlock-Käfig, der nur die Dateien des Buchs lesen darf.
* API-Keys, Token und Passwörter (SABnzbd, Indexer, Audiobookshelf, Plex, ntfy) bleiben auf dem Server und gehen nie an den Browser.

---

## Entwicklung

```bash
pip install -r requirements.txt                 # mutagen + PyYAML
npm ci && npm run build                         # TypeScript (frontend/src) -> app/static/*.js
python3 -m unittest discover -s tests           # Python-Tests (brauchen ffmpeg, kein Netz)
npm run check                                   # Typprüfung
npx playwright install chromium && npx playwright test   # Durchklick-Tests gegen einen echten Server
INBOX_DIR=/tmp/inbox LIBRARY_DIR=/tmp/lib CONFIG_DIR=/tmp/br PORT=8080 python3 app/main.py
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
| Benachrichtigungen & Protokoll | `app/notify.py`, `app/applog.py` |
| Medienserver | `app/abs.py`, `app/plex.py` |
| Entdecken | `app/discover.py`, `frontend/src/discover.ts` |
| Sprache | `app/i18n.py`, `app/static/i18n/en.json` |
| Oberfläche | `frontend/src/*.ts`, `app/static/index.html`, `style.css`; Durchklick-Tests in `e2e/` |

Inspiriert von den Open-Source-Projekten [auto-m4b](https://github.com/seanap/auto-m4b) und
[beets-audible](https://github.com/Neurrone/beets-audible); Kapiteldaten über [Audnexus](https://github.com/laxamentumtech/audnexus).
