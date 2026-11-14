# Gestaltungskonzept „Werkstatt & Regal"

Entwurf für eine neue Oberfläche von Hörwerk: drei Farbwelten (Hell, Dunkel, OLED), eine eigene Bildsprache und ein
Aufbau für iPhone, iPad und MacBook. Dies ist ein **Konzept mit klickbarem Prototyp**, die App selbst ist noch nicht
umgebaut.

| Was | Wo |
|---|---|
| Klickbarer Prototyp (HTML/CSS/JS, keine Frameworks, keine Webfonts) | [prototype/index.html](prototype/index.html) – Aufruf mit `?screen=discover\|library\|search\|log\|stats\|more\|settings&theme=light\|dark\|oled` (Entdecken zusätzlich `&tab=foryou\|genre\|kritik\|gefolgt`, Protokoll `&win=15\|60\|1440\|0`) |
| Beispiel-Screenshots | [boards/](boards/): `iphone-1.jpg`, `iphone-2.jpg`, `mac-<seite>.jpg`, `ipad-entdecken.jpg`, `ipad-hoerbuecher.jpg` |
| Kontrastprüfung der Farbwelten | `python3 docs/design/check_contrast.py` (alle Textpaare ≥ 4,5:1, aktuell 0 Verstöße) |

> Die Screenshots entstehen in einem Chromium unter Linux, nicht in Safari. Schriftdarstellung und Abstände können auf
> echten Geräten leicht abweichen; vor der Umsetzung sollte der Prototyp einmal auf iPhone, iPad und Mac geöffnet werden.

> **Wortwahl:** In der Oberfläche heißt es durchgehend **Hörbuch / Hörbücher**, nie „Buch / Bücher". Der Prototyp folgt dem schon;
> in der App kommt es mit der Umsetzung (inklusive Übersetzung `static/i18n/en.json`).

## Stand der Umsetzung (Version 1.0.0)

Das Konzept ist in der App umgesetzt – mit diesen Abweichungen und Lücken:

