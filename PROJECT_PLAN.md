# 项目整体计划

## 对原 AI 计划的审阅结论

原计划选择 Rust、复用 Tool/ReAct/沙箱等设计思想、暂缓 Semgrep/NVD/长期记忆/多 Agent，这些判断合理。需要修正的地方有：

1. 最终仓库必须是题目发布后新建的公开仓库，不能直接把旧 SecAudit 仓库改名提交，也不应继承旧历史。
2. 旧 SecAudit 是用户确认可直接复用的自有项目。通用且成熟的 Tool、沙箱、命令策略、上下文预算和压缩实现可以直接迁移并适配；Semgrep、NVD、CWE、审计工作流等业务代码不迁移。
3. 第一版拆成多个 crate 会增加接口、构建和讲解成本。先用单 crate 的模块化结构，出现真实复用需求后再拆 crate。
4. `edit_file` 不能只写一个模糊接口。第一版采用“精确字符串替换”，要求旧文本唯一匹配，避免行号漂移和模型误改；后续再考虑 unified diff。
5. 安全与可靠性不能推迟到第三天。路径沙箱、敏感文件保护、命令策略、超时、输出截断和终止条件从第一版就进入主链路。
6. 上下文压缩不是最小闭环的前置条件。先设置历史预算和明确报错，闭环稳定后再实现可测试的压缩策略。
7. 应从第一天建立 mock LLM 测试，使 Agent 循环测试不依赖费用、网络和模型随机性。

## 架构

```text
CLI
 └─ Agent（历史、步数、重复调用检测）
     ├─ LanguageModel
     │   └─ OpenAI-compatible HTTP client
     └─ ToolRegistry
         ├─ read_file / list_files / search_text
         ├─ write_file / replace_text
         ├─ memory（USER.md / MEMORY.md）
         └─ run_command
             └─ sandbox + policy + timeout + truncation
```

## 里程碑

### M1：最小可靠闭环

- 配置和 CLI；支持一次性任务与多轮交互。
- 手写 Chat Completions 请求/响应类型和 tool calling 解析。
- 实现七个本地工具（六个工作区工具 + 受控长期记忆）和统一工具注册表。
- 实现 assistant tool call -> 本地执行 -> tool observation -> 下一轮模型调用。
- 加入最大步数、空响应、未知工具、坏 JSON 参数、重复调用等终止/纠错路径。
- 单元测试覆盖沙箱、编辑、命令策略和 Agent 循环。

验收任务：在临时示例工程中创建文件、运行程序；修复一个已有 bug，并运行测试验证。

### M2：上下文与体验

- [已完成] 基于字符估算的上下文预算，中文按更保守的每字符一 token 计算；用 API 真实 prompt usage 平滑校准后续估算。
- [已完成] 超过 80% 自动执行 cheap-first 三阶段压缩：结构化 Tool Result Compaction、原子安全的 History Pruning、Semantic Summary/本地兜底；保留 system prompt、最近完整用户轮次和工具调用链。
- [已完成] 会话本地持久化以及 `/new`、`/sessions`、`/switch`、`/delete`，切换时恢复消息和 PEV revision 状态。
- [已完成] OpenAI-compatible SSE 流式文本输出，聚合 tool call 分片并在 TUI 中实时更新、结束时去重。
- [已完成] 使用真实 DeepSeek SSE 验证多段 content delta、最终聚合一致性和 `reasoning_content`。
- [已完成] `/context` 图形化弹窗，展示上下文校准估算、网格、system/tools/messages/free 分项以及真实 API 最近/累计 token。
- [已完成] `MemoryProvider` 扩展边界与本地 Markdown 长期记忆：全局 USER.md、项目 MEMORY.md、受控 memory 工具和 `/memory`。
- 可配置 allow/block 命令规则。

### M3：特色功能

- [已完成] Plan -> Execute -> Verify 状态机：PLAN 只读、修改递增 workspace revision、明确验证命令成功后才允许 DONE。
- [已完成] Plan 会话级开关与 `--no-plan`；关闭时直接 EXECUTE，但 revision 验证约束保持强制。
- [已完成] 根据项目类型选择 test/lint/build 验证命令，检测结果注入 PEV，执行仍经过命令策略。
- 失败后把验证结果送回模型，允许有限次数修复。

### M4：提交材料

- [已完成] 第一个可重置 checkout discount bug benchmark：基线 5 项测试中 2 项失败；真实模型用 8 步、24.78 秒完成一行最小修复，最终 5/5 通过。
- [已完成] 3 个固定 benchmark：Python 单文件、Python 跨文件、Rust；记录结果、步数、工具调用、耗时、revision 和文件完整性。
- 选择稳定的跨文件真实任务录制 2 分钟内视频。
- 编写 1000 汉字内 `README.txt`，包含公开仓库地址、运行方式和特色功能。
- 检查凭据、`.env`、Git 历史、zip 内容、视频格式和大小。

## 时间安排（截止 2026-09-02 24:00 北京时间）

- 08-27：完成 M1 主链路与首批测试。
- 08-28：完成真实模型冒烟与 bug-fix 闭环。
- 08-29：补齐异常路径、Windows/Linux 命令兼容和安全测试。
- 08-30：完成 M2。
- 08-31：完成 M3；若 M1/M2 尚不稳定则取消特色功能。
- 09-01：benchmark、README.txt 初稿、视频脚本与录制。
- 09-02：只做材料核对、必要修复和最终提交，不新增大功能。

## 提交红线

- 不使用 LangChain、LlamaIndex、OpenAI Agents SDK 等 Agent 框架。
- 不使用服务端托管的代码执行或文件工具。
- 不提交 API key、`.env`、个人目录或真实项目敏感内容。
- 不改写已推送历史，截止时间后不再 push。
