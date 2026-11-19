//! Estimation de la VRAM qu'un modèle demande, carte par carte. Fonction pure :
//! le serveur fournit la VRAM de chaque carte. Faite pour se tromper plutôt
//! dans le sens « ne tient pas » (seul le forfait des tampons de calcul peut
//! sous-estimer).

use serde::{Deserialize, Serialize};

use crate::gguf::GgufMeta;

const MIB: u64 = 1 << 20;
/// Tampons de calcul : forfait par carte.
const COMPUTE_PER_CARD: u64 = 512 * MIB;
/// Logits d'un ubatch de 512 jetons, en f32, sur la dernière carte.
const UBATCH: u64 = 512;
/// Vocabulaire inconnu : valeur haute (Gemma 3 en a 262 144).
const VOCAB_FALLBACK: u64 = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KvType {
    F16,
    Q8_0,
    Q4_0,
}

impl KvType {
    /// Octets par élément, en fraction (numérateur, dénominateur).
    fn bytes(self) -> (u64, u64) {
        match self {
            Self::F16 => (2, 1),
            Self::Q8_0 => (34, 32),
            Self::Q4_0 => (18, 32),
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "f16" => Some(Self::F16),
            "q8_0" => Some(Self::Q8_0),
            "q4_0" => Some(Self::Q4_0),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardVram {
    pub total_mib: u64,
    /// VRAM prise par d'autres programmes que le llama-server d'EngineX.
    pub others_mib: u64,
}

pub struct FitInput<'a> {
    pub meta: &'a GgufMeta,
    pub weights_bytes: u64,
    pub mmproj_bytes: u64,
    pub ctx: u64,
    pub parallel: u64,
    pub kv: KvType,
    /// Marge libre par carte (la même que `--fit-target`).
    pub fit_target_mib: u64,
    /// Petit modèle brouillon chargé à côté (0 sans).
    pub draft_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Fits,
    Tight,
    /// MoE trop gros pour les cartes : `--fit` envoie des experts en RAM
    /// (marche, plus lentement).
    Offload,
    NoFit,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardNeed {
    pub need_mib: u64,
    pub avail_mib: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitReport {
    pub cards: Vec<CardNeed>,
    pub verdict: Verdict,
    /// Cache KV total (toutes cartes), pour l'affichage.
    pub kv_mib: u64,
}

/// Octets du cache KV pour `ctx × parallel` jetons, `None` si les
/// métadonnées manquent.
pub fn kv_bytes(meta: &GgufMeta, ctx: u64, parallel: u64, kv: KvType) -> Option<u64> {
    let layers = meta.block_count?;
    let heads_kv = meta.head_count_kv?;
    let head_dim = || Some(meta.embedding_length? / meta.head_count?.max(1));
    let k = meta.key_length.or_else(head_dim)?;
    let v = meta.value_length.or_else(head_dim)?;
    let (num, den) = kv.bytes();
    let per_token = layers * heads_kv * (k + v) * num / den;
    Some(per_token * ctx * parallel.max(1))
}

pub fn estimate(input: &FitInput, cards: &[CardVram]) -> FitReport {
    let unknown = |kv_mib| FitReport {
        cards: Vec::new(),
        verdict: Verdict::Unknown,
        kv_mib,
    };
    let Some(kv) = kv_bytes(input.meta, input.ctx, input.parallel, input.kv) else {
        return unknown(0);
    };
    let avail: Vec<u64> = cards
        .iter()
        .map(|c| c.total_mib.saturating_sub(c.others_mib) * MIB)
        .collect();
    let total_avail: u64 = avail.iter().sum();
    let layers = input.meta.block_count.unwrap_or(1).max(1);
    if total_avail == 0 {
        return unknown(kv / MIB);
    }
    let layer = input.weights_bytes / layers;
    let vocab = input.meta.vocab_size.unwrap_or(VOCAB_FALLBACK);
    let logits = UBATCH * vocab * 4;
    let last = cards.len() - 1;
    let share = |bytes: u64, a: u64| (bytes as u128 * a as u128 / total_avail as u128) as u64;
    let needs: Vec<CardNeed> = avail
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let mut need = share(input.weights_bytes + input.draft_bytes, a)
                + layer
                + share(kv, a)
                + COMPUTE_PER_CARD
                + input.fit_target_mib * MIB;
            if i == 0 {
                need += input.mmproj_bytes;
            }
            if i == last {
                need += logits;
            }
            CardNeed {
                need_mib: need.div_ceil(MIB),
                avail_mib: a / MIB,
            }
        })
        .collect();
    let all = |pct: u64| needs.iter().all(|c| c.need_mib * 100 <= c.avail_mib * pct);
    let verdict = if all(90) {
        Verdict::Fits
    } else if all(100) {
        Verdict::Tight
    } else if input.meta.expert_count.is_some() {
        Verdict::Offload
    } else {
        Verdict::NoFit
    };
    FitReport {
        cards: needs,
        verdict,
        kv_mib: kv / MIB,
    }
}
