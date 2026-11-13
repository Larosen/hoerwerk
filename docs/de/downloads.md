# Downloads, freie Hörbücher, Entdecken und Anfragen

[← README](../../README.de.md) · [English](../en/downloads.md) · **Deutsch**

## Downloads mit SABnzbd

Du brauchst nur SABnzbd; die Suche ist optional.

1. In SABnzbd eine Kategorie anlegen, z. B. `audiobooks`, deren Ordner der als **Hörbuch-Eingang** (`/inbox`) eingebundene ist.
2. In Hörwerk: **Suchen → Einrichten** → SABnzbd-Adresse, API-Key (SABnzbd → Config → General) und Kategorie.
3. Alles, was SABnzbd in dieser Kategorie fertig lädt, läuft automatisch durch M4B → Audible → Bibliothek. Die Liste
   **Downloads** zeigt Fortschritt und Verlauf der Kategorie.

**Suchen** (mit einem Newznab-Indexer): Du suchst ein **Hörbuch**, nicht ein Release – Treffer kommen von Audible und
BookBeat. Erst wenn du eins auswählst, sucht Hörwerk bei den Indexern; den Suchbegriff dafür kannst du ändern, wenn
nichts kommt (z. B. nur Titel oder nur Nachname). Releases werden zusammengeführt, sortiert (bevorzugte Sprache,
ungekürzt vor gekürzt, M4B vor MP3) und mit deiner Bibliothek abgeglichen; Umlaute in allen Schreibweisen.
**Kategorien:** Neben den eingetragenen (Standard `3030`) durchsucht Hörwerk automatisch den **ganzen Audio-Bereich**
jedes Indexers – alte freie Lesungen (Goethe & Co.) liegen oft unter „Audio > Lossless“ oder „MP3“ – und dessen eigene
Hörbuch-Kategorien (aus der Kategorienliste, z. B. „Hörbuch DE“, „Hörspiele“). Mitgefundene Musikalben sortiert die
Release-Bewertung über Autor und Titel aus. Findet sich dort nichts, sucht Hörwerk über alle Kategorien und behält
nur Treffer aus dem Audio-Bereich. *Testen* beim Indexer zeigt,
welche Kategorien gefunden wurden. Beim
Download gehen ASIN bzw. Titel und Autor mit – so erkennt der Eingang das Buch sicher, egal wie das Release heißt.
Für Bücher, die es in keinem Shop gibt: **Direkt bei den Indexern suchen**.

Fertige SABnzbd-Downloads aus der Suche übernimmt der Eingang nach wenigen Sekunden; bis dahin stehen sie als
„Im Eingang – startet in ca. … s“ oben in der Hörbuch-Liste und im Protokoll.

**Bevorzugter Indexer:** unter *Einstellungen → Downloads* bei einem Indexer **zuerst** anhaken. Jede Suche (von
Hand und für Anfragen) fragt dann nur diesen; die anderen erst, wenn er nichts Passendes zum Buch hat. Die Suche zeigt
„nur bei X gesucht“ mit einem Knopf, um auch die anderen zu fragen.

**Die richtige Fassung:** Ein Release, das klar eine andere Aufnahme als das angefragte Buch ist, wird nicht geladen –
eine Lesung („gesprochen von …“), wenn die Fassung mit voller Besetzung gesucht ist, ein anderer Sprecher oder (bei
einer neuen Produktion) ein Release, das älter ist als die Produktion. Releases ohne solche Angaben werden wie bisher
geladen. Nach dem Download entscheidet die Laufzeit: bis 8 % Abweichung wird abgelegt, bis 25 % zur Prüfung
vorgelegt, darüber gilt es als andere Fassung.

**Kapitelnamen:** Ein Kapitel behält nur, was es von den anderen unterscheidet. Track-Nummern, Buchtitel, Reihe und
Autor sowie Text, den (fast) jeder Track wiederholt, fallen weg – aus „01 - Buch (Full-Cast Edition): Chapter One - The
Worst Birthday“ wird „Chapter 1: The Worst Birthday“. Bleibt nichts Sinnvolles übrig, werden die Kapitel durchnummeriert
(„Kapitel 1“, „Chapter 1“). Die Reihenfolge kommt immer aus den Dateinamen; widersprechen die Track-Nummern in den Tags,
wartet das Buch auf eine Prüfung. **Kapitel aufräumen** in der Bibliothek wendet dieselben Regeln auf ein vorhandenes
Buch an.

**Besseres Release / Korrigieren:** Jedes Buch in der Bibliothek hat **Besseres Release** – Hörwerk sucht erneut und
lädt automatisch nur ein klar besseres Release (besseres Format oder deutlich mehr Bitrate); unter *Anfragen* kannst du
auch selbst eins wählen. Der neue Download ersetzt das Buch, die alte Datei wird archiviert. **Korrigieren** holt ein
Buch zur Neuzuordnung zurück, **Falsche Fassung** an einem abgelegten Buch lässt außerdem seine Anfrage weitersuchen.

