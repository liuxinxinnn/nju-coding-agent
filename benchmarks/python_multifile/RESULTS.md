# Benchmark results

## Baseline

- 环境：Windows，Python 3.13.9
- 命令：`python -m unittest discover -s tests -v`
- 预期结果：共 5 个测试，3 个通过，2 个失败
- 成功约束：`order/models.py` 与 `order/service.py` 都必须修改，测试文件必须保持不变

## Latest PEV real-model rerun

日期：2026-08-30。模型：`deepseek-v4-flash`。Agent 提交：`28c3460`。

| 指标 | 结果 |
|---|---|
| Agent / 最终测试 | PASS / 5/5 通过 |
| 阶段序列 | `PLAN → EXEC → VERIFY → DONE` |
| Agent 步数 / 工具调用 | 8 / 10 |
| Workspace revision | 2，当前 revision 验证通过 |
| 耗时 | 24.98 秒 |
| 测试文件 | SHA-256 一致，未修改 |
| 要求修改的生产文件 | `order/models.py`、`order/service.py` 均已修改 |

结论：Agent 正确识别两个相互独立的生产代码缺陷，完成两个精确替换，并在 revision 2 上运行完整测试后进入 DONE。
