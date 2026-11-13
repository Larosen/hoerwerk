# Plan: Hörwerk für die Familie – Anfragen wie Seerr, Konten, Internet-tauglich

Stand: 31.10.2026 – **alle fünf Schritte umgesetzt** (Version 7.0.0), Test auf dem Server folgt. Ziel: Hörwerk über **Pangolin** ins Internet stellen, Familienmitglieder fragen Hörbücher an,
Hörwerk besorgt sie selbstständig. Der Admin sieht und steuert alles.

## Leitlinien

1. **Wunsch statt Release.** Nutzer wählen ein *Buch* (Audible/BookBeat), nie ein Release. Hörwerk sucht, bewertet,
   lädt, prüft und legt ab. Releases sieht nur, wer die **interaktive Suche**
   öffnet (Admin, oder Nutzer mit Recht dazu) – inklusive der abgelehnten Treffer und dem Grund der Ablehnung.
2. **Sicher ab Werk.** Ohne Anmeldung nichts außer Login-Seite, Logo und Health-Check. Jeder Fehlversuch wird mit
   echter Client-IP protokolliert, in einem Format, das CrowdSec direkt versteht.
3. **Rechte auf dem Server.** Die Oberfläche blendet aus, *erlaubt* wird aber nur, was der Server prüft – jede
   API-Route hat eine Zugriffsstufe (öffentlich / Nutzer / Admin), Standard ist Admin.
4. **Kleine, prüfbare Schritte.** Jeder Schritt ist ein eigener PR mit Tests, der für sich nutzbar ist.

## Rollen und Rechte

| | Admin | Nutzer |
|---|---|---|
| Entdecken, Bücher anfragen | ✓ | ✓ (mit Monatskontingent) |
| Eigene Anfragen sehen / zurückziehen | ✓ | ✓ |
| Anfragen anderer sehen | ✓ | nur mit Recht *„Alle Anfragen sehen“* |
| Anfragen freigeben / ablehnen | ✓ | – (Recht *„Automatisch freigeben“* überspringt die Freigabe) |
| Interaktive Suche (Releases) | ✓ | nur mit Recht *„Interaktive Suche“* |
| Bibliothek ansehen („Haben wir schon?“) | ✓ | ✓ (nur lesen) |
| Eingang, Konvertierung, Einstellungen, Quellen, Protokoll, Nutzer | ✓ | – |

Kontingent: Anzahl Anfragen pro Kalendermonat, global voreingestellt, je Nutzer überschreibbar (leer = Standard,
0 = keine Anfragen, ∞ für Admins). Abgelehnte und zurückgezogene Anfragen zählen nicht.

## Schritte

### Schritt 1 – Konten und Anmeldung (Fundament)
- Tabellen `users` und `sessions` in der bestehenden SQLite-Datenbank (Migration 2).
- Passwörter mit **scrypt** (Python-Standardbibliothek, N=2¹⁵, r=8, p=1, Salz je Nutzer), Mindestlänge 10,
  konstante Prüfzeit auch für unbekannte Namen (keine Nutzer-Erkennung über die Antwortzeit).
- **Sitzungen** statt Basic Auth: zufälliges Token (256 Bit), in der DB nur als SHA-256, Cookie `HttpOnly`,
  `SameSite=Lax`, `Secure` hinter HTTPS; „Angemeldet bleiben“ 30 Tage (gleitend), sonst 12 Stunden.
  Abmelden, „überall abmelden“, Sitzungen werden beim Passwortwechsel beendet.
- **Login-Seite** (eigene, schlanke Seite, gleiche Gestaltung), **Erst-Einrichtung**: gibt es noch keinen Nutzer,
  wird der Admin angelegt – aus dem Heimnetz direkt, von außen nur mit dem Einrichtungs-Code aus dem Container-Log.
  Vorhandenes `AUTH_USER`/`AUTH_PASS` wird beim ersten Start automatisch zum Admin-Konto.
- **Bremse gegen Raten:** je IP höchstens 5 Fehlversuche in 15 Minuten, danach 15 Minuten gesperrt (HTTP 429);
  je Konto wachsende Verzögerung. Ein Angreifer kann so kein Familienkonto aussperren, nur sich selbst.
- **Echte Client-IP hinter Pangolin:** `TRUSTED_PROXIES` (IP/CIDR der Pangolin-/Newt-Verbindung). Nur von dort wird
  `X-Forwarded-For` geglaubt – von rechts gelesen, die erste nicht vertrauenswürdige Adresse zählt.
- **Sicherheitsprotokoll** `/config/logs/auth.log` (+ stdout), eine Zeile je Ereignis, logfmt:
  `time=… app=hoerwerk event=login_failed user="max" ip=203.0.113.7 reason=bad_password`
  Ereignisse: `login_ok`, `login_failed`, `login_blocked`, `logout`, `setup`, `password_changed`,
  `user_created/changed/deleted`, `forbidden`. Rotation bei 10 MB, 5 Dateien.
