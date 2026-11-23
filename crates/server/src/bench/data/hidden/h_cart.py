from benchh import case, raises, run

from cart import apply_coupon, total

I = [{"price": 2.5, "qty": 2}, {"price": 1.0, "qty": 3}]


@case("quantité 3 et remise 0,5")
def _():
    assert total([{"price": 10, "qty": 3}], 0.5) == 15.0, total([{"price": 10, "qty": 3}], 0.5)

@case("arrondi : 0,1 × 3 = 0,3")
def _():
    assert total([{"price": 0.1, "qty": 3}]) == 0.3, total([{"price": 0.1, "qty": 3}])

@case("quantité 0")
def _():
    assert total([{"price": 5, "qty": 0}]) == 0

@case("remise 0 explicite")
def _():
    assert total(I, 0) == 8.0

@case("remise 1 (100 %)")
def _():
    assert total(I, 1) == 0

@case("remise négative -> ValueError")
def _():
    raises(ValueError, total, I, -0.1)

@case("SAVE20 sur 100 -> 80")
def _():
    assert apply_coupon(100, "SAVE20") == 80, apply_coupon(100, "SAVE20")

@case("SAVE10 sur 50 -> 45")
def _():
    assert apply_coupon(50, "SAVE10") == 45, apply_coupon(50, "SAVE10")

@case("coupon sur 0")
def _():
    assert apply_coupon(0, "SAVE10") == 0

@case("deux articles, quantités 5 et 2")
def _():
    assert total([{"price": 2, "qty": 5}, {"price": 3.5, "qty": 2}]) == 17.0


run()
