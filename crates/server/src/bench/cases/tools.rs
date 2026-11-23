//! Tests d'outils : le résultat ET la façon d'y arriver (journal de la session).

use super::{Case, Check, Family, Kind};
use crate::bench::journal;

pub static BIG_LOG: Case = Case {
    key: "big-log",
    title: "Analyser un gros journal (50 000 lignes)",
    family: Family::Tools,
    summary: "report.json exact (total, erreurs 5xx, top 5 des IP, p95 par route), script rejouable, fichier jamais affiché en entier",
    prompt: "access.log est le journal d'un serveur web (50 000 lignes, trop gros pour être lu en entier : travaille avec des outils et un script). \
Format d'une ligne valide : IP - - [date] \"MÉTHODE chemin HTTP/1.1\" statut octets durée « ms », par exemple : \
10.0.3.17 - - [23/Nov/2026:10:00:00 +0000] \"GET /api/search?q=lampe HTTP/1.1\" 200 5123 87ms . Certaines lignes sont incomplètes ou corrompues : ignore-les. \
Écris analyze.py (bibliothèque standard) qui produit report.json avec : total_requests (lignes valides), errors_5xx (statuts 500 à 599), \
top_ips_5xx (les 5 IP qui ont le plus d'erreurs 5xx : liste de {\"ip\", \"count\"}, du plus grand au plus petit, à égalité par IP croissante), \
p95_ms (objet : pour chaque route, c'est-à-dire le chemin sans la partie ?requête, le 95e centile des durées par la méthode du rang le plus proche : \
durées triées, valeur au rang ceil(0,95 × n), rangs comptés à partir de 1). Lance-le et vérifie le résultat.",
    files: &[],
    generated: Some(gen_log),
    protected: &["access.log"],
    hidden: &[("h_log.py", include_str!("../data/hidden/h_log.py"))],
    setup: None,
    page: None,
    checks: &[
        Check {
            name: "report.json lisible avec les 4 clés",
            points: 2,
            essential: true,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_log.py\" keys"),
        },
        Check {
            name: "total_requests exact",
            points: 3,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_log.py\" total"),
        },
        Check {
            name: "errors_5xx exact",
            points: 3,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_log.py\" errors"),
        },
        Check {
            name: "top 5 des IP en erreur, dans l'ordre",
            points: 4,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_log.py\" top"),
        },
        Check {
            name: "p95 exact pour chaque route",
            points: 4,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_log.py\" p95"),
        },
        Check {
            name: "analyze.py rejoué redonne le même report.json",
            points: 2,
            essential: false,
            kind: Kind::Shell(
                "test -s analyze.py && cp report.json /tmp/bench-report.json && rm -f report.json && timeout 120 python3 analyze.py >/dev/null && python3 -c \"import json;a=json.load(open('/tmp/bench-report.json'));b=json.load(open('report.json'));assert a==b,'résultat différent'\"",
            ),
        },
        Check {
            name: "n'a jamais affiché le journal en entier",
            points: 2,
            essential: false,
            kind: Kind::Log(journal::no_full_dump_of_log),
        },
    ],
    timeout_s: 1200,
};

