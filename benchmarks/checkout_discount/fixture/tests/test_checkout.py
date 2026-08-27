import unittest
from decimal import Decimal

from checkout import calculate_total


class CalculateTotalTests(unittest.TestCase):
    def test_total_without_discount(self):
        total = calculate_total(
            [(Decimal("12.50"), 2)],
            Decimal("5.00"),
            Decimal("0"),
        )
        self.assertEqual(total, Decimal("30.00"))

    def test_discount_applies_to_merchandise_but_not_shipping(self):
        total = calculate_total(
            [(Decimal("40.00"), 2), (Decimal("20.00"), 1)],
            Decimal("10.00"),
            Decimal("20"),
        )
        self.assertEqual(total, Decimal("90.00"))

    def test_rounds_only_the_final_total(self):
        total = calculate_total(
            [(Decimal("0.05"), 3)],
            Decimal("0.01"),
            Decimal("10"),
        )
        self.assertEqual(total, Decimal("0.15"))

    def test_rejects_discount_outside_percentage_range(self):
        with self.assertRaises(ValueError):
            calculate_total([], Decimal("0"), Decimal("-1"))
        with self.assertRaises(ValueError):
            calculate_total([], Decimal("0"), Decimal("101"))

    def test_rejects_negative_order_values(self):
        with self.assertRaises(ValueError):
            calculate_total([], Decimal("-0.01"), Decimal("0"))
        with self.assertRaises(ValueError):
            calculate_total([(Decimal("-1"), 1)], Decimal("0"), Decimal("0"))
        with self.assertRaises(ValueError):
            calculate_total([(Decimal("1"), -1)], Decimal("0"), Decimal("0"))


if __name__ == "__main__":
    unittest.main()
