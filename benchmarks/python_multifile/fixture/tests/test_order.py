import unittest
from decimal import Decimal

from order import OrderLine, calculate_order_total


class OrderDiscountTests(unittest.TestCase):
    def test_subtotal_without_discount(self) -> None:
        line = OrderLine("book", Decimal("12.50"), 2)
        self.assertEqual(line.subtotal(), Decimal("25.00"))

    def test_line_discount_is_applied(self) -> None:
        line = OrderLine("book", Decimal("12.50"), 2, Decimal("20"))
        self.assertEqual(line.discounted_total(), Decimal("20.000"))

    def test_order_total_uses_each_lines_discounted_total(self) -> None:
        lines = [
            OrderLine("book", Decimal("12.50"), 2, Decimal("20")),
            OrderLine("pen", Decimal("2.00"), 3, Decimal("50")),
        ]
        self.assertEqual(calculate_order_total(lines), Decimal("23.00"))

    def test_rejects_discount_outside_percentage_range(self) -> None:
        with self.assertRaises(ValueError):
            OrderLine("book", Decimal("10"), 1, Decimal("101"))

    def test_rejects_negative_values(self) -> None:
        with self.assertRaises(ValueError):
            OrderLine("book", Decimal("-1"), 1)
        with self.assertRaises(ValueError):
            OrderLine("book", Decimal("1"), -1)


if __name__ == "__main__":
    unittest.main()
