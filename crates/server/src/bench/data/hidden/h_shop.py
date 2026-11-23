from benchh import case, run

from shop.invoice import invoice
from shop.taxes import rate_for


@case("BD (catégorie « Livres ») au taux réduit")
def _():
    assert invoice([("BD", 1)])["tva"] == 0.77, invoice([("BD", 1)])

@case("billetterie au taux réduit")
def _():
    assert invoice([("CONCERT", 1)])["tva"] == 1.65

@case("maison au taux normal")
def _():
    assert invoice([("TAPIS", 1)])["tva"] == 15.0

@case("facture mélangée")
def _():
    got = invoice([("ROMAN", 1), ("LAMPE", 2), ("PAIN", 5)])
    assert got == {"ht": 99.8, "tva": 17.06, "ttc": 116.86, "currency": "EUR"}, got

@case("rate_for(' LIVRES ')")
def _():
    assert rate_for(" LIVRES ") == 0.055

@case("rate_for('maison')")
def _():
    assert rate_for("maison") == 0.20


run()
