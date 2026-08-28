# DeepSeek SSE result

日期：2026-08-28。模型：`deepseek-v4-flash`。直接运行项目的 `HttpLanguageModel::complete_stream`，不是单独实现的测试 HTTP 客户端。

| 指标 | 结果 |
|---|---|
| 退出码 | 0 |
| Content delta events | 94 |
| 最终中文字符数 | 167 |
| Delta 拼接与最终消息 | 完全一致 |
| `reasoning_content` | 已收到 |
| 非流式 fallback | 已排除；fallback 只会产生一次 content 回调 |

结论：真实 DeepSeek SSE 内容被逐段接收并正确聚合，最终消息无缺字、无重复。
