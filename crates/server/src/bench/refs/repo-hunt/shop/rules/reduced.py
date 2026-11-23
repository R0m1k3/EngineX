"""Catégories au taux réduit de TVA (article 278-0 bis du CGI, simplifié)."""

REDUCED = {"alimentation", "livres", "billetterie"}


def is_reduced(category):
    return category in REDUCED
