# Checkout discount bug benchmark

这是一个可重复运行的端到端修复任务，用于验证 Coding Agent 能否完成“理解需求→定位根因→最小修复→运行测试”。

## 故障设计

- 语言：Python 3，只使用标准库。
- 需求来源：`fixture/SPEC.md`。
- 故障：折扣错误地作用到运费。
- 初始基线：5 个测试中有 2 个失败，由同一个缺陷引起。
- 成功标准：Agent 不修改测试，保留输入校验，且 5 个测试全部通过。

## 运行

在仓库根目录中设置新生成的 DeepSeek 密钥，然后执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\checkout_discount\run_agent.ps1
```

脚本每次都会用 `fixture` 覆盖故障工程的核心文件，在被 Git 忽略的 `workspace` 中运行 Agent，最后独立执行完整测试并校验测试文件未被修改。脚本会输出 Agent 步数、工具调用数和耗时；执行轨迹保存到被 Git 忽略的 `last-run.log`。

只验证初始失败基线：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\checkout_discount\prepare.ps1
Push-Location .\benchmarks\checkout_discount\workspace
python -m unittest discover -s tests -v
Pop-Location
```
