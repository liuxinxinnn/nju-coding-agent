"""Order domain models."""

from dataclasses import dataclass
from decimal import Decimal


@dataclass(frozen=True)
class OrderLine:
    name: str
    unit_price: Decimal
    quantity: int
    discount_percent: Decimal = Decimal("0")

    def __post_init__(self) -> None:
        if self.unit_price < 0:
            raise ValueError("unit_price cannot be negative")
        if self.quantity < 0:
            raise ValueError("quantity cannot be negative")
        if not Decimal("0") <= self.discount_percent <= Decimal("100"):
            raise ValueError("discount_percent must be between 0 and 100")

    def subtotal(self) -> Decimal:
        return self.unit_price * self.quantity

    def discounted_total(self) -> Decimal:
        # Defect: the declared line discount is ignored.
        return self.subtotal()
