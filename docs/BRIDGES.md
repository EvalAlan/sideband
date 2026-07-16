# Matrix bridge bundle (Phase 2)

This bundle runs a private **Matrix/Synapse aggregation account** for Sideband and
mautrix bridges. Telegram and Discord are the supported baseline. Meta Messenger
and Google Chat are opt-in because they depend on unofficial/browser protocols
that providers can change without notice.

> **Privacy boundary:** a bridged conversation is **not a private Sideband
> conversation**. Messages, metadata, attachments and provider credentials pass
> through Synapse, a mautrix bridge and the upstream provider. Matrix application
> services can impersonate users in their registered namespace. Do not treat a
> bridge room as Sideband E2E, do not expose port 8008 publicly without TLS and
> access controls, and protect `bridges/data/` as credential-bearing state.

The compose port binds to `127.0.0.1` by default. It uses SQLite for an easy
single-user deployment. For a public or multi-user homeserver, use PostgreSQL,
a TLS reverse proxy, backups, TURN, federation configuration and the Synapse
production checklist.

## Components and support status

| Component | Image in this bundle | Status |
|---|---|---|
| Synapse | `matrixdotorg/synapse:v1.156.0` | Official reference homeserver image |
| Telegram | `dock.mau.dev/mautrix/telegram:v0.2606.0` | Baseline; requires a Telegram API ID/hash |
| Discord | `dock.mau.dev/mautrix/discord:v0.7.6` | Baseline; user accounts carry a provider ban risk; bot login is safer |
| Meta | `dock.mau.dev/mautrix/meta:v0.2606.0` | Optional `meta` profile; unofficial web APIs/cookies, account checks possible |
| Google Chat | `dock.mau.dev/mautrix/googlechat:v0.5.2` | Optional `googlechat` profile; legacy Python/cookie login and device-bound-session limitations |

mautrix documents that `:latest` tracks the latest **commit**, not the latest
release, so this bundle pins release tags. The tags and manifests above were
checked against the official projects/registries on 2026-07-15. Review upstream
release notes before upgrading.

The desktop AppImage bundles `sideband-bridge-matrix`. For a standalone
CLI/TUI installation, run `./build.sh tui` and keep both
`target/release/sideband` and `target/release/sideband-bridge-matrix` in the
same directory. Android exposes the bridge APIs but cannot run the desktop
sidecar yet, so provider setup is disabled there.

## 1. Initialize Synapse and bridge configs

Requirements: Docker Engine with Compose v2, about 2 GiB free RAM, and a
Sideband build that includes `sideband-bridge-matrix` next to the `sideband`
executable.

```sh
cd bridges
cp .env.example .env
# Edit MATRIX_SERVER_NAME before generating anything. It becomes the permanent
# suffix of Matrix IDs. `localhost` is suitable only for local evaluation.
$EDITOR .env
mkdir -p data/{synapse,telegram,discord,meta,googlechat}

docker compose pull synapse telegram discord
docker compose run --rm synapse generate
docker compose run --rm telegram
docker compose run --rm discord
```

The first mautrix run intentionally exits after creating `config.yaml`. Merge
the matching `*.required.example.yaml` into each generated config; those files
are partial examples, not replacements. At minimum:

* set every `homeserver.domain` to `MATRIX_SERVER_NAME`;
* use `http://synapse:8008` for `homeserver.address` (not `localhost` inside a container);
* use the compose service address (`http://telegram:29317` or
  `http://discord:29334`) for `appservice.address`;
* set the listener hostname to `0.0.0.0` and use a separate SQLite file per bridge;
* replace the example permission domain/MXID with your server name and Matrix
  admin account; do not grant `admin` to `"*"`;
* for Telegram, obtain your own `network.api_id` and `network.api_hash` from
  <https://my.telegram.org/apps>. Never publish the hash.

Generate registration files with the now-edited configs. The second run also
exits intentionally:

```sh
docker compose run --rm telegram
docker compose run --rm discord
test -f data/telegram/registration.yaml
test -f data/discord/registration.yaml
```

Do not hand-edit generated tokens. Changes to the homeserver domain,
appservice address/ID/bot/namespace or AS/HS tokens require regenerating the
registration and restarting Synapse.

## 2. Register application services and create the Matrix user

