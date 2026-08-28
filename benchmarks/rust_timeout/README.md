# Rust timeout parser benchmark

这个 benchmark 验证 Agent 能否自动检测 Rust 项目、定位字符串后缀解析缺陷，并使用 `cargo test` 完成验证。

- `src/lib.rs` 将 `s` 后缀放在 `ms` 前判断，导致毫秒值被错误拆分。
- 初始 5 个测试中 2 个失败。
- 成功要求是只修复生产代码、测试文件保持不变、5/5 测试通过，并且最新版 PEV 到达已验证的 `DONE`。

运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\rust_timeout\run_agent.ps1
```
