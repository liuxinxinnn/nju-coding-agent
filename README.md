# NJU Coding Agent

一个不依赖 Agent 框架的 Rust 编程智能体。项目已完成核心工具循环、安全边界、上下文管理和全屏终端界面。

## 目标

- 自己管理对话历史、模型响应解析、工具执行和循环终止。
- 通过 OpenAI-compatible Chat Completions API 使用模型原生 tool calling。
- 文件工具只在指定工作目录内读写，并对命令执行施加确定性的安全策略。
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

`--workspace` 是文件工具的强制边界。如果任务需要写入 `D:\NJU-Agent\test`，启动时就应把该目录设为 workspace，不要在任务文本中要求写到当前 workspace 之外。

`run_command` 固定以 workspace 为工作目录，并拦截父目录路径、显式 workspace 外绝对路径和已知危险命令，普通执行命令还需用户确认。但它不是容器或 OS 级沙箱：被执行的解释器或程序理论上仍可能自行访问其他路径。因此这里把命令机制描述为“策略限制与人工确认”，不宣称完全隔离。

不传任务文本时默认进入全屏 TUI。高风险或未知命令会在界面中弹出确认框；演示环境可显式传入 `--yes`。需要简单逐行模式时使用 `--plain`。

Plan 模式默认开启。TUI 中可用 `/plan off` 跳过独立规划阶段，或用 `/plan on` 恢复；启动时传 `--no-plan` 也可默认关闭。关闭 Plan 后流程变为 `EXECUTE → VERIFY`，但最后一次写入必须通过真实命令验证的 runtime invariant 始终开启。

TUI 中的计划和最终回答会随 DeepSeek SSE 增量实时显示；tool call 分片在本地聚合完成后才执行，流式失败且尚未显示文本时自动降级为普通请求。

TUI 快捷键：

- `Enter` 发送，`Shift+Enter` 换行。
- `Up/Down`、`PageUp/PageDown`、`Home/End` 滚动对话。
- 鼠标滚轮滚动指针所在的 Conversation/Events；`Ctrl+Up/Down`、`Ctrl+PageUp/PageDown`、`Ctrl+Home/End` 独立滚动事件栏。
- `Ctrl+P/N` 浏览输入历史，`Ctrl+L` 折叠事件栏。
- `F1` 帮助，`Ctrl+D` 退出。
- 内置命令：`/context`、`/memory`、`/plan [on|off]`、`/new`、`/sessions`、`/switch <id>`、`/delete <id>`、`/help`、`/clear`、`/status`、`/tools`、`/exit`。

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
- [x] 本地持久化多轮会话，支持新建、列表、切换和删除
- [x] 流式聚合文本、`reasoning_content` 与 tool call 参数，并在 TUI 中增量显示
- [x] 补齐模型响应与工具错误矩阵、严格编辑语义、revision-aware 去重、符号链接逃逸和上下文工具调用原子性测试
- [x] 建立 Python 单文件、Python 跨文件和 Rust 三组 benchmark，真实模型均经最新版 PEV 修复并通过独立 5/5 验收
- [x] 使用真实 DeepSeek SSE 验证 94 个增量片段正确聚合，且成功接收 `reasoning_content`
- [x] 参考 SecAudit 增加 `/context` 图形化用量弹窗，并支持会话级可选 Plan 模式
- [x] 采集非流式与 SSE 的真实 API `usage`，以真实 prompt token 校准本地字符估算并分开显示会话累计用量
- [x] 将上下文压缩升级为 cheap-first 三阶段策略：结构化压缩大型工具结果、原子安全地裁剪低价值历史、最后才调用模型语义摘要（失败时本地摘要兜底）
- [x] 增加本机长期记忆：全局 `USER.md`、按 workspace 隔离的 `MEMORY.md`、受控 `memory` 工具与 `/memory` 查看命令

三组真实模型结果及统一指标见 [benchmarks/RESULTS.md](benchmarks/RESULTS.md)。

## 可选 Plan → Execute → Verify

- 默认启用 Plan：`PLAN` 只暴露 `read_file`、`list_files` 和 `search_text`；模型先输出可执行计划。
- `/plan off` 或 `--no-plan` 跳过独立 `PLAN`，适合明确、简单的任务；设置会随当前会话持久化。
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

## 多轮会话

TUI 启动时自动恢复当前 workspace 最近使用的会话。每个会话独立保存完整消息历史、workspace、`workspace_revision`、`last_verified_revision`、Plan 开关、真实 API 累计用量和估算校准状态，所以切换回来后可以继续原有上下文与运行状态：

- `/new`：保存当前会话并新建空会话。
- `/sessions`：打开会话选择弹窗；用 `Up/Down` 选择、`Enter` 切换、`N` 新建、`Esc` 关闭，`●` 表示当前会话。
- `/switch <id>`：按完整 ID 或唯一前缀切换并恢复对话。
- `/delete <id>`：删除会话；删除当前会话时会自动建立一个空会话。

