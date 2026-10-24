# Configuration & Environment Variables

This document is the single reference for every option that the open‑source
RustDesk server binaries (`hbbs`, `hbbr`) understand: command‑line flags,
environment variables, and configuration files.

> **TL;DR** — For most people the command‑line flags shown by `hbbs --help` /
> `hbbr --help` are all you need. Environment variables are an alternative way to
> set the same options, plus a handful of extra tuning knobs that have no flag.

---

## How configuration is loaded

Both servers read their configuration from the following sources. For
**`hbbs`** the order of precedence, from highest to lowest, is:

1. **Command‑line flag** (e.g. `-p 21116`, `-k mykey`)
2. **`--config <file>`** — an INI file passed with `-c`/`--config`
3. **`.env`** — an INI file named `.env` in the working directory
4. **Inherited process environment** — variables exported before launch

A value set by a higher source overrides the same value from a lower one. Under
the hood every source is turned into a process environment variable, and the
code then reads that variable — so "flag", "config file" and "env var" are just
three ways to set the same thing.

For **`hbbr`** the precedence is: **flag** (`-b`, `-p`, `-k`) → **`.env`** →
**inherited environment**.

Both binaries initialize logging after loading configuration, so `RUST_LOG`
and `LOG_DIR` can also be supplied in `.env` / the hbbs configuration file.

---

## WebSocket reverse proxies

Both servers ignore `X-Real-IP` and `X-Forwarded-For` by default and use the
socket's source address. To trust a reverse proxy, set `TRUSTED_PROXIES` to a
comma-separated list of its IP addresses or CIDR networks, for example
`TRUSTED_PROXIES=127.0.0.1/32,::1/128`. The setting is read through the same
configuration sources as other server options. Invalid entries make startup fail
rather than enabling trust.

Only a connection from a listed proxy may supply a forwarded address. The proxy
must overwrite `X-Real-IP` (preferred) or maintain `X-Forwarded-For` correctly;
forwarded chains are read from right to left until the first untrusted hop.
Do not trust entire client networks. WebSocket handshakes must complete within
10 seconds. WebSocket and native TCP admission/control messages are limited to 128 KiB; hbbr restores normal TCP/WebSocket relay payload sizes after admission. The transport reuses the client's pinned buffer-growth/dynamic-limit fixes. This does not bound total concurrent connections. Ports 21118/21119 need not be published when no web client is used.

---

## `hbbs` — ID / rendezvous server

