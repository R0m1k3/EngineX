//! Tests visuels : pages ouvertes dans Chromium, bureau 1280×800 et téléphone
//! 390×844. Les consignes fixent les identifiants que les contrôles utilisent.

use super::{Case, Check, Family, Kind};

const fn page(path: &'static str, steps: &'static [&'static str]) -> Kind {
    Kind::Page {
        path,
        phone: false,
        steps,
    }
}

const fn phone(path: &'static str, steps: &'static [&'static str]) -> Kind {
    Kind::Page {
        path,
        phone: true,
        steps,
    }
}

const CONSOLE: Kind = Kind::ConsoleClean;

pub static LANDING: Case = Case {
    key: "landing",
    title: "Page vitrine responsive",
    family: Family::Visual,
    summary: "cartes côte à côte au bureau et empilées au téléphone, pas de débordement, ancres du menu, contraste du texte",
    prompt: "Crée la page d'accueil d'une boulangerie fictive « Le Fournil » : index.html et style.css (feuille liée), sans framework ni JavaScript. \
Contenu : un en-tête avec le nom et un menu <nav> d'au moins 3 liens vers des sections de la page (href=\"#id\"), une zone d'accroche avec un <h1>, \
une section de 3 cartes (pain, viennoiseries, pâtisseries) qui portent chacune la classe « card », une section contact, un <footer>. \
Au bureau les 3 cartes sont côte à côte ; sur téléphone elles sont empilées et rien ne dépasse de l'écran en largeur. \
Les textes doivent être bien lisibles (contraste suffisant). Soigne la mise en forme : couleurs, marges, typographie.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html"),
    checks: &[
        Check {
            name: "index.html et style.css, feuille liée",
            points: 2,
            essential: true,
            kind: Kind::Shell("test -s index.html && test -s style.css && grep -qi 'style.css' index.html"),
        },
        Check {
            name: "titre, h1 visible, menu de 3 liens visibles",
            points: 2,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"ok(document.title.trim(), 'pas de <title>');ok(visible('h1'), 'h1 absent ou caché');
const l = $$('nav a').filter(visible); ok(l.length >= 3, 'liens visibles dans <nav> : ' + l.length);"#,
                ],
            ),
        },
        Check {
            name: "chaque lien du menu mène à une section qui existe",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[r##"const l = $$('nav a'); ok(l.length >= 3, 'menu absent');
for (const a of l) { const h = a.getAttribute('href') || ''; ok(h.startsWith('#') && h.length > 1, 'lien ' + h + ' : pas une ancre'); ok(document.getElementById(decodeURIComponent(h.slice(1))), 'cible ' + h + ' absente'); }"##],
            ),
        },
        Check {
            name: "bureau : 3 cartes côte à côte, 200 px de large au moins",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const c = $$('.card').filter(visible); ok(c.length === 3, 'cartes .card visibles : ' + c.length);
const b = c.map((e) => e.getBoundingClientRect()); ok(b.every((r) => Math.abs(r.top - b[0].top) <= 4), 'cartes pas alignées sur une ligne');
ok(b.every((r) => r.width >= 200), 'largeurs : ' + b.map((r) => Math.round(r.width)).join(', '));
ok(b[0].right <= b[1].left + 1 && b[1].right <= b[2].left + 1, 'cartes qui se chevauchent');"#,
                ],
            ),
        },
        Check {
            name: "téléphone : cartes empilées, 80 % de la largeur au moins",
            points: 3,
            essential: false,
            kind: phone(
                "index.html",
                &[
                    r#"const c = $$('.card').filter(visible); ok(c.length === 3, 'cartes .card visibles : ' + c.length);
const b = c.map((e) => e.getBoundingClientRect()); ok(b[0].bottom <= b[1].top + 1 && b[1].bottom <= b[2].top + 1, 'cartes pas empilées');
ok(b.every((r) => r.width >= innerWidth * 0.8), 'largeurs : ' + b.map((r) => Math.round(r.width)).join(', ') + ' pour ' + innerWidth);"#,
                ],
            ),
        },
        Check {
            name: "téléphone : rien ne dépasse en largeur",
            points: 3,
            essential: false,
            kind: phone(
                "index.html",
                &["ok(document.querySelector('meta[name=viewport]'), 'pas de balise viewport'); noHScroll();"],
            ),
        },
        Check {
            name: "texte lisible : contraste ≥ 4,5 sur les paragraphes",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const p = $$('p').filter(visible).filter((e) => e.textContent.trim().length > 10).slice(0, 12); ok(p.length, 'aucun paragraphe');
for (const e of p) { const r = contrast(e); if (r !== null) ok(r >= 4.5, 'contraste ' + r.toFixed(2) + ' : « ' + e.textContent.trim().slice(0, 40) + ' »'); }"#,
                ],
            ),
        },
        Check {
            name: "pied de page visible après le contenu, images avec alt",
            points: 1,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"ok(visible('footer'), 'pas de <footer> visible'); const f = box('footer');
ok($$('.card').every((c) => c.getBoundingClientRect().bottom <= f.top + 1), 'footer avant les cartes');
const bad = $$('img').filter((i) => !(i.getAttribute('alt') || '').trim()); ok(!bad.length, bad.length + ' image(s) sans alt');"#,
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 2,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static CALCULATOR: Case = Case {
    key: "calculator",
    title: "Calculatrice",
    family: Family::Visual,
    summary: "calculs au clic et au clavier, priorité des opérations, 0,1 + 0,2, division par zéro",
    prompt: "Crée index.html : une calculatrice, tout dans ce seul fichier (HTML, CSS, JavaScript natif, aucune dépendance). \
L'écran est un élément #display qui affiche 0 au départ. Chaque touche est un <button> avec un attribut data-key : \"0\" à \"9\", \".\", \"+\", \"-\", \"*\", \"/\", \"=\" et \"C\" (remise à zéro). \
Les opérations respectent la priorité (2+3*4 donne 14). Les résultats sont arrondis pour éviter les erreurs d'affichage des décimaux (0.1+0.2 donne 0.3), \
avec un point décimal. Une division par zéro affiche « Erreur ». Après =, un chiffre commence un nouveau calcul et un opérateur continue avec le résultat. \
Le clavier marche aussi (événement keydown) : chiffres, . + - * /, Entrée ou = pour calculer, Échap pour effacer, Retour arrière pour effacer le dernier chiffre. \
Sur téléphone, la calculatrice tient dans l'écran et les touches font au moins 44 px de haut. Soigne l'apparence.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html"),
    checks: &[
        Check {
            name: "écran à 0 et les 18 touches visibles",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[r#"ok(txt('#display') === '0', 'écran au départ : ' + txt('#display'));
for (const k of ['0','1','2','3','4','5','6','7','8','9','.','+','-','*','/','=','C']) ok(visible(`[data-key="${k}"]`), 'touche ' + k + ' absente ou cachée');"#],
            ),
        },
        Check {
            name: "au clic : 12 + 7 = 19",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"for (const k of ['1','2','+','7','=']) await click(`[data-key="${k}"]`); ok(txt('#display') === '19', '12+7= donne ' + txt('#display'));"#,
                ],
            ),
        },
        Check {
            name: "priorité : 2 + 3 × 4 = 14",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const k of ['2','+','3','*','4','=']) await click(`[data-key="${k}"]`); ok(txt('#display') === '14', '2+3*4= donne ' + txt('#display'));"#,
                ],
            ),
        },
        Check {
            name: "0,1 + 0,2 = 0,3 (arrondi)",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const k of ['0','.','1','+','0','.','2','=']) await click(`[data-key="${k}"]`); ok(txt('#display') === '0.3', '0.1+0.2= donne ' + txt('#display'));"#,
                ],
            ),
        },
        Check {
            name: "7 / 0 affiche « Erreur », C remet à 0",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const k of ['7','/','0','=']) await click(`[data-key="${k}"]`); ok(/erreur/i.test(txt('#display')), '7/0= donne ' + txt('#display'));
