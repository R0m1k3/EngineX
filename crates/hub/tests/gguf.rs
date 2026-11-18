//! Lecteur GGUF sur des fichiers fabriqués en mémoire.

use enginex_hub::gguf::{GgufError, is_mmproj, parse, read_file};

/// Écrit un GGUF v3 minimal : en-tête + paires clé/valeur.
struct Gguf {
    kv: Vec<u8>,
    n: u64,
}

impl Gguf {
    fn new() -> Self {
        Self { kv: Vec::new(), n: 0 }
    }
    fn key(&mut self, k: &str, t: u32) {
        self.kv.extend((k.len() as u64).to_le_bytes());
        self.kv.extend(k.as_bytes());
        self.kv.extend(t.to_le_bytes());
        self.n += 1;
    }
    fn str(mut self, k: &str, v: &str) -> Self {
        self.key(k, 8);
        self.kv.extend((v.len() as u64).to_le_bytes());
        self.kv.extend(v.as_bytes());
        self
    }
    fn u32(mut self, k: &str, v: u32) -> Self {
        self.key(k, 4);
        self.kv.extend(v.to_le_bytes());
        self
    }
    fn u64(mut self, k: &str, v: u64) -> Self {
        self.key(k, 10);
        self.kv.extend(v.to_le_bytes());
        self
    }
    fn f32(mut self, k: &str, v: f32) -> Self {
        self.key(k, 6);
        self.kv.extend(v.to_le_bytes());
        self
    }
    fn i32_array(mut self, k: &str, v: &[i32]) -> Self {
        self.key(k, 9);
        self.kv.extend(5u32.to_le_bytes());
        self.kv.extend((v.len() as u64).to_le_bytes());
        for x in v {
            self.kv.extend(x.to_le_bytes());
        }
        self
    }
    fn str_array(mut self, k: &str, n: usize) -> Self {
        self.key(k, 9);
        self.kv.extend(8u32.to_le_bytes());
        self.kv.extend((n as u64).to_le_bytes());
        for i in 0..n {
            let s = format!("tok{i}");
            self.kv.extend((s.len() as u64).to_le_bytes());
            self.kv.extend(s.as_bytes());
        }
        self
    }
    fn bytes(&self) -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes()); // tenseurs
        b.extend(self.n.to_le_bytes());
        b.extend(&self.kv);
        b.extend([0u8; 64]); // table des tenseurs, poids : jamais lus
        b
    }
}

fn qwen() -> Gguf {
    Gguf::new()
        .str("general.architecture", "qwen3")
        .str("general.name", "Qwen3 0.6B")
        .str("general.size_label", "0.6B")
        .u32("general.file_type", 7)
        .u32("qwen3.context_length", 40960)
        .u32("qwen3.block_count", 28)
        .u32("qwen3.embedding_length", 1024)
        .u32("qwen3.attention.head_count", 16)
        .u32("qwen3.attention.head_count_kv", 8)
        .u32("qwen3.attention.key_length", 128)
        .u32("qwen3.attention.value_length", 128)
        .f32("general.sampling.temp", 0.6)
        .str("tokenizer.chat_template", &"{% x %}".repeat(5000))
        .str_array("tokenizer.ggml.tokens", 3000)
        .u64("split.count", 1)
}

#[test]
fn reads_metadata() {
    let m = parse(&qwen().bytes()[..]).unwrap();
    assert_eq!(m.arch, "qwen3");
    assert_eq!(m.name.as_deref(), Some("Qwen3 0.6B"));
    assert_eq!(m.file_type, Some(7));
    assert_eq!(m.context_length, Some(40960));
    assert_eq!(m.block_count, Some(28));
    assert_eq!(m.head_count_kv, Some(8));
    assert_eq!((m.key_length, m.value_length), (Some(128), Some(128)));
    assert_eq!(m.vocab_size, Some(3000));
    assert!(m.has_chat_template);
    assert_eq!(m.split_count, Some(1));
    assert!((m.sampling["temp"] - 0.6).abs() < 1e-6);
    assert!(!is_mmproj(&m, "Qwen3-0.6B-Q8_0.gguf"));
}

#[test]
fn kv_heads_per_layer_keeps_max() {
    let g = Gguf::new()
        .str("general.architecture", "gemma3")
        .i32_array("gemma3.attention.head_count_kv", &[4, 8, 4, 16]);
    assert_eq!(parse(&g.bytes()[..]).unwrap().head_count_kv, Some(16));
}

#[test]
fn missing_keys_are_none() {
    let m = parse(&Gguf::new().str("general.architecture", "llama").bytes()[..]).unwrap();
    assert_eq!((m.block_count, m.vocab_size), (None, None));
    assert!(!m.has_chat_template);
}

#[test]
fn truncated_and_bad_magic() {
    let b = qwen().bytes();
    for cut in [3, 20, 200, b.len() - 64 - 10] {
        assert_eq!(parse(&b[..cut]), Err(GgufError::Truncated), "coupé à {cut}");
    }
    assert_eq!(parse(&b"GGML1234"[..]), Err(GgufError::NotGguf));
}

#[test]
fn abusive_lengths() {
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(1u64.to_le_bytes());
    b.extend(u64::MAX.to_le_bytes()); // longueur de clé
    assert_eq!(parse(&b[..]), Err(GgufError::TooLarge("chaîne")));
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(u64::MAX.to_le_bytes()); // nombre de clés
    assert_eq!(parse(&b[..]), Err(GgufError::TooLarge("nombre de clés")));
}

#[test]
fn mmproj_and_file() {
    let g = Gguf::new().str("general.architecture", "clip");
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.gguf");
    std::fs::write(&p, g.bytes()).unwrap();
    let m = read_file(&p).unwrap();
    assert!(is_mmproj(&m, "x.gguf"));
    let q = parse(&qwen().bytes()[..]).unwrap();
    assert!(is_mmproj(&q, "unsloth/mmproj-F16.gguf"));
}
