# CrowdSec für Hörwerk

Hörwerk schreibt jedes Anmelde-Ereignis als eine Zeile nach `/config/logs/auth.log` (im AppData-Ordner, z. B.
`/mnt/user/appdata/hoerwerk/logs/auth.log`) und zusätzlich nach `docker logs`:

```
time=2026-10-31T17:02:03Z app=hoerwerk event=login_failed user="max" ip=203.0.113.7 reason=bad_password
time=2026-10-31T17:02:09Z app=hoerwerk event=login_blocked user="max" ip=203.0.113.7 reason=ip_blocked
time=2026-10-31T17:05:41Z app=hoerwerk event=login_ok user="Eva" ip=198.51.100.20
```

Ereignisse: `login_ok`, `login_failed` (Gründe `bad_password`, `unknown_user`, `disabled`, `bad_setup_code`,
`bad_old_password`), `login_blocked` (Hörwerks eigene Sperre: 5 Fehlversuche / 15 min je IP), `logout`, `setup`,
`password_changed`, `user_created`, `user_changed`, `user_deleted`, `sessions_revoked`, `forbidden`
(angemeldeter Nutzer ruft eine Admin-Funktion auf).

**Wichtig – echte IP:** Hinter Pangolin kommt jede Anfrage von der Newt-/Traefik-Verbindung. Deren Adresse muss in
`TRUSTED_PROXIES` stehen (z. B. das Docker-Netz `172.18.0.0/16` oder die IP des Newt-Containers); erst dann steht
in `ip=` die Adresse des Besuchers und nicht die des Proxys. Prüfen: einmal falsch anmelden, dann `auth.log` ansehen.

## Installation

1. **Parser** `parsers/hoerwerk-logs.yaml` → `/etc/crowdsec/parsers/s01-parse/`
2. **Szenarien** `scenarios/hoerwerk-bf.yaml` → `/etc/crowdsec/scenarios/`
   (5 Fehlversuche je IP in ~10 min; 3 verschiedene unbekannte Namen je IP)
3. **Erfassung** – eine der beiden Varianten aus `acquis-hoerwerk.yaml` nach `/etc/crowdsec/acquis.d/`:
   * **A:** CrowdSec-Agent auf dem Unraid-Server liest die Datei direkt und meldet an die LAPI auf dem VPS.
   * **B:** Vector schickt die Zeilen zum VPS (`vector-unraid.toml` → `vector-vps.toml`), der Agent dort liest sie.
4. `sudo systemctl reload crowdsec`, dann testen:
   `sudo cscli explain --file /pfad/auth.log --type hoerwerk` und nach ein paar Fehlversuchen `sudo cscli decisions list`.

Gesperrt wird am VPS vom Bouncer vor Pangolin/Traefik – die gesperrte IP erreicht Hörwerk gar nicht mehr.
