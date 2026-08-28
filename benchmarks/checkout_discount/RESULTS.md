# Benchmark results

## Baseline

- 日期：2026-08-27
- 环境：Windows，Python 3.13.9
- 命令：`python -m unittest discover -s tests -v`
- 结果：共 5 个测试，3 个通过，2 个失败
- 失败表现：应付 `90.00` 被计算为 `88.00`；应付 `0.15` 被计算为 `0.14`
- 基线结论：故障稳定可复现

## Pre-PEV real-model run

日期：2026-08-27。模型：`deepseek-v4-flash`。任务由 `run_agent.ps1` 重置后执行，并在 Agent 结束后独立运行完整测试。

| 指标 | 结果 |
|---|---|
| Agent 退出码 | 0 |
| 最终测试 | 5/5 通过，退出码 0 |
| Agent 步数 | 8 |
| 工具执行数 | 9，另有 1 次重复请求被跳过 |
| 耗时 | 24.78 秒 |
| 是否修改测试 | 否，SHA-256 校验一致 |
| 修复摘要 | 将折扣仅应用于商品小计，再加上不参与折扣的运费；仅修改一行 |

结论：第一个真实模型端到端 bug-fix benchmark 通过。执行轨迹同时暴露了两个 Agent 基础设施问题：Windows 子 PowerShell 的 UTF-8 输出、文件修改后的旧命令结果缓存。两者已在 benchmark 后修复，其中缓存行为已加入自动化回归测试。

## PEV real-model rerun

日期：2026-08-28。模型：`deepseek-v4-flash`。使用当前 revision-aware `PLAN → EXECUTE → VERIFY → DONE` 实现重跑。

| 指标 | 结果 |
|---|---|
| Agent / 最终测试 | PASS / 5/5 通过 |
| 阶段序列 | `PLAN → EXEC → VERIFY → DONE` |
| Agent 步数 / 工具调用 | 6 / 7 |
| Workspace revision | 1，当前 revision 验证通过 |
| 耗时 | 19.25 秒 |
| 测试文件 | SHA-256 一致，未修改 |
| 要求修改的生产文件 | `checkout.py` 已修改 |

结论：最新版 runtime 在修改后进入 VERIFY，只有真实测试返回退出码 0 才允许进入 DONE；独立验收再次确认 5/5 通过。
