//! Configuración persistente de Temiktli: por ahora, solo el idioma de la interfaz. Vive en
//! `config.json`, en la misma carpeta que `snippets.json`. Un archivo ausente o dañado se
//! trata como configuración vacía: nunca impide arrancar.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// `"system"` (el idioma de Windows) o un código de `i18n::LANGUAGES`. `None` = sin elegir,
    /// que equivale a `"system"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Dónde se guarda: junto a los snippets del usuario.
pub fn storage_path() -> Option<PathBuf> {
    crate::snippets::storage_path().map(|p| p.with_file_name("config.json"))
}

/// Lee la configuración; si no existe o no se entiende, la vacía.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Escribe la configuración, creando la carpeta si hace falta.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("temiktli-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("sub").join("config.json")
    }

    #[test]
    fn saves_and_loads_the_language_and_creates_the_folder() {
        let path = temp("roundtrip");
        let cfg = Config { language: Some("es".into()) };
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path), cfg);
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"language\": \"es\""));
    }

    #[test]
    fn a_missing_or_damaged_file_is_an_empty_config() {
        let path = temp("damaged");
        assert_eq!(load(&path), Config::default());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ no es json").unwrap();
        assert_eq!(load(&path), Config::default());
    }
}
