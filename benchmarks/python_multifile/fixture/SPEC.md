# 订单折扣规则

1. `OrderLine` 使用 `Decimal` 表示单价和折扣百分比。
2. `subtotal()` 返回单价乘数量。
3. `discounted_total()` 只对该行商品小计应用百分比折扣。
4. `calculate_order_total()` 必须汇总每一行的折后金额，最终统一保留两位小数。
5. 单价、数量不能为负数；折扣必须在 0 到 100 之间（含边界）。
