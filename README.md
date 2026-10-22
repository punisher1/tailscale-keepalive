**English** | [简体中文](README.zh-CN.md)

# tailscale-keepalive

Keeps a Tailscale link from dying of *idle* by sending one tiny ICMP echo to the
peer at a fixed interval. Runs as a background service on Windows, Linux, and
macOS.

```
25s → [ 64-byte ICMP echo → peer's 100.x.y.z ]
                            │
                            └─ travels inside the same WireGuard/UDP flow that
                               the relay (or direct path) uses, so every NAT on
                               the way sees traffic long before its idle timeout
                               and never reclaims the mapping
```

## Why this tool exists

This is not a theoretical problem. It came out of a real setup: a laptop behind
a wireless repeater (double NAT) that could not reach some peers directly, so
Tailscale fell back to a relay — either Tailscale's own DERP servers or a
self-hosted peer relay. The peer was also behind NAT. Two failures followed.

**Round one — black-holed relay session.** The relay connection was a
long-lived TLS connection. Something in the middle of the path silently dropped
it: the connection stayed "up", only sent and never received, and
`tailscale status` still reported it as active. `tailscale ping` timed out.
`tailscale down && tailscale up` brought it back.

**Round two — aggressive NAT reclamation.** This is what the tool is for. From
one evening onward, a NAT device on the path (not the local router — that was
ruled out — most likely a policy change at the office uplink) started dropping
idle UDP flow state after roughly 2–3 minutes instead of the usual much longer
timeout. The relay session died every time it went idle, so the link died
repeatedly: once "fixed", it never survived more than about five minutes, and
it stayed broken all night.

**The fix.** Send one ICMP echo to the peer's Tailscale IP every 25 seconds.
That packet is encrypted by WireGuard and forwarded over the very same UDP flow
the relay uses. Every NAT on the path resets its idle timer, so the mapping never
expires — and because the echo comes back, the mapping is refreshed in both
directions. Measured result: the link went from "dies every 5 minutes" to
stable for 20+ minutes immediately, and has been stable since. Cost: one ~64
byte packet every 25 seconds, about 20 bytes per second.

**The core idea, in one line:** a Tailscale relay or direct path is a UDP flow.
NAT devices expire idle UDP mappings (commonly 30s–180s, sometimes much less
after a policy change). Any periodic traffic whose interval is shorter than the
shortest timeout keeps the mapping renewed, and the link stays up.

### What this does *not* do

It cannot repair a path that is genuinely broken — the peer is off, the network
is down, or Tailscale itself has wedged its session. It only prevents the *idle*
death described above. Because telling those cases apart matters, the tool logs a
warning once replies stop arriving (after `fail_threshold` consecutive failures)
and logs again when the path recovers.

A companion "watchdog" that called `tailscale down/up` after a real outage was
useful before the keepalive existed; once the keepalive stopped the idle deaths,
the watchdog almost never triggered. It is not part of this tool.

## Install

### Unix (Linux / macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.sh | sh
```

The script detects the OS and architecture, downloads the matching binary from
GitHub Releases, and installs it to `/usr/local/bin` (or `~/.local/bin` when not
root). To register the service right away:

```sh
curl -fsSL https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.sh | \
  TS_KEEPALIVE_TARGET=100.x.y.z TS_KEEPALIVE_INSTALL_SERVICE=1 sh
```

### Windows

```powershell
irm https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.ps1 | iex
```

To also register the service (run the shell as Administrator):

```powershell
$env:TS_KEEPALIVE_TARGET = "100.x.y.z"
$env:TS_KEEPALIVE_INSTALL_SERVICE = "1"
irm https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.ps1 | iex
```

### From source

```sh
cargo install --path .          # or: cargo build --release
```

## Usage

```text
tailscale-keepalive [COMMAND] [OPTIONS]

Commands:
  run          Run the keepalive loop in the foreground (default)
  install      Register as a system service and start it
  uninstall    Stop and remove the system service
  status       Show the service state and the effective configuration
  config       Print the effective configuration and exit

Options:
  -t, --target <IP>           Peer Tailscale IP to probe, e.g. 100.x.y.z (required)
  -i, --interval <SECONDS>    Probe interval (default: 25)
  -W, --timeout <SECONDS>     Timeout for one probe (default: 2)
  -f, --fail-threshold <N>    Consecutive failures before warning (default: 3)
  -c, --config <PATH>         Configuration file path
  -L, --log-file <PATH>       Append timestamped log lines to this file
  -v, --verbose               Log every probe, not only failures
      --output <FORMAT>       `status` output: text or json
