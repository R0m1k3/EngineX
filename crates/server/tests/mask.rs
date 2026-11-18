//! Masquage : remplacement, valeurs courtes ignorées, chevauchements, flux
//! coupés à toutes les positions.

use enginex_server::mask::{Masker, Stream};

#[test]
fn replaces_exact_values_only() {
    let m = Masker::new(["ghp_secret_1".to_string(), "court".to_string()]);
    assert_eq!(m.mask("a ghp_secret_1 b ghp_secret_1"), "a *** b ***");
    assert_eq!(m.mask("court reste"), "court reste", "moins de 6 caractères");
    assert_eq!(m.mask("GHP_SECRET_1"), "GHP_SECRET_1", "casse changée non reconnue");
    assert!(Masker::new(Vec::<String>::new()).is_empty());
}

#[test]
fn longest_of_overlapping_values_wins() {
    let m = Masker::new(["abcdef".to_string(), "abcdefgh".to_string(), "efghij".to_string()]);
    assert_eq!(m.mask("x abcdefgh y"), "x *** y");
    assert_eq!(m.mask("x abcdefghij y"), "x ***ij y");
}

fn fragments(secret: &str) -> Vec<String> {
    let c: Vec<char> = secret.chars().collect();
    let mut out = Vec::new();
    for i in 0..c.len() {
        for j in i + 3..=c.len() {
            out.push(c[i..j].iter().collect());
        }
    }
    out
}

fn check(m: &Masker, secret: &str, text: &str, cuts: &[usize]) {
    let whole = m.mask(text).into_owned();
    let mut s = Stream::default();
    let mut out = Vec::new();
    let mut prev = 0;
    for &c in cuts.iter().chain(std::iter::once(&text.len())) {
        out.push(s.push(m, &text[prev..c]));
        prev = c;
    }
    out.push(s.flush(m));
    assert_eq!(out.concat(), whole, "{text:?} coupé en {cuts:?}");
    let frags = fragments(secret);
    for piece in &out {
        for f in &frags {
            assert!(
                !piece.contains(f.as_str()) || whole.contains(f.as_str()),
                "{f:?} sorti dans {piece:?} ({text:?} coupé en {cuts:?})"
            );
        }
    }
}

#[test]
fn streamed_secrets_never_leave_in_pieces() {
    let secrets = ["ghp_abcdef123", "s3cr3t!xé", "abcdefgh"];
    let m = Masker::new(secrets.iter().map(|s| s.to_string()).chain(["efghijkl".to_string()]));
    for secret in secrets {
        let text = format!("avant {secret} après {secret}");
        let bounds: Vec<usize> = (1..text.len()).filter(|i| text.is_char_boundary(*i)).collect();
        for &a in &bounds {
            check(&m, secret, &text, &[a]);
            for &b in bounds.iter().filter(|b| **b > a) {
                check(&m, secret, &text, &[a, b]);
            }
        }
    }
}

#[test]
fn held_text_comes_out_with_the_next_delta() {
    let m = Masker::new(["supersecret".to_string()]);
    let mut s = Stream::default();
    assert_eq!(s.push(&m, "voici super"), "voici ");
    assert_eq!(s.push(&m, "man"), "superman");
    assert_eq!(s.push(&m, " et super"), " et ");
    assert_eq!(s.flush(&m), "super");
}

mod bytes {
    use enginex_server::mask::{ByteStream, Masker};

    /// Octets coupés partout (y compris au milieu d'un caractère UTF-8) :
    /// même résultat que le masquage d'un bloc, aucun fragment ne sort.
    #[test]
    fn cut_anywhere_gives_the_whole_result() {
        let secret = "clé-très-secrète";
        let m = Masker::new([secret.to_string()]);
        let text = format!("é {secret} \x1b[31m{secret}\x1b[0m fin").into_bytes();
        let whole = m.mask_bytes(&text).into_owned();
        for a in 1..text.len() {
            for b in (a + 1..text.len()).step_by(3) {
                let mut s = ByteStream::default();
                let mut out = Vec::new();
                for piece in [&text[..a], &text[a..b], &text[b..]] {
                    let o = s.push(&m, piece);
                    assert!(
                        !o.windows(6).any(|w| secret.as_bytes().windows(6).any(|f| f == w))
                            || whole.windows(o.len()).any(|w| w == o.as_slice()),
                        "fragment sorti en {a}/{b}"
                    );
                    out.extend(o);
                }
                out.extend(s.flush(&m));
                assert_eq!(out, whole, "coupé en {a}/{b}");
            }
        }
    }

    #[test]
    fn invalid_utf8_passes_through() {
        let m = Masker::new(["secret42".to_string()]);
        let mut s = ByteStream::default();
        let mut out = s.push(&m, b"\xff\xfe secret");
        out.extend(s.push(&m, b"42 \xc3"));
        out.extend(s.flush(&m));
        assert_eq!(out, b"\xff\xfe *** \xc3");
    }
}
