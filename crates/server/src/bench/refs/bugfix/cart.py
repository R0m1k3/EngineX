CODES = {"SAVE10": 0.10, "SAVE20": 0.20}


def total(items, discount=0):
    """Total d'un panier : somme prix * quantité, moins la remise (0 à 1)."""
    if not 0 <= discount <= 1:
        raise ValueError("remise hors de [0, 1]")
    s = sum(it["price"] * it["qty"] for it in items)
    return round(s * (1 - discount), 2)


def apply_coupon(amount, code):
    """Applique un code promo ; un code inconnu ne change rien."""
    return amount * (1 - CODES.get(code, 0))
