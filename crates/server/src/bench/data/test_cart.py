import unittest

from cart import apply_coupon, total


class CartTest(unittest.TestCase):
    def test_total_quantities(self):
        items = [{"price": 2.5, "qty": 2}, {"price": 1.0, "qty": 3}]
        self.assertEqual(total(items), 8.0)

    def test_total_discount(self):
        items = [{"price": 2.5, "qty": 2}, {"price": 1.0, "qty": 3}]
        self.assertEqual(total(items, 0.1), 7.2)

    def test_empty(self):
        self.assertEqual(total([]), 0)

    def test_coupon(self):
        self.assertEqual(apply_coupon(100, "SAVE10"), 90)

    def test_unknown_coupon(self):
        self.assertEqual(apply_coupon(100, "NOPE"), 100)

    def test_bad_discount(self):
        with self.assertRaises(ValueError):
            total([{"price": 1, "qty": 1}], 1.5)


if __name__ == "__main__":
    unittest.main()
