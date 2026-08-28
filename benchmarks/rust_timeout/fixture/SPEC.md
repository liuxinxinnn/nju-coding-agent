# Timeout parser 规则

`parse_timeout(value)` 将短字符串解析为 `std::time::Duration`。

1. `<整数>ms` 表示毫秒。
2. `<整数>s` 表示秒。
3. 不支持其它单位。
4. 数字为空或不是无符号整数时返回 `Err("invalid timeout value")`。
5. 单位不受支持时返回 `Err("unsupported timeout unit")`。
