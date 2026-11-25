# ClashPT PC

ClashPT PC 是 Linux 个人电脑上的 Mihomo 代理管理工具，提供终端界面和命令行。默认代理地址是 `127.0.0.1:7890`，HTTP 和 SOCKS5 共用一个端口。同一台电脑上的其他用户也可以免密使用。导入的配置若指定非零 `mixed-port`，则使用该端口。

## 安装 pc-v0.1.1

此发行包适用于 Linux x86_64。包内含 ClashPT、Mihomo v1.19.20、Bash/Zsh 集成，以及 GeoIP MMDB、GeoIP DAT、GeoSite 和 ASN 数据库。安装和本地地理规则校验均可离线完成。[下载 pc-v0.1.1](https://github.com/lizuojun04/clashpt/releases/tag/pc-v0.1.1)

在能访问 GitHub 的机器上下载压缩包和校验文件：

```sh
release=https://github.com/lizuojun04/clashpt/releases/download/pc-v0.1.1
pkg=clashpt-pc-v0.1.1-linux-x86_64-offline.tar.gz
curl -fL --retry 3 --connect-timeout 60 -O "$release/$pkg" &&
  curl -fL --retry 3 --connect-timeout 60 -O "$release/SHA256SUMS"
```

如果目标电脑网络较慢，可通过 `scp` 将这两个文件传到目标电脑的同一目录。在该目录校验、解压并安装：

```sh
sha256sum -c SHA256SUMS &&
  tar -xzf clashpt-pc-v0.1.1-linux-x86_64-offline.tar.gz &&
  sh clashpt-pc-v0.1.1-linux-x86_64-offline/install.sh
```

默认安装到 `~/.local/bin`。按所用 Shell 选择以下两行，加入启动文件并在当前终端执行一次。

**Zsh**（`~/.zshrc`）

```zsh
export PATH="$HOME/.local/bin:$PATH"
source "$HOME/.local/share/clashpt/shell/clashpt.zsh"
```

**Bash**（`~/.bashrc`）

```bash
export PATH="$HOME/.local/bin:$PATH"
source "$HOME/.local/share/clashpt/shell/clashpt.bash"
```

## 卸载

从解压目录的上一级执行：

```sh
sh clashpt-pc-v0.1.1-linux-x86_64-offline/uninstall.sh
```

输入 `y` 删除 ClashPT 和 Mihomo，输入 `c` 保留 Mihomo，直接回车取消。两种卸载方式都会清除 ClashPT 保存的订阅、配置、数据库及 Shell 集成文件。卸载后，从 Shell 启动文件中移除加载 `clashpt.bash` 或 `clashpt.zsh` 的那一行。自定义安装前缀时，给卸载脚本传入相同的 `--prefix DIR`。

## 普通用户模式

导入一份完整的 Clash/Mihomo YAML，再启动代理。

```bash
clashpt profile import ./config.yaml
clashpt start
clashpt status
clashpt tui
```

用 `clashpt proxy on --shell` 获取应用所需的本地代理地址。使用 `clashpt stop` 停止 Mihomo。

TUI 默认打开 Proxies，可切换节点和测延迟。按 `1` 打开 Overview，查看上下行实时波形、流量和连接数。Rules、Conns、Logs 分别显示规则、活动连接和日志。Profiles 用于选择保存的配置。

| TUI 按键 | 操作 |
| --- | --- |
| `1`–`6` | 切换 Overview、Proxies、Rules、Conns、Logs、Profiles |
| `?` | 打开当前页面的快捷键弹窗，按 `Enter`、`Esc` 或 `?` 关闭 |
| `Tab`、左右方向键 | 在 Proxies 的策略组和节点列表之间切换焦点 |
| 上下方向键 | 移动光标 |
| `Enter` | 在 Proxies 中进入策略组或选择节点，在 Profiles 中激活配置 |
| `d`、`g`、`t` | 在 Proxies 中测试当前节点、当前策略组的节点、所有可测速节点 |
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

## 浏览器和终端代理

Firefox 等应用可手动选择 HTTP 代理，主机填 `127.0.0.1`，端口取自 `clashpt status`，用户名和密码留空。加载 Shell 集成后，`clashpt start` 和 `clashpt stop` 会同步设置或清除当前终端的代理变量。`clashpt proxy on/off` 可单独切换变量。

## TUI 配置

默认从 `~/.config/clashpt/clashpt.yaml` 读取应用设置。如果设置了绝对路径 `XDG_CONFIG_HOME`，则读取 `$XDG_CONFIG_HOME/clashpt/clashpt.yaml`。修改 `theme` 后重新打开 TUI 即可生效。颜色支持 `cyan` 等终端颜色名、带引号的十六进制颜色（例如 `"#2b3a52"`）和带引号的 ANSI 色号（例如 `"42"`）。缺少文件或省略字段时会使用[内置默认配置](src/default_clashpt.yaml)。可以保存一份 YAML 在默认路径下，按需覆盖设置。

可以指定其他配置文件：

```bash
clashpt tui --config /path/to/clashpt.yaml
```

`indicators.active_node`、`indicators.running` 和 `indicators.stopped` 分别设置当前节点、运行中和已停止的符号。对应的颜色在 `theme.active_node_indicator`、`theme.running_indicator` 和 `theme.stopped_indicator` 中设置。`clashpt status --config /path/to/clashpt.yaml` 可指定状态输出配置。`clashpt status --json` 继续输出纯 JSON。

当前配置的标记由 `indicators.active_profile` 和 `theme.active_profile_indicator` 控制，CLI 的 `profile list` 和 TUI 的 Profiles 页共用这两个设置。设为 `null` 时沿用 `status` 的运行中符号和颜色。

同一文件的 `latency_test.concurrency` 控制批量测延迟的并行数，填正整数，默认 `12`。系统模式的实际并行数取该值和已部署的 `system.gateway_workers` 中的较小值。修改后重新打开 TUI。测速完成提示会在 3 秒后消失，节点旁的延迟结果仍可查看。

## 保存和切换多个配置

普通用户模式下，可以用 `--name` 给本地配置和订阅命名。订阅 URL 需要返回一份完整的 Clash/Mihomo YAML。

```bash
clashpt profile subscribe 'https://example.com/your-subscription' --name A
clashpt profile import ./config.yaml --name B
clashpt profile list
clashpt profile use B
clashpt profile update A
clashpt profile export B
clashpt profile delete B
```

第一份保存的配置会成为当前配置。之后保存其他名称会保留当前选择。省略 `--name` 时，重复导入同一文件路径或同一订阅 URL 会更新对应的配置，其他来源自动获得独立名称。要明确替换某份配置，可以指定它的名称。

在 TUI 中按 `6` 打开 Profiles，用方向键选中配置，再按 `Enter` 激活。Mihomo 运行时会重启以应用选中的配置，现有连接可能中断。如果两个配置的代理端口不同，已启用代理的 zsh 终端需要重新执行 `clashpt proxy on` 来更新环境变量。

在 Profiles 按 `Delete` 或 `x`，再按 `y` 确认删除。删除当前配置（包括 `default`）时，会选择按名称排序后的第一份剩余配置。删掉最后一份时会停止 Mihomo，并清空当前选择。

省略 URL 参数时，`profile subscribe` 从标准输入读取 URL。`profile update [名称]` 会刷新保存的订阅。更新失败会保留上一份配置。`profile list --json` 可供脚本读取。

## 导入后的配置文件位置

普通用户模式默认把文件放在 `~/.config/clashpt/`。如果设置了绝对路径 `XDG_CONFIG_HOME`，则用 `$XDG_CONFIG_HOME/clashpt/`。可以用 `CLASHPT_DATA_DIR` 或 `--data-dir` 覆盖，命令行参数优先。

| 状态目录下的路径 | 内容 |
| --- | --- |
| `profile.json` | 保存全部配置的元数据和当前选择，每份配置的 `generation` 指向自己的配置目录 |
| `profile-<随机后缀>/source.yaml` | 本次导入的原始 YAML 副本，传给 `import` 的文件保留在原位置 |
| `profile-<随机后缀>/config.yaml` | Mihomo 实际使用的运行配置，包含 ClashPT 设置的本地代理端口和控制 socket |
| `core/` | Mihomo 工作目录。新保存的配置各有自己的缓存子目录 |
| `validate.log`、`mihomo.log` | 配置校验日志和内核运行日志 |

每次成功导入或更新会生成新的配置目录。要获取当前原始配置，可以用 `clashpt profile export`。

如果希望把状态放在仓库的 `data` 目录，先在当前终端设置环境变量，再运行导入、启动等命令。

```bash
export CLASHPT_DATA_DIR="$PWD/data/clashpt"
clashpt profile import ./config.yaml
clashpt start
```

## 系统模式（sudo + TUN）

需要运行中的 systemd 和 `/dev/net/tun`。由管理员先从可信来源将 clashpt 和 mihomo 安装到 root 拥有且普通用户不可写的 /usr/local/bin。首次部署和升级均使用这两个绝对路径。

```bash
sudo /usr/local/bin/clashpt --mihomo /usr/local/bin/mihomo system apply ./config.yaml
clashpt --system status
clashpt --system tui
```

可以在 `~/.config/clashpt/clashpt.yaml` 中设置 `system.gateway_workers`（默认 `8`），再由管理员应用。把同一文件中的 `latency_test.concurrency` 调到所需的测速并行数，重新打开 TUI。

```bash
sudo /usr/local/bin/clashpt --mihomo /usr/local/bin/mihomo \
  system apply ./config.yaml --settings "$HOME/.config/clashpt/clashpt.yaml"
```

`system apply` 将运行配置保存到 `/etc/clashpt/config.yaml`，并启动系统服务。默认授权调用 sudo 的用户通过 CLI 或 TUI 管理节点。系统服务的启停使用 `clashpt --system start` 和 `clashpt --system stop`，服务日志可用 `sudo journalctl -u clashpt.service` 查看。

管理员可以执行 `sudo /usr/local/bin/clashpt system remove` 卸载已登记的系统部署。

系统模式通过管理员应用配置。要使用普通模式中保存的一份配置，先按名称导出 YAML，再以管理员身份应用。

```bash
(umask 077; clashpt profile export A > ./subscription.yaml)
sudo /usr/local/bin/clashpt --mihomo /usr/local/bin/mihomo system apply ./subscription.yaml
```


