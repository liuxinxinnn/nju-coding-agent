# Benchmark results

## Baseline

- 环境：Windows，Rust stable
- 命令：`cargo test --offline`
- 预期结果：共 5 个测试，3 个通过，2 个失败
- 成功约束：`src/lib.rs` 必须修改，测试文件必须保持不变

## PEV real-model run

日期：2026-08-28。模型：`deepseek-v4-flash`。

| 指标 | 结果 |
|---|---|
| Agent / 最终测试 | PASS / 5/5 通过 |
| 项目检测 | Rust / `Cargo.toml` |
| 阶段序列 | `PLAN → EXEC → VERIFY → DONE` |
| Agent 步数 / 工具调用 | 6 / 8 |
| Workspace revision | 1，当前 revision 验证通过 |
| 耗时 | 17.51 秒 |
| 测试文件 | SHA-256 一致，未修改 |
| 要求修改的生产文件 | `src/lib.rs` 已修改 |

结论：Agent 自动选择 Rust 验证路径，最小重排 `ms` 与 `s` 后缀分支，并在 `cargo test --offline` 通过后进入 DONE。
