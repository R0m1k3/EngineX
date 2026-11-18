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
            Self::AlreadySetup => StatusCode::CONFLICT,
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Self::Internal(e) = &self {
            tracing::error!("erreur interne : {e:#}");
        }
        let retry = match &self {
            Self::RateLimited(s) => Some(*s),
            Self::Busy => Some(1),
            _ => None,
        };
        let mut res = (self.status(), Json(serde_json::json!({ "error": self.to_string() }))).into_response();
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
