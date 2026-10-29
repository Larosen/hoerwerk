# Stage 1: our own minimal FFmpeg 9 – only the audio formats Better Renamer reads, AAC/M4B output,
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
FROM --platform=$BUILDPLATFORM node:24-slim AS web
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
RUN pip install --no-cache-dir -r /tmp/requirements.txt && rm /tmp/requirements.txt
COPY docker/entrypoint.sh /entrypoint.sh
COPY app/ /app/
COPY --from=web /src/app/static/*.js /app/static/

ENV DATA_DIR=/data \
    CONFIG_DIR=/config \
    INBOX_DIR=/inbox \
    LIBRARY_DIR=/audiobooks \
    PORT=8080 \
    PUID=99 \
    PGID=100 \
    UMASK=000 \
    PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1

RUN mkdir -p /data /config /inbox /audiobooks && chmod +x /entrypoint.sh
VOLUME ["/data", "/config", "/inbox", "/audiobooks"]
EXPOSE 8080

LABEL org.opencontainers.image.title="Better Renamer" \
      org.opencontainers.image.description="Stapel-Umbenennen im Browser plus Hörbuch-Automatik: Eingang -> M4B mit Kapiteln -> Audible-Tags -> Bibliothek" \
      org.opencontainers.image.source="https://github.com/Larosen/rename_merge" \
      net.unraid.docker.webui="http://[IP]:[PORT:8080]/" \
      net.unraid.docker.icon="https://raw.githubusercontent.com/Larosen/rename_merge/main/unraid/icon.png"

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
  CMD python -c "import os, urllib.request; urllib.request.urlopen('http://127.0.0.1:%s/api/health' % os.environ.get('PORT', '8080'))" || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/entrypoint.sh"]
CMD ["python", "main.py"]