pub static BROWSER_BUG: Case = Case {
    key: "browser-bug",
    title: "Reproduire et corriger un bug au navigateur",
    family: Family::Tools,
    summary: "le panier marche (compteur 5, total 199,50 €), navigateur utilisé pour reproduire puis revérifier après la correction",
    prompt: "Michael signale : « Sur la page produit (site/index.html), le bouton Ajouter au panier ne fait rien. » \
Lance le site (par exemple python3 -m http.server 8000 dans site/), ouvre-le avec le navigateur (mcp__enginex__browser_open), reproduis le problème, \
trouve les causes et corrige-les. Puis revérifie dans le navigateur : ajoute 2 articles, puis 3 : le compteur doit afficher 5 et le total 199,50 €. \
Écris dans RAPPORT.md, en quelques phrases, chaque cause trouvée et sa correction.",
    files: &[
        ("site/index.html", include_str!("../data/site/index.html")),
        ("site/app.js", include_str!("../data/site/app.js")),
        ("site/style.css", include_str!("../data/site/style.css")),
    ],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("site/index.html"),
    checks: &[
        Check {
            name: "2 puis 3 articles : le compteur affiche 5",
            points: 4,
            essential: true,
            kind: Kind::Page {
                path: "site/index.html",
                phone: false,
                steps: &[
                    r#"await type('#qty', '2'); await click('#add-to-cart'); await type('#qty', '3'); await click('#add-to-cart');
ok(txt('#cart-count') === '5', '#cart-count : ' + txt('#cart-count'));"#,
                ],
            },
        },
        Check {
            name: "total 199,50 €",
            points: 3,
            essential: false,
            kind: Kind::Page {
                path: "site/index.html",
                phone: false,
                steps: &[
                    r#"await type('#qty', '2'); await click('#add-to-cart'); await type('#qty', '3'); await click('#add-to-cart');
ok(near(num(txt('#cart-total')), 199.5), '#cart-total : ' + txt('#cart-total'));"#,
                ],
            },
        },
        Check {
            name: "aucune erreur dans la console",
            points: 2,
            essential: false,
            kind: Kind::ConsoleClean,
        },
        Check {
            name: "a ouvert la page avec le navigateur",
            points: 3,
            essential: true,
            kind: Kind::Log(journal::opened_browser),
        },
        Check {
            name: "a cliqué ou saisi dans la page (reproduction)",
            points: 2,
            essential: false,
            kind: Kind::Log(journal::interacted_in_browser),
        },
        Check {
            name: "a revérifié au navigateur après sa dernière modification",
            points: 3,
            essential: false,
            kind: Kind::Log(journal::browser_after_last_write),
        },
        Check {
            name: "RAPPORT.md : les deux causes (sélecteur, quantité lue comme texte)",
            points: 3,
            essential: false,
            kind: Kind::Shell(
                "test -s RAPPORT.md || { echo 'RAPPORT.md absent'; exit 1; }; r=0; grep -qiE 'add-cart|add-to-cart|s[ée]lecteur|selector|querySelector|identifiant|\\bid\\b' RAPPORT.md || { echo 'cause 1 (sélecteur) non citée'; r=1; }; grep -qiE 'parse|Number\\(|nombre|cha[iî]ne|string|texte|concat' RAPPORT.md || { echo 'cause 2 (quantité en texte) non citée'; r=1; }; exit $r",
            ),
        },
    ],
    timeout_s: 1200,
};

pub static REPO_HUNT: Case = Case {
    key: "repo-hunt",
    title: "Enquête dans un dépôt de 40 fichiers",
    family: Family::Tools,
    summary: "bug de TVA trouvé et corrigé, tests cachés, commit « fix: », recherche par outils, tests relancés après la correction",
    prompt: "Ce dossier est un dépôt git : une petite boutique en Python (paquet shop/, une quarantaine de fichiers). \
Le service compta signale que la TVA des factures est fausse pour certains produits : tests/test_invoice.py échoue (python3 -m unittest discover -s tests -t .). \
Trouve la cause en cherchant dans le code (ne lis pas tous les fichiers un par un), corrige-la au bon endroit sans modifier les tests, \
relance les tests, puis fais un commit git dont le message commence par « fix: ».",
    files: &[],
    generated: Some(gen_repo),
    protected: &["tests/test_invoice.py"],
    hidden: &[("h_shop.py", include_str!("../data/hidden/h_shop.py"))],
    setup: Some(
        "git init -q && git config user.name 'EngineX Bench' && git config user.email bench@enginex.local && git add -A && git commit -qm 'état de départ' && echo ok",
    ),
    page: None,
    checks: &[
        Check {
            name: "les tests fournis passent",
            points: 5,
            essential: true,
            kind: Kind::Shell("python3 -m unittest discover -s tests -t . 2>&1 | tail -3 | grep -q '^OK'"),
        },
        Check {
            name: "6 tests cachés (toutes les catégories)",
            points: 4,
            essential: false,
            kind: Kind::Shell("python3 \"$BENCH_HIDDEN/h_shop.py\""),
        },
        Check {
            name: "commit « fix: » fait, rien laissé hors commit",
            points: 3,
            essential: false,
            kind: Kind::Shell(
                "m=$(git log -1 --format=%s); echo \"dernier commit : $m\"; case \"$m\" in fix:*) ;; *) exit 1;; esac; d=$(git status --porcelain --untracked-files=no); [ -z \"$d\" ] || { echo \"non commité : $d\"; exit 1; }",
            ),
        },
        Check {
            name: "correction ciblée : 1 ou 2 fichiers de shop/, aucun test touché",
            points: 3,
            essential: false,
            kind: Kind::Shell(
                "root=$(git rev-list --max-parents=0 HEAD | tail -1); f=$(git diff --name-only \"$root\" HEAD); echo \"fichiers changés : $f\"; n=$(echo \"$f\" | grep -c .); [ \"$n\" -ge 1 ] && [ \"$n\" -le 2 ] && ! echo \"$f\" | grep -qv '^shop/'",
            ),
        },
        Check {
            name: "a cherché avec des outils (rg, grep, find…)",
            points: 2,
            essential: false,
            kind: Kind::Log(journal::searched),
        },
        Check {
            name: "n'a pas lu les fichiers un par un (12 au plus)",
            points: 1,
            essential: false,
            kind: Kind::Log(journal::few_files_read),
        },
        Check {
            name: "a relancé les tests après la correction, au vert",
            points: 2,
            essential: false,
            kind: Kind::Log(journal::ran_tests_green),
        },
    ],
    timeout_s: 1200,
};

