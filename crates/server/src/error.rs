//! Erreurs de l'API : `{"error":"<code>"}` avec le statut de la variante.

use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad_request")]
    BadRequest,
    #[error("password_too_short")]
    PasswordTooShort,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad_credentials")]
    BadCredentials,
    #[error("bad_setup_code")]
    BadSetupCode,
    #[error("forbidden_origin")]
    ForbiddenOrigin,
    #[error("not_found")]
    NotFound,
    #[error("method_not_allowed")]
    MethodNotAllowed,
    #[error("already_setup")]
    AlreadySetup,
    #[error("rate_limited")]
    RateLimited(u64),
    #[error("busy")]
    Busy,
    #[error("model_in_use")]
    ModelInUse,
    #[error("engine_in_use")]
    EngineInUse,
    #[error("job_running")]
    JobRunning,
    #[error("already_downloaded")]
    AlreadyDownloaded,
    #[error("already_queued")]
    AlreadyQueued,
    #[error("nofit_confirm_required")]
    NofitConfirmRequired,
    #[error("env_override")]
    EnvOverride,
    #[error("gated_repo")]
    GatedRepo,
    #[error("disk_full")]
    DiskFull,
    #[error("name_taken")]
    NameTaken,
    #[error("no_active_preset")]
    NoActivePreset,
    #[error("vault_locked")]
    VaultLocked,
    #[error("invalid_name")]
    InvalidName,
    #[error("not_empty")]
    NotEmpty,
    #[error("engine_not_ready")]
    EngineNotReady,
    #[error("no_vision")]
    NoVision,
    #[error("turn_in_progress")]
    TurnInProgress,
    #[error("sandbox_unavailable")]
    SandboxUnavailable,
    #[error("sandbox_timeout")]
    SandboxTimeout,
    #[error("too_many_sandboxes")]
    TooManySandboxes,
    #[error("folder_busy")]
    FolderBusy,
    #[error("changed_since")]
    ChangedSince,
    #[error("no_snapshot")]
    NoSnapshot,
    #[error("not_git")]
    NotGit,
    /// Création de la copie séparée impossible (fin du journal en `detail`).
    #[error("copy_failed")]
    CopyFailed(String),
    #[error("recipe_exists")]
    RecipeExists,
    #[error("no_recipe")]
    NoRecipe,
    /// `true` : trop gros (413), sinon type refusé (400).
    #[error("attachment_invalid")]
    AttachmentInvalid(bool),
    /// Hugging Face ou ghcr injoignable ou en erreur (détail dans les logs).
    #[error("upstream_error")]
    Upstream(String),
    #[error("internal")]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest | Self::PasswordTooShort => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::BadCredentials => StatusCode::UNAUTHORIZED,
            Self::BadSetupCode | Self::ForbiddenOrigin => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            Self::AlreadySetup
            | Self::ModelInUse
            | Self::EngineInUse
            | Self::JobRunning
            | Self::AlreadyDownloaded
            | Self::AlreadyQueued
            | Self::NofitConfirmRequired
            | Self::EnvOverride
            | Self::NameTaken
            | Self::NoActivePreset
            | Self::VaultLocked
            | Self::NotEmpty
            | Self::NoVision
            | Self::TurnInProgress
            | Self::TooManySandboxes
            | Self::FolderBusy
            | Self::ChangedSince
            | Self::NoSnapshot
            | Self::RecipeExists
            | Self::NoRecipe => StatusCode::CONFLICT,
            Self::NotGit => StatusCode::BAD_REQUEST,
            Self::CopyFailed(_) => StatusCode::BAD_GATEWAY,
            Self::InvalidName | Self::AttachmentInvalid(false) => StatusCode::BAD_REQUEST,
            Self::AttachmentInvalid(true) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::EngineNotReady | Self::SandboxUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::SandboxTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::GatedRepo => StatusCode::FORBIDDEN,
            Self::DiskFull => StatusCode::INSUFFICIENT_STORAGE,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Busy => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<enginex_hub::hf::HubError> for ApiError {
    fn from(e: enginex_hub::hf::HubError) -> Self {
        use enginex_hub::hf::HubError;
        match e {
            HubError::Gated | HubError::Status(401 | 403) => Self::GatedRepo,
            HubError::Status(404) => Self::NotFound,
            e => Self::Upstream(e.to_string()),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match &self {
            Self::Internal(e) => tracing::error!("erreur interne : {e:#}"),
            Self::Upstream(e) => tracing::warn!("service distant : {e}"),
            _ => {}
        }
        let retry = match &self {
            Self::RateLimited(s) => Some(*s),
            Self::Busy => Some(1),
            _ => None,
        };
        let mut body = serde_json::json!({ "error": self.to_string() });
        if let Self::CopyFailed(d) = &self {
            body["detail"] = serde_json::json!(d);
        }
        let mut res = (self.status(), Json(body)).into_response();
        if let Some(s) = retry
            && let Ok(v) = HeaderValue::from_str(&s.to_string())
        {
            res.headers_mut().insert(header::RETRY_AFTER, v);
        }
        res
    }
}

/// `Json<T>` dont tout rejet devient `BadRequest`.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(v)| Self(v))
            .map_err(|_: JsonRejection| ApiError::BadRequest)
    }
}

/// Erreur des routes `/v1/*`, au format OpenAI
/// `{"error":{"code","message","type"}}` (Codex lit `error.code` et
/// `error.message`) : exception écrite à la règle `{"error":"<code>"}`.
#[derive(Debug)]
pub struct V1Error {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl V1Error {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "route inconnue")
    }
}

impl IntoResponse for V1Error {
    fn into_response(self) -> Response {
        let kind = match self.status.as_u16() {
            401 => "authentication_error",
            429 => "rate_limit_error",
            400..=499 => "invalid_request_error",
            _ => "server_error",
        };
        let body = serde_json::json!({"error": {"code": self.code, "message": self.message, "type": kind}});
        (self.status, Json(body)).into_response()
    }
}