Edit `data/synapse/homeserver.yaml` and add this top-level YAML:

```yaml
app_service_config_files:
  - /bridges/telegram/registration.yaml
  - /bridges/discord/registration.yaml
```

For local evaluation, create the Matrix account Sideband will use. Put a long
random value in `homeserver.yaml` temporarily:

```yaml
registration_shared_secret: "REPLACE_WITH_OUTPUT_OF_OPENSSL_RAND_HEX_32"
```

Then start Synapse, create the user, and remove the shared secret afterward:

```sh
openssl rand -hex 32
docker compose up -d synapse
docker compose exec synapse register_new_matrix_user \
  http://localhost:8008 -c /data/homeserver.yaml
# Choose localpart `sideband`, a strong unique password, and admin=yes.
# Remove registration_shared_secret, then:
docker compose restart synapse
```

Start the baseline bridges:

```sh
docker compose up -d telegram discord
docker compose ps
docker compose logs --tail=100 synapse telegram discord
curl --fail http://127.0.0.1:8008/_matrix/client/versions
```

If Synapse reports an unreadable/missing registration, check the container path
and YAML indentation. If a bridge loops, verify its homeserver domain/address,
appservice address, database, permissions and that its registration was made
from the current config.

## 3. Provider login

Use a normal Matrix client pointed at `http://127.0.0.1:8008` for local testing,
log in as `@sideband:<MATRIX_SERVER_NAME>`, and open an unencrypted direct room
with each bot. Keeping management rooms unencrypted avoids bridge key-management
setup; this does **not** make provider traffic private.

### Telegram (`@telegrambot:<server>`)

* QR: send `login qr`, then Telegram mobile **Settings → Devices → Link Desktop
  Device** and scan.
* Phone: send `login phone +15551234567`; enter the six-digit code delivered to
  an already logged-in official Telegram client, then 2FA if prompted.
* Relay bot: `login bot <BOT_TOKEN>` (more limited).

Telegram no longer permits registration from third-party clients. Use an
established account; upstream warns new accounts using third-party clients can
look suspicious.

### Discord (`@discordbot:<server>`)

* Send `login-qr`, scan and approve in Discord mobile. CAPTCHA challenges are
  not supported.
* If QR fails, send `login-token user <TOKEN>` using the Authorization value
  extracted from a browser network request.
* Safer bot mode: create a Discord application/bot, enable Server Members and
  Message Content intents, then send `login-token bot <TOKEN>`. Add it to guilds
  with the required permissions and use `guilds` in the management room.

A user token is highly sensitive and self-bot-like use may violate provider
expectations or cause a ban. Prefer a bot where its feature limits are acceptable.

## 4. Sideband integration boundary

Matrix and mautrix are internal implementation details. Sideband users choose
**Connected apps → Add chat app → Telegram/Discord/Google Chat/Messenger** and
complete that provider's QR, browser, password, cookie, or verification-code
flow. The app does not expose a homeserver field, Matrix credentials, bot rooms,
or raw connector JSON.

Release builds embed the bridge backend endpoint. Developers can select the
local integration stack at compile time with `SIDEBAND_BRIDGE_BACKEND_URL`; the
default development build uses `http://127.0.0.1:8008`. This is a build/operator
setting, not profile state or an application preference.

The current Matrix sidecar and management-bot command parser are scaffolding for
the integration tests. They still require an internal Matrix session and are
not the final end-user authentication path. Before Connected Apps ships, replace
that layer with structured provider provisioning/login adapters, automatic
internal service authentication, authoritative portal ownership checks, and an
Android-capable backend path. Do not ask users to supply Matrix usernames,
homeserver URLs, bot room IDs, or config JSON.

### Structured authentication adapters

Do not parse management-bot prose. Use the bridges' machine-readable APIs:

| Provider | Adapter | User flow |
|---|---|---|
| Telegram | mautrix Bridge v2 provisioning v3 | QR scan (preferred), or phone → code → optional 2FA password |
| Messenger | mautrix Bridge v2 provisioning v3 | Provider cookie fields, or Messenger Lite credential steps when enabled |
| Discord | mautrix-discord legacy provisioning API | WebSocket QR login; token login is advanced fallback only |
| Google Chat | mautrix-googlechat legacy provisioning API | Required Google Chat cookie fields |

