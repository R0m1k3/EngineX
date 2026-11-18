//! Tests d'un moteur avant de l'utiliser : version, cartes vues, `--fit`.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use crate::oci::UpdateError;
use crate::probe;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestReport {
    pub build: String,
    pub gpus: usize,
    pub has_fit: bool,
}

/// Cartes CUDA vues par `--list-devices`.
fn cuda_count(info: &probe::LlamaInfo) -> usize {
    info.devices.iter().filter(|d| d.id.starts_with("CUDA")).count()
}

/// Teste un binaire : il démarre, donne son build (égal à `expected_build`
/// si fourni), voit au moins `expected_gpus` cartes CUDA et propose `--fit`.
pub async fn test_engine(
    bin: &Path,
    expected_build: Option<&str>,
    expected_gpus: usize,
) -> Result<TestReport, UpdateError> {
    let info = probe::read_llama(bin).await;
    let version = info
        .version
        .clone()
        .ok_or_else(|| UpdateError::TestFailed(format!("--version : {}", info.error.clone().unwrap_or_default())))?;
    let build =
        probe::build_of(&version).ok_or_else(|| UpdateError::TestFailed(format!("version illisible : {version}")))?;
    if let Some(want) = expected_build
        && want != build
    {
        return Err(UpdateError::TestFailed(format!("build {build} au lieu de {want}")));
    }
    let gpus = cuda_count(&info);
    if gpus < expected_gpus {
        return Err(UpdateError::TestFailed(format!(
            "{gpus} carte(s) CUDA vue(s) au lieu de {expected_gpus}"
        )));
    }
    let help = probe::help(bin)
        .await
        .map_err(|e| UpdateError::TestFailed(format!("--help : {e}")))?;
    let has_fit = probe::has_fit(&help);
    if !has_fit {
        return Err(UpdateError::TestFailed("llama-server sans --fit".into()));
    }
    Ok(TestReport { build, gpus, has_fit })
}

/// Comme `test_engine`, sans exiger `--fit` (moteur de l'image, recours).
pub async fn describe(bin: &Path) -> Result<TestReport, UpdateError> {
    let info = probe::read_llama(bin).await;
    let version = info
        .version
        .clone()
        .ok_or_else(|| UpdateError::TestFailed("--version".into()))?;
    let build = probe::build_of(&version).unwrap_or(version);
    let has_fit = probe::help(bin).await.is_ok_and(|h| probe::has_fit(&h));
    Ok(TestReport {
        build,
        gpus: cuda_count(&info),
        has_fit,
    })
}
