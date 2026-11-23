import unittest

from dateparse import parse_date


class T(unittest.TestCase):
    def test_iso(self):
        self.assertEqual(parse_date("2026-11-22"), (2026, 11, 22))

    def test_fr_slash(self):
        self.assertEqual(parse_date("22/11/2026"), (2026, 11, 22))

    def test_fr_words(self):
        self.assertEqual(parse_date("22 Novembre 2026"), (2026, 11, 22))

    def test_en(self):
        self.assertEqual(parse_date("Nov 22, 2026"), (2026, 11, 22))

    def test_invalid(self):
        with self.assertRaises(ValueError):
            parse_date("2026-02-30")


if __name__ == "__main__":
    unittest.main()
