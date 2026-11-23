use super::*;

#[test]
fn outils_et_plafond() {
    for n in [
        "browser_open",
        "browser_snapshot",
        "browser_find",
        "browser_click",
        "browser_click_xy",
        "browser_type",
        "browser_key",
        "browser_scroll",
        "browser_screenshot",
    ] {
        assert!(is_tool(n), "{n}");
    }
    assert!(!is_tool("web_screenshot"));
    assert!(!auto_shot_tool("browser_snapshot") && !auto_shot_tool("browser_screenshot"));
    assert!(auto_shot_tool("browser_find"));
    let s = "é".repeat(MAX_OUTPUT);
    assert_eq!(cap_output(&s), s);
    assert_eq!(cap_output(&format!("{s}x")), format!("{s}\n…[tronqué]"));
}

#[test]
fn nombres_comme_le_go() {
    assert_eq!(go_num(640.0).to_string(), "640");
    assert_eq!(go_num(412.5).to_string(), "412.5");
    assert_eq!(go_num(-3.0).to_string(), "-3");
}

#[test]
fn grille() {
    let mut img = image::RgbaImage::from_pixel(250, 150, image::Rgba([40, 80, 120, 255]));
    grid::draw(&mut img);
    let px = |x, y| img.get_pixel(x, y).0;
    assert_eq!(px(0, 0), [40, 80, 120, 255]);
    // 255×130/255 + 40×125/255 → 149 ; 0 + 80×(1−a) → 39 ; 200a + 120(1−a) → 160.
    assert_eq!(px(100, 50), [149, 39, 160, 255]);
    assert_eq!(px(200, 140), [149, 39, 160, 255]);
    assert_eq!(px(50, 100), [149, 39, 160, 255]);
    assert_eq!(px(101, 50), [40, 80, 120, 255]);
    // « 100 » en haut de la ligne : fond sombre, chiffre jaune.
    assert_eq!(px(100, 1), [0, 0, 0, 255]);
    assert_eq!(px(104, 4), [255, 255, 0, 255]);
    // Une image illisible passe telle quelle.
    assert_eq!(grid::overlay_grid(b"pas une image"), b"pas une image");
}

#[test]
fn pages_de_blocage() {
    for t in [
        "# Access Denied\nhttps://boutique.test/",
        "# Just a moment… captcha\nhttps://x.test/",
        "[ok] cliqué [4]\n\n# Accès refusé\nhttps://x.test/",
        "# 404 - Page introuvable\nhttps://x.test/a",
        "# Are you human?\nhttps://x.test/",
        "# Attention Required! | Cloudflare — you have been blocked\nu",
    ] {
        assert!(looks_blocked(t), "{t}");
        assert!(guard(t.to_string(), 1).contains("page d'erreur ou de blocage"));
    }
    for t in [
        "# Boutique — robes d'été\nhttps://x.test/",
        "[ok] cliqué [2]\n\n(page inchangée depuis le dernier affichage — les mêmes numéros restent valides)",
        // Le mot dans le corps, pas dans un titre.
        "# Panier\nhttps://x.test/\n\n[1] link \"Accès refusé ? aide\"",
        // Titre trop loin : au-delà des six premières lignes.
        "a\nb\nc\nd\ne\nf\n# Access denied",
    ] {
        assert!(!looks_blocked(t), "{t}");
    }
    assert_eq!(guard("# Accueil\nu".into(), 3), "# Accueil\nu");
}

/// Le rappel de conclure arrive à la 25e action du tour, et ensuite.
#[test]
fn rappel_de_conclure() {
    assert_eq!(ACTION_BUDGET, 25);
    assert_eq!(guard("[ok]".into(), 24), "[ok]");
    let r = guard("[ok]".into(), 25);
    assert!(r.starts_with("[ok]\n\n⏱ 25 actions dans le navigateur"), "{r}");
    assert!(guard("[ok]".into(), 31).contains("⏱ 31 actions"));
    // Les deux rappels ensemble, le blocage d'abord.
    let r = guard("# Forbidden\nu".into(), 30);
    let (b, c) = (r.find("⚠ Le site").unwrap(), r.find("⏱ 30").unwrap());
    assert!(b < c);
}

