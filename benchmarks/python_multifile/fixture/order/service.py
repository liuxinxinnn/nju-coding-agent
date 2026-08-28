"""Order calculation service."""

from decimal import Decimal, ROUND_HALF_UP
from typing import Iterable

from .models import OrderLine


def calculate_order_total(lines: Iterable[OrderLine]) -> Decimal:
    # Defect: aggregation bypasses OrderLine.discounted_total().
    total = sum((line.subtotal() for line in lines), Decimal("0"))
    return total.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)