await click('[data-key="C"]'); ok(txt('#display') === '0', 'après C : ' + txt('#display'));"#,
                ],
            ),
        },
        Check {
            name: "clavier : 9*9 Entrée = 81, Échap, Retour arrière",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[r#"document.activeElement && document.activeElement.blur();
for (const k of ['9','*','9','Enter']) await key(k); ok(txt('#display') === '81', '9*9 Entrée donne ' + txt('#display'));
await key('Escape'); ok(txt('#display') === '0', 'Échap donne ' + txt('#display'));
for (const k of ['1','2','3','Backspace']) await key(k); ok(txt('#display') === '12', '123 puis Retour arrière donne ' + txt('#display'));"#],
            ),
        },
        Check {
            name: "après = : un chiffre recommence, un opérateur continue",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const k of ['2','+','2','=','5']) await click(`[data-key="${k}"]`); ok(txt('#display') === '5', '2+2= puis 5 donne ' + txt('#display'));
await click('[data-key="C"]'); for (const k of ['2','+','2','=','+','3','=']) await click(`[data-key="${k}"]`); ok(txt('#display') === '7', '2+2= puis +3= donne ' + txt('#display'));"#,
                ],
            ),
        },
        Check {
            name: "téléphone : tient dans l'écran, touches de 44 px",
            points: 2,
            essential: false,
            kind: phone(
                "index.html",
                &[
                    r#"noHScroll(); const b = $$('[data-key]').filter(visible); ok(b.length >= 17, 'touches visibles : ' + b.length);
const small = b.filter((e) => e.getBoundingClientRect().height < 43.5); ok(!small.length, small.length + ' touche(s) de moins de 44 px');
ok(b.every((e) => e.getBoundingClientRect().right <= innerWidth + 1), 'touches hors de l’écran');"#,
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 1,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static TODO: Case = Case {
    key: "todo",
    title: "Todo-list",
    family: Family::Visual,
    summary: "ajout au clic et à Entrée, cocher, supprimer, filtres, compteur, rechargement réel de la page",
    prompt: "Crée index.html : une todo-list en JavaScript natif, tout dans ce seul fichier, sans dépendance. \
Un champ #new et un bouton #add ajoutent une tâche (la touche Entrée dans le champ aussi) ; un texte vide ou fait d'espaces est ignoré ; le champ se vide après l'ajout. \
La liste #list contient un <li> par tâche, avec une case à cocher (input type=checkbox) qui ajoute ou retire la classe « done » sur le <li>, et un bouton de classe « del » qui supprime la tâche. \
#remaining affiche le nombre de tâches non faites. Trois boutons #filter-all, #filter-active et #filter-done n'affichent que les tâches correspondantes. \
Les tâches et leur état sont enregistrés dans localStorage sous la clé « todos » et rechargés à l'ouverture. Un texte très long passe à la ligne sans déborder. \
La page s'adapte au téléphone. Soigne l'apparence.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html"),
    checks: &[
        Check {
            name: "ajout au clic et à Entrée, champ vidé",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"await type('#new', 'lait'); await click('#add'); ok(need('#new').value === '', 'champ non vidé');
await type('#new', 'pain'); await key('Enter', '#new'); const n = $$('#list li').length; ok(n === 2, 'tâches après clic + Entrée : ' + n);
ok(txt('#list').includes('lait') && txt('#list').includes('pain'), 'textes absents');"#,
                ],
            ),
        },
        Check {
            name: "texte vide ignoré",
            points: 1,
            essential: false,
            kind: page(
                "index.html",
                &[
                    "await type('#new', '   '); await click('#add'); ok($$('#list li').length === 0, 'tâche vide ajoutée');",
                ],
            ),
        },
        Check {
            name: "cocher : classe done et compteur à jour",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const t of ['a','b','c']) { await type('#new', t); await click('#add'); }
ok(/\b3\b/.test(txt('#remaining')), 'compteur au départ : ' + txt('#remaining'));
await click('#list li input[type=checkbox]'); ok($$('#list li')[0].classList.contains('done'), 'classe done absente');
ok(/\b2\b/.test(txt('#remaining')), 'compteur après une case cochée : ' + txt('#remaining'));
await click('#list li input[type=checkbox]'); ok(!$$('#list li')[0].classList.contains('done'), 'décocher ne retire pas done');"#,
                ],
            ),
        },
        Check {
            name: "supprimer une tâche",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const t of ['x','y']) { await type('#new', t); await click('#add'); }
