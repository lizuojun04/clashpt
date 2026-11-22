# ClashPT

ClashPT 是面向 Linux 的 Mihomo 管理工具。TUI 和 CLI 可以查看策略组、切换节点和查询运行状态。普通用户模式提供本地 HTTP/SOCKS 代理，系统模式通过 systemd 启用 TUN。

## 安装

准备 Rust 工具链和 Mihomo，并确保 `mihomo` 在 `PATH` 中。也可以用 `--mihomo /path/to/mihomo` 指定内核。

```bash
cargo install --path . --locked
```

安装后可直接运行 `clashpt`。也可以在仓库里用 `bash scripts/run.sh <命令>` 运行，例如 `bash scripts/run.sh status`。

## 普通用户模式

导入一份完整的 Clash/Mihomo YAML，再启动代理。

```bash
clashpt profile import ./config.yaml
clashpt start
clashpt status
clashpt tui
```

将 `status` 输出的本地代理地址填入需要使用代理的应用。使用 `clashpt stop` 停止 Mihomo。

TUI 默认打开 Overview，显示上下行实时波形、流量和连接数。Proxies 用于切换节点和测延迟，Rules、Conns、Logs 分别显示规则、活动连接和日志。

| TUI 按键 | 操作 |
| --- | --- |
| `1`–`5` | 切换 Overview、Proxies、Rules、Conns、Logs |
| `Tab`、左右方向键 | 在 Proxies 的策略组和节点列表之间切换焦点 |
| 上下方向键 | 移动光标 |
| `Enter` | 在 Proxies 中进入当前策略组或选中当前节点 |
| `d`、`t` | 在 Proxies 中测试当前节点、批量测试所有可测速节点 |
| `End` | 在 Logs 中回到最新日志 |
| `r` | 刷新列表 |
| `q`、`Esc` | 退出界面 |

命令行也可以操作节点。把示例中的组名、节点名换成实际值。

```bash
clashpt nodes list
clashpt nodes select '组名' '节点名'
clashpt nodes list --json
clashpt status --json
```

## 订阅

订阅 URL 需要返回一份完整的 Clash/Mihomo YAML。首次订阅后用 `start` 启动，后续用 `profile update` 重新下载并校验。更新失败时会保留上一份配置。

```bash
clashpt profile subscribe 'https://example.com/your-subscription'
clashpt start
clashpt profile update
```

省略 URL 参数时，`profile subscribe` 从标准输入读取一行 URL。

## 导入后的配置文件位置

普通用户模式默认把文件放在 `~/.config/clashpt/`。如果设置了绝对路径 `XDG_CONFIG_HOME`，则用 `$XDG_CONFIG_HOME/clashpt/`。可以用 `CLASHPT_DATA_DIR` 或 `--data-dir` 覆盖，命令行参数优先。

| 状态目录下的路径 | 内容 |
| --- | --- |
| `profile.json` | 当前配置的元数据，`generation` 字段指向当前配置目录 |
| `profile-<随机后缀>/source.yaml` | 本次导入的原始 YAML 副本，传给 `import` 的文件保留在原位置 |
| `profile-<随机后缀>/config.yaml` | Mihomo 实际使用的运行配置，包含 ClashPT 设置的本地代理和控制端口 |
| `validate.log`、`mihomo.log` | 配置校验日志和内核运行日志 |

每次成功导入或更新会生成新的配置目录。要获取当前原始配置，可以用 `clashpt profile export`。

如果希望把状态放在仓库的 `data` 目录，先在当前终端设置环境变量，再运行导入、启动等命令。

```bash
export CLASHPT_DATA_DIR="$PWD/data/clashpt"
clashpt profile import ./config.yaml
clashpt start
```

## 系统模式（sudo + TUN）

需要运行中的 systemd 和 `/dev/net/tun`。首次应用时指定 Mihomo 的实际路径。

```bash
sudo "$(command -v clashpt)" --mihomo "$(command -v mihomo)" system apply ./config.yaml
clashpt --system status
clashpt --system tui
```

`system apply` 将运行配置保存到 `/etc/clashpt/config.yaml`，并启动系统服务。默认授权调用 sudo 的用户通过 CLI 或 TUI 管理节点。系统服务的启停使用 `clashpt --system start` 和 `clashpt --system stop`，服务日志可用 `sudo journalctl -u clashpt.service` 查看。

要将订阅用于系统模式，先用普通用户模式订阅并导出 YAML，再以管理员身份应用。

```bash
(umask 077; clashpt profile export > ./subscription.yaml)
sudo "$(command -v clashpt)" --mihomo "$(command -v mihomo)" system apply ./subscription.yaml
```
