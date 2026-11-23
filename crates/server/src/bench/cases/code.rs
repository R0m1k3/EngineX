//! Tests de code sans rendu : contrôlés à 100 % par la machine. Les scripts
//! `data/hidden/h_*.py` ne sont copiés qu'à l'audit ; leur dernière ligne
//! `BENCH_SCORE a/b` donne les points au prorata.

use super::{Case, Check, Family, Kind};
use crate::bench::journal;

const UNTOUCHED: Check = Check {
    name: "fichiers de test fournis non modifiés",
    points: 2,
    essential: false,
    kind: Kind::Log(journal::protected_untouched),
};

const RAN_TESTS: Check = Check {
    name: "a lancé les tests lui-même, dernier passage au vert",
    points: 2,
    essential: false,
    kind: Kind::Log(journal::ran_tests_green),
};

pub static BUGFIX: Case = Case {
    key: "bugfix",
    title: "Corriger un bug (panier)",
    family: Family::Code,
    summary: "les tests fournis et 10 tests cachés (quantités, remises, coupons, arrondis)",
    prompt: "Le fichier cart.py calcule le total d'un panier, mais les tests de test_cart.py échouent. Lis les deux fichiers, trouve toutes les causes et corrige cart.py \
pour qu'il respecte ce que disent ses docstrings (une remise est comprise entre 0 et 1, sinon ValueError). \
Ne modifie pas test_cart.py. Lance les tests pour le vérifier : python3 -m unittest -v",
    files: &[
        ("cart.py", include_str!("../data/cart.py")),
        ("test_cart.py", include_str!("../data/test_cart.py")),
    ],
    generated: None,
    protected: &["test_cart.py"],
    hidden: &[("h_cart.py", include_str!("../data/hidden/h_cart.py"))],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "les 6 tests fournis passent",
            points: 6,
            essential: true,
            kind: Kind::Shell("python3 -m unittest -q test_cart 2>&1 | tail -3 | grep -q '^OK'"),
        },
        Check {
            name: "10 tests cachés",
            points: 10,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_cart.py\""),
        },
        UNTOUCHED,
        RAN_TESTS,
    ],
    timeout_s: 900,
};

pub static DATES: Case = Case {
    key: "dates",
    title: "Lire des dates (avec ses tests)",
    family: Family::Code,
    summary: "les 4 formats demandés, 15 dates cachées valides, 10 textes cachés invalides, ses propres tests",
    prompt: "Écris dateparse.py avec une fonction parse_date(texte) qui renvoie un tuple (année, mois, jour). Formats à accepter : « 2026-11-22 », « 22/11/2026 », \
« 22 novembre 2026 » (mois français en toutes lettres, accents compris, majuscules ou non, jour sur 1 ou 2 chiffres), « Nov 22, 2026 » (mois anglais abrégé sur 3 lettres). \
Tout autre texte, ou une date impossible (30 février, 29 février d'une année non bissextile, mois 13…), lève ValueError. \
Écris aussi test_dateparse.py (unittest) avec au moins 5 tests, et fais-les passer : python3 -m unittest -v",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[("h_dates.py", include_str!("../data/hidden/h_dates.py"))],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "les 4 exemples de la consigne donnent (2026, 11, 22)",
            points: 4,
            essential: true,
            kind: Kind::Shell(
                "python3 -c \"from dateparse import parse_date as p\nfor s in ['2026-11-22','22/11/2026','22 novembre 2026','Nov 22, 2026']:\n    r = p(s)\n    assert tuple(r) == (2026, 11, 22), (s, r)\"",
            ),
        },
        Check {
            name: "15 dates cachées valides",
            points: 6,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_dates.py\" good"),
        },
        Check {
            name: "10 textes cachés invalides → ValueError",
            points: 5,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_dates.py\" bad"),
        },
        Check {
            name: "ses propres tests : 5 au moins, tous au vert",
            points: 3,
            essential: false,
            kind: Kind::Shell(
                "out=$(python3 -m unittest -v test_dateparse 2>&1); echo \"$out\" | tail -4; echo \"$out\" | tail -3 | grep -q '^OK' && [ $(echo \"$out\" | grep -c ' ok$') -ge 5 ]",
            ),
        },
        RAN_TESTS,
    ],
    timeout_s: 900,
};

