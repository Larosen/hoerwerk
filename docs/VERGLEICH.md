# hoerwerk:next ausprobieren – neben dem laufenden Hörwerk

`ghcr.io/larosen/hoerwerk:next` ist dieselbe App mit dem neuen Backend in Rust statt Python: dieselbe Oberfläche,
dieselbe Datenbank, dieselben Einstellungen. Bevor es das normale Image ersetzt, lässt es sich gefahrlos **daneben**
ausprobieren – auf einer Kopie der AppData, mit der Bibliothek nur zum Lesen.

Das laufende Hörwerk bleibt dabei unverändert und arbeitet ganz normal weiter.

## Was im Vergleichsbetrieb passiert – und was nicht

Mit `COMPARE_MODE=true` (im Unraid-Template schon gesetzt) gilt für die zweite Instanz:

| macht sie | macht sie nicht |
|---|---|
| zeigt Bibliothek, Anfragen, Statistik, Konten, Einstellungen aus der Kopie | startet keine Downloads (auch nicht per Knopf) |
| sucht bei Indexern, wenn du es in der Oberfläche anstößt | sucht nicht von selbst für offene Anfragen |
| liest Entdecken (Audible, Rezensionen) | schickt keine Benachrichtigungen (ntfy) |
| wandelt um, was du in **ihren eigenen** Eingang legst | lässt Audiobookshelf/Plex nicht neu einlesen |
| | schreibt nichts in die Bibliothek (nur lesend eingebunden) |

Ein Buch, das du in ihren Eingang legst, wird umgewandelt und bleibt dann mit einem Fehler beim Ablegen stehen –
die Bibliothek ist ja schreibgeschützt. Das ist so gewollt.

## Auf Unraid einrichten

1. **AppData kopieren.** Hörwerk im Docker-Tab kurz stoppen, dann im Terminal:

   ```sh
   cp -a /mnt/user/appdata/hoerwerk /mnt/user/appdata/hoerwerk-next
   mkdir -p /mnt/user/appdata/hoerwerk-next/eingang
   ```

   Hörwerk wieder starten. (Gestoppt, damit die Datenbank in einem sauberen Zustand kopiert wird.)

2. **Container anlegen.** Docker → *Add Container* → als *Template* die Adresse
   `https://raw.githubusercontent.com/Larosen/hoerwerk/main/unraid/hoerwerk-next.xml` eintragen. Prüfen:
   - *WebUI-Port* **8081** (nicht der des laufenden Hörwerk)
   - *AppData (Kopie)* `/mnt/user/appdata/hoerwerk-next` – **nie** den Ordner des laufenden Hörwerk
   - *Hörbuch-Bibliothek* dieselbe wie beim laufenden Hörwerk, Zugriff **Read Only**
   - *Eigener Eingang* `/mnt/user/appdata/hoerwerk-next/eingang/`
   - *PUID/PGID/UMASK* wie beim laufenden Hörwerk

3. **Öffnen:** `http://tower:8081` – Anmeldung mit denselben Konten wie im laufenden Hörwerk.

Im Protokoll steht beim Start: „Vergleichsbetrieb: keine Downloads, keine automatische Suche für Anfragen, keine
Benachrichtigungen, keine Scans in Audiobookshelf/Plex“.

### Mit docker compose

```yaml
  hoerwerk-next:
    image: ghcr.io/larosen/hoerwerk:next
    ports: ["8081:8080"]
    environment:
      COMPARE_MODE: "true"
      PUID: "99"
      PGID: "100"
    volumes:
      - ./hoerwerk-next:/config               # Kopie von ./hoerwerk (bei gestopptem Hörwerk kopiert)
      - ./hoerwerk-next/eingang:/inbox
      - /pfad/zur/bibliothek:/audiobooks:ro
    security_opt: ["no-new-privileges:true"]
    cap_drop: [ALL]
    cap_add: [CHOWN, SETUID, SETGID]
```

## Vergleichen

Am einfachsten im Browser: beide Adressen nebeneinander öffnen und durchklicken.

Gründlicher mit dem Skript `contrib/compare-next.py` (nur Python, nichts zu installieren). Es meldet sich bei
beiden an, liest dieselben Bereiche (Einstellungen, Konten, Bibliothek, Anfragen, Statistik, Quellen,
Benachrichtigungen, Hörverlauf, Entdecken) und zeigt, wo sich die Antworten unterscheiden. Es ändert nichts.

```sh
python3 compare-next.py http://tower:8080 http://tower:8081
```

```
✓ Einstellungen: gleich
✓ Konten: gleich
✓ Bibliothek (Bücher): gleich
✗ Anfragen: 1 abweichende Stelle
    [3].status
      alt: 'downloading'
      neu: 'searching'
…
```

Was seit dem Kopieren passiert ist (neue Anfragen, inzwischen abgelegte Bücher), unterscheidet sich natürlich –
dann einfach neu kopieren (Schritt 1) und den Container neu starten.

Ein Unterschied, der sich nicht so erklären lässt? Bitte als Issue melden, mit der Ausgabe des Skripts.

## Wieder entfernen

Container `hoerwerk-next` löschen und den Ordner `/mnt/user/appdata/hoerwerk-next`. Am laufenden Hörwerk ändert
sich dadurch nichts.
