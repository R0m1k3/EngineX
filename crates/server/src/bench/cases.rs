//! Les 10 cas de la page Tester : consigne, fichiers de départ, fichiers de test
//! protégés (restaurés avant les critères) et critères exécutés par `bash -lc`
//! dans la sandbox du cas (code 0 = critère réussi).
//!
//! Les critères DOM passent par jsdom, installé à la demande dans /tmp/bench
//! (internet ouvert dans les sandboxes) : le rendu visuel reste jugé à la main.
//! Les pages testées au DOM tiennent en un seul fichier HTML (consigne) : jsdom les
//! charge sous http://localhost/ (localStorage indisponible sous file://).

pub struct Criterion {
    pub name: &'static str,
    pub script: &'static str,
}

pub struct Case {
    pub key: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
    /// Fichiers donnés au modèle.
    pub files: &'static [(&'static str, &'static str)],
    /// Fichiers de test fournis : réécrits tels quels avant les critères.
    pub protected: &'static [&'static str],
    pub criteria: &'static [Criterion],
    /// Plafond de temps de tour (secondes, ≤ 1200).
    pub timeout_s: u64,
}

pub fn all() -> &'static [Case] {
    &CASES
}

pub fn get(key: &str) -> Option<&'static Case> {
    CASES.iter().find(|c| c.key == key)
}

/// Prépare jsdom puis exécute le script Node donné (argument 1 = fichier HTML).
macro_rules! dom {
    ($file:expr, $js:expr) => {
        concat!(
            "mkdir -p /tmp/bench && cd /tmp/bench && ([ -d node_modules/jsdom ] || (npm init -y >/dev/null 2>&1; npm i --no-audit --no-fund jsdom >/dev/null 2>&1)); ",
            "cat > /tmp/bench/check.cjs <<'JSEOF'\n",
            "const {JSDOM}=require('/tmp/bench/node_modules/jsdom');const path=require('path');\n",
            "const file=path.resolve(process.env.BENCH_DIR,'", $file, "');\n",
            "const ok=(c,m)=>{if(!c){console.error('KO: '+m);process.exit(1)}};\n",
            "(async()=>{const errors=[];const vc=new (require('/tmp/bench/node_modules/jsdom').VirtualConsole)();vc.on('jsdomError',e=>errors.push(String(e.message)));\n",
            "const dom=await JSDOM.fromFile(file,{url:'http://localhost/',runScripts:'dangerously',pretendToBeVisual:true,virtualConsole:vc});\n",
            "await new Promise(r=>setTimeout(r,300));const w=dom.window,d=w.document;\n",
            $js,
            "\n})().catch(e=>{console.error('KO: '+e.message);process.exit(1)});\n",
            "JSEOF\n",
            "node /tmp/bench/check.cjs"
        )
    };
}

