# ClashPT

ClashPT 是面向 Linux 的 Mihomo 管理工具。TUI 和 CLI 可以查看策略组、切换节点和查询运行状态。普通用户模式提供本地 HTTP/SOCKS 代理，系统模式通过 systemd 启用 TUN。

##

```bash
cargo build --release --locked
mkdir -p ~/.local/bin
install -m 755 target/release/clashpt ~/.local/bin/clashpt
```

## 安装

准备 Rust 工具链和一个可执行的 Mihomo 内核。普通用户模式默认从 `PATH` 查找 `mihomo`，也可以用 `--mihomo /path/to/mihomo` 或 `CLASHPT_MIHOMO` 指定路径。系统模式首次执行 `system apply` 时，会将指定的 Mihomo 复制到 `/etc/clashpt/mihomo`。

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

## zsh 代理环境变量

在 `~/.zshrc` 中添加下面的别名。将示例中的 `7891` 换成 `clashpt status` 显示的混合代理端口。

```zsh
alias proxy_on='
export HTTP_PROXY=http://127.0.0.1:7891
export HTTPS_PROXY=http://127.0.0.1:7891
export ALL_PROXY=socks5h://127.0.0.1:7891
export http_proxy=http://127.0.0.1:7891
export https_proxy=http://127.0.0.1:7891
export all_proxy=socks5h://127.0.0.1:7891
'
alias proxy_off='unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy'
```

要在成功启停内核时自动设置或清理变量，可以在 `~/.zshrc` 中再加载集成脚本。将路径换成仓库的实际位置。

```zsh
source /path/to/clashpt/scripts/clashpt.zsh
```

加载脚本后，`clashpt start/stop` 会同步更新当前 zsh 的代理变量，`clashpt proxy on/off` 只操作变量并打印结果。代理端口取自已导入的配置。`clash` 也可以作为命令名使用。直接执行二进制时，`proxy on/off` 会在框中显示可执行的命令。用 `proxy on --shell` 或 `proxy off --shell` 可获取纯 `export` 或 `unset`，方便脚本调用。

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

第一份保存的配置会成为当前配置。之后保存其他名称会保留当前选择。省略 `--name` 时，重复导入同一文件路径或同一订阅 URL 会更新对应的配置，其他来源自动获得独立名称。要明确替换某份配置，可以指定它的名称。旧的单份配置会显示为 `default`。

在 TUI 中按 `6` 打开 Profiles，用方向键选中配置，再按 `Enter` 激活。Mihomo 运行时会重启以应用选中的配置，现有连接可能中断。如果两个配置的代理端口不同，已启用代理的 zsh 终端需要重新执行 `clashpt proxy on` 来更新环境变量。

在 Profiles 按 `Delete` 或 `x`，再按 `y` 确认删除。删除当前配置（包括 `default`）时，会选择按名称排序后的第一份剩余配置。删掉最后一份时会停止 Mihomo，并清空当前选择。

省略 URL 参数时，`profile subscribe` 从标准输入读取 URL。`profile update [名称]` 会刷新保存的订阅。更新失败会保留上一份配置。`profile list --json` 可供脚本读取。

## 导入后的配置文件位置

普通用户模式默认把文件放在 `~/.config/clashpt/`。如果设置了绝对路径 `XDG_CONFIG_HOME`，则用 `$XDG_CONFIG_HOME/clashpt/`。可以用 `CLASHPT_DATA_DIR` 或 `--data-dir` 覆盖，命令行参数优先。

| 状态目录下的路径 | 内容 |
| --- | --- |
| `profile.json` | 保存全部配置的元数据和当前选择，每份配置的 `generation` 指向自己的配置目录 |
| `profile-<随机后缀>/source.yaml` | 本次导入的原始 YAML 副本，传给 `import` 的文件保留在原位置 |
| `profile-<随机后缀>/config.yaml` | Mihomo 实际使用的运行配置，包含 ClashPT 设置的本地代理和控制端口 |
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

需要运行中的 systemd 和 `/dev/net/tun`。首次应用时指定 Mihomo 的实际路径。

```bash
sudo "$(command -v clashpt)" --mihomo "$(command -v mihomo)" system apply ./config.yaml
clashpt --system status
clashpt --system tui
```

可以在 `~/.config/clashpt/clashpt.yaml` 中设置 `system.gateway_workers`（默认 `8`），再由管理员应用。把同一文件中的 `latency_test.concurrency` 调到所需的测速并行数，重新打开 TUI。

```bash
sudo "$(command -v clashpt)" --mihomo "$(command -v mihomo)" \
  system apply ./config.yaml --settings "$HOME/.config/clashpt/clashpt.yaml"
```

`system apply` 将运行配置保存到 `/etc/clashpt/config.yaml`，并启动系统服务。默认授权调用 sudo 的用户通过 CLI 或 TUI 管理节点。系统服务的启停使用 `clashpt --system start` 和 `clashpt --system stop`，服务日志可用 `sudo journalctl -u clashpt.service` 查看。

系统模式通过管理员应用配置。要使用普通模式中保存的一份配置，先按名称导出 YAML，再以管理员身份应用。

```bash
(umask 077; clashpt profile export A > ./subscription.yaml)
sudo "$(command -v clashpt)" --mihomo "$(command -v mihomo)" system apply ./subscription.yaml
```


