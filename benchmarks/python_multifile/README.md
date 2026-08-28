# Python multi-file discount benchmark

这个 benchmark 验证 Agent 能否完成跨文件修复，而不是只修改一个表达式。

- `order/models.py` 中单行折后金额计算忽略了折扣。
- `order/service.py` 中订单汇总绕过了模型的折后金额方法。
- 初始 5 个测试中 2 个失败。
- 成功要求是两个生产文件都发生修改、测试文件保持不变、5/5 测试通过，并且最新版 PEV 到达已验证的 `DONE`。

运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\python_multifile\run_agent.ps1
```