/// Même action sans effet : avertie dès la deuxième fois ; un effet, ou une
/// autre action, remet le compte à zéro.
#[test]
fn action_repetee() {
    let mut r = Repeat::default();
    let same = "[ok] cliqué [3]\n\n(page inchangée depuis le dernier affichage — les mêmes numéros restent valides)";
    assert_eq!(r.note("click 3", same.into()), same);
    assert!(r.note("click 3", same.into()).ends_with("ce qui bloque)."));
    assert!(r.note("click 3", same.into()).contains("sans effet 3 fois"));
    // Une autre action sans effet repart de 1.
    assert_eq!(r.note("click 4", same.into()), same);
    let refused = format!("[refusé] [4] {CLICK_DISABLED}");
    assert!(r.note("click 4", refused.clone()).contains("sans effet 2 fois"));
    // Une action qui change la page efface tout.
    assert_eq!(
        r.note("click 4", "[ok] cliqué [4]\n\n# A".into()),
        "[ok] cliqué [4]\n\n# A"
    );
    assert_eq!(r.note("click 4", refused.clone()), refused);
}

/// `browser_find` : éléments (avec leurs états) puis texte simple à lire.
#[test]
fn recherche_elements_et_texte() {
    let refs = vec![
        "[12] radio \"256 Go\" [coché]".to_string(),
        "[13] radio \"512 Go\"".to_string(),
        "[20] button \"Continuer\" [désactivé]".to_string(),
    ];
    let texts = vec!["Livraison estimée : 12 nov.".to_string()];
    assert_eq!(
        format_find("go", &refs, &[]),
        "3 élément(s) contenant « go » (le 1er est amené dans la vue) — clique par numéro :\n[12] radio \"256 Go\" [coché]\n[13] radio \"512 Go\"\n[20] button \"Continuer\" [désactivé]"
    );
    assert_eq!(
        format_find("livraison", &[], &texts),
        "Texte de la page contenant « livraison » (à lire, pas à cliquer) :\n- Livraison estimée : 12 nov."
    );
    let both = format_find("x", &refs[..1], &[texts[0].clone(), "Prix : 9 €".into()]);
    assert!(both.contains("[coché]\n\nTexte de la page contenant « x »"), "{both}");
    assert!(both.ends_with(":\n- Livraison estimée : 12 nov.\n- Prix : 9 €"));
    assert!(format_find("zz", &[], &[]).starts_with("Rien ne contient « zz » sur cette page"));
}

/// Les scripts injectés : la case cachée passe par sa cible visible, et
/// chaque ligne de la liste porte l'état de l'élément.
#[test]
fn scripts_cases_cachees() {
    let snap = js::snapshot_js();
    assert!(snap.contains("function ajTarget(el)"));
    assert!(snap.contains("ajActionable(el)&&ajVis(ajTarget(el))"));
    assert!(snap.contains("var r=ajTarget(el).getBoundingClientRect()"));
    assert!(snap.contains("+ajState(el));"));
    for st in ["' [coché]'", "' [sélectionné]'", "' [désactivé]'"] {
        assert!(js::HELPERS_JS.contains(st), "{st}");
    }
    assert!(js::FIND_JS.contains("return {refs:refs,texts:texts};"));
    assert!(js::FIND_JS.contains("+ajState(el));"));
}

#[tokio::test]
async fn navigateur_absent() {
    let b = Browser::new(None);
    assert!(!b.enabled());
    let out = b.exec("s", "browser_open", &Map::new(), true).await.unwrap();
    assert!(out.result.starts_with("[erreur]"), "{}", out.result);
    let b = Browser::new(Some("http://127.0.0.1:9".into()));
    let mut args = Map::new();
    args.insert("url".into(), json!("data:text/html,x"));
    let out = b.exec("s", "browser_open", &args, true).await.unwrap();
    assert!(out.result.contains("injoignable"), "{}", out.result);
    assert!(b.exec("s", "web_read", &args, true).await.is_none());
}

