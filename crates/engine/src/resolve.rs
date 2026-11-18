//! Quel binaire llama-server lancer : une seule règle pour le superviseur et
//! la page Système.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "tag", rename_all = "snake_case")]
pub enum Origin {
    /// Imposé par `ENGINEX_LLAMA_BIN` (tests, dépannage).
    Env,
    /// Moteur installé dans `<engines_dir>/<tag>/`.
    Active(String),
    /// Binaire fourni par l'image (`/app/llama-server`).
    Image,
}

/// Nom réservé du moteur de l'image.
pub const IMAGE_TAG: &str = "image";

/// Chemin du binaire d'un moteur installé.
pub fn installed_bin(engines_dir: &Path, tag: &str) -> PathBuf {
    engines_dir.join(tag).join("llama-server")
}

/// 1) `env_override` ; 2) moteur actif si son binaire existe ; 3) l'image.
pub fn resolve_bin(
    env_override: Option<&Path>,
    active_tag: Option<&str>,
    engines_dir: &Path,
    image_bin: &Path,
) -> (PathBuf, Origin) {
    if let Some(p) = env_override {
        return (p.to_path_buf(), Origin::Env);
    }
    if let Some(tag) = active_tag.filter(|t| *t != IMAGE_TAG) {
        let bin = installed_bin(engines_dir, tag);
        if bin.is_file() {
            return (bin, Origin::Active(tag.to_string()));
        }
    }
    (image_bin.to_path_buf(), Origin::Image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order() {
        let dir = tempfile::tempdir().unwrap();
        let image = Path::new("/app/llama-server");
        let env = Path::new("/tmp/fake");
        assert_eq!(
            resolve_bin(Some(env), Some("server-cuda-b1"), dir.path(), image),
            (env.to_path_buf(), Origin::Env)
        );
        // Actif absent du disque : image.
        assert_eq!(
            resolve_bin(None, Some("server-cuda-b1"), dir.path(), image).1,
            Origin::Image
        );
        let bin = installed_bin(dir.path(), "server-cuda-b1");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"x").unwrap();
        assert_eq!(
            resolve_bin(None, Some("server-cuda-b1"), dir.path(), image),
            (bin, Origin::Active("server-cuda-b1".into()))
        );
        assert_eq!(resolve_bin(None, Some("image"), dir.path(), image).1, Origin::Image);
        assert_eq!(resolve_bin(None, None, dir.path(), image).1, Origin::Image);
    }
}