| Thema | Stand |
|---|---|
| Drei Farbwelten (Hell · Dunkel · OLED), Kontrast ≥ 4,5:1 | **umgesetzt** (`static/style.css`), Prüfung: `docs/design/check_contrast.py` |
| Farbwelt je Konto oder je Gerät | **umgesetzt** unter **Konto → Anzeige** (nicht unter Einstellungen: die sind nur für Admins, die Farbwelt braucht aber jedes Familienkonto) |
| Seitenleiste (iPad/Mac), Tab-Leiste und „Mehr" (iPhone) | **umgesetzt** |
| Entdecken mit vier Reitern, Genre-Chips, Kritik-Karten, Namen als Fließtext | **umgesetzt**; die Regale und der „Vorschlag des Tages" aus dem Prototyp sind **nicht** übernommen (die Kartenliste bleibt) |
| Hörbücher als Regal oder Liste, Aktionen als Symbole mit kurzen Namen | **umgesetzt** |
| Protokoll mit Zeitraum (15 Min · 60 Min · 24 Std · Alles) | **umgesetzt**, Standard 60 Min |
| Fortschritt als Tonspur (Downloads, Umwandlung, Anfragen), Anmeldung mit Tonspur | **umgesetzt** |
| Texte gestrichen (Statuszeile in Kritik, „Was gezeigt wird", Quellenzeile in Suchen) | **umgesetzt**; „Hörbuch" statt „Buch" durchgehend |
| iPhone-App (Home-Bildschirm) mit Anleitung | **umgesetzt**: Manifest, Symbole, Hinweis unter *Konto*, [Anleitung](../de/iphone-app.md) |
| Buchfarbe aus dem Cover (Karten und Detailspalte getönt) | **umgesetzt**: Farbe aus dem Bild im Browser, Online-Cover über den Cover-Zwischenspeicher des Servers (`/api/cover`); ohne Cover aus dem Farbton des Ersatz-Covers. Diagramme in der Statistik haben keine Buchfarbe |
| Detailspalte am Mac (Entdecken, Suchen) und Blatt auf iPad/iPhone, mit „Ähnlich wie“ | **umgesetzt**: `GET /api/book/detail` (Beschreibung und Audible-Ähnliche), Spalte ab 1240 px |
| Messing als zweiter Akzent | umgesetzt für „neu" und Hinweise (Zähler, Spitzenwert in der Statistik) |

## 1. Befund: Was an der heutigen Oberfläche nicht trägt

Grundlage sind die Screenshots der Version 7.20.0 und der Stylesheet `static/style.css`.

1. **Kaltes Grauschwarz.** Das Dunkel (`#131412`, `#1a1b19`) ist ein neutrales, leicht grünliches Grau mit blassem Text. Es
   gibt keine Wärme und keinen zweiten Farbton, deshalb wirkt es bedrückend.
2. **Alles ist eine Zeile.** Hörbücher, Suchtreffer und Protokoll sind Listen aus gleich aussehenden Textzeilen mit
   Haarlinien. Nichts lädt zum Stöbern ein, und nichts zeigt, was gerade wichtig ist.
3. **Cover wirken aufgeklebt.** Sie haben je nach Seite andere Größen, keinen gemeinsamen Radius und keine Verbindung zur
   Umgebung. Auf einer einfarbigen Karte stehen sie wie Fremdkörper.
4. **Statistik ohne Zusammenhang.** Die Diagramme verwenden Farben, die weder zu den Covern noch zur Oberfläche passen.
5. **Handy-Seiten sind verkleinerte Desktop-Seiten.** Suche und Protokoll passen auf das iPhone, sind aber nicht dafür
   gedacht: keine Tab-Leiste, kleine Tippflächen, Kopfzeile mit vielen Texten.
6. **Kein roter Faden zwischen den Seiten.** Es gibt eine Akzentfarbe (Petrol), aber keine wiedererkennbare Form.

## 2. Leitidee: Werkstatt & Regal

Hörwerk hat zwei Gesichter. In der **Werkstatt** wird gearbeitet: Ordner kommen an, werden zusammengefügt, getaggt,
abgelegt. Im **Regal** steht das Ergebnis und wird durchstöbert. Die Oberfläche macht beides sichtbar, mit Mitteln, die
man aus Bibliothek und Buchhandlung kennt:

- **Papier, Leinen, Messing** statt Glas und Leuchtfarben.
- **Gesetzte Schrift** (Serife für Titel, Systemschrift für Bedienelemente) statt weicher Schwebetexte.
- **Handwerkliche Details:** Regalbrett, Buchrücken, Katalogzeile mit Punktlinie, Wellenform als Fortschritt.

Bewusst **nicht** verwendet: Verläufe hinter Text, Unschärfe-Flächen (Glas), Leuchteffekte, schwebende Überschriften,
Neon-Akzente, Illustrationen im KI-Stil.

### Fünf Gestaltungsregeln

1. **Wärme statt Neutralgrau.** Alle Flächen und Texte haben einen leichten Braunton, in allen drei Farbwelten.
2. **Zwei Akzente mit klaren Rollen.** *Buchleinen* (Grün) bedeutet „handeln, laufend, aktiv". *Messing* bedeutet „neu,
   Auszeichnung, Highlight". Warnung (Bernstein) und Fehler (Siegellack, Terrakotta) kommen nur für Zustände vor.
3. **Das Buch ist der Held.** Cover sind immer quadratisch, gleich gerundet, mit Kante, Rückenlicht und Papierkorn. Alles
   andere ordnet sich ihnen unter.
4. **Eine Form, die nur Hörwerk hat:** die **Tonspur**. Fortschritt wird als Wellenform gezeigt, nicht als Balken.
5. **Gleiche Bedeutung, gleiche Farbe** auf jeder Seite (Grün = läuft, Bernstein = braucht dich, Rot = Fehler,
   Messing = neu).

## 3. Die drei Farbwelten

Alle Werte stehen als CSS-Variablen im Prototyp (`:root`, `[data-theme="dark"]`, `[data-theme="oled"]`).

| Token | Hell · *Papier* | Dunkel · *Abend* | OLED · *Nacht* |
|---|---|---|---|
| Hintergrund `--bg` | `#F3EDE0` Creme | `#1D1916` Espresso | `#000000` |
| Fläche `--surface` | `#FBF8F0` | `#27221F` | `#0B0A09` |
| Linie `--line` | `#DCD2BC` | `#3B342D` | `#2A2520` |
| Text `--ink` | `#2B2520` Tinte | `#F1E9DB` | `#EDE5D5` (gedimmt, kein Reinweiß) |
| Gedämpft `--muted` | `#665B4B` | `#BBAE99` | `#A99D89` |
| Akzent Leinen `--accent` | `#1B6454` | `#63BDA6` | `#6FCFB7` |
| Akzent Messing `--brass` | `#8A5A0B` | `#E3AE55` | `#F0BC5E` |
| Warnung / Fehler | `#97520F` / `#A03B28` | `#E8A04A` / `#E88468` | `#F0A94F` / `#EE8C70` |

**Warum das nicht mehr bedrückend wirkt:** Das Dunkel ist ein warmes Espresso, kein Grau, die Schrift ist Creme statt
Weiß, und das Messing sorgt für einen warmen Gegenpunkt zum Grün. Das Hell ist kein Weiß, sondern getöntes Papier und
blendet nicht.

**OLED-Besonderheiten:** Der Hintergrund ist echtes Schwarz (Pixel aus, spart Akku und wirkt auf OLED-Displays tief).
Flächen werden nur über Haarlinien und eine kaum sichtbare Tönung getrennt, nicht über Grautöne. Schatten entfallen, die
Cover bekommen stattdessen eine hellere Kante. Der Text ist leicht gedimmt, damit er auf Schwarz nicht flimmert.

**Kontrast:** [check_contrast.py](check_contrast.py) prüft 18 Text-Hintergrund-Paare je Farbwelt gegen WCAG 2.2 (≥ 4,5:1).
Ein erster Durchlauf fand im hellen Thema mehrere Werte unter 4,5 (Eyebrow-Text 2,9; Messing 3,5; Warnung 3,9). Sie sind
im Prototyp korrigiert, aktuell gibt es 0 Verstöße.

## 4. Das Cover als Buch, die Buchfarbe als Klammer

- **Form:** quadratisch (Hörbuch-Standard), Radius 5 px (groß 7 px), 1 px Kante, heller Rücken links, feines Papierkorn,
  Kontaktschatten (Hell/Dunkel) oder Lichtkante (OLED).
- **Buchfarbe `--c`:** Jedes Hörbuch bekommt eine Farbe, die aus seinem Cover stammt. Sie färbt die Karte um das Cover
  (Mischung aus Fläche und Buchfarbe, 9–16 %), die Detailansicht und die Balken in der Statistik. So sitzt das Cover in
  seiner Umgebung, statt darauf zu liegen.
- **Regal:** In Hörbuchlisten stehen die Cover auf einem schmalen Brett, Titel und Autor stehen darunter wie ein
  Katalogschild.
- **Statistik:** Diagramme verwenden dieselbe sechsteilige Farbreihe (`--s1…--s6`), je Farbwelt in Helligkeit und Sättigung
  abgestimmt. Der aktuelle Wert ist immer messingfarben, alles andere Leinen.

## 5. Wiedererkennbare Bausteine

| Baustein | Aussehen | Verwendung |
|---|---|---|
| **Tonspur** | Fortschritt als Wellenform aus schmalen Balken; der gespielte Teil ist grün oder messing | Werkbank, Seitenleiste, Downloads, Kapitel |
| **Stationen** | vier Punkte auf einer Linie: Eingang → Verbinden → Taggen → Ablegen | Jedes Hörbuch in Arbeit |
| **Regalbrett** | dünnes Brett unter einer Reihe Cover | Entdecken, Hörbücher |
| **Katalogzeile** | Name … Punktlinie … Wert, wie in einem Inhaltsverzeichnis | Details, Kapitel, Metadaten |
| **Eyebrow** | kleine Kapitälchen-Zeile über der Überschrift (Datum, Zähler, Quelle) | jede Seite |
| **Prüfurteil** | farbige Kennzeichen „Richtiges Hörbuch / Deutsch / ungekürzt" plus Punktering | Suchen (Releases) |

## 6. Die drei Geräte

Ein Aufbau, drei Ausprägungen. Umschaltung rein über CSS (kein Rust-Code nötig), Breiten unter 780 px, ab 780 px und
ab 1240 px.

```
iPhone (< 780 px)                 iPad quer (780–1239 px)             MacBook (≥ 1240 px)
┌────────────────────┐            ┌────┬──────────────────────┐       ┌────┬──────────────────┬─────────┐
│ Hörwerk   Hell|Dkl │            │ ▤  │ Entdecken            │       │ ▤  │ Entdecken        │ Details │
│ SAMSTAG            │            │Ent.│ [Für dich][Krimi]…   │       │Ent.│ [Für dich][Krimi]│ [Cover] │
│ Entdecken          │            │Such│ ┌──────────────────┐ │       │Such│ ┌──────────────┐ │ Titel   │
│ [Für dich][Krimi]… │            │Anf.│ │ [Cover] Heute    │ │       │Anf.│ │ [Cover] Heute│ │ Autor   │
│ ┌────────────────┐ │            │Büch│ │  für dich        │ │       │Büch│ │  für dich    │ │ Sprecher…│
│ │   [Cover]      │ │            │────│ └──────────────────┘ │       │────│ └──────────────┘ │ Laufzeit…│
│ │ Titel · Autor  │ │            │Prot│ ═Regal══════════     │       │Prot│ ═Regal═══════════ │ [Anfragen]│
│ │ [Anfragen][Det]│ │            │Stat│ [▇][▇][▇][▇][▇]      │       │Stat│ [▇][▇][▇][▇][▇]  │ Weiter-  │
│ └────────────────┘ │            │    │ ═Neu von …══════     │       │    │ ═Neu von …═══════ │ stöbern: │
│ ═ Passt zu dir ═══ │            │▁▂▅▇│ [▇][▇][▇][▇]         │       │▁▂▅▇│ [▇][▇][▇][▇]     │ [Chips]  │
│ [▇][▇][▇] →        │            │Hell│                      │       │Hell│                  │         │
├────────────────────┤            └────┴──────────────────────┘       └────┴──────────────────┴─────────┘
│ Ent. Such. Anf. Büch. Mehr │     Seitenleiste 216 px                Seitenleiste 232 px, Detail 352 px
└────────────────────┘
```

- **iPhone:** Tab-Leiste unten (Daumenreichweite, `env(safe-area-inset-bottom)`), alle Tippflächen ≥ 44 px, Chips und Regale
  wischen seitlich mit Einrasten. Das Detail öffnet als Blatt von unten (Bottom Sheet).
- **iPad:** Seitenleiste mit Beschriftung, größere Cover im Regal (4–5 sichtbar), Touch und Zeiger werden beide
  unterstützt (`@media (hover: hover)` für Hover-Zustände).
- **MacBook:** Seitenleiste, Inhalt und feste Detailspalte. Ein Klick auf ein Cover füllt die Detailspalte, ohne die Seite
  zu verlassen.
- **Skalierung:** Titel in `clamp()`, Cover-Text in `cqw` (Container Queries), damit auch kleine Cover lesbar bleiben;
  Einrückung über eine fließende Randvariable `--gut`.

### Navigation auf dem iPhone

Fünf Tabs unten: **Entdecken · Suchen · Anfragen · Hörbücher · Mehr**. Alles Weitere steckt hinter **Mehr**: eine Liste mit
**Protokoll** (mit rotem Hinweis „1 Fehler", der auch als Punkt am Tab erscheint), **Statistik**, **Einstellungen** und dem
**Konto** (Name, Abmelden). Ein erneuter Tipp auf „Mehr" führt immer zurück zu dieser Liste. Auf iPad und Mac steht dieselbe
Struktur als Seitenleiste dauerhaft sichtbar (`iphone-2.jpg`, Spalte „Mehr").

### Farbwelt in den Einstellungen

Die Umschalter für Hell/Dunkel/OLED verschwinden aus der Kopfzeile und Seitenleiste. Sie wohnen unter
**Einstellungen → Anzeige** als vier Karten mit Vorschau: *Automatisch* (folgt dem Gerät), *Hell*, *Dunkel*, *OLED*. Die Wahl
gehört zum Konto, nicht zum Gerät. Die Datenbank hat dafür bereits `users.prefs` und die Schnittstelle `/api/auth/prefs`;
zusätzlich wird die Wahl lokal gemerkt, damit die Seite beim Laden nicht in der falschen Farbe aufblitzt. Als Option steht
„Nur dieses Gerät" bereit (z. B. OLED auf dem iPhone, Hell am Mac).

## 7. Stöber-Schleife: Was zum Weitermachen einlädt

Der Hauptgrund, in der App zu bleiben, ist ein Pfad, der nie in einer Sackgasse endet:

1. **Entdecken beginnt mit einem Vorschlag des Tages** („Weil du *Dracula* gehört hast") statt mit einer Liste.
2. **Vier Reiter bleiben:** *Für dich*, *Nach Genre* (Bestseller und Neuheiten je Genre), *Aus der Kritik* (Deutschlandfunk Kultur,
   Perlentaucher) und *Gefolgt*. Die Genre-Chips sind immer sichtbar. Regale sind seitlich wischbar und führen mit „Alle ansehen" weiter.
   Das Genre „Gratis" entfällt.
3. **Jedes Detail endet mit „Weiterstöbern":** *Mehr von diesem Autor*, *Gelesen von diesem Sprecher*, *Ähnlich wie …*,
   *Düster & atmosphärisch*. Ein Tipp öffnet das nächste Regal, nie eine leere Seite.
4. **Anfragen ist ein Tipp** und zeigt den Zustand gleich am Cover (angefragt → lädt → in der Bibliothek). Man muss die
   Seite dafür nicht verlassen.
5. **Genre-Chips** („Krimi", „Klassiker", „Gruselig" …) filtern, ohne neue Seite.
6. **Die Werkbank bleibt sichtbar** (Mini-Tonspur in der Seitenleiste, Hinweis auf dem iPhone): Man sieht, dass etwas
   passiert, und kommt mit einem Tipp zurück.

## 8. Die Seiten im Einzelnen

- **Entdecken:** Reiter *Für dich · Nach Genre · Aus der Kritik · Gefolgt*, darunter Genre-Chips. *Für dich* beginnt mit dem Vorschlag
  des Tages und drei Regalen. *Aus der Kritik* zeigt Karten mit „Besprochen bei … · Kritik lesen", Anfragen und Suchen. Mehrere
  Autoren oder Sprecher stehen als Fließtext („Jacob Grimm, Wilhelm Grimm", „… u. a."), nie als Kästen. (`mac-entdecken.jpg`,
  `mac-entdecken-kritik.jpg`, `ipad-entdecken.jpg`)
- **Hörbücher:** Werkbank mit Stationen und Tonspur oben, darunter „Ein Hörbuch braucht dich", dann das Regal; ein Schalter wechselt
  zwischen Regal und Liste. (`mac-hoerbuecher.jpg`)
- **Suchen:** großes Suchfeld ohne Zeile mit Quellen darüber; Treffer mit Cover und Anfragen-Knopf, darunter die beurteilten Releases mit
  Punktering und Urteilen. Abgelehnte Releases sind gedimmt und nennen den Grund. (`mac-suchen.jpg`)
- **Protokoll:** wie ein Werkstattbuch (Tagesüberschrift in Serife, Farbstreifen je Schweregrad, Quelle als Kennzeichen, Fehler
  hinterlegt). Neu: **Zeitraum 15 Min. · 60 Min. · 24 Std. · Alles**; die Seite zeigt, wie viele ältere Einträge ausgeblendet sind. (`mac-protokoll.jpg`)
- **Statistik:** vier Kennzahlen, Hörwoche, Genreband, Monatsverlauf und meistgehörte Autoren mit Buchfarben. Auf dem iPhone steht
  derselbe Inhalt untereinander, die Kennzahlen in zwei Spalten. (`mac-statistik.jpg`, `iphone-2.jpg`)
- **Einstellungen → Anzeige:** Farbwelt-Karten und „Gilt für". (`mac-einstellungen.jpg`)

## 9. Technische Umsetzung in Hörwerk

Die Oberfläche ist TypeScript in `frontend/src` und `static/style.css`, ausgeliefert vom Rust-Server. Der Umbau ist
überwiegend CSS und braucht wenig neue Logik.

| Thema | Vorschlag |
|---|---|
| **Design-Tokens** | Die Variablen in `static/style.css` (heute `--bg`, `--panel`, `--accent`, …) durch die Tokens des Prototyps ersetzen; die Namen bleiben, soweit möglich, damit der Umbau seitenweise geht. |
| **Dritte Farbwelt** | `frontend/src/theme.ts` kennt heute `auto`, `light`, `dark`. Neu: `oled`; „Automatisch" folgt weiter dem System (Hell/Dunkel), OLED wird bewusst gewählt. Die Texte laufen über `t(…)` und `static/i18n/en.json`. |
| **Namen** | Die heutige Zerschießung bei mehreren Autoren kommt von einer doppelt vergebenen CSS-Klasse: `.person-link` (Entdecken, Fließtext) wird von der späteren Regel für die Personenkarten überschrieben (`static/style.css`, Zeile 928). Im Umbau bekommen beide Verwendungen eigene Klassen. |
| **Protokoll-Zeitraum** | Die Einträge haben schon einen Zeitstempel; der Zeitraum filtert im Browser, der Server muss nichts ändern. Das Protokoll ist ein Puffer der letzten 3000 Einträge, „24 Std." zeigt also, was davon noch im Puffer liegt. |
| **Buchfarbe** | Beim Taggen aus dem Cover eine dominante Farbe berechnen (verkleinern, Median-Cut oder K-Means in Rust), in der Datenbank speichern und über die API als `color` mitgeben. Für Cover von Audible/BookBeat in Suche und Entdecken derselbe Weg beim Laden. Fehlt die Farbe, gilt ein Standardton. |
| **Schriften** | Keine Webfonts: Charter für Titel (Apple-Geräte liefern sie mit, Rückfall Georgia), Systemschrift für Bedienelemente. Das hält das Image klein, lädt nichts von fremden Servern und rendert auf jedem Gerät scharf. |
| **Tonspur** | Reiner DOM mit schmalen Balken, Anzahl nach Breite, im Prototyp 25 Zeilen JS. Für echte Kapitel kann die Höhe später aus den Kapitellängen kommen. |
| **Geräte** | Medienabfragen bei 780 px und 1240 px; `env(safe-area-inset-*)`, `dvh`, `viewport-fit=cover`; Tippflächen ≥ 44 px; `prefers-reduced-motion` bleibt respektiert. |
| **Barrierefreiheit** | Kontrast ≥ 4,5:1 (Skript als CI-Schritt), Fokusring in Akzentfarbe, Status nie nur über Farbe (Symbol oder Text). |

## 10. Vorgeschlagene Reihenfolge

| Phase | Inhalt | Ergebnis |
|---|---|---|
| 1 | Tokens und Farbwelten (Hell, Dunkel, OLED), Schrift, Cover-Komponente | alle Seiten sehen sofort wärmer und einheitlicher aus |
| 2 | Gerüst: Tab-Leiste (iPhone), Seitenleiste (iPad/Mac), Detailspalte | die Navigation ist auf jedem Gerät richtig |
| 3 | Hörbücher und Werkbank (Tonspur, Stationen, Regal) | der Kern von Hörwerk wird sichtbar |
| 4 | Entdecken mit Regalen und Weiterstöbern | die Stöber-Schleife |
| 5 | Suche und Protokoll | beurteilte Releases und Werkstattbuch |
| 6 | Statistik und Buchfarben | harmonische Diagramme |
| 7 | Feinschliff, Screenshots für README, Test auf echten Geräten | Abnahme |

**Tests je Phase:** `cargo test`, `python3 -m unittest discover -s tests` und `-s contract`, `npm run check`, die
Playwright-Klicktests (`npx playwright test`) und das Kontrastskript. Zusätzlich eine Screenshot-Matrix (3 Geräte × 3
Farbwelten × 5 Seiten) als sichtbare Abnahme.

## 11. Entschieden und offen

**Entschieden:** Hörbücher statt Bücher · Zeile „Audible, BookBeat, Indexer" in *Suchen* entfällt · Genre „Gratis" entfällt, „Aus der Kritik"
bleibt · Entdecken mit den vier Reitern · Protokoll mit Zeitraum · Farbwelt unter Einstellungen → Anzeige, pro Konto.

**Noch offen** (siehe Antwort im Chat): Texte ohne Mehrwert (Liste unten), Standardzeitraum im Protokoll, Farbwelt je Konto oder je
Gerät, Messing als zweiter Akzent, Regalbrett, Detailspalte am Mac, Buchfarbe aus dem Cover.

**Kandidaten für Streichungen (bitte je Gruppe Ja/Nein):**
1. Statuszeile in *Aus der Kritik*: „Stand: … · 231 besprochene Hörbücher warten noch auf ihr Hörbuch".
2. Hinweiszeile unter *Für dich*: „Passend zu dem, was du zuletzt gehört hast."
3. Das einklappbare „Was gezeigt wird" unter den Vorschlägen.
4. Die „So funktioniert's"-Boxen (sind schon abschaltbar).
5. Kleine Zeilen über den Überschriften („48 Hörbücher · 612 Stunden", „Dein Hörjahr", „Live · letzte 60 Min.").
6. Erklärzeile unter *Releases*: „Jedes Release wird vor dem Laden beurteilt – das Beste steht oben."
7. Untertitel in der Statistik („Womit du die Zeit verbringst", „Farben stammen von den Covern" …).