#[tokio::test]
async fn prise_de_main() {
    let b = Browser::new(Some("http://127.0.0.1:9".into()));
    b.set_taken("s", true);
    let out = b.exec("s", "browser_snapshot", &Map::new(), true).await.unwrap();
    assert!(out.result.starts_with("[attente]"), "{}", out.result);
    b.set_taken("s", false);
    assert!(!b.taken("s"));
}

#[tokio::test]
async fn adresse_resolue_en_ip() {
    assert_eq!(resolve("http://localhost:9222").await.unwrap(), "http://127.0.0.1:9222");
    assert!(resolve("ws://x").await.is_err());
}

/// Vrai Chromium (`ENGINEX_TEST_BROWSER=http://127.0.0.1:9222`, job CI) :
/// page `data:`, snapshot, saisie, clic, capture.
#[tokio::test]
#[ignore = "lancé par la CI avec un vrai Chromium (ENGINEX_TEST_BROWSER)"]
async fn vrai_chromium() {
    let url = std::env::var("ENGINEX_TEST_BROWSER").expect("ENGINEX_TEST_BROWSER");
    let b = Browser::new(Some(url));
    assert!(
        b.version().await.unwrap()["Browser"]
            .as_str()
            .unwrap()
            .contains("Chrome")
    );
    let html = "<title>Essai</title><input id=q placeholder=Nom><button onclick=\"document.title='ok:'+q.value\">Envoyer</button>";
    let enc: String = html
        .chars()
        .map(|c| match c {
            ' ' | '"' | '<' | '>' | '\'' | '+' | '#' | '%' => format!("%{:02X}", c as u32),
            c => c.to_string(),
        })
        .collect();
    let page = format!("data:text/html,{enc}");
    let mut a = Map::new();
    a.insert("url".into(), json!(page));
    let out = b.exec("s1", "browser_open", &a, true).await.unwrap();
    assert!(out.result.contains("Envoyer"), "{}", out.result);
    let re = regex::Regex::new(r"\[(\d+)\][^\n]*(Nom|textbox)").unwrap();
    let input: i64 = re.captures(&out.result).expect("champ numéroté")[1].parse().unwrap();
    let mut t = Map::new();
    t.insert("ref".into(), json!(input));
    t.insert("text".into(), json!("Michael"));
    b.exec("s1", "browser_type", &t, true).await.unwrap();
    let re = regex::Regex::new(r"\[(\d+)\][^\n]*Envoyer").unwrap();
    let btn: i64 = re.captures(&out.result).expect("bouton numéroté")[1].parse().unwrap();
    let mut c = Map::new();
    c.insert("ref".into(), json!(btn));
    b.exec("s1", "browser_click", &c, true).await.unwrap();
    let s = b.get("s1").await.unwrap();
    let title = s.eval("document.title").await.unwrap().unwrap();
    assert_eq!(title, json!("ok:Michael"));
    let shot = b.exec("s1", "browser_screenshot", &Map::new(), true).await.unwrap();
    let png = shot.image.expect("capture");
    assert_eq!(&png[1..4], b"PNG");
    // Regarder : une image du screencast arrive.
    let mut rx = s.events.subscribe();
    start_screencast(&s).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let m = rx.recv().await.unwrap();
            if m["method"] == "Page.screencastFrame" {
                return m;
            }
        }
    })
    .await
    .expect("image du screencast");
    assert!(frame["params"]["data"].as_str().unwrap().len() > 100);
    stop_screencast(&s).await;
    // Deux sessions, deux onglets.
    b.exec("s2", "browser_open", &a, true).await.unwrap();
    assert!(!Arc::ptr_eq(&b.get("s1").await.unwrap(), &b.get("s2").await.unwrap()));
    b.close("s1").await;
    b.close("s2").await;
}

