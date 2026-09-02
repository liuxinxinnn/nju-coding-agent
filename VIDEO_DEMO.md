# 两分钟视频演示案例

## 案例目标

演示 Agent 修复一个可复现的 Python 跨文件订单折扣缺陷。初始完整测试为 3/5 通过；正确修复必须同时修改 `order/models.py` 和 `order/service.py`，不得修改测试。这个案例能在一次任务中展示自动项目检测、Plan、工具调用、跨文件精确编辑、workspace revision、真实 Verify、SSE 流式输出和独立验收。

## 录制前准备（不要录入视频）

在 PowerShell 中执行：

```powershell
cd D:\NJU-Agent\nju-coding-agent

# 在开始录屏前设置，画面中不要出现真实密钥。
$env:DEEPSEEK_API_KEY = "你的密钥"

# 使用新的临时数据目录，保证视频从空会话开始。
$env:CODING_AGENT_DATA_DIR = Join-Path $env:TEMP ("nju-agent-video-" + [Guid]::NewGuid())

# Windows PowerShell 5.1 需要显式使用 UTF-8，否则中文 UTF-8 无 BOM 文件会显示乱码。
chcp 65001 > $null
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$env:PYTHONIOENCODING = "utf-8"

# 提前编译，避免把编译等待录进视频。
cargo build --release

# 恢复带有两个缺陷的干净演示工程。
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\python_multifile\prepare.ps1
```

为了在视频中立即展示长期记忆和多会话，录制前先用同一个 `CODING_AGENT_DATA_DIR` 预置数据。运行：

```powershell
.\target\release\nju-coding-agent.exe --yes --workspace D:\NJU-Agent\nju-coding-agent\benchmarks\python_multifile\workspace
```

在 Agent Input 中依次输入：

```text
请记住：我偏好使用中文回答，修改代码后优先运行完整测试。
```

```text
/new
```

```text
这是预置会话 B，请只回复“会话 B 已建立”。
```

```text
/new
```

按 `Ctrl+D` 退出，再次重置故障工程：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\python_multifile\prepare.ps1
Clear-Host
```

这些预置步骤不录入视频。它们不会修改 benchmark 生产代码，只在 Git 忽略的本地数据目录中建立记忆和会话。

建议终端大小约为 150 列、40 行，系统显示缩放保持 100% 或 125%。确认桌面、终端标题和环境中没有显示 API key、个人文件或无关通知后再开始录屏。

## 视频中的完整操作

### 1. 先展示要完成的编程任务

开始录屏后，先在 PowerShell 显示业务规则：

```powershell
Get-Content -LiteralPath .\benchmarks\python_multifile\workspace\SPEC.md -Encoding UTF8
```

开场直接说：

> 我要演示的是一个真实的 Python 跨文件修复任务。这个订单结算模块中，每件商品包含单价、数量和折扣百分比。正确逻辑是先计算每行商品的折后金额，再汇总订单，最后统一保留两位小数。

接着运行初始完整测试：

```powershell
Push-Location .\benchmarks\python_multifile\workspace
python -m unittest discover -s tests -v
Pop-Location
```

应看到 5 个测试中 2 个失败。此时说：

> 当前五项测试有两项失败：单行商品小计二十五元、折扣百分之二十时，期望二十元，程序却返回二十五元；多件折扣商品期望总额二十三元，实际返回三十一元。接下来不手动定位，交给 Agent 自主读取规格、代码和测试完成修复。

不要手动修改任何文件。

### 2. 启动全屏 Agent

```powershell
.\target\release\nju-coding-agent.exe --workspace D:\NJU-Agent\nju-coding-agent\benchmarks\python_multifile\workspace
```

视频中故意不传 `--yes`，这样 Agent 第一次执行测试命令时会展示 Confirmation 安全弹窗。按 `Y` 允许；危险命令和 workspace 越界路径即使用 `--yes` 也不会被放行。

### 2.1 用户功能快闪（剪辑后约 8 秒）

进入 TUI 后快速展示：

1. 按 `F1`，展示快捷键和全部内置命令，按 `Esc` 关闭。
2. 输入 `/tools`，展示七个本地工具。
3. 输入 `/plan off`，再输入 `/plan on`，展示 Plan 可选但 Verify 约束不可关闭。
4. 输入 `/memory`，展示预置的 `USER.md` 中文和测试偏好。
5. 输入 `/sessions`，展示弹窗、`●` 当前会话、revision 和 Plan 状态；用 `Up/Down` 移动一次后按 `Esc` 关闭。

这一段用快切，每个界面保留 1–2 秒。旁白：

> 终端界面支持帮助、工具列表、可选 Plan、本地长期记忆和多会话恢复。会话和记忆都保存在 Git 忽略的本地数据目录，不进入源码仓库。

### 3. 输入这段任务

```text
这是一个订单折扣计算模块。每条商品包含单价、数量和折扣百分比；系统应该先计算每条商品的折后金额，再汇总订单，并在最终统一保留两位小数。

