//! Estimation de la VRAM qu'un modèle demande, carte par carte. Fonction pure :
//! le serveur fournit la VRAM de chaque carte. Faite pour se tromper plutôt
//! dans le sens « ne tient pas » (seul le forfait des tampons de calcul peut
//! sous-estimer).

use serde::{Deserialize, Serialize};

use crate::gguf::GgufMeta;

const MIB: u64 = 1 << 20;
/// Tampons de calcul : forfait par carte.
const COMPUTE_PER_CARD: u64 = 512 * MIB;
/// Lot physique par défaut de llama.cpp (`-ub`).
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
    /// Sans flash attention, llama.cpp réserve sur chaque carte la matrice
    /// d'attention entière (têtes × ubatch × contexte, en f32).
    pub flash_attn: bool,
    /// Lot physique (`-ub`), 0 = défaut de llama.cpp (512).
    pub ubatch: u64,
    /// Brouillon MTP : un contexte de plus (KV d'une couche, tampons).
    pub mtp: bool,
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
    /// Ce qui ne tient pas sur les cartes : `--fit` le met en RAM et le
    /// processeur le calcule (lent).
    #[serde(default)]
    pub cpu_mib: u64,
    /// Matrice d'attention réservée en plus sans flash attention, par carte.
    #[serde(default)]
    pub no_fa_mib: u64,
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
    let nextn = meta.nextn_predict_layers.unwrap_or(0).min(layers);
    let main = layers - nextn;
    // Hybride : seules les couches d'attention (une sur N) ont un cache KV ;
    // les couches récurrentes gardent un état fixe par séquence (f32).
    let (attn, recurrent) = match meta.full_attention_interval {
        Some(n) if n > 1 => (main / n + nextn, main - main / n),
        _ => (layers, 0),
    };
    let per_token = attn * heads_kv * (k + v) * num / den;
    let state = meta.ssm.map_or(0, |s| {
        let conv = s.conv_kernel.saturating_sub(1) * (s.inner_size + 2 * s.group_count * s.state_size);
        let ssm = s.state_size * s.inner_size;
        recurrent * (conv + ssm) * 4
    });
    Some((per_token * ctx + state) * parallel.max(1))
}

pub fn estimate(input: &FitInput, cards: &[CardVram]) -> FitReport {
    let unknown = |kv_mib| FitReport {
        cards: Vec::new(),
        verdict: Verdict::Unknown,
        kv_mib,
        cpu_mib: 0,
        no_fa_mib: 0,
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
    let ub = if input.ubatch == 0 { UBATCH } else { input.ubatch };
    let vocab = input.meta.vocab_size.unwrap_or(VOCAB_FALLBACK);
    let logits = ub * vocab * 4;
    // Tampons de calcul : proportionnels au lot physique.
    let compute = COMPUTE_PER_CARD * ub.max(UBATCH) / UBATCH;
    let no_fa = if input.flash_attn {
        0
    } else {
        input.meta.head_count.unwrap_or(32) * ub * input.ctx * 4
    };
    let mtp = if input.mtp { kv / layers + compute + no_fa } else { 0 };
    let last = cards.len() - 1;
    let share = |bytes: u64, a: u64| (bytes as u128 * a as u128 / total_avail as u128) as u64;
    let needs: Vec<CardNeed> = avail
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let mut need = share(input.weights_bytes + input.draft_bytes, a)
                + layer
                + share(kv, a)
                + compute
                + no_fa
                + input.fit_target_mib * MIB;
            if i == 0 {
                need += input.mmproj_bytes;
            }
            if i == last {
                need += logits + mtp;
            }
            CardNeed {
                need_mib: need.div_ceil(MIB),
                avail_mib: a / MIB,
            }
        })
        .collect();
    // Le besoin compte déjà la marge `fit_target` de `--fit` : tient s'il
    // passe marge comprise, serré s'il ne passe qu'en l'entamant.
    let margin = input.fit_target_mib;
    let verdict = if needs.iter().all(|c| c.need_mib <= c.avail_mib) {
        Verdict::Fits
    } else if needs.iter().all(|c| c.need_mib.saturating_sub(margin) <= c.avail_mib) {
        Verdict::Tight
    } else if input.meta.expert_count.is_some() {
        Verdict::Offload
    } else {
        Verdict::NoFit
    };
    let cpu_mib = needs.iter().map(|c| c.need_mib.saturating_sub(c.avail_mib)).sum();
    FitReport {
        cards: needs,
        verdict,
        kv_mib: kv / MIB,
        cpu_mib,
        no_fa_mib: no_fa.div_ceil(MIB),
    }
}