static CASES: [Case; 10] = [
    Case {
        key: "static-page",
        title: "Page HTML statique mise en forme",
        prompt: "Crée la page d'accueil d'une boulangerie fictive « Le Fournil » : index.html et style.css, sans framework ni JavaScript. \
La page a un en-tête avec un menu de 3 liens au moins, une zone d'accroche, une section de 3 cartes (pain, viennoiseries, pâtisseries), un pied de page. \
Elle doit s'adapter au téléphone (balise viewport et au moins une règle @media). Soigne la mise en forme : couleurs, marges, typographie.",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "index.html et style.css existent",
                script: "test -s index.html && test -s style.css",
            },
            Criterion {
                name: "structure : title, h1, 3 liens, viewport",
                script: "python3 - <<'PY'\nimport re,sys\nh=open('index.html',encoding='utf-8').read()\nok=re.search(r'<title>\\s*\\S',h) and re.search(r'<h1',h) and len(re.findall(r'<a\\s',h))>=3 and re.search(r'name=.viewport',h)\nsys.exit(0 if ok else 1)\nPY",
            },
            Criterion {
                name: "style : 15 déclarations, @media, feuille liée",
                script: "grep -q 'style.css' index.html && [ $(grep -o ';' style.css | wc -l) -ge 15 ] && grep -q '@media' style.css",
            },
        ],
        timeout_s: 1200,
    },
    Case {
        key: "counter",
        title: "Compteur interactif en JavaScript",
        prompt: "Crée index.html : un compteur. Un élément #count affiche la valeur (0 au départ), trois boutons #inc (+1), #dec (−1) et #reset (retour à 0). \
Le compteur ne descend jamais sous 0. JavaScript natif, tout dans index.html, aucune dépendance.",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "index.html existe",
                script: "test -s index.html",
            },
            Criterion {
                name: "3 clics sur + donnent 3",
                script: dom!(
                    "index.html",
                    "const c=()=>d.querySelector('#count').textContent.trim();ok(c()==='0','départ '+c());for(let i=0;i<3;i++)d.querySelector('#inc').click();ok(c()==='3','après 3 clics : '+c());ok(errors.length===0,'erreurs : '+errors.join(';'));"
                ),
            },
            Criterion {
                name: "− redescend, jamais sous 0, reset remet à 0",
                script: dom!(
                    "index.html",
                    "const c=()=>d.querySelector('#count').textContent.trim();const click=s=>d.querySelector(s).click();click('#inc');click('#inc');click('#dec');ok(c()==='1','dec : '+c());click('#dec');click('#dec');ok(c()==='0','plancher : '+c());click('#inc');click('#reset');ok(c()==='0','reset : '+c());"
                ),
            },
        ],
        timeout_s: 900,
    },
    Case {
        key: "todo",
        title: "Todo-list avec stockage local",
        prompt: "Crée index.html : une todo-list en JavaScript natif, sans dépendance. Un champ #new et un bouton #add ajoutent une tâche dans la liste #list (un <li> par tâche). \
Chaque tâche a une case à cocher qui ajoute/retire la classe « done » sur le <li>, et un bouton .del qui la supprime. \
Les tâches sont enregistrées dans localStorage sous la clé « todos » et rechargées à l'ouverture.",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "index.html existe",
                script: "test -s index.html",
            },
            Criterion {
                name: "ajout de 2 tâches : 2 <li>, champ vidé",
                script: dom!(
                    "index.html",
                    "const add=t=>{const i=d.querySelector('#new');i.value=t;d.querySelector('#add').click()};add('lait');add('pain');ok(d.querySelectorAll('#list li').length===2,'nombre de li : '+d.querySelectorAll('#list li').length);ok(d.querySelector('#new').value==='','champ non vidé');"
                ),
            },
            Criterion {
                name: "cocher met « done », supprimer retire la tâche",
                script: dom!(
                    "index.html",
                    "const add=t=>{const i=d.querySelector('#new');i.value=t;d.querySelector('#add').click()};add('a');add('b');const cb=d.querySelector('#list li input[type=checkbox]');ok(cb,'pas de case');cb.click();ok(d.querySelector('#list li').classList.contains('done'),'classe done absente');d.querySelector('#list li .del').click();ok(d.querySelectorAll('#list li').length===1,'suppression');"
                ),
            },
            Criterion {
                name: "enregistré dans localStorage « todos »",
                script: dom!(
                    "index.html",
                    "const i=d.querySelector('#new');i.value='café';d.querySelector('#add').click();const v=w.localStorage.getItem('todos');ok(v&&v.includes('café'),'localStorage : '+v);"
                ),
            },
        ],
        timeout_s: 1200,
    },
    Case {
        key: "bugfix",
        title: "Corriger un bug dans un fichier fourni",
        prompt: "Le fichier cart.py calcule le total d'un panier, mais les tests de test_cart.py échouent. Lis les deux fichiers, trouve les causes et corrige cart.py. \
Ne modifie pas test_cart.py. Lance les tests pour le vérifier : python3 -m unittest -v",
        files: &[
            (
                "cart.py",
                "CODES = {\"SAVE10\": 0.10, \"SAVE20\": 0.20}\n\n\ndef total(items, discount=0):\n    \"\"\"Total d'un panier : somme prix * quantité, moins la remise (0 à 1).\"\"\"\n    s = 0\n    for it in items:\n        s += it[\"price\"]\n    return round(s * (1 - discount), 2)\n\n\ndef apply_coupon(amount, code):\n    \"\"\"Applique un code promo ; un code inconnu ne change rien.\"\"\"\n    return amount * CODES.get(code, 0)\n",
            ),
            (
                "test_cart.py",
                "import unittest\n\nfrom cart import apply_coupon, total\n\n\nclass CartTest(unittest.TestCase):\n    def test_total_quantities(self):\n        items = [{\"price\": 2.5, \"qty\": 2}, {\"price\": 1.0, \"qty\": 3}]\n        self.assertEqual(total(items), 8.0)\n\n    def test_total_discount(self):\n        items = [{\"price\": 2.5, \"qty\": 2}, {\"price\": 1.0, \"qty\": 3}]\n        self.assertEqual(total(items, 0.1), 7.2)\n\n    def test_empty(self):\n        self.assertEqual(total([]), 0)\n\n    def test_coupon(self):\n        self.assertEqual(apply_coupon(100, \"SAVE10\"), 90)\n\n    def test_unknown_coupon(self):\n        self.assertEqual(apply_coupon(100, \"NOPE\"), 100)\n\n    def test_bad_discount(self):\n        with self.assertRaises(ValueError):\n            total([{\"price\": 1, \"qty\": 1}], 1.5)\n\n\nif __name__ == \"__main__\":\n    unittest.main()\n",
            ),
        ],
        protected: &["test_cart.py"],
        criteria: &[
            Criterion {
                name: "les 6 tests passent",
                script: "python3 -m unittest -q test_cart 2>&1 | tail -3 | grep -q '^OK'",
            },
            Criterion {
                name: "cart.py garde total et apply_coupon",
                script: "grep -q 'def total' cart.py && grep -q 'def apply_coupon' cart.py",
            },
            Criterion {
                name: "total pondère par la quantité (autre jeu de valeurs)",
                script: "python3 -c \"from cart import total; assert total([{'price':10,'qty':3}], 0.5)==15.0\"",
            },
        ],
        timeout_s: 900,
    },
    Case {
        key: "dates",
        title: "Fonction avec ses propres tests",
        prompt: "Écris dateparse.py avec une fonction parse_date(texte) qui renvoie un tuple (année, mois, jour). Formats à accepter : « 2026-11-22 », « 22/11/2026 », \
« 22 novembre 2026 » (mois français, majuscule ou non), « Nov 22, 2026 » (mois anglais abrégé). Tout autre texte, ou une date impossible (30 février), lève ValueError. \
Écris aussi test_dateparse.py (unittest) avec au moins 5 tests, et fais-les passer : python3 -m unittest -v",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "les 4 formats donnent (2026, 11, 22)",
                script: "python3 -c \"from dateparse import parse_date as p; assert all(p(s)==(2026,11,22) for s in ['2026-11-22','22/11/2026','22 novembre 2026','Nov 22, 2026'])\"",
            },
            Criterion {
                name: "texte invalide ou date impossible → ValueError",
                script: "python3 -c \"\nfrom dateparse import parse_date as p\nfor s in ['bonjour','2026-02-30','31/04/2026','']:\n    try: p(s)\n    except ValueError: continue\n    raise SystemExit('pas de ValueError pour '+repr(s))\n\"",
            },
            Criterion {
                name: "ses propres tests : ≥ 5, tous OK",
                script: "out=$(python3 -m unittest -v test_dateparse 2>&1); echo \"$out\" | tail -3 | grep -q '^OK' && [ $(echo \"$out\" | grep -c ' ok$') -ge 5 ]",
            },
        ],
        timeout_s: 900,
    },
    Case {
        key: "rest-api",
        title: "API REST minimale",
        prompt: "Écris server.py (bibliothèque standard Python uniquement) : une API JSON sur le port 8000. \
GET /items → liste ; POST /items avec {\"name\": \"...\"} → crée (201) et renvoie l'objet avec son id ; GET /items/<id> → l'objet ou 404 ; DELETE /items/<id> → 204 ou 404. \
Les données restent en mémoire. Lance-le en arrière-plan et vérifie avec curl.",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "POST /items → 201 avec id",
                script: "(python3 server.py >/dev/null 2>&1 & echo $! >/tmp/srv.pid); sleep 2; r=$(curl -s -o /tmp/o.json -w '%{http_code}' -X POST -H 'Content-Type: application/json' -d '{\"name\":\"x\"}' localhost:8000/items); kill $(cat /tmp/srv.pid) 2>/dev/null; [ \"$r\" = 201 ] && grep -q '\"id\"' /tmp/o.json",
            },
            Criterion {
                name: "GET /items liste l'objet créé ; GET /items/<id> le renvoie",
                script: "(python3 server.py >/dev/null 2>&1 & echo $! >/tmp/srv.pid); sleep 2; id=$(curl -s -X POST -d '{\"name\":\"zz\"}' -H 'Content-Type: application/json' localhost:8000/items | python3 -c 'import sys,json;print(json.load(sys.stdin)[\"id\"])'); a=$(curl -s localhost:8000/items); b=$(curl -s localhost:8000/items/$id); kill $(cat /tmp/srv.pid) 2>/dev/null; echo \"$a\" | grep -q zz && echo \"$b\" | grep -q zz",
            },
            Criterion {
                name: "id inconnu → 404 ; DELETE → 204 puis 404",
                script: "(python3 server.py >/dev/null 2>&1 & echo $! >/tmp/srv.pid); sleep 2; id=$(curl -s -X POST -d '{\"name\":\"d\"}' -H 'Content-Type: application/json' localhost:8000/items | python3 -c 'import sys,json;print(json.load(sys.stdin)[\"id\"])'); n=$(curl -s -o /dev/null -w '%{http_code}' localhost:8000/items/99999); d=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE localhost:8000/items/$id); g=$(curl -s -o /dev/null -w '%{http_code}' localhost:8000/items/$id); kill $(cat /tmp/srv.pid) 2>/dev/null; [ \"$n\" = 404 ] && [ \"$d\" = 204 ] && [ \"$g\" = 404 ]",
            },
        ],
        timeout_s: 1200,
    },
    Case {
        key: "refactor",
        title: "Refactorer un fichier sans changer son comportement",
        prompt: "report.py contient une seule grosse fonction, difficile à lire. Refactore-le en plusieurs fonctions courtes (aucune de plus de 25 lignes), avec des noms clairs, \
sans changer le résultat de build_report. test_report.py doit continuer à passer sans modification : python3 -m unittest -v",
        files: &[("report.py", REPORT_PY), ("test_report.py", REPORT_TEST_PY)],
        protected: &["test_report.py"],
        criteria: &[
            Criterion {
                name: "les tests fournis passent",
                script: "python3 -m unittest -q test_report 2>&1 | tail -3 | grep -q '^OK'",
            },
            Criterion {
                name: "aucune fonction de plus de 25 lignes",
                script: "python3 - <<'PY'\nimport ast,sys\nt=ast.parse(open('report.py',encoding='utf-8').read())\nbad=[f.name for f in ast.walk(t) if isinstance(f,ast.FunctionDef) and f.end_lineno-f.lineno+1>25]\nprint(bad)\nsys.exit(1 if bad else 0)\nPY",
            },
            Criterion {
                name: "au moins 4 fonctions, build_report conservée",
                script: "python3 - <<'PY'\nimport ast,sys\nt=ast.parse(open('report.py',encoding='utf-8').read())\nn=[f.name for f in ast.walk(t) if isinstance(f,ast.FunctionDef)]\nsys.exit(0 if len(n)>=4 and 'build_report' in n else 1)\nPY",
            },
        ],
        timeout_s: 900,
    },
    Case {
        key: "canvas-game",
        title: "Petit jeu sur canvas",
        prompt: "Crée index.html : un jeu de Snake jouable au clavier (flèches), dessiné sur un <canvas id=\"game\">, tout dans ce seul fichier, sans dépendance. \
Le serpent grandit en mangeant, la partie s'arrête au contact d'un mur ou de lui-même, le score s'affiche dans #score, et la touche Entrée relance la partie.",
        files: &[],
        protected: &[],
        criteria: &[
            Criterion {
                name: "index.html avec canvas#game et #score",
                script: "grep -q 'id=.game' index.html && grep -q '<canvas' index.html && grep -q 'id=.score' index.html",
            },
            Criterion {
                name: "écoute le clavier et boucle de jeu",
                script: "grep -q 'keydown' index.html && (grep -q 'requestAnimationFrame' index.html || grep -q 'setInterval' index.html)",
            },
            Criterion {
                name: "le script s'analyse sans erreur de syntaxe",
                script: "python3 - <<'PY'\nimport re,subprocess,sys\nh=open('index.html',encoding='utf-8').read()\ns='\\n'.join(re.findall(r'<script[^>]*>(.*?)</script>',h,re.S))\nopen('/tmp/g.js','w').write(s)\nsys.exit(subprocess.call(['node','--check','/tmp/g.js']) if s.strip() else 1)\nPY",
            },
        ],
        timeout_s: 1200,
    },
    Case {
        key: "dashboard",
        title: "Tableau de bord à partir d'un JSON",
        prompt: "data.json contient les ventes d'un magasin (12 lignes). Crée dashboard.html : un tableau avec toutes les lignes (région, produit, montant, mois), \
une ligne de total, un tri au clic sur l'en-tête de chaque colonne (clic répété = ordre inverse) et un graphique en barres du montant par région dessiné en SVG ou canvas. \
Un fichier HTML ouvert depuis le disque ne peut pas lire data.json avec fetch : recopie les données dans la page. Pas de dépendance externe.",
        files: &[("data.json", DATA_JSON)],
        protected: &["data.json"],
        criteria: &[
            Criterion {
                name: "dashboard.html existe",
                script: "test -s dashboard.html",
            },
            Criterion {
                name: "12 lignes de données et un total exact",
                script: dom!(
                    "dashboard.html",
                    "const rows=d.querySelectorAll('tbody tr');ok(rows.length>=12,'lignes : '+rows.length);const data=JSON.parse(require('fs').readFileSync(path.resolve(process.env.BENCH_DIR,'data.json'),'utf8'));const sum=data.reduce((a,r)=>a+r.montant,0);const txt=d.body.textContent.replace(/[\\s\\u00a0\\u202f]/g,'');ok(txt.includes(String(sum))||txt.includes(String(sum).replace('.',',')),'total '+sum+' absent');"
                ),
            },
            Criterion {
                name: "clic sur l'en-tête « montant » trie, 2e clic inverse",
                script: dom!(
                    "dashboard.html",
                    "const ths=[...d.querySelectorAll('th')];const th=ths.find(t=>/montant/i.test(t.textContent));ok(th,'pas d’en-tête montant');const col=ths.indexOf(th);const vals=()=>[...d.querySelectorAll('tbody tr')].map(r=>parseFloat(r.children[col].textContent.replace(/[^0-9,.-]/g,'').replace(',','.')));th.click();const a=vals();th.click();const b=vals();const asc=x=>x.every((v,i)=>i===0||x[i-1]<=v);const desc=x=>x.every((v,i)=>i===0||x[i-1]>=v);ok((asc(a)&&desc(b))||(desc(a)&&asc(b)),'tri non inversé');"
                ),
            },
            Criterion {
                name: "graphique SVG ou canvas présent",
                script: "grep -q -e '<svg' -e '<canvas' -e 'createElementNS' dashboard.html",
            },
        ],
        timeout_s: 1200,
    },
    Case {
        key: "multi-file",
        title: "Tâche multi-fichiers suivant une spécification",
        prompt: "Lis SPEC.md et réalise ce qu'elle demande. test_notes.py est fourni : il doit passer sans modification (python3 -m unittest -v).",
        files: &[("SPEC.md", SPEC_MD), ("test_notes.py", NOTES_TEST_PY)],
        protected: &["test_notes.py"],
        criteria: &[
            Criterion {
                name: "paquet notes/ avec __init__, store.py, cli.py, __main__.py",
                script: "test -f notes/__init__.py && for f in store cli __main__; do test -s notes/$f.py || exit 1; done",
            },
            Criterion {
                name: "les tests fournis passent",
                script: "python3 -m unittest -q test_notes 2>&1 | tail -3 | grep -q '^OK'",
            },
            Criterion {
                name: "README.md présent avec un exemple d'usage",
                script: "test -s README.md && grep -q 'python3 -m notes' README.md",
            },
        ],
        timeout_s: 1200,
    },
];

