//! Lecteur GGUF sur des fichiers fabriqués en mémoire.

use enginex_hub::gguf::{GgufError, is_mmproj, parse, read_file, read_shards};

/// Écrit un GGUF v3 minimal : en-tête + paires clé/valeur.
struct Gguf {
    kv: Vec<u8>,
    n: u64,
    tensors: Vec<String>,
}

impl Gguf {
    fn new() -> Self {
        Self {
            kv: Vec::new(),
            n: 0,
            tensors: Vec::new(),
        }
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
    fn tensor(mut self, name: &str) -> Self {
        self.tensors.push(name.into());
        self
    }
    fn bytes(&self) -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend((self.tensors.len() as u64).to_le_bytes());
        b.extend(self.n.to_le_bytes());
        b.extend(&self.kv);
        for t in &self.tensors {
            b.extend((t.len() as u64).to_le_bytes());
            b.extend(t.as_bytes());
            b.extend(2u32.to_le_bytes()); // dimensions
            b.extend(4096u64.to_le_bytes());
            b.extend(4096u64.to_le_bytes());
            b.extend(8u32.to_le_bytes()); // type
            b.extend(0u64.to_le_bytes()); // décalage
        }
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
    assert_eq!((m.expert_count, m.nextn_predict_layers), (None, None));
}

#[test]
fn moe_and_mtp_keys() {
    // Clés lues par llama.cpp b12464 : `%s.expert_count`, `%s.expert_used_count`,
    // `%s.nextn_predict_layers`.
    let g = Gguf::new()
        .str("general.architecture", "qwen35moe")
        .u32("qwen35moe.expert_count", 256)
        .u32("qwen35moe.expert_used_count", 8)
        .u32("qwen35moe.nextn_predict_layers", 1);
    let m = parse(&g.bytes()[..]).unwrap();
    assert_eq!(
        (m.expert_count, m.expert_used_count, m.nextn_predict_layers),
        (Some(256), Some(8), Some(1))
    );
    // 0 = pas de MTP (modèle converti sans).
    let g = Gguf::new()
        .str("general.architecture", "qwen35")
        .u32("qwen35.nextn_predict_layers", 0);
    assert_eq!(parse(&g.bytes()[..]).unwrap().nextn_predict_layers, None);
    // Ancien JSON stocké en base, sans ces champs.
    let old: enginex_hub::gguf::GgufMeta = serde_json::from_str(r#"{"arch":"llama","name":null,"size_label":null,"file_type":null,"context_length":null,"block_count":null,"embedding_length":null,"head_count":null,"head_count_kv":null,"key_length":null,"value_length":null,"sliding_window":null,"vocab_size":null,"has_chat_template":false,"split_count":null,"sampling":{}}"#).unwrap();
    assert_eq!(old.nextn_predict_layers, None);
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

#[test]
fn mtp_needs_the_last_block_tensor() {
    let base = || {
        Gguf::new()
            .str("general.architecture", "qwen35")
            .u32("qwen35.block_count", 4)
            .u32("qwen35.nextn_predict_layers", 1)
            .tensor("blk.0.attn_q.weight")
    };
    // Clé sans les poids MTP (GGUF converti sans) : pas de MTP.
    let m = parse(&base().bytes()[..]).unwrap();
    assert_eq!(
        (m.nextn_predict_layers, m.mtp_block, m.has_mtp()),
        (Some(1), None, false)
    );
    // Tenseur d'un autre bloc que le dernier : pas de MTP non plus.
    let m = parse(&base().tensor("blk.2.nextn.eh_proj.weight").bytes()[..]).unwrap();
    assert!(!m.has_mtp());
    let m = parse(&base().tensor("blk.3.nextn.eh_proj.weight").bytes()[..]).unwrap();
    assert!(m.has_mtp());
    // En-tête coupé dans la table des tenseurs : métadonnées lues quand même.
    let full = base().tensor("blk.3.nextn.eh_proj.weight").bytes();
    let cut = &full[..full.len() - 120];
    let m = parse(cut).unwrap();
    assert_eq!((m.block_count, m.has_mtp()), (Some(4), false));
    // Fragments : le tenseur MTP est dans le second fichier.
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("m-00001-of-00002.gguf");
    let b = dir.path().join("m-00002-of-00002.gguf");
    std::fs::write(&a, base().bytes()).unwrap();
    std::fs::write(
        &b,
        Gguf::new()
            .u32("split.no", 1)
            .tensor("blk.3.nextn.eh_proj.weight")
            .bytes(),
    )
    .unwrap();
    assert!(!read_file(&a).unwrap().has_mtp());
    assert!(read_shards(&[a, b]).unwrap().has_mtp());
}