pub static REST_API: Case = Case {
    key: "rest-api",
    title: "API REST",
    family: Family::Code,
    summary: "création, lecture, suppression, et 8 requêtes cachées (erreurs 400/404, JSON, accents)",
    prompt: "Écris server.py (bibliothèque standard Python uniquement) : une API JSON sur le port donné par la variable d'environnement PORT (8000 par défaut). \
GET /items → liste ; POST /items avec {\"name\": \"...\"} → crée (201) et renvoie l'objet avec son id ; GET /items/<id> → l'objet ou 404 ; DELETE /items/<id> → 204 ou 404. \
Un POST sans name ou au JSON invalide → 400. Toute autre route → 404. Les réponses sont en Content-Type application/json (UTF-8). Les données restent en mémoire. \
Lance-le en arrière-plan et vérifie chaque route avec curl.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[("h_api.py", include_str!("../data/hidden/h_api.py"))],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "POST /items → 201 avec id et name",
            points: 4,
            essential: true,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_api.py\" basic 8111"),
        },
        Check {
            name: "GET /items et GET /items/<id>",
            points: 3,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_api.py\" read 8112"),
        },
        Check {
            name: "404 sur un id inconnu, DELETE → 204 puis 404",
            points: 3,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_api.py\" delete 8113"),
        },
        Check {
            name: "8 requêtes cachées (400, 404, JSON, accents)",
            points: 8,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_api.py\" extra 8114"),
        },
        Check {
            name: "a vérifié avec curl",
            points: 2,
            essential: false,
            kind: Kind::Log(journal::used_curl),
        },
    ],
    timeout_s: 1200,
};

pub static REFACTOR: Case = Case {
    key: "refactor",
    title: "Refactorer sans rien casser",
    family: Family::Code,
    summary: "même texte que l'original sur 300 entrées tirées au hasard, fonctions de 25 lignes au plus",
    prompt: "report.py contient une seule grosse fonction, difficile à lire. Refactore-le en plusieurs fonctions courtes (aucune de plus de 25 lignes), avec des noms clairs, \
sans changer le résultat de build_report pour aucune entrée. test_report.py doit continuer à passer sans modification : python3 -m unittest -v",
    files: &[
        ("report.py", REPORT_PY),
        ("test_report.py", include_str!("../data/test_report.py")),
    ],
    generated: None,
    protected: &["test_report.py"],
    hidden: &[
        ("h_report.py", include_str!("../data/hidden/h_report.py")),
        ("report_orig.py", REPORT_PY),
    ],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "les tests fournis passent",
            points: 4,
            essential: true,
            kind: Kind::Shell("python3 -m unittest -q test_report 2>&1 | tail -3 | grep -q '^OK'"),
        },
        Check {
            name: "même résultat que l'original sur 300 entrées au hasard",
            points: 6,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_report.py\" | tail -6"),
        },
        Check {
            name: "aucune fonction de plus de 25 lignes",
            points: 4,
            essential: false,
            kind: Kind::Shell(
                "python3 - <<'PY'\nimport ast,sys\nt=ast.parse(open('report.py',encoding='utf-8').read())\nbad=[(f.name,f.end_lineno-f.lineno+1) for f in ast.walk(t) if isinstance(f,(ast.FunctionDef,ast.AsyncFunctionDef)) and f.end_lineno-f.lineno+1>25]\nprint(bad or 'toutes ≤ 25 lignes')\nsys.exit(1 if bad else 0)\nPY",
            ),
        },
        Check {
            name: "4 fonctions au moins, build_report gardée",
            points: 2,
            essential: false,
            kind: Kind::Shell(
                "python3 - <<'PY'\nimport ast,sys\nt=ast.parse(open('report.py',encoding='utf-8').read())\nn=[f.name for f in ast.walk(t) if isinstance(f,ast.FunctionDef)]\nprint(n)\nsys.exit(0 if len(n)>=4 and 'build_report' in n else 1)\nPY",
            ),
        },
        UNTOUCHED,
        RAN_TESTS,
    ],
    timeout_s: 900,
};

pub static MULTI_FILE: Case = Case {
    key: "multi-file",
    title: "Paquet multi-fichiers d'après une spécification",
    family: Family::Code,
    summary: "structure du paquet, tests fournis, 8 tests cachés de la bibliothèque et de la ligne de commande, README",
    prompt: "Lis SPEC.md et réalise ce qu'elle demande. test_notes.py est fourni : il doit passer sans modification (python3 -m unittest -v).",
    files: &[
        ("SPEC.md", include_str!("../data/notes_spec.md")),
        ("test_notes.py", include_str!("../data/test_notes.py")),
    ],
    generated: None,
    protected: &["test_notes.py"],
    hidden: &[("h_notes.py", include_str!("../data/hidden/h_notes.py"))],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "paquet notes/ : __init__, store, cli, __main__",
            points: 2,
            essential: true,
            kind: Kind::Shell(
                "test -f notes/__init__.py && for f in store cli __main__; do test -s notes/$f.py || { echo \"notes/$f.py absent\"; exit 1; }; done",
            ),
        },
        Check {
            name: "les tests fournis passent",
            points: 5,
            essential: true,
            kind: Kind::Shell("python3 -m unittest -q test_notes 2>&1 | tail -3 | grep -q '^OK'"),
        },
        Check {
            name: "8 tests cachés (ids, fichier absent, erreurs, CLI)",
            points: 7,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_notes.py\""),
        },
        Check {
            name: "README.md avec un exemple d'usage",
            points: 2,
            essential: false,
            kind: Kind::Shell("test -s README.md && grep -q 'python3 -m notes' README.md"),
        },
        UNTOUCHED,
        RAN_TESTS,
    ],
    timeout_s: 1200,
};

const REPORT_PY: &str = include_str!("../data/report.py");
