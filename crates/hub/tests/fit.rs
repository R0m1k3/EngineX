//! Estimation de place : valeurs calculées à la main depuis la formule de
//! `hub::fit` (pas mesurées sur une carte).

use enginex_hub::{
    fit::{CardVram, FitInput, KvType, Verdict, estimate, kv_bytes},
    gguf::GgufMeta,
};

/// 27B fictif : 64 couches, 8 têtes KV, clés/valeurs de 128, vocabulaire 152 064.
fn meta() -> GgufMeta {
    GgufMeta {
        arch: "test".into(),
        block_count: Some(64),
        head_count: Some(64),
        head_count_kv: Some(8),
        embedding_length: Some(5120),
        key_length: Some(128),
        value_length: Some(128),
        vocab_size: Some(152_064),
        ..Default::default()
    }
}

const CARDS: [CardVram; 2] = [
    CardVram {
        total_mib: 16311,
        others_mib: 0,
    },
    CardVram {
        total_mib: 12288,
        others_mib: 0,
    },
];

fn run(ctx: u64, cards: &[CardVram]) -> enginex_hub::fit::FitReport {
    let m = meta();
    estimate(
        &FitInput {
            meta: &m,
            weights_bytes: (16.5 * (1u64 << 30) as f64) as u64,
            mmproj_bytes: 0,
            ctx,
            parallel: 1,
            kv: KvType::Q8_0,
            fit_target_mib: 1024,
            draft_bytes: 0,
        },
        cards,
    )
}

#[test]
fn kv_formula() {
    // 64 × 8 × 256 × 34/32 = 139 264 octets par jeton.
    assert_eq!(kv_bytes(&meta(), 1, 1, KvType::Q8_0), Some(139_264));
    assert_eq!(kv_bytes(&meta(), 1, 1, KvType::F16), Some(262_144));
    assert_eq!(kv_bytes(&meta(), 65536, 2, KvType::Q4_0), Some(73_728 * 65536 * 2));
    // Sans key_length : embedding / têtes = 80.
    let m = GgufMeta {
        key_length: None,
        value_length: None,
        ..meta()
    };
    assert_eq!(kv_bytes(&m, 1, 1, KvType::F16), Some(64 * 8 * 160 * 2));
}

#[test]
fn verdicts_by_context() {
    let r = run(65536, &CARDS);
    assert_eq!(r.kv_mib, 8704);
    assert_eq!(r.cards[0].need_mib, 16401);
    assert_eq!(r.cards[1].need_mib, 13097);
    assert_eq!(r.verdict, Verdict::NoFit);
    let r = run(32768, &CARDS);
    assert_eq!((r.cards[0].need_mib, r.cards[1].need_mib), (13919, 11227));
    assert_eq!(r.verdict, Verdict::Tight); // 2e carte à 91 %
    let r = run(16384, &CARDS);
    assert_eq!((r.cards[0].need_mib, r.cards[1].need_mib), (12678, 10292));
    assert_eq!(r.verdict, Verdict::Fits);
}

#[test]
fn others_reduce_available() {
    let busy = [
        CardVram {
            total_mib: 16311,
            others_mib: 4000,
        },
        CARDS[1],
    ];
    let r = run(16384, &busy);
    assert_eq!(r.cards[0].avail_mib, 12311);
    assert_ne!(r.verdict, Verdict::Fits);
}

#[test]
fn unknown_without_metadata_or_cards() {
    let m = GgufMeta {
        block_count: None,
        ..meta()
    };
    let input = FitInput {
        meta: &m,
        weights_bytes: 1,
        mmproj_bytes: 0,
        ctx: 1,
        parallel: 1,
        kv: KvType::F16,
        fit_target_mib: 0,
        draft_bytes: 0,
    };
    assert_eq!(estimate(&input, &CARDS).verdict, Verdict::Unknown);
    assert_eq!(run(1024, &[]).verdict, Verdict::Unknown);
}

#[test]
fn moe_too_big_offloads_and_draft_counts() {
    // 40 Gio de poids sur 28 Gio de cartes : dense = ne tient pas, MoE =
    // experts en RAM.
    let dense = meta();
    let moe = GgufMeta {
        expert_count: Some(128),
        ..meta()
    };
    let input = |m| FitInput {
        meta: m,
        weights_bytes: 40 << 30,
        mmproj_bytes: 0,
        ctx: 8192,
        parallel: 1,
        kv: KvType::Q8_0,
        fit_target_mib: 1024,
        draft_bytes: 0,
    };
    assert_eq!(estimate(&input(&dense), &CARDS).verdict, Verdict::NoFit);
    assert_eq!(estimate(&input(&moe), &CARDS).verdict, Verdict::Offload);
    // Le brouillon s'ajoute aux poids.
    let base = run(8192, &CARDS);
    let m = meta();
    let with = estimate(
        &FitInput {
            meta: &m,
            weights_bytes: (16.5 * (1u64 << 30) as f64) as u64,
            mmproj_bytes: 0,
            ctx: 8192,
            parallel: 1,
            kv: KvType::Q8_0,
            fit_target_mib: 1024,
            draft_bytes: 1 << 30,
        },
        &CARDS,
    );
    let sum = |r: &enginex_hub::fit::FitReport| r.cards.iter().map(|c| c.need_mib).sum::<u64>();
    assert!(
        (1023..=1025).contains(&(sum(&with) - sum(&base))),
        "{} {}",
        sum(&with),
        sum(&base)
    );
}