Bridge v2 exposes `GET /_matrix/provision/v3/login/flows`,
`POST /_matrix/provision/v3/login/start/{flowID}`, and typed step submissions at
`/v3/login/step/{loginProcessID}/{stepID}/{stepType}`. Its typed steps cover
`user_input`, `cookies`, `display_and_wait` (QR/code), `webauthn`, and
`complete`. Telegram and Meta implement this interface directly. Sideband
renders those fields and submits maps keyed by the bridge-provided field IDs;
users never paste connector JSON.

Discord currently has its own provisioning API: `/v1/login` is a WebSocket QR
flow, while `/v1/login/token` accepts a token. Google Chat exposes
`POST /v1/login` with named cookie fields and `GET /v1/whoami`. These require
provider-specific adapters behind the same Sideband login-step model.

#### Implementation status (`bridges/provisioning`)

The Bridge v2 provisioning v3 login state machine lives in the isolated
`bridges/provisioning` crate (reqwest + serde only — **no matrix-sdk**), so it is
exercised deterministically against a fake Bridge v2 HTTP service (`wiremock`)
without the heavy sidecar and without any live provider or Matrix credentials:

- `ProvisioningClient` — bearer-authenticated `list_flows` / `start` / step
  submission against `/_matrix/provision/v3/login/...`.
- `LoginSession` — drives a flow: begin → `display_and_wait` (QR/code) →
  long-poll `wait()` → `user_input`/`cookies` `submit(map)` → `complete`, mapping
  each step to a `LoginUpdate` the UI renders. Secret fields (`password`,
  `2fa_code`, `token`, `cookie`) are flagged and never echoed back to the UI;
  HTTP error bodies are never surfaced verbatim (they can carry tokens).

- `acquire_internal_session` — establishes Sideband's internal Matrix account
  with the backend: password **login**, falling back to Synapse shared-secret
  **register** (`GET`/`POST /_synapse/admin/v1/register`, HMAC-SHA1 MAC) when the
  account does not exist yet. The user supplies nothing.

Covered today (deterministic, in `run-tests.sh fast`): **Telegram QR** (incl. the
optional 2FA-password step) and internal-session register/login. Run directly
with `cargo test --manifest-path bridges/provisioning/Cargo.toml`.

**Internal session is wired:** on `Hello` with no stored session,
`src/bin/sideband-bridge-matrix.rs` calls `acquire_internal_session`, restores it
into the matrix-sdk client, and persists it `0600`. Credentials are app-owned —
the core generates a per-profile password (`<profile>/bridge-matrix/internal.secret`,
`0600`) and injects it + the homeserver via `bridge_connector_config`; an optional
`registration_shared_secret` comes from env `SIDEBAND_BRIDGE_REG_SECRET`
(operator/dev only, never compiled). So the *"Connected Apps service is
unavailable"* error clears once the backend is reachable, with no Matrix setup UX.

**Provider login not yet wired:** the sidecar still uses the legacy management-bot
text path (`classify_login_text`/`relay_login_event`). Replacing that with
`LoginSession` (per-bridge provisioning base URL + auth, an async wait/submit
task, authoritative portal-ownership checks) plus the Discord/Google-Chat/Meta
adapters is the next step, and can only be validated against a running stack (§6).

Authoritative implementation references:

- Bridge v2 login model: <https://github.com/mautrix/go/blob/f6531777f56c4a8276b65c1439e991b860c1ecb9/bridgev2/login.go>
- Bridge v2 provisioning routes/auth: <https://github.com/mautrix/go/blob/f6531777f56c4a8276b65c1439e991b860c1ecb9/bridgev2/matrix/provisioning.go>
- Bridge v2 OpenAPI contract: <https://github.com/mautrix/go/blob/f6531777f56c4a8276b65c1439e991b860c1ecb9/bridgev2/matrix/provisioning.yaml>
- Telegram flows: <https://github.com/mautrix/telegram/blob/be664e7da73c9720e58e459288d1030cc3739d8e/pkg/connector/login.go>
- Discord provisioning: <https://github.com/mautrix/discord/blob/da5e548b7f84d99ce8a838d8098eba269ca5c3f6/provisioning.go>
- Meta flows: <https://github.com/mautrix/meta/blob/980c8c039bcb74eb18b6532277211c315e441843/pkg/connector/login.go>
- Google Chat provisioning: <https://github.com/mautrix/googlechat/blob/a835bccd269eace6c28196a3f869eb1fb3c8acf7/mautrix_googlechat/web/auth.py>