| Variable | CLI flag | Default | Description |
|---|---|---|---|
| `KEY` | `-k`, `--key` | `-` | Public key clients must use, a base64 secret key, or `-` / `_` to load or generate a key pair (`id_ed25519`, `id_ed25519.pub`). `-` and `_` have the same behavior, so explicitly passing `-k _` to `hbbs` is unnecessary. An empty value is refused unless `ALLOW_ANONYMOUS=Y`; see [Keys](#keys-and-encryption). |
| `BIND` | `-b`, `--bind` | all interfaces | **Available since 1.1.17.** Local IPv4 or IPv6 address on which all `hbbs` TCP, UDP, and WebSocket listeners bind. This does not change the addresses advertised to clients. Supported by `--config`, `.env`, and the inherited environment. |
| `PORT` | `-p`, `--port` | `21116` | Main TCP/UDP listening port. `hbbs` also binds `PORT-1` (NAT type test) and `PORT+2` (WebSocket). |
| `RELAY-SERVERS` | `-r`, `--relay-servers` | *(empty)* | Optional relay server override handed to clients, as comma-separated `host` or `host:port` values. Leave empty when `hbbr` uses the same address as `hbbs` and the standard port `21117`; clients derive it automatically. Set this only when the relay uses a different IP/hostname or a non-standard port. |
| `RMEM` | `-M`, `--rmem` | `0` (system default) | UDP receive‑buffer size in bytes. Raise the OS limit first: `sudo sysctl -w net.core.rmem_max=52428800`. |
| *(config file)* | `-c`, `--config` | *(none)* | Path to an extra INI config file (see precedence above). |
| `TEST_HBBS` 🅴 | *(none)* | *(auto)* | UDP self‑test target checked at start‑up. Set to `no` to skip the check (useful behind some NATs/proxies), or to an explicit `host:port`. |
| `ALWAYS_USE_RELAY` 🅴 | *(none)* | `N` | `Y` forces every session through a relay (disables direct/hole‑punched connections). At runtime, send `always-use-relay Y` or `always-use-relay N` to the `hbbs` [loopback console](#runtime-console). |
| `DB_URL` 🅴 | *(none)* | `./db_v2.sqlite3` | Path/URL of the SQLite database file. See [Database](#database). |
| `MAX_DATABASE_CONNECTIONS` 🅴 | *(none)* | `1` | Size of the SQLite connection pool. |
| `MAX_PUNCH_REQUESTS` 🅴 | *(none)* | `100000` | Number of punch-request records kept in memory for the `punch-requests` [console](#runtime-console) command; the oldest is dropped when full. `0` disables recording. |

🅴 = set through the inherited process environment.

> `PORT_FOR_API` / `KEY_FOR_API` are only used by RustDesk Server **Pro** and its
> API; they have no effect in the open‑source server.

---

## `hbbr` — relay server

| Variable | CLI flag | Default | Description |
|---|---|---|---|
| `KEY` | `-k`, `--key` | *(empty)* | No default key material: startup requires the same non-empty key as `hbbs`; `-` / `_` load or generate a pair in the working directory. An empty key is refused unless `ALLOW_ANONYMOUS=Y` is explicit. |
| `BIND` | `-b`, `--bind` | all interfaces | **Available since 1.1.17.** Local IPv4 or IPv6 address on which the relay TCP and WebSocket listeners bind. Supported by `.env` and the inherited environment; `hbbr` does not support `--config`. |
| `PORT` | `-p`, `--port` | `21117` | Relay listening port. `hbbr` also binds `PORT+2` for WebSocket relay. **Note:** when set via the `PORT` env var (not `-p`), `hbbr` listens on `PORT + 1`, so a shared `PORT=21116` makes `hbbs`=21116 and `hbbr`=21117. |

### Relay bandwidth / QoS

These have no CLI flag and can also be changed through the `hbbr`
[loopback console](#runtime-console) (`tb`, `sb`, `ls`, `dt`, `t`, …; send `h`
for help).

| Variable | Default | Unit | Description |
|---|---|---|---|
| `SINGLE_BANDWIDTH` | `128` | Mb/s | Normal maximum bandwidth for each relay connection. |
| `TOTAL_BANDWIDTH` | `1024` | Mb/s | Aggregate bandwidth cap shared by all relay connections. |
| `LIMIT_SPEED` | `32` | Mb/s | Per-connection cap applied after a connection is downgraded, and to IPs in `blacklist.txt`. |
| `DOWNGRADE_THRESHOLD` | `0.66` | ratio (0–1) | Fraction of `SINGLE_BANDWIDTH` that a connection's lifetime-average throughput must exceed to trigger downgrade. |
| `DOWNGRADE_START_CHECK` | `1800` | seconds | Delay before a connection becomes eligible for the lifetime-average downgrade check. |

Downgrade is decided independently for each connection; it does **not** check
aggregate relay congestion. After `DOWNGRADE_START_CHECK`, a connection is
capped to `LIMIT_SPEED` once its average throughput since it started exceeds
`SINGLE_BANDWIDTH * DOWNGRADE_THRESHOLD`. A lone transfer can therefore be
downgraded even when the relay is otherwise idle. `TOTAL_BANDWIDTH` is a
separate aggregate cap.

These may also be placed in `.env` using the uppercase spellings shown above
(e.g. `SINGLE_BANDWIDTH=256`).

### Blocklists / blacklists (files, not env vars)

`hbbr` reads two optional files from its working directory at start‑up:

* **`blacklist.txt`** — IPs that are **bandwidth‑limited** (one IP per line;
  anything after the first space on a line is ignored).
* **`blocklist.txt`** — IPs that are **refused** outright.

Both can also be edited live through the `hbbr` loopback console (`ba`/`br`,
`Ba`/`Br`).

### Runtime console

The runtime consoles are TCP command transports built into the services; they
are not `rustdesk-utils` commands or interactive standard-input consoles. A
connection from a loopback address is treated as a single console command:

```bash
# hbbs: toggle forced relay on PORT-1 (21115 by default)
printf 'always-use-relay Y' | nc 127.0.0.1 21115

# hbbr: list commands on its relay PORT (21117 by default)
printf 'h' | nc 127.0.0.1 21117
```

Use the corresponding configured ports if you changed `PORT`.

---

## Database

At runtime the database location comes from **`DB_URL`** (default
`./db_v2.sqlite3`). If unset, `hbbs` creates the SQLite file in its working
directory.

> **Do not confuse `DB_URL` with `DATABASE_URL`.** The `DATABASE_URL` entry in
> the repository's `.env` is used **only at compile time** by `sqlx` to check SQL
> queries; it is **not** read by the running server. Setting `DATABASE_URL` on a
> running server has no effect — use `DB_URL`.

---

## Logging

Both binaries reuse `flexi_logger`. `RUST_LOG` defaults to `info`; `LOG_DIR`
defaults to `logs` relative to the working directory. Logging starts after
configuration loading. Files use separate `hbbs` / `hbbr` basenames, rotate daily
or at 10 MiB, and retain 31 historical files (not a guaranteed 31 days). Every record that
passes `RUST_LOG` is also duplicated to stdout. If `LOG_DIR` cannot be created or
written, the service logs a warning and continues with stdout only. Restrict the directory ACL and
configure monitoring / external collection for your deployment.

```bash
RUST_LOG=info LOG_DIR=/var/log/remote-access hbbs
```

---

## Keys and encryption

The `KEY` / `-k` value can be:

* a **public key** string — clients must present the matching key;
* a **base64‑encoded 64‑byte secret key** — the server derives the public key
  from it;
* **`-` or `_`** — the server loads a key pair from the working directory or
  generates one on first start, writing `id_ed25519` (private) and
  `id_ed25519.pub` (public);
* **empty** - refused by default. `ALLOW_ANONYMOUS=Y` explicitly opts into the
  legacy open mode without key validation. `ALLOW_ANONYMOUS=N` (or unset) is the
  default; other values are rejected. `Y` does not turn off a configured key.

`hbbs` defaults to `-` and loads or generates a key pair. `hbbr` has no default
key material: configure the same non-empty KEY as hbbs, or use `hbbr -k _` in
a working directory containing the same key files. A deployment relying on the
old empty relay KEY must be updated before restarting this version. The `_`
value is not stricter than `-`.

**A public key is not a secret personnel credential.** Key matching does not
implement account authentication, RBAC or offboarding. Host-side password,
manual approval, optional 2FA and session permission checks remain necessary.

To supply your own key pair, place `id_ed25519` and `id_ed25519.pub` in the
process's **current working directory** before first start. That directory may
differ from the directory containing the executable. For the supervisor Docker
image, the working directory is `/data`.

---

## Reduced feature scope

This build keeps service/runtime logs and peer registration storage, not a
centralized audit service, account backend or admin dashboard. `AUDIT_LISTEN` and
`AUDIT_TOKEN` no longer enable a listener. The client no longer uploads audit
records, even when a custom API server was configured; its audit-token option
and durable outbox/replay code have been removed.

Existing audit database tables, old configuration values and `.audit-outbox`
files are not deleted automatically, but no new audit tables/records are created
and old outboxes are no longer replayed. Archive or clean them explicitly if
needed. These changes do not remove peer records or the service log rotation.
`ALLOW_ANONYMOUS` still defaults to `N`; see Keys and encryption. `LOG_DIR` and
`RUST_LOG` remain supported; see Logging. Service logs are for troubleshooting,
not attribution of remote users' file or desktop operations.

---

## Docker image variables

The supervisor image (`rustdesk/rustdesk-server-s6`) starts both binaries with
s6 and adds a few convenience variables handled by its service scripts, **not**
by `hbbs`/`hbbr` directly:

| Variable | Default | Description |
|---|---|---|
| `RELAY` | `relay.example.com` | Passed to `hbbs` as `-r $RELAY` (your public address). |
| `ENCRYPTED_ONLY` | `1` | `1` adds `-k _` to both servers. Set this (or a shared non-empty `KEY`) for the hardened relay; the old empty default now fails startup. |
| `KEY_PUB` | *(unset)* | If set, written to `/data/id_ed25519.pub` on first start. |
| `KEY_PRIV` | *(unset)* | If set, written to `/data/id_ed25519` on first start. Provide **both** `KEY_PUB` and `KEY_PRIV`, or neither. |

Any variable from the tables above can also be passed straight through the
container's environment (e.g. `-e ALWAYS_USE_RELAY=Y`, `-e RUST_LOG=debug`).

The classic scratch image (`rustdesk/rustdesk-server`) contains only the
binaries and does **not** implement `RELAY`, `ENCRYPTED_ONLY`, `KEY_PUB`, or
`KEY_PRIV`; those variables are ignored by that image.

---

## Examples

### Command line — non-standard ports

```bash
# Tell clients where the relay listens because it is not using port 21117.
hbbs -p 22116 -r rustdesk.example.com:22117
hbbr -p 22117 -k _
```

### `.env` file (working directory)

```ini
# Non-standard ports shared by both binaries; hbbr listens on PORT+1.
relay-servers=rustdesk.example.com:22117
KEY=_
PORT=22116
```

### docker-compose

```yaml
services:
  rustdesk-server:
    image: rustdesk/rustdesk-server-s6:latest
    environment:
      - RELAY=rustdesk.example.com:21117
      - ENCRYPTED_ONLY=1
      - ALWAYS_USE_RELAY=Y
      - RUST_LOG=info
      - SINGLE_BANDWIDTH=256
    ports:
      - "21115:21115"
      - "21116:21116"
      - "21116:21116/udp"
      - "21117:21117"
      - "21118:21118"
      - "21119:21119"
    volumes: ["./data:/data"]
    restart: unless-stopped
```

### systemd

```ini
[Service]
Environment=ALWAYS_USE_RELAY=Y
Environment=RUST_LOG=info
ExecStart=/usr/bin/hbbs
```

---

## Port reference

| Port | Proto | Server | Purpose |
|---|---|---|---|
| 21115 | TCP | hbbs | NAT type test (`PORT-1`) |
| 21116 | TCP + UDP | hbbs | ID registration / rendezvous / hole punching (`PORT`) |
| 21117 | TCP | hbbr | Relay (`hbbr PORT`) |
| 21118 | TCP | hbbs | WebSocket rendezvous (`PORT+2`) |
| 21119 | TCP | hbbr | WebSocket relay (`hbbr PORT+2`) |

Ports 21118/21119 are only needed for the web client; you can omit them
otherwise.
