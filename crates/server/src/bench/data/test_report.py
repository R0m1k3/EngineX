import unittest

from report import build_report

LINES = [
    "# ventes",
    "stylo;papeterie;2,50",
    "cahier;Papeterie;4",
    "lampe;maison;12.9",
    "",
    "mauvaise ligne",
    "gomme;papeterie;-1",
    "plante;maison;abc",
    "tapis;maison;30",
]

EXPECTED = """MAISON
  tapis           30.00
  lampe           12.90
  total           42.90

PAPETERIE
  cahier           4.00
  stylo            2.50
  total            6.50

TOTAL GENERAL 49.40
Plus grosse ligne : tapis (30.00)"""


class ReportTest(unittest.TestCase):
    def test_full(self):
        self.assertEqual(build_report(LINES), EXPECTED)

    def test_empty(self):
        self.assertEqual(build_report([]), "TOTAL GENERAL 0.00\nAucune ligne valide")


if __name__ == "__main__":
    unittest.main()