```

Typical run:

```sh
tailscale-keepalive -t 100.x.y.z -i 25 -L /var/log/tailscale-keepalive.log -v
```

To find the peer IP, run `tailscale status` and read the first column for the
peer you care about.

## Configuration

Settings can come from four places. **Highest priority wins:**
command line → environment → config file → built-in defaults.

### Config file

Default locations (the system-wide path is used when running as root, which is
the case for the service):

| Platform | Path |
| --- | --- |
| Linux | `/etc/tailscale-keepalive/config.toml` (root) or `~/.config/tailscale-keepalive/config.toml` |
| macOS | `/etc/tailscale-keepalive/config.toml` (root) or `~/Library/Application Support/tailscale-keepalive/config.toml` |
| Windows | `%ProgramData%\tailscale-keepalive\config.toml` (service) or `%APPDATA%\tailscale-keepalive\config.toml` |

`TS_KEEPALIVE_CONFIG` overrides the path.

```toml
target = "100.x.y.z"
interval = 25
timeout = 2
fail_threshold = 3
log_file = "/var/log/tailscale-keepalive/keepalive.log"
verbose = false
```

`install` writes this file for the service from whatever settings you passed, so
the service and your shell agree.

### Environment variables

| Variable | Meaning |
| --- | --- |
| `TS_KEEPALIVE_TARGET` | peer IP to probe |
| `TS_KEEPALIVE_INTERVAL` | probe interval, seconds |
| `TS_KEEPALIVE_TIMEOUT` | per-probe timeout, seconds |
| `TS_KEEPALIVE_FAIL_THRESHOLD` | consecutive failures before warning |
| `TS_KEEPALIVE_LOG_FILE` | log file path |
| `TS_KEEPALIVE_VERBOSE` | `1`/`true`/`yes` |
| `TS_KEEPALIVE_CONFIG` | config file path |
| `TS_KEEPALIVE_LOCK` | single-instance lock file path |

## Running as a service

```sh
sudo tailscale-keepalive -t 100.x.y.z install     # Linux/macOS
tailscale-keepalive -t 100.x.y.z install          # Windows, elevated shell
```

| Platform | Mechanism | Unit |
| --- | --- | --- |
| Linux | systemd | `/etc/systemd/system/tailscale-keepalive.service` |
| macOS | launchd | `/Library/LaunchDaemons/com.punisher1.tailscale-keepalive.plist` |
| Windows | Service Control Manager | service name `tailscale-keepalive`, auto start |

Check and remove:

```sh
tailscale-keepalive status
sudo tailscale-keepalive uninstall
```

The service starts at boot, restarts if it exits, and logs to its own file
because a service has no console. On Linux and macOS the ICMP socket needs a raw
socket, so the service runs as root.

## Logs and troubleshooting

Every line is timestamped. That is deliberate: when the link is dying
intermittently, the log is the evidence you need to see *how often* and *when*.

```text
2026-10-22 10:01:32 INFO  starting: target=100.x.y.z interval=25s timeout=2s fail_threshold=3 log_file=-
2026-10-22 10:01:32 DEBUG probe ok target=100.x.y.z rtt=46.1ms elapsed=46.4ms
2026-10-22 10:05:07 WARN  link looks down: target=100.x.y.z has not replied to 3 consecutive probes...
2026-10-22 10:09:41 INFO  link recovered: target=100.x.y.z replied after 3 failed probe(s)
2026-10-22 11:20:12 INFO  stats: 100 probes, 2 without reply, 0 consecutive
```

| Symptom | Meaning |
| --- | --- |
| `another instance is already running` | A loop already owns the lock file; stop it or use `status`. |
| `failed to open an ICMP socket` | No permission for a raw socket: run as root, or grant `CAP_NET_RAW`. |
| `link looks down` warnings | The path is genuinely broken, not idle-dead. Check `tailscale status` / `tailscale ping`. |
| `timeout must not exceed interval` | Invalid config; a probe cannot outlive its own period. |

## Building and releasing

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Pushing a tag matching `v*` triggers `.github/workflows/release.yml`, which
builds binaries for Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), and
Windows (x86_64), and attaches them to a GitHub Release. `ci.yml` runs the same
checks on every push and pull request.

## License

MIT — see [LICENSE](LICENSE).
