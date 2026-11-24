# CrowdSec for Hörwerk

Hörwerk writes every sign-in event as one line to `/config/logs/auth.log` (in the AppData folder, e.g.
`/mnt/user/appdata/hoerwerk/logs/auth.log`) and also to `docker logs`:

```
time=2026-10-31T17:02:03Z app=hoerwerk event=login_failed user="max" ip=203.0.113.7 reason=bad_password
time=2026-10-31T17:02:09Z app=hoerwerk event=login_blocked user="max" ip=203.0.113.7 reason=ip_blocked
time=2026-10-31T17:05:41Z app=hoerwerk event=login_ok user="Eva" ip=198.51.100.20
```

Events: `login_ok`, `login_failed` (reasons `bad_password`, `unknown_user`, `disabled`, `bad_setup_code`,
`bad_old_password`), `login_blocked` (Hörwerk's own block: 5 failed attempts / 15 min per IP), `logout`, `setup`,
`password_changed`, `user_created`, `user_changed`, `user_deleted`, `sessions_revoked`, `totp_on`, `totp_off`,
`ip_unblocked` (an admin lifts Hörwerk's block of an address early),
`forbidden` (a signed-in user calls an admin function). A wrong two-factor code is
`login_failed … reason=bad_totp` and so counts towards the scenario too.

**Important – the real IP:** behind Pangolin every request comes from the Newt/Traefik connection. Its address has to be
in `TRUSTED_PROXIES` (e.g. the Docker network `172.18.0.0/16` or the IP of the Newt container); only then does `ip=`
hold the visitor's address and not the proxy's. To check: sign in wrongly once, then look at `auth.log`.

## Installation

1. **Parser** `parsers/hoerwerk-logs.yaml` → `/etc/crowdsec/parsers/s01-parse/`
2. **Scenarios** `scenarios/hoerwerk-bf.yaml` → `/etc/crowdsec/scenarios/`
   (5 failed attempts per IP in ~10 min; 3 different unknown names per IP)
3. **Acquisition** – one of the two variants from `acquis-hoerwerk.yaml` into `/etc/crowdsec/acquis.d/`:
   * **A:** a CrowdSec agent on the Unraid server reads the file directly and reports to the LAPI on the VPS.
   * **B:** Vector ships the lines to the VPS (`vector-unraid.toml` → `vector-vps.toml`), the agent there reads them.
4. `sudo systemctl reload crowdsec`, then test:
   `sudo cscli explain --file /path/auth.log --type hoerwerk` and, after a few failed attempts, `sudo cscli decisions list`.

The block happens on the VPS, by the bouncer in front of Pangolin/Traefik – the blocked IP never reaches Hörwerk.
