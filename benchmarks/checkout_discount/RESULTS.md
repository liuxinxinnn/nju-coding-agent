# Benchmark results

## Baseline

- 日期：2026-08-27
- 环境：Windows，Python 3.13.9
- 命令：`python -m unittest discover -s tests -v`
- 结果：共 5 个测试，3 个通过，2 个失败
- 失败表现：应付 `90.00` 被计算为 `88.00`；应付 `0.15` 被计算为 `0.14`
- 基线结论：故障稳定可复现

## Real-model agent run

等待使用新生成的 `DEEPSEEK_API_KEY` 执行 `run_agent.ps1` 后填写：

| 指标 | 结果 |
|---|---|
| Agent 退出码 | 待测 |
| 最终测试 | 待测 |
| Agent 步数 | 待测 |
| 工具调用数 | 待测 |
| 耗时 | 待测 |
| 是否修改测试 | 待测 |
| 修复摘要 | 待测 |