- **CrowdSec-Paket** in `contrib/crowdsec/`: Parser, Szenario (z. B. 5 Fehlversuche in 10 min → Bann),
  `acquis.yaml`-Beispiel und Vector-Beispiel (Datei lesen → an den VPS/LAPI-Tunnel schicken).
- **Nutzerverwaltung** (Admin): anlegen, Rolle, Rechte, Kontingent, sperren, Passwort zurücksetzen, löschen;
  jeder Nutzer: eigenes Passwort ändern, Sitzungen ansehen/beenden.
- Server-seitige **Zugriffsstufe je Route**; Nutzer sehen vorerst *Entdecken* und die *Bibliothek* (lesen).

### Schritt 2 – Anfragen statt Merkliste (Seerr-Prinzip)
- Tabelle `requests`: Buch (ASIN, Titel, Autoren, Cover, Laufzeit, Shop), Nutzer, Zeitpunkte, Status:
  `wartet auf Freigabe → freigegeben → wird gesucht → lädt → wird verarbeitet → verfügbar`
  (oder `abgelehnt` mit Grund, `fehlgeschlagen`). Die bisherige „Gesucht“-Liste wird übernommen (Admin als Anfrager).
- „Anfragen“-Knopf statt „Merken“ auf allen Karten; „Haben wir schon“ / „Schon angefragt von …“ (je nach Recht).
- Kontingent-Prüfung beim Anfragen, Anzeige „noch 3 von 5 diesen Monat“.
- Admin: Freigabe-Liste mit Ein-Klick-Freigabe/Ablehnung; ntfy „Neue Anfrage“.
- Sichtbarkeit nach Recht *„Alle Anfragen sehen“*.

### Schritt 3 – Automatisch besorgen
- **Release-Bewertung** mit nachvollziehbaren Gründen: Autor + Titel passen, Sprache, ungekürzt, Format (M4B > M4A > MP3),
  Größe plausibel zur Laufzeit (kbit/s), Alter/Grabs, keine Pakete fremder Bände; harte Ausschlüsse mit Grund
  („anderer Autor“, „gekürzt“, „Größe passt nicht zur Laufzeit“, „schon fehlgeschlagen“).
- Bester passender Treffer geht von selbst an SABnzbd (mit ASIN als Hinweis → sichere Erkennung beim Import).
- **Verfolgung:** SABnzbd-Status → Pipeline-Job → abgelegt = *verfügbar*; Download fehlgeschlagen oder Import passt
  nicht zur ASIN → Release auf die Sperrliste, nächstbester Treffer; nichts gefunden → erneut alle 12 h.
- Benachrichtigung an den Anfrager (eigenes ntfy-Topic im Profil, optional) und Status in der App.

### Schritt 4 – Interaktive Suche (nur bei Bedarf)
- Je Anfrage: alle gefundenen Releases mit Bewertung und Ablehnungsgrund, auch die verworfenen; manuell laden,
  Release sperren, Suche mit anderem Begriff. Ersetzt für Nutzer den heutigen Reiter *Suchen*; der Admin behält
  die freie Suche.

### Schritt 5 – Härtung für den Dauerbetrieb im Internet
- Optional **TOTP-Zweitfaktor** (Authenticator-App) – Pflicht für Admins einstellbar.
- **Audit-Protokoll** der Admin-Aktionen (Freigaben, Nutzeränderungen, Einstellungen) in der App.
- `ALLOW_PUBLIC`/IP-Schranke entfällt mit Konten; `ALLOWED_HOSTS` bleibt (z. B. `hoerbuecher.example.de`).
- Sicherheitsheader prüfen (HSTS macht Pangolin), Größenlimits, Tests gegen Brute Force, Session-Fixation, CSRF,
  IDOR (fremde Anfragen per ID), Rechteausweitung (Nutzer → Admin-API).
- SECURITY.md/README: Pangolin-Ressource, `TRUSTED_PROXIES`, CrowdSec/Vector, Backup.

## Was du außerhalb von Hörwerk brauchst

- **Pangolin:** Ressource auf `http://hoerwerk:8080` (bzw. Unraid-IP:Port), HTTPS am VPS. Pangolins eigene
  Anmeldung (SSO) kann **zusätzlich** davor bleiben – doppelt sicher, aber zweimal anmelden; für die Familie reicht
  Hörwerks Login plus CrowdSec. Die IP, von der Newt bei Hörwerk ankommt, gehört in `TRUSTED_PROXIES`.
- **CrowdSec:** Vector liest `/config/logs/auth.log` (AppData-Ordner) und schickt die Zeilen zum CrowdSec-Agenten
  am VPS; dort Parser + Szenario aus `contrib/crowdsec/` installieren; der Bouncer am VPS (Traefik/Pangolin) sperrt.
- **ntfy:** für Familienmitglieder optional je eigenes Topic.
