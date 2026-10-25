use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Корень установки Discord (там лежит Update.exe).
    pub discord_root: Option<PathBuf>,
    /// Установленная копия демона, которую шим запускает по требованию.
    pub daemon_exe: Option<PathBuf>,
    /// Шим поставлен и демон должен следить за его целостностью.
    #[serde(default)]
    pub shim_installed: bool,
    /// Обработчик `discord://` указывает на заглушку, демон возвращает его после перезаписи Discord.
    #[serde(default)]
    pub handler_hook: bool,
    /// Что стояло в обработчике до нас (для отката).
    #[serde(default)]
    pub handler_original: Option<String>,
    /// Язык интерфейса: `ru` или `auto`; пусто — английский.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl Config {
    pub fn load() -> Config {
        std::fs::read(paths::config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(paths::data_dir())?;
        // Через временный файл: демон, шим и CLI читают конфиг в любой момент, а оборванная или частичная запись
        // читалась бы как «пусто» и тихо выключала бы слежение за шимом.
        let tmp = paths::data_dir().join("config.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        std::fs::rename(&tmp, paths::config_path())
    }

    pub fn root(&self) -> PathBuf {
        self.discord_root.clone().unwrap_or_else(paths::default_discord_root)
    }
}
