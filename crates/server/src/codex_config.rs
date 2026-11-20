//! `config.toml` de Codex prêt à copier, pour le preset actif (Paramètres →
//! API ; l'étape 4 le réutilise pour les sandboxes).

use axum::{Json, extract::State, http::HeaderMap};
use enginex_engine::args::PresetSettings;
use serde_json::{Value, json};

use crate::{auth::origin, error::ApiError, presets, state::AppState};

/// Variable d'environnement où Codex lit la clé.
pub const ENV_KEY: &str = "ENGINEX_API_KEY";

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).expect("chaîne")
}

/// Seuil de compactage automatique de Codex : 75 % du contexte (Loki,
/// `COMPACT_TRIGGER_FRAC`). Codex le compare à `usage.total_tokens` de la
/// dernière réponse, entre deux appels d'outils aussi ; un compactage efface
/// tout le travail du tour, il doit rester rare. Le dépassement éventuel est
/// couvert par le plafond de Codex (95 %) et par le relais (appel tronqué
/// remplacé par un message).
///
/// Le seuil laisse aussi la place d'une réponse entière : `reserve` = budget
/// de réflexion + marge (voir `reserve_for`). Sans cela, un prompt de 45k sur
/// 65k de contexte passait sous les 75 %, puis la réflexion (16k) remplissait
/// le contexte : llama-server coupait (`length`) et le tour échouait.
pub fn compact_limit(ctx: u64, reserve: u64) -> u64 {
    let frac = (ctx as f64 * 0.75).round() as u64;
    frac.min(ctx.saturating_sub(reserve)).max(ctx / 4)
}

/// Place à garder libre après le dernier prompt : réflexion + réponse et
/// sortie d'outil (10 240 jetons).
pub fn reserve_for(s: &PresetSettings) -> u64 {
    let think = if s.reasoning == enginex_engine::args::Reasoning::On {
        s.reasoning_budget.unwrap_or(0)
    } else {
        0
    };
    think + 10_240
}

/// Consigne de résumé du compactage (`compact_prompt`) : celle de Codex, plus
/// la règle du mode Code de Loki (fichiers touchés et leur état). Le
/// compactage efface les appels d'outils du tour : sans cette liste, le modèle
/// réécrit des fichiers déjà faits.
pub const COMPACT_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary \
for another LLM that will resume the task. Everything except the user messages and your summary will be erased, \
including every tool call and its output.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- FILES: every file created or modified so far, one line each: path, approximate line count, and state \
(COMPLETE, or PARTIAL with the last section written and the sections still missing)
- What remains to be done (clear next steps, in order)
- Any critical data, commands, examples, or references needed to continue

Files are on disk and intact. The next LLM must NOT rewrite a file marked COMPLETE; it may inspect it in \
slices (wc -l, grep -n, sed -n 'a,bp') and continue a PARTIAL file from its last section. Be concise and \
structured.";

/// Texte du `config.toml` (sans la clé, lue dans `ENGINEX_API_KEY`).
/// Config Codex ; `mcp` : URL du serveur MCP d'EngineX (sandboxes seulement,
/// il exige leur clé interne).
pub fn render(model: &str, ctx: u64, reserve: u64, base_url: &str, mcp: Option<&str>) -> String {
    let compact = compact_limit(ctx, reserve);
    let mut out = format!(
        "model = {model}\n\
         model_provider = \"enginex\"\n\
         model_context_window = {ctx}\n\
         model_auto_compact_token_limit = {compact}\n\
         model_reasoning_summary = \"none\"\n\
         show_raw_agent_reasoning = true\n\
         web_search = \"disabled\"\n\
         compact_prompt = {prompt}\n\
         \n\
         [model_providers.enginex]\n\
         name = \"EngineX\"\n\
         base_url = {base}\n\
         env_key = \"{ENV_KEY}\"\n",
        model = toml_str(model),
        base = toml_str(base_url),
        prompt = toml_str(COMPACT_PROMPT),
    );
    if let Some(url) = mcp {
        // `default_tools_approval_mode` doit rester dans la section du serveur :
        // sans elle, chaque appel d'outil demande une approbation en mode prudent.
        out.push_str(&format!(
            "\n[mcp_servers.enginex]\n\
             url = {}\n\
             bearer_token_env_var = \"{ENV_KEY}\"\n\
             default_tools_approval_mode = \"approve\"\n",
            toml_str(url)
        ));
    }
    out
}

pub async fn codex_config(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let id = presets::active_id(&state).await?.ok_or(ApiError::NoActivePreset)?;
    let p = presets::preset(&state, id).await?;
    let s: PresetSettings = serde_json::from_str(&p.settings)?;
    let base_url = format!("{}/v1", origin(&state, &headers));
    Ok(Json(json!({
        "base_url": base_url,
        "toml": render(
            &p.name,
            crate::engine_api::effective_ctx(&state, id, s.ctx),
            reserve_for(&s),
            &base_url,
            None,
        ),
        "env_key": ENV_KEY,
    })))
}

#[cfg(test)]
mod tests {
    use super::compact_limit;

    #[test]
    fn compact_at_three_quarters() {
        assert_eq!(compact_limit(65536, 0), 49152);
        assert_eq!(compact_limit(16384, 0), 12288);
    }

    #[test]
    fn compact_leaves_room_for_reasoning() {
        // 65536 - (16384 + 10240) = 38912 : un prompt de 45k déclenche le compactage.
        assert_eq!(compact_limit(65536, 26624), 38912);
        // contexte trop petit : jamais sous 25 %.
        assert_eq!(compact_limit(16384, 26624), 4096);
    }

    #[test]
    fn toml_has_compact_prompt() {
        let t = super::render("qwen", 65536, 0, "http://x/v1", None);
        let v: toml::Value = toml::from_str(&t).expect("TOML valide");
        assert_eq!(v["model_auto_compact_token_limit"].as_integer(), Some(49152));
        assert!(
            v["compact_prompt"]
                .as_str()
                .unwrap()
                .contains("must NOT rewrite a file marked COMPLETE")
        );
    }
}
