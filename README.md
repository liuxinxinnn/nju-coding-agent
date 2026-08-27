# NJU Coding Agent

一个不依赖 Agent 框架的 Rust 编程智能体。项目当前处于第一里程碑开发阶段。

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
- [ ] 使用真实模型完成创建文件与修复 bug 的冒烟任务

## 已实现的终止与错误路径

- 每轮最多执行指定步数，避免模型无限循环。
- 相同工具调用重复三次时中止，避免无效消耗。
- 未知工具、坏 JSON 参数、工具失败都会变成 observation 交回模型纠正。
- 模型既不返回文本也不返回工具调用时明确报错。
- 工具输出统一截断，避免大文件或命令输出撑爆上下文。
- 上下文达到配置窗口的 80% 时自动生成本地摘要，并压缩到约 60% 的预算目标。

详细计划见 [PROJECT_PLAN.md](PROJECT_PLAN.md)。
