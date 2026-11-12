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
RUN mkdir -p static && npx tsc -p tsconfig.json

# Stage 3: the server (rust/hoerwerk) and hw-run, the jail for ffmpeg/ffprobe (Landlock + seccomp, rust/hw-run).
# Cargo.lock pins every crate; --locked refuses a build that would change it. The server reads the settings and
# routes of the API contract at build time (contract/settings.json, contract/routes.json).
FROM rust:1.97-slim-trixie AS rust
WORKDIR /src/rust
COPY contract/settings.json contract/routes.json /src/contract/
COPY rust/ ./
RUN cargo build --release --locked -p hw-run -p hoerwerk && ./target/release/hw-run --status

# Stage 4: the app – Debian 13 "trixie" (stable, security updates via the weekly rebuild), one Rust program,
# our ffmpeg and the web UI. No Python, no interpreter at all.
FROM debian:trixie-slim
# Debian's security updates first (the base image is only rebuilt every few weeks; fixes like the one for
# zlib arrive in between), then tini for clean signal handling, CA certificates for HTTPS (Audible,
# indexers, ntfy), time zones for TZ
RUN apt-get update \
 && apt-get upgrade -y --no-install-recommends \
 && apt-get install -y --no-install-recommends tini ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=ffmpeg /opt/ffmpeg/bin/ffmpeg /opt/ffmpeg/bin/ffprobe /usr/local/bin/
COPY --from=rust /src/rust/target/release/hw-run /src/rust/target/release/hoerwerk /usr/local/bin/
COPY docker/entrypoint.sh /entrypoint.sh
COPY static/ /app/static/
COPY --from=web /src/static/*.js /app/static/
WORKDIR /app

# which commit this image was built from – shown under Settings → Health (set by the release build)
ARG COMMIT=""
ENV HOERWERK_COMMIT=$COMMIT \
    CONFIG_DIR=/config \
    INBOX_DIR=/inbox \
    LIBRARY_DIR=/audiobooks \
    PORT=8080 \
    PUID=99 \
    PGID=100 \
    UMASK=000

RUN mkdir -p /config /inbox /audiobooks && chmod +x /entrypoint.sh
VOLUME ["/config", "/inbox", "/audiobooks"]
EXPOSE 8080

LABEL org.opencontainers.image.title="Hörwerk" \
      org.opencontainers.image.description="Hörbuch-Automatik: Eingang -> M4B mit Kapiteln -> Audible-Tags -> Bibliothek (Audiobookshelf, Plex)" \
      org.opencontainers.image.source="https://github.com/Larosen/hoerwerk" \
      net.unraid.docker.webui="http://[IP]:[PORT:8080]/" \
      net.unraid.docker.icon="https://raw.githubusercontent.com/Larosen/hoerwerk/main/unraid/icon.png"

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["/usr/local/bin/hoerwerk", "health"]

ENTRYPOINT ["/usr/bin/tini", "--", "/entrypoint.sh"]
CMD ["/usr/local/bin/hoerwerk"]