All four bridge projects are AGPL-3.0. Shipping modified network services
requires making the corresponding source available under the AGPL. Google Chat,
Discord user accounts, and Meta rely on unofficial consumer-service access;
login breakage, checkpoints, and provider account restrictions remain product
risks and must be stated in the Connected Apps UI.

## 5. Optional/experimental providers

### Meta Messenger

```sh
docker compose --profile meta pull meta
docker compose --profile meta run --rm meta
# Merge meta.required.example.yaml; leave network.mode=messenger.
docker compose --profile meta run --rm meta
```

Add `/bridges/meta/registration.yaml` to Synapse's
`app_service_config_files`, restart Synapse, then:

```sh
docker compose --profile meta up -d meta
```

In a direct room with `@metabot:<server>`, send `login`. The bot asks for a
browser request copied as cURL (or cookie JSON) from a private messenger.com
session. This exposes account cookies to the bridge and Meta may require CAPTCHA,
phone verification, or password reset. Sideband sends the documented `login` command and relays the bot's subsequent
cookie/input prompts:

```sh
sideband bridge add --id messenger-main --network messenger \
  --name Messenger
```

### Google Chat

```sh
docker compose --profile googlechat pull googlechat
docker compose --profile googlechat run --rm googlechat
# Merge googlechat.required.example.yaml.
docker compose --profile googlechat run --rm googlechat
```

Add `/bridges/googlechat/registration.yaml` to Synapse, restart it, and run
`docker compose --profile googlechat up -d googlechat`. In a direct room with
`@googlechatbot:<server>`, extract the `COMPASS`, `SSID`, `SID`, `OSID`, and
`HSID` cookies from a private <https://chat.google.com> session and send:

```text
login-cookie {"compass":"...","ssid":"...","sid":"...","osid":"...","hsid":"..."}
```

Close the private browser window promptly so refresh credentials are not
invalidated. Device Bound Session Credentials can prevent this flow (upstream
suggests Firefox or disabling that Chrome feature). Sideband sends
`login-cookie` and relays the bridge's cookie prompt.

## 6. Manual end-to-end verification

There is no meaningful automated live provider test without real provider and
Matrix credentials. Verify manually for every enabled provider:

1. `docker compose ps` shows Synapse and bridge containers running; inspect logs
   for appservice transaction/authentication errors.
2. The bot responds to `help` in its direct room and reports `status` as logged in.
3. Send a unique marker from the upstream app (for example
   `SB-IN-20260715-1`); confirm it appears in Matrix and in
   `sideband bridge conversations` / Sideband's bridged history.
4. Send a different marker from Sideband; confirm it appears once in Matrix and
   at the upstream provider, attributed to the expected account.
5. Test a DM and a group/guild, one attachment, one reply, and a restart:
   `docker compose restart synapse telegram discord`. Confirm sessions recover
   and no duplicate portal appears.
6. Confirm the Sideband UI labels these conversations non-native/not private.
   Stop immediately if a bridged room is presented as native E2E.

Back up `data/synapse` and each enabled bridge directory together. Logout from
provider bots before destroying state if you want upstream sessions revoked.

## Authoritative references

* Synapse official Docker image: <https://hub.docker.com/r/matrixdotorg/synapse>
* Synapse application services: <https://element-hq.github.io/synapse/latest/application_services.html>
* mautrix Docker setup and image/tag policy: <https://docs.mau.fi/bridges/general/docker-setup.html>
* mautrix appservice registration: <https://docs.mau.fi/bridges/general/registering-appservices.html>
* Telegram authentication: <https://docs.mau.fi/bridges/go/telegram/authentication.html>
* Discord authentication: <https://docs.mau.fi/bridges/go/discord/authentication.html>
* Meta authentication: <https://docs.mau.fi/bridges/go/meta/authentication.html>
* Google Chat authentication: <https://docs.mau.fi/bridges/python/googlechat/authentication.html>