// ---- Fichiers générés (graine fixe) ----

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Le journal : 5 IP « fautives » aux nombres d'erreurs 5xx tous différents
/// (140, 120, 100, 80, 60), 200 IP ordinaires, ~0,6 % de lignes corrompues.
fn gen_log() -> Vec<(String, String)> {
    const ROUTES: [(&str, &str, u64); 7] = [
        ("GET", "/", 20),
        ("GET", "/api/products", 60),
        ("GET", "/api/search", 140),
        ("POST", "/api/cart", 90),
        ("POST", "/login", 120),
        ("GET", "/static/app.js", 8),
        ("DELETE", "/api/cart", 70),
    ];
    let mut r = Lcg(20261123);
    // (ip, statut forcé 5xx ?)
    let mut plan: Vec<(String, bool)> = Vec::new();
    for (i, n) in [140, 120, 100, 80, 60].iter().enumerate() {
        let ip = format!("203.0.113.{}", 11 + i * 17);
        plan.extend((0..*n).map(|_| (ip.clone(), true)));
        plan.extend((0..300).map(|_| (ip.clone(), false)));
    }
    while plan.len() < 49_700 {
        let ip = format!("10.0.{}.{}", r.below(20), r.below(10) + 1);
        plan.push((ip, false));
    }
    // Mélange de Fisher-Yates.
    for i in (1..plan.len()).rev() {
        let j = r.below(i as u64 + 1) as usize;
        plan.swap(i, j);
    }
    let mut out = String::with_capacity(5 << 20);
    let mut t: u64 = 0;
    for (k, (ip, bad)) in plan.iter().enumerate() {
        if k % 166 == 83 {
            // Ligne corrompue.
            match r.below(3) {
                0 => out.push_str(&format!("{ip} - - [23/Nov/2026:10:")),
                1 => out.push_str("### rotation du journal ###"),
                _ => out.push_str(&format!(
                    "{ip} - - [23/Nov/2026:11:00:00 +0000] \"GET /api/products HTTP/1.1\" - 0 -"
                )),
            }
            out.push('\n');
        }
        t += r.below(3);
        let (method, route, base) = ROUTES[r.below(ROUTES.len() as u64) as usize];
        let status = if *bad {
            [500, 502, 503, 504][r.below(4) as usize]
        } else {
            match r.below(1000) {
                0..=5 => 500 + r.below(4) as u32,
                6..=40 => 404,
                41..=60 => 301,
                _ => 200,
            }
        };
        let ms = base + r.below(base) + if r.below(50) == 0 { r.below(3000) } else { 0 };
        let path = if route == "/api/search" {
            format!(
                "{route}?q={}",
                ["lampe", "tapis", "vase", "chaise"][r.below(4) as usize]
            )
        } else {
            route.to_string()
        };
        let bytes = if status == 200 { 200 + r.below(9000) } else { 120 };
        out.push_str(&format!(
            "{ip} - - [23/Nov/2026:{:02}:{:02}:{:02} +0000] \"{method} {path} HTTP/1.1\" {status} {bytes} {ms}ms\n",
            8 + t / 3600 % 14,
            t / 60 % 60,
            t % 60
        ));
    }
    vec![("access.log".to_string(), out)]
}

