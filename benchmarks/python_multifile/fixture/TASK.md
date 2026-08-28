# 任务

订单折扣功能的两个测试失败了。请根据 `SPEC.md` 和现有测试定位跨文件根因，修复生产代码，并运行完整测试确认结果。

约束：

- 不要修改测试文件。
- `order/models.py` 和 `order/service.py` 中各有一个相关缺陷，两个生产文件都需要修复。
- 保留现有输入校验和公开函数签名。
- 尽量做最小修改。
- 验证命令为 `python -m unittest discover -s tests -v`。