当前完整测试共 5 项，其中 2 项失败：单行商品小计为 25.00、折扣为 20% 时，期望折后金额为 20.00，实际返回 25.00；包含多条折扣商品的订单期望总额为 23.00，实际返回 31.00。

请阅读 SPEC.md、生产代码和现有测试，形成修复计划，定位跨文件根因并做最小修改。不要修改测试，不要删除现有输入校验，不要改变公开函数签名。完成后运行 python -m unittest discover -s tests -v，只有全部测试通过才结束。
```

按 Enter 发送。模型具体措辞和步骤数可能变化，但正确轨迹必须包含：

```text
DETECT  Python/unittest
PLAN    检查 SPEC、实现和测试
EXEC    先复现失败
EXEC    精确修改两个生产文件，revision 递增到 2
VERIFY  运行完整测试并得到 exit_code: 0
DONE    当前 revision 2 已验证
```

Agent 运行时可以连续讲解：

> 这个 Agent 使用 Rust 自主实现，没有使用 Agent 框架。它通过模型原生 tool calling 调用本地文件和命令工具。PLAN 阶段只允许读取、列表和搜索，不能改代码或运行命令；进入 EXECUTE 后才执行精确文本替换。每次成功写入都会增加 workspace revision。

> 右侧可以看到两次 replace_text 将 revision 递增到二。最后的 VERIFY 不信任模型文字中的“已测试”，只接受真实命令返回的零退出码。只有 last verified revision 等于当前 workspace revision，运行时才允许进入 DONE。

需要回看被顶出的工具调用时，把鼠标放在 Events 栏内滚动；也可使用 `Ctrl+Up/Down`、`Ctrl+PageUp/PageDown` 和 `Ctrl+Home/End`。回看时 Events 标题显示 `paused`，按 `Ctrl+End` 恢复跟随最新事件。

如果模型在检查代码后已经有充分证据，可能直接修复而不重复执行初始失败测试；这不影响最终验收。视频开头已经独立展示了稳定的失败基线。

### 4. 展示运行时状态

Agent 完成后输入：

```text
/status
```

重点展示 `workspace revision 2` 和已验证状态。随后输入：

```text
/context
```

用 3 至 5 秒展示真实 API token usage、校准后的上下文估算和图形用量条，然后关闭弹窗。按 `Ctrl+D` 退出 Agent。

退出前用 6–8 秒快速展示剩余交互：

1. 把鼠标放在 Events 上滚，标题变为 `paused`；按 `Ctrl+End` 恢复追尾。
2. 按两次 `Ctrl+L`，展示 Events 折叠和展开。
3. 输入 `/sessions`，用 `Up/Down` 选择预置会话并按 `Enter` 切换；再打开 `/sessions` 切回主任务会话。
4. 输入 `/clear`，展示“只清空界面，Agent 历史仍保留”的提示。

`/new`、`/switch <id>`、`/delete <id>` 已在 F1 帮助中展示；Sessions 弹窗中的 `N` 也可直接新建会话。视频不实际删除会话，避免为了演示引入不必要的破坏性操作。

### 5. 独立验收

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\python_multifile\verify_video_demo.ps1
```

