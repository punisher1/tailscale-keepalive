[English](README.md) | **简体中文**

# tailscale-keepalive

定期向对端的 Tailscale IP 发一个很小的 ICMP echo，防止链路因为*空闲*而在 NAT
上被回收。常驻后台服务，支持 Windows、Linux、macOS。

```
每 25 秒 → [ 64 字节 ICMP echo → 对端 100.x.y.z ]
                              │
                              └─ 它被封装在 Tailscale 中继（或直连）用的同一条
                                 WireGuard/UDP 流里，路径上每台 NAT 的空闲计时器
                                 被持续清零，映射不会过期
```

## 为什么会有这个工具

不是为了解决一个假想问题，它来自一次真实的排障。

现场：一台笔记本在公司网络内，经无线路由中继上网（双层 NAT），Tailscale
无法和部分节点直连，只能走中继（Tailscale 官方 DERP 或自建 peer relay）；
对端节点也在 NAT 后面。随后出现了两轮故障。

**第一轮：中继会话被黑洞化。** 中继连接是一条长连接，路径中间某个设备把它
静默丢弃了——连接还"挂着"，只发不收，`tailscale status` 仍显示 active，
`tailscale ping` 超时。`tailscale down && tailscale up` 可以恢复。

**第二轮：NAT 激进回收，这才是本工具的动因。** 某天傍晚起，路径上某台 NAT
（不是本地路由器，已排除；最大嫌疑是公司网络出口的策略变更）开始对空闲的
UDP 流变得激进：空闲约 2–3 分钟就删除映射，中继会话随即死亡。于是链路反复
中断——每次"救活"后活不过 5 分钟，持续一整夜。

**解法。** 每 25 秒向对端的 Tailscale IP 发一个 ICMP echo。这个包被 WireGuard
加密，走的正是中继使用的那条 UDP 流。路径上每台 NAT 的空闲计时器被持续清零，
映射永不超时；而且 echo 有回包，双向映射同时被保活。实测效果：链路从"每 5
分钟必死"变成连续 20 分钟以上零故障，此后长期稳定。成本：每 25 秒一个约 64
字节的包，约 20 B/s，可以忽略。

**一句话原理：** Tailscale 的中继或直连本质是一条 UDP 流；NAT 设备对空闲 UDP
映射有空闲超时（常见 30s–180s，策略变更后可能更短）；只要流上的周期性流量
间隔小于最短超时，映射就会持续续期，链路不断。

### 它做不到什么

它无法修复真正断掉的链路——对端关机、断网、或 Tailscale 自身会话卡死。
它只防上面说的"空闲死"。因为区分这两件事很重要，工具在连续
`fail_threshold` 次探测无回包时会记一条 WARN，恢复时再记一条。

早期还有一个配套"看门狗"（真断时先软探测、无效则 `tailscale down/up`）。
保活生效后它几乎不再触发。它不属于本工具。

## 安装

### Unix（Linux / macOS）

```sh
curl -fsSL https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.sh | sh
```

脚本自动识别系统和架构，从 GitHub Releases 下载对应二进制，安装到
`/usr/local/bin`（非 root 时用 `~/.local/bin`）。想顺带注册系统服务：

```sh
curl -fsSL https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.sh | \
  TS_KEEPALIVE_TARGET=100.x.y.z TS_KEEPALIVE_INSTALL_SERVICE=1 sh
```

### Windows

```powershell
irm https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.ps1 | iex
```

顺带注册服务（用管理员 PowerShell）：

```powershell
$env:TS_KEEPALIVE_TARGET = "100.x.y.z"
$env:TS_KEEPALIVE_INSTALL_SERVICE = "1"
irm https://raw.githubusercontent.com/punisher1/tailscale-keepalive/main/install.ps1 | iex
```

### 从源码构建

```sh
cargo install --path .          # 或：cargo build --release
```

## 用法

```text
tailscale-keepalive [COMMAND] [OPTIONS]

Commands:
  run          前台运行（默认）
  install      注册为系统服务并启动
  uninstall    停止并卸载系统服务
  status       查看服务状态和生效配置
  config       打印生效配置后退出

Options:
  -t, --target <IP>           对端 Tailscale IP，如 100.x.y.z（必填）
  -i, --interval <SECONDS>    探测间隔（默认 25）
  -W, --timeout <SECONDS>     单次探测超时（默认 2）
  -f, --fail-threshold <N>    连续失败多少次后告警（默认 3）
  -c, --config <PATH>         配置文件路径
  -L, --log-file <PATH>       追加写带时间戳的日志文件
  -v, --verbose               记录每一次探测，而不只是失败
      --output <FORMAT>       `status` 输出格式：text 或 json
```

