//! Traduction entre l'API Responses (ce que parle Codex) et
//! `/v1/chat/completions` de `llama-server`. Code pur, sans réseau.
//!
//! La traduction de la requête est déterministe : même historique, mêmes
//! octets (`serde_json` sans `preserve_order`, donc clés triées), pour que le
//! cache de prompt de `llama-server` serve d'un tour à l'autre.

pub mod history;
pub mod names;
pub mod request;
pub mod sampling;
pub mod stream;
pub mod types;

pub use names::NameMap;
pub use request::{COMPACTION_MARK, Ctx, Think, TranslateError, Translated, parse_and_translate, to_chat};
pub use stream::{Event, Translator, random_id};
pub use types::{Input, ResponsesRequest};