最终应显示：

```text
Tests exit code: 0
Tests unchanged: True
order/models.py changed: True
order/service.py changed: True
VIDEO_DEMO_RESULT: PASS
```

验收时说：

> Agent 结束后，这个外部脚本会再运行一次完整测试，并通过 SHA-256 确认测试文件没有被修改、两个目标生产文件确实发生了变化。现在五项测试全部通过，任务完成。

### 6. 用一条命令展示核心实现

```powershell
Select-String -Path .\src\agent.rs `
  -Pattern 'pub enum AgentPhase','workspace_revision =','last_verified_revision = Some','last_verified_revision == Some','FinishBlocked' |
  Select-Object LineNumber, Line
```

最后说：

> 代码中的 AgentPhase 实现 PLAN、EXECUTE、VERIFY 和 DONE，workspace revision 将文件变化与验证证据关联起来。这是确定性运行时约束，而不只是 prompt 中的口头要求。

### 7. 简要展示三组 benchmark 覆盖

```powershell
Select-String -Path .\benchmarks\RESULTS.md `
  -Pattern 'checkout_discount|python_multifile|rust_timeout'
```

最后补充：

> 项目使用三个可复现 benchmark，分别覆盖 Python 单文件订单折扣、Python 跨文件订单计算和 Rust 后缀解析。三组任务初始都是五项测试中两项失败，修复后均为五项全部通过，并由外部脚本确认测试文件没有修改、目标生产文件确实变化，而且最新 workspace revision 已被真实测试验证。

## 在界面中确认 Verify invariant

右侧 Events 要保留这组证据：

```text
EXEC    workspace rev 1 · replace_text
EXEC    workspace rev 2 · replace_text
VERIFY  run_command · python -m unittest discover -s tests -v
RESULT  exit_code: 0
VERIFY  PASS rev 2
DONE    进入 DONE 阶段
```

再输入 `/status`，显示：

```text
阶段：DONE
版本：rev 2 / ✓
```

这两处合起来证明：最新写入得到 revision 2，真实命令以零退出码验证 revision 2，然后才进入 DONE。`Ctrl+End` 中的 `End` 是键盘导航区的 End 键；笔记本通常是 `Fn+Right`，因此可能需要按 `Ctrl+Fn+Right`。没有 End 键时，也可在 Events 区持续向下滚动到最新事件。

## 设计亮点完整版（答辩用）

