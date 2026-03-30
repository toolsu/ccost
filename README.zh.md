# ccost <img src="https://raw.githubusercontent.com/toolsu/ccost/main/logo.svg" alt="ccost" width="35" />

分析 Claude Code 本地对话日志和 statusline 数据中的 token 使用量与费用。

读取 `~/.claude/projects/` 下的 JSONL 文件，去重流式条目，基于 LiteLLM 定价计算费用，支持多种分组维度，输出为表格、JSON、Markdown、HTML、CSV、TSV 或终端 braille 图表（见[截图](#截图)）。

[`ccost sl` 子命令](#状态栏数据分析ccost-sl)（请先[配置 statusline.jsonl](#配置-statuslinejsonl)）分析 `~/.claude/statusline.jsonl`，用于速率限制追踪、会话汇总、预算估算和费用对比（参见 [`ccost` 与 `ccost sl` 的对比](#ccost-与-ccost-sl)）。

使用 Rust 编写，速度极快，测试完善。同时也是 [CC仪表盘（Claude Code 仪表盘）](https://github.com/toolsu/ccdashboard)（一个 Tauri 桌面应用）的底层库。

[![crates.io](https://img.shields.io/crates/v/ccost)](https://crates.io/crates/ccost) [![npm](https://img.shields.io/npm/v/ccost)](https://www.npmjs.com/package/ccost) [![CI](https://github.com/toolsu/ccost/actions/workflows/ci.yml/badge.svg)](https://github.com/toolsu/ccost/actions/workflows/ci.yml) [![License](https://img.shields.io/badge/license-MIT-brightgreen)](https://github.com/toolsu/ccost/blob/main/LICENSE)

[English Docs](https://github.com/toolsu/ccost#readme)

## 安装

```bash
npm i -g ccost
```

或从 [GitHub Releases](https://github.com/toolsu/ccost/releases) 下载预编译二进制文件。

或通过 Cargo（从源码编译）：

```bash
cargo install ccost
```

## 快速开始

```bash
ccost                                    # 默认：按天、按模型分组
ccost --from 2026-03-01 --to 2026-03-31 # 指定日期范围
ccost --per model --cost decimal         # 按模型，2位小数费用
ccost --chart cost                       # 费用趋势图

ccost sl                                 # 5小时速率限制窗口（默认）
ccost sl --per action                    # 速率限制时间线
ccost sl --per 5h --cost decimal         # 预算估算
ccost sl --chart 5h                      # 速率限制图表
```

## CLI 参考

### 分组

| 参数 | 说明 |
|------|------|
| `--per <dim>` | 分组维度：`day`、`hour`、`month`、`session`、`project`、`model`。最多指定2个做嵌套分组。默认 `--per day --per model`。 |
| `--order <order>` | 排序：`asc`（默认）或 `desc`。 |

两级分组生成父子行，如 `--per day --per model` 每天下面展开各模型明细。

### 日期过滤

| 参数 | 说明 |
|------|------|
| `--from <date>` | 起始日期（含）。`YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM:SS`。 |
| `--to <date>` | 结束日期（含）。 |
| `--5hfrom`、`--5hto` | 以指定时间为起点/终点的5小时窗口。 |
| `--1wfrom`、`--1wto` | 以指定时间为起点/终点的1周窗口。 |

`--5hfrom` 和 `--5hto` 互斥，`--1wfrom` 和 `--1wto` 互斥。`5h`/`1w` 快捷方式不能和 `--from`/`--to` 同时使用。

### 记录过滤

| 参数 | 说明 |
|------|------|
| `--project <name>` | 按项目路径过滤（不区分大小写，子串匹配）。 |
| `--model <name>` | 按模型名过滤（不区分大小写，子串匹配）。 |
| `--session <id>` | 按会话 ID 过滤（不区分大小写，子串匹配）。 |

### 时区

| 参数 | 说明 |
|------|------|
| `--tz <tz>` | `UTC`、固定偏移（`+08:00`）或 IANA 名称（`Asia/Shanghai`）。默认：系统本地时区。 |

### 费用显示

| 参数 | 说明 |
|------|------|
| `--cost <mode>` | `true`（默认）：整数 `$1`。`decimal`：`$1.23`。`false`：隐藏费用。 |

### 输出

| 参数 | 说明 |
|------|------|
| `--output <fmt>` | `json`、`markdown`、`html`、`txt`、`csv`、`tsv`。写入文件（默认 `ccost.<ext>`）。 |
| `--filename <path>` | 自定义输出文件名。 |
| `--table <mode>` | `auto`（默认）、`full`、`compact`。终端宽度 < 120 时 auto 选 compact。 |
| `--copy <fmt>` | 复制到剪贴板。支持 `pbcopy`/`xclip`/`wl-copy`/OSC52。独立于 `--output`。 |

### 图表

| 参数 | 说明 |
|------|------|
| `--chart <mode>` | 终端 braille 折线图：`cost` 或 `token`。 |

粒度根据时间范围自动选择（小时/天/月）。`--chart` 只支持 `--output txt`。

### 定价

| 参数 | 说明 |
|------|------|
| `--live-pricing` | 从 LiteLLM 获取最新定价（需网络）。 |
| `--pricing-data <path>` | 自定义定价 JSON 文件。 |

### 其他

| 参数 | 说明 |
|------|------|
| `--claude-dir <dir>` | 覆盖 Claude 配置目录。默认 `~/.claude` 和 `~/.config/claude`。 |
| `--help` | 显示帮助。 |
| `--version` | 显示版本。 |

## 状态栏数据分析（`ccost sl`）

分析 `~/.claude/statusline.jsonl`，这是 Claude Code 状态栏 hook 生成的文件，每次操作记录速率限制、费用、时长和上下文窗口使用情况（参见 [`ccost` 与 `ccost sl` 的对比](#ccost-与-ccost-sl)）。

### 配置 statusline.jsonl

#### 自动配置

在 Claude Code 中运行（任意系统）：

```
/statusline Set up a command-type statusline using ~/.claude/statusline.sh (or ~/.claude/statusline.ps1 on Windows). The script should read stdin and append {"ts":<unix_epoch>,"data":<stdin_json>} to ~/.claude/statusline.jsonl. Create the script and configure settings.json.
```

#### 手动配置

##### Linux、macOS、WSL

在 `~/.claude/settings.json` 中添加：

```json
  "statusLine": {
    "type": "command",
    "command": "~/.claude/statusline.sh"
  }
```

创建 `~/.claude/statusline.sh`：

```bash
#!/bin/bash
input=$(cat)
echo "{\"ts\":$(date +%s),\"data\":$input}" >> ~/.claude/statusline.jsonl
```

```sh
chmod +x ~/.claude/statusline.sh
```

如果已有状态栏脚本，在末尾加上 `echo` 那行即可。

##### Windows (PowerShell)

在 `~/.claude/settings.json` 中添加：

```json
  "statusLine": {
    "type": "command",
    "command": "powershell -NoProfile -File ~/.claude/statusline.ps1"
  }
```

创建 `~/.claude/statusline.ps1`：

```powershell
$input = $Input | Out-String
$ts = [int](New-TimeSpan -Start (Get-Date '1970-01-01') -End (Get-Date).ToUniversalTime()).TotalSeconds
$line = "{""ts"":$ts,""data"":" + $input.Trim() + "}"
Add-Content -Path "$env:USERPROFILE\.claude\statusline.jsonl" -Value $line -Encoding UTF8
```

### 视图模式

| 命令 | 说明 |
|------|------|
| `ccost sl` | 5小时窗口（默认） |
| `ccost sl --per action` | 速率限制时间线，含每次操作的费用 |
| `ccost sl --per session` | 按会话 |
| `ccost sl --per project` | 按项目 |
| `ccost sl --per day` | 按天 |
| `ccost sl --per 1h` | 5小时窗口内的1小时细分 |
| `ccost sl --per 5h` | 5小时速率限制窗口 |
| `ccost sl --per 1w` | 1周速率限制窗口 |
| `ccost sl --chart 5h` | 5小时速率限制百分比图表 |
| `ccost sl --chart 1w` | 1周速率限制百分比图表 |
| `ccost sl --chart cost` | 累计费用图表 |

`session/project/day/1h/5h/1w` 视图共享列：`Cost`、`Duration`、`API Time`、`Lines +/-`、`Sess`、`5h%`、`1w%`。窗口视图（`1h/5h/1w`）额外显示 `Est 5h Budget` 或 `Est 1w Budget`。`1h` 视图还有 `5h Resets`。`action` 视图有独立布局：`Time`、`Cost`、`5h%`、`1w%`、`5h Resets`、`Session`。

`5h%` 和 `1w%` 列显示观测到的最小-最大范围（如 `1-29%`）。

### 状态栏专用参数

| 参数 | 说明 |
|------|------|
| `--file <path>` | 状态栏文件路径。默认 `~/.claude/statusline.jsonl`。 |
| `--nopromo` | 禁用促销调整。见下文。 |
| `--cost-diff` | 对比状态栏自报费用和 LiteLLM 计算费用。仅 `--per session`。 |

所有共享参数（`--from`、`--to`、`--tz`、`--session`、`--project`、`--model`、`--cost`、`--output`、`--filename`、`--copy`、`--order`、`--table`）均适用于 `ccost sl`。

### 分段聚合

会话关闭后恢复时，累计计数器（费用、时长、token、行数）会归零。`ccost sl` 检测这些重置并正确跨分段求和。

速率限制是账户级别的，不随会话重置。

注意：`--per day` 将每个会话分配到其开始的那一天。跨午夜的会话不会拆分。

### 预算估算

窗口视图根据费用和速率限制百分比变化量估算总预算：

```
Est 5h Budget = Cost * 100 / delta_5h%    (用于 --per 1h 和 --per 5h)
Est 1w Budget = Cost * 100 / delta_1w%    (用于 --per 1w)
```

其中 `delta% = max% - min%`，即窗口内速率限制实际消耗量。

由于 `ccost sl` 只能看到 Claude Code 的费用，其他 Claude 产品（网页版、桌面端、移动端）的使用会增大分母但不增加分子，因此估算值是真实预算的**下界**，差距取决于非 CC 用量的占比。

### 促销调整

预算估算默认会针对已知的2倍用量促销期（2025年12月、2026年3月）进行调整。促销期间速率限制预算翻倍，同样的花费只消耗一半的百分比。用 `--nopromo` 禁用：

```sh
ccost sl --per 5h --nopromo
```

部分重叠促销期的窗口按重叠比例调整。

## 示例

```bash
# HTML 报告，2位小数费用
ccost --per project --cost decimal --output html --filename report.html

# 过滤特定会话
ccost --session abc123 --from 2026-03-23

# UTC 时区，降序
ccost --tz UTC --order desc --from 2026-03-01

# 只看 opus 模型，不显示费用
ccost --model opus --cost false

# 从指定时间开始的5小时窗口
ccost --5hfrom 2026-03-24T10:00:00

# 实时定价
ccost --live-pricing

# 最近一周的费用图表
ccost --chart cost --1wto 2026-03-25

# 按月导出 CSV
ccost --per month --output csv --filename usage.csv

# 两级分组：项目 > 模型
ccost --per project --per model --table full

# 复制 JSON 到剪贴板
ccost --copy json

# 状态栏：预算估算
ccost sl --per 5h --cost decimal

# 状态栏：费用对比
ccost sl --per session --cost-diff --cost decimal

# 状态栏：速率限制图表
ccost sl --chart 5h
```

## 库 API

ccost 也是一个 Rust 库。

```toml
[dependencies]
ccost = { path = "../ccost" }
```

### 处理流程

```
load_records -> calculate_cost -> group_records -> format_*
```

```rust
use ccost::*;

let options = LoadOptions {
    from: Some("2026-03-01".to_string()),
    to: Some("2026-03-31".to_string()),
    tz: Some("UTC".to_string()),
    model: Some("opus".to_string()),
    ..Default::default()
};

let result = load_records(&options);
let pricing = load_pricing();
let priced = calculate_cost(&result.records, Some(&pricing));

let dimensions = vec![GroupDimension::Day, GroupDimension::Model];
let group_options = GroupOptions {
    order: SortOrder::Asc,
    tz: Some("UTC".to_string()),
};
let grouped = group_records(&priced, &dimensions, Some(&group_options));

use ccost::formatters::table::TableOptions;
let table_opts = TableOptions {
    dimension_label: "Date/Model".to_string(),
    price_mode: PriceMode::Decimal,
    compact: false,
    color: Some(true),
};
let table = format_table(&grouped.data, &grouped.totals, &table_opts);
print!("{}", table);
```

### 主要类型

```rust
struct LoadOptions {
    claude_dir: Option<String>,
    from: Option<String>,       // YYYY-MM-DD 或 YYYY-MM-DDTHH:MM:SS
    to: Option<String>,
    tz: Option<String>,         // UTC、+HH:MM 或 IANA 名称
    project: Option<String>,    // 子串过滤
    model: Option<String>,
    session: Option<String>,
}

enum GroupDimension { Day, Hour, Month, Session, Project, Model }

struct GroupedData {
    label: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    input_cost: f64,
    cache_creation_cost: f64,
    cache_read_cost: f64,
    output_cost: f64,
    total_cost: f64,
    children: Option<Vec<GroupedData>>,
}
```

### 格式化器

| 函数 | 输出 |
|------|------|
| `format_table()` | Unicode 表格，可选 ANSI 颜色 |
| `format_txt()` | 纯文本表格，无颜色 |
| `format_json()` | JSON，含 `meta`、`data`、`totals`、`dedup` |
| `format_markdown()` | GFM Markdown 表格 |
| `format_html()` | 独立 HTML 页面，支持列排序 |
| `format_csv()` | RFC 4180 CSV |
| `format_tsv()` | TSV |
| `render_chart()` | Braille 折线图 |

## 工作原理

### 数据源

读取 Claude Code 本地存储的 JSONL 对话日志：
- `~/.claude/projects/*/`（会话文件和 `subagents/` 子目录）
- `~/.config/claude/projects/*/`（备用位置）

两个位置都会扫描，通过符号链接检测去重。

### 去重

流式 API 会产生多条相同 `messageId:requestId` 的记录，`output_tokens` 递增。只保留每个 key 的最大 `output_tokens` 记录。去重是全局的，也处理父会话和 subagent 文件之间的重复。

### 定价

按记录计算费用，模型到价格的查找分三级：
1. 精确匹配
2. 去掉日期后缀（`-YYYYMMDD` / `@YYYYMMDD`）后重试
3. `claude-*` key 中的子串匹配

定价数据编译时从 LiteLLM 打包。用 `--live-pricing` 获取最新，或 `--pricing-data` 指定自定义文件。

### `ccost` 与 `ccost sl`

以下对比 `ccost`（数据来自 `~/.claude/projects/`）和 `ccost sl`（数据来自 `statusline.jsonl`）。

`~/.claude/projects/` 包含每个 API 请求的 JSONL 日志，记录了详细的 token 计数（输入、输出、缓存创建、缓存读取）。`ccost` 基于这些计数和 LiteLLM 定价计算费用。

`~/.claude/statusline.jsonl` 包含 Claude Code 状态栏的定期快照，其中有服务端返回的累计费用，以及5小时和1周速率限制使用百分比，这些数据在其他地方无法获取。

通过 `ccost sl --per session --cost-diff` 可以看到，当 LiteLLM 定价准确时，从 `~/.claude/projects/` 计算的费用与 `statusline.jsonl` 中服务端返回的费用非常接近。statusline 的费用有时可能偏低，因为它不一定包含 subagent 的费用。由于 `~/.claude/projects/` 有完整的逐请求 token 明细，它可能是更可靠的费用来源。

`statusline.jsonl` 的核心价值在于速率限制百分比，而非费用。因此 [Claude Code 仪表盘](https://github.com/toolsu/ccdashboard) 使用的是从 `~/.claude/projects/` 计算的费用数据，结合 `statusline.jsonl` 中的5小时和1周速率限制百分比数据。

### 项目路径解码

目录名如 `-home-user-workspace-test` 解码为 `/home/user/workspace/test`。

## 截图

### 终端输出

![终端输出](https://raw.githubusercontent.com/toolsu/ccost/main/screenshot_terminal.png)

### 状态栏分析（`ccost sl`）

![状态栏输出](https://raw.githubusercontent.com/toolsu/ccost/main/screenshot_terminal_sl.png)

### HTML 报告

![HTML 报告](https://raw.githubusercontent.com/toolsu/ccost/main/screenshot_html.png)

## 开发

```bash
cargo test             # 运行所有测试
cargo build --release  # 优化构建
cargo run -- --help    # CLI 帮助
```

## 许可证

MIT
