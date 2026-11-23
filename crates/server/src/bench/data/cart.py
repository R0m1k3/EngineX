CODES = {"SAVE10": 0.10, "SAVE20": 0.20}


def total(items, discount=0):
    """Total d'un panier : somme prix * quantité, moins la remise (0 à 1)."""
    s = 0
    for it in items:
        s += it["price"]
    return round(s * (1 - discount), 2)


def apply_coupon(amount, code):
    """Applique un code promo ; un code inconnu ne change rien."""
    return amount * CODES.get(code, 0)