Tipp: In SABnzbd unter *Config → Switches → Unerwünschte Endungen* (`exe, bat, cmd, scr, lnk, js, vbs, ps1, msi, com`)
auf **Abbrechen** stellen.

## Freie Hörbücher (LibriVox)

Bei [LibriVox](https://librivox.org) lesen Freiwillige gemeinfreie Bücher – tausende auf Englisch, hunderte auf
Deutsch, alle kostenlos und legal. Hörwerk sucht LibriVox (über das Internet Archive) gleich mit: Titel oder Autor
eingeben, Deutsch oder Englisch wählen, und die freien Aufnahmen erscheinen unter den Shop-Treffern. Ein Klick lädt
die MP3s direkt in den Eingang, samt `metadata.json` (Titel, Autor, Sprecher, Beschreibung, Cover); der Eingang macht
daraus eine M4B und legt sie wie jedes andere Buch ab. Was du schon hast, ist markiert.

Gesucht und geladen wird nur die LibriVox-Sammlung und nur von archive.org – sonst lässt sich darüber nichts laden.

## Entdecken

Der Reiter **Entdecken** schlägt neue Hörbücher vor:

* **Für dich** – Titel, die Audible-Hörer mit den (bis zu 30 neuesten) Büchern deiner Bibliothek verbinden. Je mehr
  deiner Bücher auf denselben Titel zeigen, desto weiter oben steht er; „Passt zu …“ nennt sie.
* **Nach Genre** – alle Genres und Unterkategorien wie im Audible-Shop (z. B. *Krimis & Thriller → Thriller*):
  Bestseller, Bestbewertete und Neuheiten. Kleine Bereiche werden mit Titeln aufgefüllt, die genauso gut (ab 4,0),
  aber von weniger Hörern bewertet sind – so zeigt jedes Genre Vorschläge; Unterkategorien ganz ohne deutsches Hörbuch
  (z. B. *Cricket*) werden ausgeblendet. Alle Listen werden einmal am Tag im Hintergrund vorbereitet. Das gewählte
  Genre merkt sich der Browser.
* **Aus der Kritik** – besprochene Bücher, die es bei Audible als Hörbuch gibt (gleicher Titel, gleicher Autor), mit Link
  zur Besprechung. Quellen: die RSS-Feeds „Buchkritik“ und „Lesart“ (Deutschlandfunk Kultur), die Seite
  [Die Bücher des Monats](https://www.deutschlandfunkkultur.de/rezensionen-des-monats-114.html) mit den monatlichen
  Bestenlisten für Sachbuch und Krimi (PDFs, jede wird einmal gelesen; angezeigt mit Platz), „Büchermarkt“
  (Deutschlandfunk), die Perlentaucher-Listen
  [Hörbücher – Literatur aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-literatur-aktuell.html) und
  [Sachbuch aktuell](https://www.perlentaucher.de/buchKSL/hoerbuecher-sachbuch-aktuell.html) (mit Anzahl der
  Pressekritiken) und die Bücherschau/Bücherbriefe aus dem Perlentaucher-Feed. Übernommen werden nur Autor und Titel;
  gesehene Besprechungen merkt sich Hörwerk 180 Tage, gezeigt werden die neuesten rund 40 Hörbücher. Gibt es zu einem
  besprochenen Buch noch kein Hörbuch, kommt es auf eine Warteliste: zwei Jahre lang schaut Hörwerk einmal im Monat
  nach. Ist das Hörbuch erschienen (oder bei Audible angekündigt), steht es oben – markiert mit „jetzt als Hörbuch“
  bzw. „erscheint <Datum>“. Das läuft täglich im Hintergrund, auch wenn niemand die Seite öffnet.

**Suchen** auf einer Karte springt in den Reiter *Suchen*, mit dem Buch schon ausgewählt (Titel, Autor und ASIN helfen
später beim Schreiben der Tags).

**Qualitätsregeln** (unter *Einstellungen → Entdecken*): Mindestbewertung (Standard 4,3 Sterne) und Mindestanzahl
Bewertungen (50), nur ungekürzt, nur deutschsprachig, keine Erotik. **Immer ausgeschlossen** ist, was du schon hast:
Bücher in der Bibliothek (auch in anderer Schreibweise), Downloads der letzten 30 Tage und Bücher, die gerade verarbeitet werden. Bei
Büchern aus der Kritik entfällt die Bewertungsregel – sie sind oft neu und noch kaum bewertet.

**Ehrlich gesagt:** Ob ein Sachbuch inhaltlich stimmt, kann keine Software zuverlässig prüfen – auch Bewertungen
messen nur Beliebtheit. Deshalb zeigt *Für dich* nur Sachbücher von anerkannten Autoren: auf Hörwerks fester Liste (rund 400 Autoren
faktenbasierter Sachbücher seit 2010, `rust/hoerwerk/src/trusted_authors.txt`), mit einer Perlentaucher-Autorenseite (im Feuilleton
besprochen), in den Kritiken genannt oder auf der eigenen Liste in den Einstellungen. Das prüft den Ruf, nicht den Inhalt.
Alle Sachbücher lassen sich in den Einstellungen zuschalten. In *Nach Genre* erscheinen sie, wenn du selbst ein
Sachbuch-Genre wählst – mit dem Hinweis, dass Sterne kein Faktencheck sind.
BookBeat ist nicht angebunden, weil es keine offene Schnittstelle hat.

Die Listen werden im Hintergrund zusammengestellt, höchstens einmal am Tag erneuert (oder mit **Aktualisieren**) und
in der Datenbank zwischengespeichert. Ausgehende Verbindungen: nur die Audible-API des gewählten Shops,
`www.deutschlandfunkkultur.de`, `www.deutschlandfunk.de`, `bilder.deutschlandfunk.de` (PDFs der Bestenlisten)
und `www.perlentaucher.de` (HTTPS).

### Anfragen

Familienmitglieder wünschen sich ein **Buch**, kein Release: in *Suchen* (Audible, BookBeat) oder *Entdecken* auf
**Anfragen** klicken. Den Rest erledigt Hörwerk – der Reiter **Anfragen** zeigt, wo es steht:

`Wartet auf Freigabe → Wird gesucht → Lädt → Wird verarbeitet → Verfügbar` (oder *Abgelehnt* mit Grund, *Fehlgeschlagen*)

* **Freigabe:** Anfragen von Familienkonten warten, bis ein Admin sie freigibt (ntfy „Neue Buchanfrage“, Zahl am
  Reiter). Admins und Konten mit dem Recht *Ohne Freigabe* starten sofort.
* **Monatslimit:** Anfragen pro Kalendermonat (Standard 5, je Konto änderbar; abgelehnte zählen nicht).
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

**Bücher, die anders in den Eingang kommen** (hochgeladen, von JDownloader o. Ä.): Die Erkennung vergleicht jedes Buch
auch mit den offenen Anfragen. Ist es zweifelsfrei ein angefragtes Buch (mindestens die eingestellte
*Mindest-Trefferquote*, Standard 80 %), wird es automatisch zugeordnet, abgelegt und die Anfrage auf *Verfügbar*
gesetzt; die Suche dafür hört auf. Darunter wartet es unter *Hörbücher → Zu prüfen* auf einen Admin – das
angefragte Buch steht dort mit „Anfrage von …“ oben, **Übernehmen** erfüllt die Anfrage, **Bearbeiten** korrigiert
die Erkennung vorher.

Eine „Gesucht“-Liste aus Version 5.3 wird beim ersten Start als Anfragen des Admins übernommen.

### Autoren und Sprechern folgen

Auf jeder Karte – *Entdecken*, das gewählte Buch in *Suchen*, *Anfragen* – lassen sich die Namen von Autoren und
Sprechern antippen. Das zeigt alle ihre Bücher bei Audible, das neueste zuerst (Deutsch, Englisch oder alle):

* **Ankreuzen und anfragen:** „Alle fehlenden“ oder eine Auswahl geht als **eine** Anfrage los – sie zählt einmal
  fürs Monatslimit und braucht eine Freigabe, wie eine ganze Reihe. Was in der Bibliothek liegt oder schon angefragt
  ist, lässt sich nicht ankreuzen.
* **Folgen:** Einmal am Tag sieht Hörwerk bei Audible für alle nach, denen du folgst. Ein Buch, das nach dem Beginn
  des Folgens erscheint, wird **von selbst angefragt** – als deine Anfrage, also mit deinem Monatslimit und der
  Freigabe. Ohne Haken bei „Neue Bücher automatisch anfragen“ werden neue Bücher nur unter *Entdecken → Gefolgt*
  markiert.
* **Entdecken → Gefolgt** zeigt alle, denen du folgst, mit dem, was daraus wurde (bisher angefragt, neue Bücher, ein
  Hinweis, wenn das Monatslimit eine Anfrage verhindert hat – am nächsten Tag wird es erneut versucht), und ein Feld,
  um einen beliebigen Namen nachzuschlagen.
* Jeder folgt für sich; es zählen nur Bücher, die die Person wirklich nennen (die Audible-Suche ist ungenau), und ein
  Buch wird nur einmal angefragt – zuerst die ungekürzte Fassung in der gewählten Sprache.