Windows 默认保存到 `%LOCALAPPDATA%\nju-coding-agent\sessions`；也可通过 `CODING_AGENT_DATA_DIR` 指定本地数据根目录。回退目录 `.nju-coding-agent-data/` 已加入 Git 忽略。会话中可能包含代码和用户输入，因此这些 JSON 文件只用于本机运行，不进入仓库。

当前会话是平面列表而不是树状分支。只分叉聊天却不同时恢复 Git/worktree 或文件快照，会出现“对话回到了旧节点、代码仍是新版本”的不一致，因此本项目没有把不完整的树状会话包装成已实现功能。

## 长期记忆

长期记忆没有照搬 SecAudit 面向漏洞审计的三层 CWE/实体索引，而是采用适合通用 Coding Agent 的两个 Markdown 作用域：

- `USER.md`：跨 workspace 的语言、输出风格和操作偏好。
- `MEMORY.md`：当前 workspace 的项目知识、架构决策和长期规则。

Windows 默认位置为 `%LOCALAPPDATA%\nju-coding-agent\memory`；项目记忆目录由规范化 workspace 路径生成稳定键。它们位于仓库之外，回退目录也受 Git 忽略。`/memory` 可查看实际路径与当前内容；也可以要求 Agent“记住某条长期规则”，由 `memory` 工具执行 `append / replace / remove`。该工具限制单次和文件大小、要求 replace/remove 唯一匹配，并拒绝疑似 API key、password、token 等凭据。

代码通过 `MemoryProvider` trait 隔离存储接口，默认实现是 `MarkdownMemoryStore`。这是一个可测试、可替换的内部扩展点；目前没有实现动态加载第三方代码的通用插件系统，避免为了展示概念引入额外供应链和生命周期复杂度。

记忆快照在每个用户轮次开始时注入 system prompt，轮次内由工具写入的新记忆从下一用户轮次生效。记忆只被标记为参考上下文，不能覆盖系统安全规则。明确的“记住/请记住/please remember”任务会跳过无意义的 PLAN，并由 runtime 阻止模型在 `memory` 工具成功前只做口头确认；“你记得……吗”一类查询也会直接从注入的记忆回答，不会去 workspace 搜索仓库外的记忆文件。

当前采用显式持久化为主：用户明确要求记住时强制写入；普通任务中模型也可主动调用 `memory` 保存明显的长期规则，但不会在每轮结束后后台自动抽取全部对话。这样避免把临时结论、模型误判或敏感内容静默固化。若以后增加自动抽取，应采用“候选记忆 → 确定性敏感信息过滤 → 必要时用户确认”，而不是直接自动写入。

## 上下文压缩与真实用量

当前 prompt 达到窗口 80% 时依次执行：

1. **Tool Result Compaction**：把旧的大型工具结果压缩为结构化证据，保留工具名、call id、成功/失败状态、exit code、原长度、确定性摘要以及 head/tail。
2. **History Pruning**：移除旧 reasoning、工具调用旁白、临时阶段提示和 runtime 已标记的重复 observation；删除重复 observation 时同时删除对应 tool call，保持调用与结果原子性。
3. **Semantic Summary**：若仍未降到 60% 目标，才额外调用模型总结旧历史；调用失败或返回空内容时自动使用确定性本地摘要。

所有便宜步骤都只处理最近完整用户轮次之前的历史，因此最近的 assistant tool call 与对应 tool result 不会被拆开。模型摘要不是唯一容错路径。

流式请求发送 `stream_options.include_usage=true`，解析 `choices=[]` 的最终 usage 块；普通响应也解析同一 usage 字段。最近一次真实 `prompt_tokens` 与发送前的原始本地估算形成校准倍率，并用平滑方式更新后续上下文估算。`/context` 同时展示“校准后的当前 prompt 估算”和“真实 API 最近一次/会话累计用量”，不会把累计消费误当成当前上下文占用。

## 已实现的终止与错误路径

- 每轮最多执行指定步数，避免模型无限循环。
- 去重键包含工具名、参数和 `workspace_revision`；同一 revision 中相同调用只实际执行一次，文件修改后相同读取或测试会在新 revision 重新执行。
- 未知工具、坏 JSON 参数、工具失败都会变成 observation 交回模型纠正。
- 模型既不返回文本也不返回工具调用时明确报错。
- 工具输出统一截断，历史中的大结果还会优先经过 cheap-first 压缩，避免大文件或命令输出撑爆上下文。
- 最新的读取、列表和搜索结果会标记为当前 workspace 的权威证据，提醒模型不得沿用旧会话中的冲突值。
- 上下文达到配置窗口的 80% 时启动 cheap-first 三阶段压缩，并以约 60% 为预算目标。
- `/context` 打开与 SecAudit 风格一致的图形化弹窗，展示校准估算、100 格网格、system/tools/messages/free 明细以及真实 API 最近/累计 token。
- 相同非只读命令批准一次后，本次运行中不再重复询问；`--yes` 可自动批准普通命令，但不能绕过危险命令与 workspace 外路径拦截。

详细计划见 [PROJECT_PLAN.md](PROJECT_PLAN.md)。
