# Bolo API + admin on Plesk (PRIMARY deployment)

Primary: self-hosted Node service in `backend/node` (wraps the tested `backend/worker` code).
Cloudflare Worker (`wrangler.toml`) is an archived alternative. Nothing here is deployed yet.

## URLs

| URL | Served by |
| --- | --- |
| `https://bolo.thebetawebsite.com/` and all other paths | existing static website (unchanged) |
| `https://bolo.thebetawebsite.com/v1/...` | Node API (desktop `base_url` = `https://bolo.thebetawebsite.com`) |
| `https://officebolo.thebetawebsite.com/` | Office UI (API at `/api/...`, setup at `/setup`) |
| `https://bolo.thebetawebsite.com/office` and `/admin` | 301 to the same path on `https://officebolo.thebetawebsite.com` |

Health check: `GET /v1/health`.

## Proxy only these paths (do NOT enable Plesk "Node.js" for the whole domain)

Enabling the Plesk Node.js extension on the domain would hand every request to Node and
break the static site. Instead run Node as a background service on 127.0.0.1:8787 and proxy
three prefixes only.

**Plesk > Websites & Domains > bolo.thebetawebsite.com > Apache & nginx Settings >
"Additional nginx directives"** (if nginx proxies to Apache, which is the Plesk default):

```nginx
location ^~ /v1/ {
    proxy_pass http://127.0.0.1:8787;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
    client_max_body_size 36m;
    proxy_read_timeout 90s;
}
location = /office { return 301 https://officebolo.thebetawebsite.com/; }
location ^~ /office/ { rewrite ^/office/?(.*)$ https://officebolo.thebetawebsite.com/$1 permanent; }
location = /admin { return 301 https://officebolo.thebetawebsite.com/; }
location ^~ /admin/ { rewrite ^/admin/?(.*)$ https://officebolo.thebetawebsite.com/$1 permanent; }
```

Office itself is the `officebolo` subdomain, one shared Node process (do not start a second Passenger app). On that subdomain's Additional nginx directives:

```nginx
location ^~ / {
    proxy_pass http://127.0.0.1:8787;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto $scheme;
    client_max_body_size 36m;
    proxy_read_timeout 90s;
}
```

Before saving, confirm the static site has no existing `/v1`, `/office` or `/admin` folders or
routes. Plesk validates the directives; a bad one is rejected without touching the site.
(If the domain is Apache-only without nginx, use the equivalent `ProxyPass /v1/ http://127.0.0.1:8787/v1/`
and `ProxyPass /office http://127.0.0.1:8787/office` in "Additional Apache directives" for HTTPS.)

## Run the service (SSH as the subscription user)

```bash
cd ~/bolo-api              # OUTSIDE httpdocs, so nothing is web-readable
npm ci --omit=optional
# env file: ~/bolo-api/.env (chmod 600), keys from node/.env.example
# Preferred (built bundle):
pm2 start "node dist/server.mjs" --name bolo-api
# Or source: node --env-file=.env --import tsx node/server.ts
pm2 save && pm2 startup     # or a systemd unit / Plesk scheduled task "@reboot"
```

Must run as ONE instance (no cluster mode): state is in memory and snapshotted to Supabase.

## Supabase (project oahexfxwouuhplxdjxqf, ap-south-1)

Apply only `backend/node/state.sql` (table `public.bolo_state`, RLS forced, no policies,
anon/authenticated revoked, service_role only). The older `supabase/migrations/*` schema is
NOT used by the Node service; do not apply it.

## Environment values (see node/.env.example)

* `SUPABASE_URL=https://oahexfxwouuhplxdjxqf.supabase.co`
* `SUPABASE_SERVICE_ROLE_KEY` (server .env only)
* `SMTP2GO_API_KEY`, `SMTP2GO_SENDER` (verified sender on thebetawebsite.com)
* `GROQ_API_KEYS` (comma-separated, one per pool)
* `ADMIN_ACCOUNTS` (`role:email:Label;...`) — bootstrap Owner is `info@kingdomvision.com`
* `ADMIN_TOTP_SECRETS` (`email:BASE32;...`) — owner authenticator secret
* `OFFICE_HOST` — optional, default `officebolo.thebetawebsite.com`. That host serves Office at `/`
* `OFFICE_SETUP_TOKEN` — random ≥24-char one-time token; open `https://officebolo.thebetawebsite.com/setup` to set the owner password (disables itself after success even if env remains)
* Office email codes use the mail settings already required above: `SMTP2GO_SENDER` and either `SMTP_HOST` (plus `SMTP_PORT`, `SMTP_USER`, `SMTP_PASS` when the server needs auth) or `SMTP2GO_API_KEY`. The process exits listing any missing names and does not print values.
* Optional `ADMIN_OWNER_PASSWORD_HASH` — `scrypt$...` only; persisted snapshot hash always wins over env
* Additional Office staff are created in **Office → Team** (owner only); their password hashes and TOTP secrets persist in Supabase `bolo_state`
* optional `ADMIN_ALLOWED_IPS`, fair-use limits

## Secrets handling

* Type secrets directly into `~/bolo-api/.env` over SSH (or Plesk File Manager), `chmod 600`, outside `httpdocs`.
* Never in git, chat, tickets or logs. The service never logs keys, OTPs, audio or text.
* Generate each admin TOTP secret on the server (`node -e "const a='ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';console.log([...crypto.getRandomValues(new Uint8Array(20))].map(b=>a[b%32]).join(''))"`) and scan it into an authenticator app.
* Rotate: edit `.env`, `pm2 restart bolo-api`.