const REPORT_PY: &str = r##"def build_report(lines):
    # lines : liste de "nom;categorie;montant" ; renvoie le rapport en texte
    rows = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(";")
        if len(parts) != 3:
            continue
        name = parts[0].strip()
        cat = parts[1].strip().lower()
        try:
            amount = float(parts[2].replace(",", "."))
        except ValueError:
            continue
        if amount <= 0:
            continue
        rows.append((name, cat, amount))
    groups = {}
    for name, cat, amount in rows:
        if cat not in groups:
            groups[cat] = []
        groups[cat].append((name, amount))
    out = []
    grand = 0
    for cat in sorted(groups):
        items = sorted(groups[cat], key=lambda x: (-x[1], x[0]))
        subtotal = 0
        out.append(cat.upper())
        for name, amount in items:
            out.append("  " + name.ljust(12) + ("%.2f" % amount).rjust(9))
            subtotal += amount
        out.append("  " + "total".ljust(12) + ("%.2f" % subtotal).rjust(9))
        out.append("")
        grand += subtotal
    out.append("TOTAL GENERAL " + ("%.2f" % grand))
    if rows:
        best = max(rows, key=lambda r: (r[2], r[0]))
        out.append("Plus grosse ligne : " + best[0] + " (" + ("%.2f" % best[2]) + ")")
    else:
        out.append("Aucune ligne valide")
    return "\n".join(out)