#[test]
fn console_lines() {
    use serde_json::json;
    let ex = json!({"exceptionDetails": {"text": "Uncaught", "lineNumber": 11, "url": "http://h/app.js",
        "exception": {"description": "TypeError: x is not a function\n    at http://h/app.js:12:3"}}});
    assert_eq!(
        console_line("Runtime.exceptionThrown", &ex).unwrap(),
        "exception : TypeError: x is not a function (http://h/app.js:12)"
    );
    let call = json!({"type": "error", "args": [{"type": "string", "value": "oups"}, {"type": "number", "value": 42},
        {"type": "object", "description": "Error: boum\n    at f"}],
        "stackTrace": {"callFrames": [{"url": "http://h/", "lineNumber": 0}]}});
    assert_eq!(
        console_line("Runtime.consoleAPICalled", &call).unwrap(),
        "console.error : oups 42 Error: boum (http://h/:1)"
    );
    assert!(console_line("Runtime.consoleAPICalled", &json!({"type": "log", "args": []})).is_none());
    let log = json!({"entry": {"level": "error", "url": "http://h/absent.js",
        "text": "Failed to load resource: the server responded with a status of 404 (Not Found)"}});
    assert_eq!(
        console_line("Log.entryAdded", &log).unwrap(),
        "Failed to load resource: the server responded with a status of 404 (Not Found) (http://h/absent.js)"
    );
    assert!(console_line("Log.entryAdded", &json!({"entry": {"level": "warning", "text": "x"}})).is_none());
    let icon = json!({"entry": {"level": "error", "url": "http://h/favicon.ico", "text": "Failed to load resource"}});
    assert!(console_line("Log.entryAdded", &icon).is_none());
    assert!(console_line("Page.loadEventFired", &json!({})).is_none());
    let long = json!({"type": "error", "args": [{"value": "a".repeat(1000)}]});
    assert!(console_line("Runtime.consoleAPICalled", &long).unwrap().chars().count() < 330);
}

#[test]
fn console_block() {
    assert_eq!(format_console(&[]), "");
    let lines: Vec<String> = ["a", "a", "a", "b"].iter().map(|s| s.to_string()).collect();
    assert_eq!(
        format_console(&lines),
        "Erreurs de la page (console du navigateur) :\n- a (×3)\n- b"
    );
    let many: Vec<String> = (0..20).map(|i| format!("e{i}")).collect();
    let out = format_console(&many);
    assert!(
        out.contains("- e14") && !out.contains("- e15") && out.ends_with("… et 5 autre(s)"),
        "{out}"
    );
}

/// Vrai Chromium : pages servies sans réseau (`serve_local`), feuille de
/// style relative et `localStorage` sur l'origine fictive.
#[tokio::test]
#[ignore = "lancé par la CI avec un vrai Chromium (ENGINEX_TEST_BROWSER)"]
async fn real_chromium_serves_local_pages() {
    let url = std::env::var("ENGINEX_TEST_BROWSER").expect("ENGINEX_TEST_BROWSER");
    let ep = resolve(&url).await.unwrap();
    let s = Session::attach(&ep).await.unwrap();
    let files: LocalFiles = Arc::new(|path: &str| match path {
        "/index.html" => (
            200,
            "text/html".into(),
            b"<link rel=stylesheet href=s.css><p id=p>ok</p><script>localStorage.x='1'</script>".to_vec(),
        ),
        "/s.css" => (200, "text/css".into(), b"#p{color:rgb(1,2,3)}".to_vec()),
        _ => (404, "text/plain".into(), b"non".to_vec()),
    });
    s.serve_local("http://bench.enginex.test", files).await.unwrap();
    s.navigate("http://bench.enginex.test/index.html").await.unwrap();
    let v = s
        .eval("getComputedStyle(document.getElementById('p')).color + '|' + localStorage.x")
        .await
        .unwrap();
    assert_eq!(v, Some(json!("rgb(1, 2, 3)|1")));
    s.close().await;
}