1. **不依赖 Agent 框架**：使用 Rust 自主实现模型循环、消息历史、工具注册、输出解析、错误回填和终止条件，只使用 OpenAI-compatible 模型 API 和原生 tool calling。
2. **PEV + revision runtime invariant**：PLAN 只读，EXECUTE 修改，VERIFY 使用真实命令。写入递增 `workspace_revision`，只有 `last_verified_revision == workspace_revision` 才允许 DONE，不依赖模型自觉。
3. **严格工具与解析容错**：`replace_text` 只允许唯一匹配；未知工具、坏 JSON、缺参数、空 choices 和不完整流式 tool call 都有明确错误路径，工具错误会回填模型纠正。
4. **revision-aware 去重**：重复调用键包含工具名、参数和 workspace revision。文件修改后，相同读取或测试会在新 revision 上重新执行，不会误用旧结果。
5. **workspace 安全边界**：文件工具使用 canonicalize 和符号链接检查阻止越界，敏感文件默认隐藏；命令经过 Allow/Block/Confirm 策略。同时诚实说明命令不是 OS 级沙箱。
6. **确定性项目检测**：根据 `Cargo.toml`、Python 测试目录、`package.json`、Maven、Gradle、Go 和 .NET 标志选择验证命令；检测不到才由模型决定。
7. **真实 SSE 与 Token usage**：逐片聚合文本、`reasoning_content` 和 tool call JSON；解析 API 的真实 usage，用 prompt token 校准本地字符估算，与会话累计消耗分开显示。
8. **cheap-first 三阶段上下文压缩**：先结构化压缩旧的大型工具结果，再裁剪旧 reasoning 和低价值历史，仍然超限才调用 LLM 生成语义摘要，失败时有确定性本地摘要兜底。tool call 与 result 原子保留。
9. **分层上下文与长期记忆**：第一层是当前会话短期上下文；第二层是跨 workspace 的 `USER.md` 用户偏好；第三层是按 workspace 隔离的 `MEMORY.md` 项目事实和架构决策。持久记忆实际上是两个 Markdown 作用域，采用显式写入为主，并拒绝疑似密钥。
10. **会话持久化与 TUI**：每个会话保存消息、workspace、revision、验证状态、Plan 和 Token 校准；Sessions 弹窗可以上下选择恢复。TUI 还提供流式文本、事件栏、状态、上下文图和命令确认。
11. **外部可重复验收**：三组自建 benchmark 覆盖 Python 单文件、Python 跨文件和 Rust。Agent 结束后独立复测，并检查测试未改、指定生产文件已改、最新 revision 已验证。

## 设计亮点 30 秒视频版

> 这个 Agent 不依赖 Agent 框架，由 Rust 自主实现模型循环、工具和终止条件。核心是 PEV 与 revision invariant：每次写入递增版本，只有真实测试以零退出码验证最新版本才允许 DONE。上下文采用 cheap-first 三阶段压缩；记忆分为会话、用户偏好和项目规则。再结合 workspace 沙箱、SSE 流式输出和外部复测，提高编程任务完成的可靠性。

## 建议时间轴和解说词

| 时间 | 画面 | 可直接使用的解说词 |
|---|---|---|
| 0:00-0:10 | 显示 `SPEC.md` | 说明订单折扣模块和“逐行折扣后再汇总”规则。 |
| 0:10-0:20 | 执行初始测试 | 说明 5 项中 2 项失败，展示 25/20 和 31/23 差异。 |
| 0:20-0:30 | TUI 功能快闪 | F1、`/tools`、Plan on/off、`/memory`、`/sessions` 每项保留 1–2 秒。 |
| 0:30-0:38 | 粘贴任务 | 长文本自动换行，说明 workspace 沙箱和 Python/unittest 检测。 |
| 0:38-1:10 | PLAN、EXECUTE、VERIFY | 将模型等待加速 2–4 倍，保留流式文字、工具调用、Confirmation、两次 revision 和验证命令。 |
| 1:10-1:25 | DONE 与运行状态 | `/status`、`/context`、Events 滚动、`Ctrl+End`、`Ctrl+L` 折叠/展开。 |
| 1:25-1:37 | 会话与清屏快闪 | `/sessions` 上下选择并切换后切回，最后 `/clear`。 |
| 1:37-1:50 | 外部独立验收 | 展示 5/5、测试未改、两个生产文件已改和 `VIDEO_DEMO_RESULT: PASS`。 |
| 1:50-1:58 | `rg` 展示 Rust 核心状态 | 总结 PEV 与 revision 是 runtime invariant，不只是 prompt 提示。 |

模型等待时间可以在剪辑中加速或删除，但建议保留关键工具调用、两次 revision 变化、VERIFY 的 `exit_code: 0` 和最终独立 PASS。最终 MP4 必须少于 2 分钟、少于 200 MB。

## 失败时重录

不要在失败后的 workspace 上直接重试。先重新执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\python_multifile\prepare.ps1
```

然后使用新的会话重新启动 Agent。这样每次录制都从相同的 3/5 基线开始。
