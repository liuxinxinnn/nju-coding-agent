# Real DeepSeek SSE smoke test

这个实测直接调用项目的 `HttpLanguageModel::complete_stream`，实时输出 content delta，并检查：

- 至少收到两个 content delta，从而排除普通请求单次回填。
- 所有 delta 拼接后与最终聚合 `Message.content` 完全一致。
- 中文正文不少于 80 个字符。
- 记录是否收到 `reasoning_content`。

运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\benchmarks\sse_stream\run.ps1
```

API key 只从环境变量读取，日志不会记录 key。
