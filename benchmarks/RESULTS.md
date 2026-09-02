# Benchmark summary

三组 benchmark 都会重建独立 workspace，在 Agent 结束后独立运行完整测试，并校验测试文件未被修改。`run_case.ps1` 还要求轨迹到达 `DONE`，且最新 `workspace_revision` 有真实 `exit_code: 0` 的验证证据。

## Baseline matrix

| Benchmark | Project | Scope | Initial result | Required production changes |
|---|---|---|---|---|
| `checkout_discount` | Python/unittest | 单文件逻辑缺陷 | 3/5 pass | `checkout.py` |
| `python_multifile` | Python/unittest | 跨文件折扣缺陷 | 3/5 pass | `order/models.py`, `order/service.py` |
| `rust_timeout` | Rust/cargo | 后缀解析缺陷 | 3/5 pass | `src/lib.rs` |

## Latest PEV real-model runs

| Benchmark | Result | Steps | Tool calls | Time | Revision | Tests unchanged | Required files changed |
|---|---:|---:|---:|---:|---:|---:|---:|
| `checkout_discount` | PASS 5/5 | 6 | 7 | 15.74s | 1 ✓ | Yes | Yes |
| `python_multifile` | PASS 5/5 | 8 | 10 | 24.98s | 2 ✓ | Yes | Yes (2 files) |
| `rust_timeout` | PASS 5/5 | 6 | 8 | 21.41s | 1 ✓ | Yes | Yes |

三次运行均使用 `deepseek-v4-flash`，日期为 2026-08-30，对应 Agent 提交 `28c3460`。详细根因、修改和验收信息见各目录的 `RESULTS.md`；原始轨迹及机器可读 JSON 被 Git 忽略，避免把完整模型对话当成提交材料。

## Real SSE smoke

真实 `HttpLanguageModel::complete_stream` 实测 PASS：94 个 content delta、167 个中文字符，delta 拼接与最终消息完全一致，并收到 `reasoning_content`。多次增量回调同时排除了普通请求 fallback。