"##;

const REPORT_TEST_PY: &str = r##"import unittest

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
"##;

const DATA_JSON: &str = r##"[
  {"region": "Nord", "produit": "Lampe", "montant": 120.5, "mois": "janvier"},
  {"region": "Nord", "produit": "Tapis", "montant": 310, "mois": "février"},
  {"region": "Nord", "produit": "Plante", "montant": 45.25, "mois": "mars"},
  {"region": "Sud", "produit": "Lampe", "montant": 200, "mois": "janvier"},
  {"region": "Sud", "produit": "Tapis", "montant": 95.75, "mois": "février"},
  {"region": "Sud", "produit": "Plante", "montant": 60, "mois": "mars"},
  {"region": "Est", "produit": "Lampe", "montant": 150, "mois": "janvier"},
  {"region": "Est", "produit": "Tapis", "montant": 410.5, "mois": "février"},
  {"region": "Est", "produit": "Plante", "montant": 30, "mois": "mars"},
  {"region": "Ouest", "produit": "Lampe", "montant": 80, "mois": "janvier"},
  {"region": "Ouest", "produit": "Tapis", "montant": 275, "mois": "février"},
  {"region": "Ouest", "produit": "Plante", "montant": 55.5, "mois": "mars"}
]
"##;

const SPEC_MD: &str = r##"# Spécification : carnet de notes en ligne de commande