典型用法：

```sh
tailscale-keepalive -t 100.x.y.z -i 25 -L /var/log/tailscale-keepalive.log -v
```

对端 IP 用 `tailscale status` 查看，第一列就是。

## 配置

配置有四种来源，**优先级从高到低**：命令行 → 环境变量 → 配置文件 → 内置默认值。

### 配置文件

默认位置（以 root 运行时用系统级路径，服务场景就是如此）：

| 平台 | 路径 |
| --- | --- |
| Linux | `/etc/tailscale-keepalive/config.toml`（root）或 `~/.config/tailscale-keepalive/config.toml` |
| macOS | `/etc/tailscale-keepalive/config.toml`（root）或 `~/Library/Application Support/tailscale-keepalive/config.toml` |
| Windows | `%ProgramData%\tailscale-keepalive\config.toml`（服务）或 `%APPDATA%\tailscale-keepalive\config.toml` |

`TS_KEEPALIVE_CONFIG` 可以覆盖该路径。

```toml
target = "100.x.y.z"
interval = 25
timeout = 2
fail_threshold = 3
log_file = "/var/log/tailscale-keepalive/keepalive.log"
verbose = false
```

`install` 会把你传入的配置写成这个文件，让服务和你的命令行保持一致。

### 环境变量

| 变量 | 含义 |
| --- | --- |
| `TS_KEEPALIVE_TARGET` | 对端 IP |
| `TS_KEEPALIVE_INTERVAL` | 探测间隔，秒 |
| `TS_KEEPALIVE_TIMEOUT` | 单次探测超时，秒 |
| `TS_KEEPALIVE_FAIL_THRESHOLD` | 连续失败多少次后告警 |
| `TS_KEEPALIVE_LOG_FILE` | 日志文件路径 |
| `TS_KEEPALIVE_VERBOSE` | `1`/`true`/`yes` |
| `TS_KEEPALIVE_CONFIG` | 配置文件路径 |
| `TS_KEEPALIVE_LOCK` | 单例锁文件路径 |

## 作为系统服务运行

```sh
sudo tailscale-keepalive -t 100.x.y.z install     # Linux/macOS
tailscale-keepalive -t 100.x.y.z install          # Windows，需管理员
```

| 平台 | 机制 | 单元文件 |
| --- | --- | --- |
| Linux | systemd | `/etc/systemd/system/tailscale-keepalive.service` |
| macOS | launchd | `/Library/LaunchDaemons/com.punisher1.tailscale-keepalive.plist` |
| Windows | 服务控制管理器 | 服务名 `tailscale-keepalive`，开机自启 |

查看与卸载：

```sh
tailscale-keepalive status
sudo tailscale-keepalive uninstall
```

服务开机自启、退出后自动重启，并写自己的日志文件（服务没有控制台）。Linux 和
macOS 上 ICMP 需要原始套接字，因此服务以 root 运行。

## 日志与排障

每行日志都带时间戳。这是刻意的：链路间歇性死亡时，日志是判断"多久死一次、
什么时候开始"的关键证据。

```text
2026-10-22 10:01:32 INFO  starting: target=100.x.y.z interval=25s timeout=2s fail_threshold=3 log_file=-
2026-10-22 10:01:32 DEBUG probe ok target=100.x.y.z rtt=46.1ms elapsed=46.4ms
2026-10-22 10:05:07 WARN  link looks down: target=100.x.y.z has not replied to 3 consecutive probes...
2026-10-22 10:09:41 INFO  link recovered: target=100.x.y.z replied after 3 failed probe(s)
2026-10-22 11:20:12 INFO  stats: 100 probes, 2 without reply, 0 consecutive
```

| 现象 | 含义 |
| --- | --- |
| `another instance is already running` | 已有实例持有锁文件，先停掉它，或看 `status`。 |
| `failed to open an ICMP socket` | 没有原始套接字权限：用 root 运行，或授予 `CAP_NET_RAW`。 |
| `link looks down` 告警 | 链路真的断了，不是空闲死。检查 `tailscale status` / `tailscale ping`。 |
| `timeout must not exceed interval` | 配置不合法：探测超时不能超过探测周期。 |

## 构建与发布

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

推送 `v*` 形式的 tag 会触发 `.github/workflows/release.yml`，为 Linux
（x86_64、aarch64）、macOS（Intel、Apple Silicon）、Windows（x86_64）构建二进制
并发布到 GitHub Release。`ci.yml` 在每次 push 和 PR 上跑同样的检查。

## 许可证

MIT，见 [LICENSE](LICENSE)。
