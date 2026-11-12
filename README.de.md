# Hörwerk

**Hörbücher beschaffen, anfragen und entdecken – und am Ende liegt jedes Buch als eine saubere, getaggte M4B in der Bibliothek.**

[![Build](https://github.com/Larosen/hoerwerk/actions/workflows/docker.yml/badge.svg)](https://github.com/Larosen/hoerwerk/actions/workflows/docker.yml)
[![Release](https://img.shields.io/github/v/release/Larosen/hoerwerk)](CHANGELOG.md)
[![Lizenz: MIT](https://img.shields.io/badge/Lizenz-MIT-green)](LICENSE)

[English](README.md) · **Deutsch**

![Ein Ordner mit MP3s landet im Eingang und wird zu einer M4B in der Bibliothek](docs/screenshots/de/demo.gif)

Hörwerk ist entstanden, weil mein Hörbuchordner ein Chaos war: CD-Rips in `CD1`/`CD2`, MP3s mit Namen wie
`Track 01`, fehlende Cover, und die Familie hat neue Bücher per Nachricht bestellt. Zusammenfügen mit dem einen
Werkzeug, Taggen mit dem anderen und Suchen von Hand hat pro Buch einen Abend gekostet. Hörwerk läuft auf dem NAS neben
Audiobookshelf oder Plex und erledigt das alles in einem Durchgang.

## Was es kann

- **Eingang → Bibliothek, ohne Zutun.** Ordner ablegen (MP3, M4A, FLAC, `CD1`/`CD2` …) und **eine M4B mit
  Kapiteln** bekommen, abgelegt als `Autor/Serie/03 - Titel`, mit Cover, Sprecher, Serie und Beschreibung von Audible.
- **Anfragen wie bei Overseerr.** Familienkonten entdecken Bücher und fragen sie an; du gibst frei, Hörwerk besorgt
  sie und zeigt den Fortschritt von Download und Umwandlung.
- **Eine Suche, die Hörbücher versteht.** Jedes Release wird vor dem Laden beurteilt: richtiges Buch, richtige
  Sprache, ungekürzt und die *richtige Aufnahme* – nicht die Lesung, wenn das Hörspiel gefragt war.
- **Freie Hörbücher eingebaut.** LibriVox (gemeinfrei) auf Deutsch und Englisch, mit einem Klick in den Eingang.
- **Eine aufgeräumte Bibliothek.** Erkennt Doppelte, wandelt alte MP3-Bücher um, räumt Kapitelnamen auf, sucht für
  ein vorhandenes Buch ein besseres Release – und löscht die alte Fassung nie ohne geprüfte neue.
- **Klein und abgesichert.** Ein Rust-Programm, eigenes schlankes ffmpeg in einer Sandbox, Zweitfaktor, signierte Images.

| Anfragen mit Fortschritt | Entdecken |
|---|---|
| ![Anfragen](docs/screenshots/de/requests.png) | ![Entdecken](docs/screenshots/de/discover.png) |

![Bibliothek](docs/screenshots/de/library.png)

## Der Vergleich

Bisher brauchte man dafür mehrere Werkzeuge und viel Handarbeit:

| | Zu M4B zusammenfügen | Audible-Tags | Suchen & Laden | Anfragen für die Familie |
|---|---|---|---|---|
| [m4b-tool](https://github.com/sandreas/m4b-tool) | ✅ Kommandozeile | – | – | – |
| [beets-audible](https://github.com/Neurrone/beets-audible) | – | ✅ Kommandozeile | – | – |
| Readarr | – | – | ✅ (eingestellt, nie aus der Beta) | – |
| **Hörwerk** | ✅ automatisch | ✅ automatisch | ✅ je Release beurteilt | ✅ |

Außerdem gibt es Listenarr, ein neueres Projekt für Hörbuch-Downloads – schau, was besser zu dir passt.

## Schnellstart

**Unraid:** *Docker → Add Container*, als Template
`https://raw.githubusercontent.com/Larosen/hoerwerk/main/unraid/hoerwerk.xml`, Eingang und Bibliothek eintragen,
`http://<unraid-ip>:8080` öffnen und das Admin-Konto anlegen.

**docker compose:**

```bash
curl -O https://raw.githubusercontent.com/Larosen/hoerwerk/main/docker-compose.yml
# die beiden Ordnerpfade und PUID/PGID (siehe `id`) eintragen, dann:
docker compose up -d
```

Image: `ghcr.io/larosen/hoerwerk:latest` (oder eine feste Version wie `:7.17.1`) – amd64, jede Woche für
Sicherheitsupdates neu gebaut, mit cosign signiert.

## Was es nicht ist

- **Kein Player.** Hörwerk füllt die Bibliothek; gehört wird mit Audiobookshelf, Plex oder jeder App, die M4B spielt.
- **Keine Quelle für Bücher.** Es arbeitet mit *deinen* Dateien, *deinem* SABnzbd und *deinen* Indexern. Die einzige
  eingebaute Quelle ist LibriVox. Es entfernt keinen Kopierschutz.
- **Nur amd64** bisher; arm64 steht auf der [Roadmap](docs/ROADMAP.md).

## Dokumentation

| | |
|---|---|
| [Hörbücher](docs/de/guide.md) | Eingang, Hochladen, Zuordnung, Doppelte, Kapitel, Fassungen, Audiobookshelf und Plex |
| [Downloads, Entdecken, Anfragen](docs/de/downloads.md) | SABnzbd und Indexer, LibriVox, Vorschläge, Anfragen der Familie |
| [Konfiguration](docs/de/configuration.md) | Konten, Zugriff von außen, Benachrichtigungen, Variablen, Backup |
| [Sicherheit](SECURITY.de.md) | wovor Hörwerk schützt und wie das getestet wird |
| [Changelog](CHANGELOG.md) · [Roadmap](docs/ROADMAP.md) · [Entwicklung](docs/de/development.md) | |

> Hörwerk verwaltet Audiodateien, die du nutzen darfst – eigene Käufe, eigene Aufnahmen oder gemeinfreie Werke.
> Es enthält keine Medien und steht in keiner Verbindung zu Audible, Amazon, Apple, LibriVox oder anderen genannten
> Diensten. Die Bücher in den Screenshots sind gemeinfreie Klassiker mit ausgedachten Covern.

## Lizenz

[MIT](LICENSE). Das Docker-Image enthält außerdem FFmpeg, aus dem offiziellen Quelltext ohne GPL-Teile gebaut und
damit unter der [LGPL 2.1](https://www.ffmpeg.org/legal.html); das Bauskript ist [docker/build-ffmpeg.sh](docker/build-ffmpeg.sh).
Inspiriert von [auto-m4b](https://github.com/seanap/auto-m4b) und [beets-audible](https://github.com/Neurrone/beets-audible);
Kapiteldaten über [Audnexus](https://github.com/laxamentumtech/audnexus).
