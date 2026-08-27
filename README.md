# NJU Coding Agent

一个不依赖 Agent 框架的 Rust 编程智能体。项目已完成核心工具循环、安全边界、上下文管理和全屏终端界面。

## 目标

- 自己管理对话历史、模型响应解析、工具执行和循环终止。
- 通过 OpenAI-compatible Chat Completions API 使用模型原生 tool calling。
- 只在指定工作目录内读写文件，并对命令执行施加确定性的安全策略。
- 保持实现小而清晰，使每个关键设计都能在面试中解释和辩护。

## 运行

推荐使用 DeepSeek 官方 OpenAI-compatible API。只需设置密钥；默认模型为支持 tool calling 的 `deepseek-v4-flash`，默认上下文窗口为 1M：

```powershell
$env:DEEPSEEK_API_KEY = "新生成的密钥"
```

可选覆盖项为 `DEEPSEEK_BASE_URL`、`DEEPSEEK_MODEL` 和 `CODING_AGENT_CONTEXT_WINDOW`。也继续支持通用的 `CODING_AGENT_API_KEY`、`CODING_AGENT_BASE_URL`、`CODING_AGENT_MODEL`。

执行单个任务：

```powershell
cargo run -- --workspace D:\path\to\project "修复测试失败并运行测试验证"
```

`--workspace` 是文件与命令的硬边界。如果任务需要写入 `D:\NJU-Agent\test`，启动时就应把该目录设为 workspace，不要在任务文本中要求写到当前 workspace 之外。

不传任务文本时默认进入全屏 TUI。高风险或未知命令会在界面中弹出确认框；演示环境可显式传入 `--yes`。需要简单逐行模式时使用 `--plain`。

TUI 快捷键：

- `Enter` 发送，`Shift+Enter` 换行。
- `Up/Down`、`PageUp/PageDown`、`Home/End` 滚动对话。
- `Ctrl+P/N` 浏览输入历史，`Ctrl+L` 折叠事件栏。
- `F1` 帮助，`Ctrl+D` 退出。
- 内置命令：`/help`、`/clear`、`/status`、`/tools`、`/exit`。

## 当前里程碑

- [x] 需求与旧项目复用边界审查
- [x] OpenAI-compatible LLM 客户端
- [x] Agent 循环与对话历史
- [x] 文件、搜索、编辑、命令工具
- [x] 安全边界与自动化测试
- [x] 从 SecAudit 适配上下文预算与本地压缩，保留最近完整用户轮次
- [x] DeepSeek V4 默认配置及思考模式 `reasoning_content` 回传
- [x] 参考 SecAudit 迁移全屏终端对话界面、事件栏和命令确认弹窗
- [x] 使用真实 DeepSeek 模型完成文件读取、命令运行与结果确认冒烟任务
- [x] 建立可重置的 checkout 故障工程，验证修复前失败、预期最小修复后 5 项测试全部通过
- [x] 使用真实模型完成可复现 bug 的定位、一行最小修复和 5/5 测试验证
- [x] 实现运行时约束的 `PLAN → EXECUTE → VERIFY → DONE` 状态机
- [x] 确定性检测 Rust、Python、Node.js、Maven、Gradle、Go 和 .NET 项目并选择验证命令

## Plan → Execute → Verify

- `PLAN` 只暴露 `read_file`、`list_files` 和 `search_text`；模型必须先输出可执行计划。
- 每次成功 `write_file` 或 `replace_text` 都会递增 `workspace_revision`。
- 只有明确的 test/build/lint/program 命令且真实 `exit_code: 0` 才更新 `last_verified_revision`。
- 发生过写入时，仅当 `last_verified_revision == workspace_revision` 才允许进入 `DONE`。模型的文本声明不能绕过该约束。
- TUI 事件栏展示 `PLAN / EXEC / VERIFY / DONE`，Runtime 区域展示当前 revision 及验证状态。

## 自动项目检测

Agent 优先读取 workspace 根目录中的确定性标志，并把结果注入 PLAN 和 VERIFY：

- `Cargo.toml` → `cargo test`
- `pyproject.toml` / pytest 配置 → `python -m pytest`
- `tests/*.py` → `python -m unittest discover -s tests -v`
- `package.json` → 优先 `npm test`，否则选择 build/lint script
- `pom.xml` / Gradle 配置 / `go.mod` / `.sln` / `.csproj` → 对应原生测试命令

检测不到时不猜测或安装依赖，而是让模型根据已读取的项目配置选择验证命令。命令仍必须经过安全策略并真实返回退出码 0。

## 已实现的终止与错误路径

- 每轮最多执行指定步数，避免模型无限循环。
- 工作区状态未变时，相同工具参数只实际执行一次；成功写入或编辑文件后会废弃旧的读取与测试结果，允许重新验证。
- 未知工具、坏 JSON 参数、工具失败都会变成 observation 交回模型纠正。
- 模型既不返回文本也不返回工具调用时明确报错。
- 工具输出统一截断，避免大文件或命令输出撑爆上下文。
- 上下文达到配置窗口的 80% 时自动生成本地摘要，并压缩到约 60% 的预算目标。
- 相同非只读命令批准一次后，本次运行中不再重复询问；`--yes` 可自动批准普通命令，但不能绕过危险命令与 workspace 外路径拦截。

详细计划见 [PROJECT_PLAN.md](PROJECT_PLAN.md)。