await click('#list li .del'); const n = $$('#list li').length; ok(n === 1, 'tâches après suppression : ' + n); ok(txt('#list').includes('y'), 'mauvaise tâche supprimée');"#,
                ],
            ),
        },
        Check {
            name: "filtres actives / faites / toutes",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const t of ['un','deux','trois']) { await type('#new', t); await click('#add'); }
await click($$('#list li input[type=checkbox]')[1]);
const shown = () => $$('#list li').filter(visible).map((l) => l.textContent.replace(/\s+/g, ' ').trim());
await click('#filter-active'); let s = shown(); ok(s.length === 2 && !s.some((t) => t.includes('deux')), 'filtre actives : ' + s.join(' | '));
await click('#filter-done'); s = shown(); ok(s.length === 1 && s[0].includes('deux'), 'filtre faites : ' + s.join(' | '));
await click('#filter-all'); s = shown(); ok(s.length === 3, 'filtre toutes : ' + s.length);"#,
                ],
            ),
        },
        Check {
            name: "après rechargement de la page : tâches et cases conservées",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"for (const t of ['café','sucre']) { await type('#new', t); await click('#add'); }
await click('#list li input[type=checkbox]'); ok(localStorage.getItem('todos'), 'rien sous la clé todos');"#,
                    "@reload",
                    r#"const l = $$('#list li'); ok(l.length === 2, 'tâches après rechargement : ' + l.length);
ok(l[0].classList.contains('done') && l[0].querySelector('input[type=checkbox]').checked, 'état coché perdu');
ok(!l[1].classList.contains('done'), 'la 2e tâche est cochée');"#,
                ],
            ),
        },
        Check {
            name: "téléphone : pas de débordement, champ assez large",
            points: 2,
            essential: false,
            kind: phone(
                "index.html",
                &[
                    r#"await type('#new', 'tâche'); await click('#add'); noHScroll(); ok(visible('#new'), 'champ caché');
ok(box('#new').width >= innerWidth * 0.5, 'champ de ' + Math.round(box('#new').width) + ' px');"#,
                ],
            ),
        },
        Check {
            name: "texte très long : passe à la ligne sans déborder",
            points: 1,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"await type('#new', 'mot'.repeat(80) + ' ' + 'texte long '.repeat(20)); await click('#add');
const li = need('#list li'); const lb = li.getBoundingClientRect(), lst = box('#list');
ok(lb.right <= lst.right + 1 && li.scrollWidth <= li.clientWidth + 2, 'la tâche déborde de la liste'); noHScroll();"#,
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 2,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static DASHBOARD: Case = Case {
    key: "dashboard",
    title: "Tableau de bord des ventes",
    family: Family::Visual,
    summary: "12 lignes, total exact, tri, filtre par région, barres SVG proportionnelles aux montants",
    prompt: "data.json contient les ventes d'un magasin (12 lignes). Crée dashboard.html, sans dépendance externe. \
Un fichier HTML ouvert depuis le disque ne peut pas lire data.json avec fetch : recopie les données dans la page. \
1) Un tableau : en-têtes <th> Région, Produit, Montant, Mois dans <thead>, une ligne <tr> par vente dans <tbody>. Un clic sur un en-tête trie selon cette colonne, un 2e clic inverse l'ordre. \
2) #total affiche la somme des montants des lignes affichées. \
3) Un <select id=\"region\"> : une option de valeur vide « Toutes » puis une option par région (valeur = nom de la région) ; il filtre le tableau et met #total à jour. \
4) Un graphique en barres <svg id=\"chart\"> du montant total par région : une barre <rect> par région avec l'attribut data-region=\"<région>\", sa hauteur proportionnelle au montant. \
La page reste utilisable sur téléphone sans que la page entière défile en largeur. Soigne l'apparence.",
    files: &[("data.json", DATA_JSON)],
    generated: None,
    protected: &["data.json"],
    hidden: &[],
    setup: None,
    page: Some("dashboard.html"),
    checks: &[
        Check {
            name: "12 lignes, 4 colonnes",
            points: 3,
            essential: true,
            kind: page(
                "dashboard.html",
                &[
                    r#"const r = $$('tbody tr'); ok(r.length === 12, 'lignes : ' + r.length); ok(r.every((x) => x.children.length >= 4), 'colonnes manquantes');
ok($$('thead th').length >= 4, 'en-têtes : ' + $$('thead th').length);"#,
                ],
            ),
        },
        Check {
            name: "total exact (1 832,50)",
            points: 3,
            essential: true,
            kind: page(
                "dashboard.html",
                &["ok(near(num(txt('#total')), 1832.5), '#total affiche ' + txt('#total'));"],
            ),
        },
        Check {
            name: "tri par montant, 2e clic inverse",
            points: 3,
            essential: false,
            kind: page(
                "dashboard.html",
                &[
                    r#"const th = $$('thead th'); const h = th.find((t) => /montant/i.test(t.textContent)); ok(h, 'pas d’en-tête Montant'); const col = th.indexOf(h);
const vals = () => $$('tbody tr').map((r) => num(r.children[col].textContent));
const asc = (x) => x.every((v, i) => i === 0 || x[i - 1] <= v), desc = (x) => x.every((v, i) => i === 0 || x[i - 1] >= v);
await click(h); const a = vals(); await click(h); const b = vals();
ok((asc(a) && desc(b)) || (desc(a) && asc(b)), 'ordre après 2 clics : ' + a.join(',') + ' puis ' + b.join(','));"#,
                ],
            ),
        },
        Check {
            name: "tri par région (ordre alphabétique)",
            points: 1,
            essential: false,
            kind: page(
                "dashboard.html",
                &[
                    r#"const th = $$('thead th'); const h = th.find((t) => /r[ée]gion/i.test(t.textContent)); ok(h, 'pas d’en-tête Région'); const col = th.indexOf(h);
const vals = () => $$('tbody tr').map((r) => r.children[col].textContent.trim());
const sorted = (x, d) => x.every((v, i) => i === 0 || d * x[i - 1].localeCompare(v, 'fr') <= 0);
await click(h); const a = vals(); await click(h); const b = vals(); ok((sorted(a, 1) && sorted(b, -1)) || (sorted(a, -1) && sorted(b, 1)), 'régions : ' + a.join(','));"#,
                ],
            ),
        },
        Check {
            name: "filtre « Sud » : 3 lignes et total 355,75",
            points: 3,
            essential: false,
            kind: page(
                "dashboard.html",
                &[
                    r#"const s = need('#region'); ok([...s.options].some((o) => o.value === ''), 'pas d’option Toutes (valeur vide)');
s.value = 'Sud'; s.dispatchEvent(new Event('input', { bubbles: true })); s.dispatchEvent(new Event('change', { bubbles: true })); await sleep(100);
const r = $$('tbody tr').filter(visible); ok(r.length === 3, 'lignes affichées : ' + r.length); ok(near(num(txt('#total')), 355.75), 'total filtré : ' + txt('#total'));
s.value = ''; s.dispatchEvent(new Event('change', { bubbles: true })); await sleep(100); ok($$('tbody tr').filter(visible).length === 12, 'retour à Toutes');"#,
                ],
            ),
        },
        Check {
            name: "graphique : 4 barres proportionnelles aux montants",
            points: 4,
            essential: false,
            kind: page(
                "dashboard.html",
                &[
                    r#"ok(visible('svg#chart'), 'pas de svg#chart visible'); const want = { Nord: 475.75, Sud: 355.75, Est: 590.5, Ouest: 410.5 };
const h = {}; for (const r of $$('#chart rect[data-region]')) h[r.getAttribute('data-region')] = r.getBoundingClientRect().height;
for (const k of Object.keys(want)) ok(h[k] > 0, 'barre ' + k + ' absente ou vide');
const ratio = h.Est / want.Est; for (const k of Object.keys(want)) ok(Math.abs(h[k] / want[k] - ratio) <= ratio * 0.03, 'barre ' + k + ' pas proportionnelle (' + h[k].toFixed(1) + ' px)');"#,
                ],
            ),
        },
        Check {
            name: "téléphone : la page ne défile pas en largeur",
            points: 2,
            essential: false,
            kind: phone("dashboard.html", &["noHScroll();"]),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 1,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static SNAKE: Case = Case {
    key: "snake",
    title: "Jeu Snake",
    family: Family::Visual,
    summary: "dessin sur canvas, déplacements au clavier, le serpent mange et grandit, mur = fin, Entrée relance",
    prompt: "Crée index.html : un jeu Snake dessiné sur un <canvas id=\"game\"> d'au moins 300×300 px, tout dans ce seul fichier, sans dépendance. \
Le serpent démarre seul vers la droite, à au moins 5 cases du bord droit, à une vitesse de 80 à 250 ms par case. Les flèches du clavier (keydown) changent sa direction ; un demi-tour est ignoré. \
Manger la nourriture fait grandir le serpent d'une case et ajoute 1 au score affiché dans #score. Toucher un mur ou lui-même arrête la partie (message visible) ; Entrée relance une partie neuve (score 0). \
Pour les tests automatiques, expose window.__game = { state(), setFood(x, y) } : state() renvoie { snake: [{x, y}, …] (tête en premier, en cases), dir: 'up'|'down'|'left'|'right', \
food: {x, y}, score, over (booléen), cols, rows } ; setFood(x, y) place la nourriture sur cette case. Soigne l'apparence.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html"),
    checks: &[
        Check {
            name: "canvas de 300 px au moins, dessiné, #score visible",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"const c = need('canvas#game'); ok(visible(c), 'canvas caché'); const b = c.getBoundingClientRect(); ok(b.width >= 299 && b.height >= 299, 'canvas de ' + Math.round(b.width) + '×' + Math.round(b.height));
await sleep(300); const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; const colors = new Set();
for (let i = 0; i < d.length && colors.size < 3; i += 4 * 97) colors.add(d[i] + ',' + d[i + 1] + ',' + d[i + 2] + ',' + d[i + 3]);
ok(colors.size >= 2, 'canvas vide'); ok(visible('#score'), '#score caché');"#,
                ],
            ),
        },
        Check {
            name: "window.__game.state() complet",
            points: 2,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"ok(window.__game && typeof __game.state === 'function' && typeof __game.setFood === 'function', 'window.__game absent');
const s = __game.state(); ok(Array.isArray(s.snake) && s.snake.length >= 1 && 'x' in s.snake[0], 'snake mal formé');
ok(s.cols >= 10 && s.rows >= 10, 'grille ' + s.cols + '×' + s.rows); ok(typeof s.over === 'boolean' && typeof s.score === 'number', 'over/score mal formés');
ok(s.food && 'x' in s.food, 'food mal formé');"#,
                ],
            ),
        },
        Check {
            name: "avance seul vers la droite",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const a = __game.state().snake[0]; await sleep(900); const s = __game.state(); ok(!s.over, 'partie finie toute seule');
ok(s.snake[0].x > a.x && s.snake[0].y === a.y, 'tête de ' + JSON.stringify(a) + ' à ' + JSON.stringify(s.snake[0]));"#,
                ],
            ),
        },
        Check {
            name: "flèche bas : la tête descend",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"await sleep(300); const a = __game.state().snake[0]; await key('ArrowDown', document.body); await sleep(700); const s = __game.state();
ok(s.dir === 'down', 'direction : ' + s.dir); ok(s.snake[0].y > a.y, 'tête de ' + JSON.stringify(a) + ' à ' + JSON.stringify(s.snake[0]));"#,
                ],
            ),
        },
        Check {
            name: "mange : score +1, serpent +1, #score à jour",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const s0 = __game.state(); const h = s0.snake[0]; __game.setFood(h.x + 2, h.y);
await until(() => __game.state().score > s0.score || __game.state().over, 3000, 'la nourriture n’est pas mangée');
await sleep(400); const s = __game.state(); ok(!s.over, 'partie finie'); ok(s.score === s0.score + 1, 'score ' + s.score);
ok(s.snake.length === s0.snake.length + 1, 'longueur ' + s0.snake.length + ' → ' + s.snake.length); ok(txt('#score').includes(String(s.score)), '#score affiche ' + txt('#score'));"#,
                ],
            ),
        },
        Check {
            name: "mur à droite : la partie s'arrête",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const s0 = __game.state(); __game.setFood(0, s0.rows - 1); await until(() => __game.state().over, 15000, 'pas de fin de partie au mur');
ok(__game.state().snake[0].x >= s0.cols - 2, 'fin de partie loin du mur');"#,
                ],
            ),
        },
        Check {
            name: "Entrée relance une partie neuve",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"const s0 = __game.state(); __game.setFood(0, s0.rows - 1); await until(() => __game.state().over, 15000, 'pas de fin de partie');
await key('Enter', document.body); await sleep(300); const s = __game.state(); ok(!s.over, 'toujours finie après Entrée'); ok(s.score === 0, 'score ' + s.score);
await sleep(500); ok(!__game.state().over, 'repartie finie aussitôt');"#,
                ],
            ),
        },
        Check {
            name: "demi-tour ignoré",
            points: 1,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"await sleep(300); const a = __game.state().snake[0]; await key('ArrowLeft', document.body); await sleep(600); const s = __game.state();
ok(!s.over, 'le demi-tour tue le serpent'); ok(s.snake[0].x > a.x, 'le serpent a fait demi-tour');"#,
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 1,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static SIGNUP: Case = Case {
    key: "signup",
    title: "Formulaire d'inscription",
    family: Family::Visual,
    summary: "messages d'erreur sous les champs, bouton bloqué tant que c'est invalide, envoi sans rechargement",
    prompt: "Crée index.html : un formulaire d'inscription, tout dans ce seul fichier, JavaScript natif. Champs : #name (2 caractères au moins), #email (forme x@y.z), \
#password (8 caractères au moins dont au moins une lettre et un chiffre), #confirm (identique au mot de passe), une case #terms (accepter les conditions) et un bouton #submit. \
Sous chaque champ, un élément #name-error, #email-error, #password-error, #confirm-error affiche un message clair dès que le champ a été modifié et qu'il est invalide ; \
il est vide ou caché quand le champ est valide. #submit est désactivé (disabled) tant qu'un champ est invalide ou que la case n'est pas cochée. \
À l'envoi, la page ne se recharge pas : le formulaire disparaît et un élément #success affiche un message de bienvenue avec le nom saisi. \
Sur téléphone, les champs prennent presque toute la largeur et rien ne dépasse. Soigne l'apparence.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html"),
    checks: &[
        Check {
            name: "champs et bouton visibles, bouton désactivé au départ",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"for (const s of ['#name','#email','#password','#confirm','#terms','#submit']) ok(visible(s), s + ' absent ou caché');
ok(need('#submit').disabled, '#submit actif au départ');"#,
                ],
            ),
        },
        Check {
            name: "email invalide : message ; corrigé : message retiré",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[r#"const shown = (s) => visible(s) && txt(s).length > 0;
await type('#email', 'pas-un-email'); ok(shown('#email-error'), 'pas de message pour « pas-un-email »');
await type('#email', 'a@b'); ok(shown('#email-error'), 'pas de message pour « a@b »');
await type('#email', 'marie@exemple.fr'); ok(!shown('#email-error'), 'message encore là pour un email valide');"#],
            ),
        },
        Check {
            name: "mot de passe : 8 caractères, une lettre, un chiffre",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[r#"const shown = (s) => visible(s) && txt(s).length > 0;
for (const bad of ['abc1', 'abcdefgh', '12345678']) { await type('#password', bad); ok(shown('#password-error'), 'pas de message pour « ' + bad + ' »'); }
await type('#password', 'abcdefg1'); ok(!shown('#password-error'), 'message pour un mot de passe valide');"#],
            ),
        },
        Check {
            name: "confirmation différente : message",
            points: 2,
            essential: false,
            kind: page(
                "index.html",
                &[r#"const shown = (s) => visible(s) && txt(s).length > 0;
await type('#password', 'abcdefg1'); await type('#confirm', 'abcdefg2'); ok(shown('#confirm-error'), 'pas de message');
await type('#confirm', 'abcdefg1'); ok(!shown('#confirm-error'), 'message pour une confirmation juste');"#],
            ),
        },
        Check {
            name: "bouton actif seulement quand tout est valide et la case cochée",
            points: 3,
            essential: false,
            kind: page(
                "index.html",
                &[
                    r#"await type('#name', 'Marie'); await type('#email', 'marie@exemple.fr'); await type('#password', 'abcdefg1'); await type('#confirm', 'abcdefg1');
ok(need('#submit').disabled, 'actif sans la case cochée'); await click('#terms'); ok(!need('#submit').disabled, 'désactivé alors que tout est valide');
await type('#name', 'M'); ok(need('#submit').disabled, 'actif avec un nom d’une lettre');"#,
                ],
            ),
        },
        Check {
            name: "envoi : message de bienvenue avec le nom, sans rechargement",
            points: 3,
            essential: true,
            kind: page(
                "index.html",
                &[
                    r#"window.__still = 1; await type('#name', 'Marie Curie'); await type('#email', 'marie@exemple.fr'); await type('#password', 'abcdefg1'); await type('#confirm', 'abcdefg1');
await click('#terms'); await click('#submit'); await sleep(300); ok(window.__still === 1, 'la page s’est rechargée');
ok(visible('#success') && txt('#success').includes('Marie'), '#success : ' + ($('#success') ? txt('#success') : 'absent'));
ok(!visible('#name'), 'le formulaire est encore affiché');"#,
                ],
            ),
        },
        Check {
            name: "téléphone : champs larges, rien ne dépasse",
            points: 2,
            essential: false,
            kind: phone(
                "index.html",
                &[
                    "noHScroll(); for (const s of ['#name','#email','#password','#confirm']) ok(box(s).width >= innerWidth * 0.6, s + ' : ' + Math.round(box(s).width) + ' px');",
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 1,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

pub static CALENDAR: Case = Case {
    key: "calendar",
    title: "Calendrier mensuel",
    family: Family::Visual,
    summary: "grille lundi-dimanche juste, jour du jour, navigation entre les mois, années bissextiles, sélection d'un jour",
    prompt: "Crée index.html : un calendrier mensuel, tout dans ce seul fichier, JavaScript natif. \
La date du jour vient de l'adresse si elle porte ?date=AAAA-MM-JJ (sinon la vraie date) ; le calendrier s'ouvre sur ce mois. \
#title affiche le mois et l'année en français (« novembre 2026 »). La grille a 7 colonnes, de lundi à dimanche, avec les noms des jours en tête. \
Chaque jour du mois affiché est un élément de classe « day » avec l'attribut data-date=\"AAAA-MM-JJ\" ; les cases vides avant le 1er n'ont pas la classe day. \
Le jour du jour porte la classe « today », les samedis et dimanches la classe « weekend ». Les boutons #prev et #next passent au mois précédent / suivant. \
Un clic sur un jour lui met la classe « selected » (retirée des autres) et #details affiche la date en toutes lettres (« lundi 23 novembre 2026 »). \
Sur téléphone, la grille garde ses 7 colonnes sans dépasser de l'écran. Soigne l'apparence.",
    files: &[],
    generated: None,
    protected: &[],
    hidden: &[],
    setup: None,
    page: Some("index.html?date=2026-11-23"),
    checks: &[
        Check {
            name: "novembre 2026 : titre et 30 jours",
            points: 3,
            essential: true,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"ok(/novembre\s+2026/i.test(txt('#title')), '#title : ' + txt('#title'));
const d = $$('.day[data-date]').filter((e) => e.dataset.date.startsWith('2026-11-')); ok(d.length === 30, 'jours de novembre : ' + d.length);
ok(!$$('.day[data-date]').some((e) => !/^\d{4}-\d\d-\d\d$/.test(e.dataset.date)), 'data-date mal formé');"#,
                ],
            ),
        },
        Check {
            name: "grille juste : le 1er est un dimanche, le 2 un lundi en 1re colonne",
            points: 4,
            essential: true,
            kind: page(
                "index.html?date=2026-11-23",
                &[r#"const b = (d) => box(`.day[data-date="2026-11-${d}"]`);
ok(Math.abs(b('02').left - b('09').left) < 2 && Math.abs(b('01').left - b('08').left) < 2, 'colonnes irrégulières');
const xs = $$('.day[data-date^="2026-11-"]').map((e) => e.getBoundingClientRect().left); ok(Math.abs(Math.min(...xs) - b('02').left) < 2, 'le lundi 2 n’est pas en 1re colonne');
ok(Math.abs(Math.max(...xs) - b('01').left) < 2, 'le dimanche 1er n’est pas en dernière colonne'); ok(b('01').top < b('02').top - 2, 'le 1er et le 2 sur la même ligne');
ok(Math.abs(b('02').top - b('08').top) < 2, 'semaine du 2 au 8 pas sur une ligne');"#],
            ),
        },
        Check {
            name: "jour du jour (23) marqué « today », lui seul",
            points: 2,
            essential: false,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"const t = $$('.today'); ok(t.length === 1 && t[0].dataset.date === '2026-11-23', 'today sur : ' + t.map((e) => e.dataset.date).join(','));"#,
                ],
            ),
        },
        Check {
            name: "week-ends marqués",
            points: 1,
            essential: false,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"const w = (d) => need(`.day[data-date="2026-11-${d}"]`).classList.contains('weekend');
ok(w('01') && w('07') && w('08') && !w('02') && !w('06'), 'classe weekend mal placée');"#,
                ],
            ),
        },
        Check {
            name: "#next ×3 : février 2027, 28 jours, le 1er un lundi",
            points: 3,
            essential: false,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"for (let i = 0; i < 3; i++) await click('#next'); ok(/f[ée]vrier\s+2027/i.test(txt('#title')), '#title : ' + txt('#title'));
const d = $$('.day[data-date^="2027-02-"]'); ok(d.length === 28, 'jours : ' + d.length);
const xs = d.map((e) => e.getBoundingClientRect().left); ok(Math.abs(Math.min(...xs) - box('.day[data-date="2027-02-01"]').left) < 2, 'le 1er février 2027 n’est pas en colonne lundi');"#,
                ],
            ),
        },
        Check {
            name: "#prev : octobre 2026 (31 jours) ; février 2028 bissextile (29 jours)",
            points: 2,
            essential: false,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"await click('#prev'); ok(/octobre\s+2026/i.test(txt('#title')), '#title : ' + txt('#title')); ok($$('.day[data-date^="2026-10-"]').length === 31, 'octobre incomplet');
location.href = location.pathname + '?date=2028-02-10';"#,
                    "@wait",
                    r#"ok(/f[ée]vrier\s+2028/i.test(txt('#title')), '#title : ' + txt('#title')); ok($$('.day[data-date^="2028-02-"]').length === 29, 'février 2028 : ' + $$('.day[data-date^="2028-02-"]').length + ' jours');"#,
                ],
            ),
        },
        Check {
            name: "clic sur un jour : sélection et date en toutes lettres",
            points: 2,
            essential: false,
            kind: page(
                "index.html?date=2026-11-23",
                &[
                    r#"await click('.day[data-date="2026-11-10"]'); await click('.day[data-date="2026-11-23"]');
const s = $$('.selected'); ok(s.length === 1 && s[0].dataset.date === '2026-11-23', 'selected sur : ' + s.map((e) => e.dataset.date).join(','));
ok(/lundi\s+23\s+novembre\s+2026/i.test(txt('#details')), '#details : ' + txt('#details'));"#,
                ],
            ),
        },
        Check {
            name: "téléphone : 7 colonnes sans débordement",
            points: 2,
            essential: false,
            kind: phone(
                "index.html?date=2026-11-23",
                &[
                    r#"noHScroll(); const b = (d) => box(`.day[data-date="2026-11-${d}"]`); ok(Math.abs(b('02').top - b('08').top) < 2, 'la semaine ne tient plus sur une ligne');
ok(b('08').right <= innerWidth + 1, 'le dimanche dépasse de l’écran');"#,
                ],
            ),
        },
        Check {
            name: "aucune erreur dans la console",
            points: 1,
            essential: false,
            kind: CONSOLE,
        },
    ],
    timeout_s: 1200,
};

const DATA_JSON: &str = include_str!("../data/dashboard.json");
