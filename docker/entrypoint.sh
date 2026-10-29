#!/bin/sh
# Runs the app as PUID:PGID with UMASK – the usual convention on Unraid
# (defaults 99:100 = nobody:users, so files stay accessible over SMB).
set -e

PUID="${PUID:-99}"
PGID="${PGID:-100}"
UMASK="${UMASK:-000}"
export UMASK

if [ "$(id -u)" = "0" ]; then
    if ! getent group "$PGID" >/dev/null 2>&1; then
        groupadd -o -g "$PGID" renamer
    fi
    if ! getent passwd "$PUID" >/dev/null 2>&1; then
        useradd -o -u "$PUID" -g "$PGID" -d /config -M -s /usr/sbin/nologin renamer
    fi
    mkdir -p /config
    chown "$PUID:$PGID" /config 2>/dev/null || true
    # Unmapped volumes are created root-owned; hand them to the app user
    for d in /inbox /audiobooks; do
        if [ -d "$d" ] && [ "$(stat -c %u "$d")" = "0" ]; then
            chown "$PUID:$PGID" "$d" 2>/dev/null || true
        fi
    done
    umask "$UMASK"
    echo "Better Renamer: starte als UID $PUID / GID $PGID, umask $UMASK"
    exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups -- "$@"
fi

# Already started as a non-root user (e.g. `user:` in docker-compose)
umask "$UMASK"
exec "$@"