Réalise un paquet Python `notes` (bibliothèque standard seulement) :

- `notes/store.py` : classe `Store(path)` ; méthodes `add(text) -> int` (renvoie l'id, à partir de 1),
  `all() -> list[dict]` (clés `id`, `text`, `done`), `done(id)`, `remove(id)`. Les données sont écrites
  dans le fichier JSON `path` à chaque changement. Un id inconnu lève `KeyError`.
- `notes/cli.py` : fonction `main(argv)` qui lit le fichier indiqué par la variable d'environnement
  `NOTES_FILE` (défaut `notes.json`). Commandes : `add TEXTE`, `list`, `done ID`, `rm ID`.
  `list` affiche une ligne par note : `[x] 1 texte` (fait) ou `[ ] 1 texte`.
  Un id inconnu affiche `introuvable` sur la sortie d'erreur et retourne le code 1.
- `notes/__main__.py` : appelle `main` pour que `python3 -m notes list` fonctionne.
- `notes/__init__.py` : peut être vide.
- `README.md` : explique l'usage avec des exemples `python3 -m notes add "acheter du pain"`.
"##;

const NOTES_TEST_PY: &str = r##"import os
import subprocess
import sys
import tempfile
import unittest

from notes.store import Store


def run(env_file, *args):
    env = dict(os.environ, NOTES_FILE=env_file)
    return subprocess.run([sys.executable, "-m", "notes", *args], capture_output=True, text=True, env=env)


class StoreTest(unittest.TestCase):
    def test_store(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "n.json")
            s = Store(p)
            self.assertEqual(s.add("a"), 1)
            self.assertEqual(s.add("b"), 2)
            s.done(1)
            self.assertEqual(Store(p).all(), [
                {"id": 1, "text": "a", "done": True},
                {"id": 2, "text": "b", "done": False},
            ])
            s.remove(2)
            self.assertEqual(len(Store(p).all()), 1)
            with self.assertRaises(KeyError):
                s.done(99)


class CliTest(unittest.TestCase):
    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "n.json")
            run(p, "add", "acheter du pain")
            run(p, "add", "appeler Paul")
            run(p, "done", "1")
            r = run(p, "list")
            self.assertEqual(r.stdout.splitlines(), ["[x] 1 acheter du pain", "[ ] 2 appeler Paul"])
            r = run(p, "rm", "7")
            self.assertEqual(r.returncode, 1)
            self.assertIn("introuvable", r.stderr)


if __name__ == "__main__":
    unittest.main()
"##;
