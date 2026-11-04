# Stage 1: our own minimal FFmpeg 9 – only the audio formats Hörwerk reads, AAC/M4B output,
# no network code, no external libraries, hardened toolchain (docker/build-ffmpeg.sh).
# Source: official FFmpeg repository (GitHub mirror), release tag pinned to its commit.
FROM debian:trixie-slim AS ffmpeg
ARG FFMPEG_VERSION=9.0.2
ARG FFMPEG_COMMIT=946fcce07b6dcd0331c8cc609192aeff5e1924f8
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential nasm pkg-config git ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY docker/build-ffmpeg.sh /usr/local/bin/build-ffmpeg.sh
RUN git clone --quiet --depth 1 --branch "n${FFMPEG_VERSION}" https://github.com/FFmpeg/FFmpeg.git /src \
 && test "$(git -C /src rev-parse HEAD)" = "$FFMPEG_COMMIT" \
 && sh /usr/local/bin/build-ffmpeg.sh /src /opt/ffmpeg \
 && ! /opt/ffmpeg/bin/ffmpeg -hide_banner -protocols | grep -qE "^ *(http|https|tcp|udp|rtmp|hls)$"

# Stage 2: the web UI – TypeScript (frontend/src) compiled to plain scripts. Runs on the build
# machine's own platform: the output is the same JavaScript for every image architecture.
FROM --platform=$BUILDPLATFORM node:26-slim AS web
WORKDIR /src
COPY package.json package-lock.json tsconfig.json ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY frontend/ frontend/
RUN mkdir -p app/static && npx tsc -p tsconfig.json

# Stage 3: the app. Python 3.15 on Debian 13 "trixie" (stable, security updates via the weekly rebuild).
FROM python:3.15-slim-trixie

# tini for clean signal handling; ffmpeg/ffprobe from stage 1 (only libc/libm needed)
RUN apt-get update \
 && apt-get install -y --no-install-recommends tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=ffmpeg /opt/ffmpeg/bin/ffmpeg /opt/ffmpeg/bin/ffprobe /usr/local/bin/

WORKDIR /app
COPY requirements.txt /tmp/requirements.txt
# pip is only needed for this one install: afterwards it goes, with the wheels Python brings along
# (ensurepip, setuptools). Their bundled libraries (urllib3, msgpack …) are never used by Hörwerk, but would
# carry their vulnerabilities into the image. The import check makes sure nothing the app needs went with them.
RUN pip install --no-cache-dir -r /tmp/requirements.txt && rm /tmp/requirements.txt \
 && pip uninstall -y pip setuptools wheel 2>/dev/null; \
    STDLIB="$(python -c 'import sysconfig; print(sysconfig.get_paths()["stdlib"])')" \
 && rm -rf "$STDLIB/ensurepip" /usr/local/bin/pip /usr/local/bin/pip3* \
 && find / -xdev \( -name '*.whl' -o -name 'setuptools*' -o -name 'pip-*.dist-info' \) -path '*python3*' \
      -exec rm -rf {} + 2>/dev/null; \
    python -c "import mutagen, yaml, sqlite3, ssl; print('python deps ok')"
COPY docker/entrypoint.sh /entrypoint.sh
COPY app/ /app/
COPY --from=web /src/app/static/*.js /app/static/

ENV CONFIG_DIR=/config \
    INBOX_DIR=/inbox \
    LIBRARY_DIR=/audiobooks \
    PORT=8080 \
    PUID=99 \
    PGID=100 \
    UMASK=000 \
    PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1

RUN mkdir -p /config /inbox /audiobooks && chmod +x /entrypoint.sh
VOLUME ["/config", "/inbox", "/audiobooks"]
EXPOSE 8080

LABEL org.opencontainers.image.title="Hörwerk" \
      org.opencontainers.image.description="Hörbuch-Automatik: Eingang -> M4B mit Kapiteln -> Audible-Tags -> Bibliothek (Audiobookshelf, Plex)" \
      org.opencontainers.image.source="https://github.com/Larosen/rename_merge" \
      net.unraid.docker.webui="http://[IP]:[PORT:8080]/" \
      net.unraid.docker.icon="https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/icon.png"

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
  CMD python -c "import os, urllib.request; urllib.request.urlopen('http://127.0.0.1:%s/api/health' % os.environ.get('PORT', '8080'), timeout=4).read()" || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/entrypoint.sh"]
CMD ["python", "main.py"]
