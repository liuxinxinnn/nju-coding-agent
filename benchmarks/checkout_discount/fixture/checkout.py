"""Shopping-cart checkout calculations."""

from decimal import Decimal, ROUND_HALF_UP


CENT = Decimal("0.01")


def _money(value: Decimal) -> Decimal:
    return value.quantize(CENT, rounding=ROUND_HALF_UP)


def calculate_total(
    items: list[tuple[Decimal, int]],
    shipping_fee: Decimal,
    discount_percent: Decimal,
) -> Decimal:
    """Calculate the final charge for an order."""
    if not Decimal("0") <= discount_percent <= Decimal("100"):
        raise ValueError("discount_percent must be between 0 and 100")
    if shipping_fee < 0:
        raise ValueError("shipping_fee cannot be negative")
    if any(price < 0 or quantity < 0 for price, quantity in items):
        raise ValueError("item price and quantity cannot be negative")

    subtotal = sum(
        (price * quantity for price, quantity in items),
        start=Decimal("0"),
    )
    discount_factor = (Decimal("100") - discount_percent) / Decimal("100")

    return _money((subtotal + shipping_fee) * discount_factor)