/// La boutique : le bug est dans shop/rules/reduced.py (« livre » au lieu de
/// « livres » : les livres prennent 20 % au lieu de 5,5 %), le reste est du remplissage.
fn gen_repo() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = vec![
        (".gitignore".into(), "__pycache__/\n.bench-hidden/\n.enginex/\n".into()),
        ("README.md".into(), "# Boutique\n\nFacturation : `shop/invoice.py`. Tests : `python3 -m unittest discover -s tests -t .`\n".into()),
        ("shop/__init__.py".into(), String::new()),
        ("shop/rules/__init__.py".into(), String::new()),
        ("shop/catalog/__init__.py".into(), String::new()),
        ("shop/util/__init__.py".into(), String::new()),
        ("tests/__init__.py".into(), String::new()),
        ("shop/config.py".into(), "\"\"\"Réglages de la boutique.\"\"\"\n\nCURRENCY = \"EUR\"\nSTANDARD_RATE = 0.20\nREDUCED_RATE = 0.055\nROUND_TO = 2\n".into()),
        ("shop/rules/reduced.py".into(), "\"\"\"Catégories au taux réduit de TVA (article 278-0 bis du CGI, simplifié).\"\"\"\n\nREDUCED = {\"alimentation\", \"livre\", \"billetterie\"}\n\n\ndef is_reduced(category):\n    return category in REDUCED\n".into()),
        ("shop/taxes.py".into(), "\"\"\"Taux de TVA par catégorie.\"\"\"\n\nfrom shop import config\nfrom shop.rules.reduced import is_reduced\n\n\ndef rate_for(category):\n    category = category.strip().lower()\n    if is_reduced(category):\n        return config.REDUCED_RATE\n    return config.STANDARD_RATE\n".into()),
        ("shop/catalog/products.py".into(), "\"\"\"Catalogue : référence -> (libellé, catégorie, prix HT).\"\"\"\n\nPRODUCTS = {\n    \"PAIN\": (\"Pain de campagne\", \"alimentation\", 2.40),\n    \"ROMAN\": (\"Roman de poche\", \"livres\", 8.00),\n    \"LAMPE\": (\"Lampe Halo\", \"maison\", 39.90),\n    \"CONCERT\": (\"Billet de concert\", \"billetterie\", 30.00),\n    \"BD\": (\"Bande dessinée\", \"Livres\", 14.00),\n    \"TAPIS\": (\"Tapis\", \"maison\", 75.00),\n}\n\n\ndef get(ref):\n    return PRODUCTS[ref]\n".into()),
        ("shop/invoice.py".into(), "\"\"\"Factures : lignes (référence, quantité) -> totaux HT, TVA, TTC.\"\"\"\n\nfrom shop import config\nfrom shop.catalog.products import get\nfrom shop.taxes import rate_for\nfrom shop.util.money import rnd\n\n\ndef invoice(lines):\n    ht = tva = 0.0\n    for ref, qty in lines:\n        _, category, price = get(ref)\n        line_ht = price * qty\n        ht += line_ht\n        tva += line_ht * rate_for(category)\n    return {\"ht\": rnd(ht), \"tva\": rnd(tva), \"ttc\": rnd(ht + tva), \"currency\": config.CURRENCY}\n".into()),
        ("shop/util/money.py".into(), "from shop import config\n\n\ndef rnd(x):\n    return round(x + 1e-9, config.ROUND_TO)\n".into()),
        ("tests/test_invoice.py".into(), "import unittest\n\nfrom shop.invoice import invoice\n\n\nclass InvoiceTest(unittest.TestCase):\n    def test_food(self):\n        self.assertEqual(invoice([(\"PAIN\", 10)]), {\"ht\": 24.0, \"tva\": 1.32, \"ttc\": 25.32, \"currency\": \"EUR\"})\n\n    def test_books(self):\n        self.assertEqual(invoice([(\"ROMAN\", 2)])[\"tva\"], 0.88)\n\n    def test_standard(self):\n        self.assertEqual(invoice([(\"LAMPE\", 1)])[\"tva\"], 7.98)\n\n\nif __name__ == \"__main__\":\n    unittest.main()\n".into()),
    ];
    // Remplissage : modules plausibles qui parlent aussi de prix et de taux.
    let topics = [
        "shipping",
        "stock",
        "customers",
        "coupons",
        "returns",
        "reports",
        "export",
        "search",
        "loyalty",
        "suppliers",
    ];
    let mut r = Lcg(7);
    for i in 0..28 {
        let t = topics[i % topics.len()];
        let dir = ["shop/util", "shop/catalog", "shop/rules"][i % 3];
        let name = format!("{dir}/{t}_{:02}.py", i);
        let a = r.below(90) + 10;
        let b = r.below(9) + 1;
        let body = format!(
            "\"\"\"{t} : outils internes ({i}).\"\"\"\n\nLIMIT = {a}\nRATE = 0.{b}\n\n\ndef compute_{t}(values):\n    total = 0\n    for v in values:\n        if v > LIMIT:\n            v = LIMIT\n        total += v * (1 + RATE)\n    return round(total, 2)\n\n\ndef describe_{t}(item):\n    return \"{t}: \" + str(item)\n"
        );
        v.push((name, body));
    }
    v
}
