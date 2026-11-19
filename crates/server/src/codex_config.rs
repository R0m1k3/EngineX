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

/// Seuil de compactage automatique de Codex : 80 % du contexte, et jamais
/// plus que le contexte moins la sortie max, pour qu'historique et réponse
/// tiennent toujours ensemble (Codex vérifie aussi ce seuil entre deux appels
/// d'outils d'un même tour). Pas en dessous de la moitié du contexte, sinon
/// il compacterait sans arrêt.
pub fn compact_limit(ctx: u64, max_output: Option<u64>) -> u64 {
    let pct = (ctx as f64 * 0.8).round() as u64;
    let room = max_output.map_or(pct, |n| ctx.saturating_sub(n));
    pct.min(room).max(ctx / 2)
}

/// Texte du `config.toml` (sans la clé, lue dans `ENGINEX_API_KEY`).
/// Config Codex ; `mcp` : URL du serveur MCP d'EngineX (sandboxes seulement,
/// il exige leur clé interne).
pub fn render(model: &str, ctx: u64, max_output: Option<u64>, base_url: &str, mcp: Option<&str>) -> String {
    let compact = compact_limit(ctx, max_output);
    let mut out = format!(
        "model = {model}\n\
         model_provider = \"enginex\"\n\
         model_context_window = {ctx}\n\
         model_auto_compact_token_limit = {compact}\n\
         model_reasoning_summary = \"none\"\n\
         show_raw_agent_reasoning = true\n\
         web_search = \"disabled\"\n\
         \n\
         [model_providers.enginex]\n\
         name = \"EngineX\"\n\
         base_url = {base}\n\
         env_key = \"{ENV_KEY}\"\n",
        model = toml_str(model),
        base = toml_str(base_url),
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
            s.max_output,
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
    fn compact_keeps_room_for_the_answer() {
        // Preset de l'Unraid : 65k de contexte, 32k de sortie max.
        assert_eq!(compact_limit(65536, Some(32768)), 32768);
        // Petite sortie : 80 % du contexte.
        assert_eq!(compact_limit(65536, Some(4096)), 52429);
        // Sortie illimitée : 80 %.
        assert_eq!(compact_limit(65536, None), 52429);
        // Sortie plus grande que le contexte : la moitié, jamais 0.
        assert_eq!(compact_limit(16384, Some(32768)), 8192);
    }
}
