//! Установка, откат и починка шима.
//!
//! Шим ставится на два файла в корне Discord, оба не являются процессами Chromium:
//!
//! * `Update.exe` (Squirrel): панель задач, автозагрузка, перезапуск после обновления;
//! * `Discord.exe` (корневая заглушка): ярлыки рабочего стола и меню «Пуск».
//!
//! Настоящий файл переименовывается в `<имя>.real.exe`, на его место копируется шим.
//! Шим опознаётся по маркеру внутри файла, а не по сравнению с копией,
//! чтобы не принять старую версию шима за настоящий файл.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Строка, которая есть только в бинарнике шима. Не менять между версиями.
pub const SHIM_MARKER: &str = "discordctl-shim-marker::2c9f1e47";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Update,
    Stub,
}

pub const TARGETS: [Target; 2] = [Target::Update, Target::Stub];

impl Target {
    pub fn file(self) -> &'static str {
        match self {
            Target::Update => "Update.exe",
            Target::Stub => "Discord.exe",
        }
    }
    pub fn real_file(self) -> &'static str {
        match self {
            Target::Update => "Update.real.exe",
            Target::Stub => "Discord.real.exe",
        }
    }
    pub fn path(self, root: &Path) -> PathBuf {
        root.join(self.file())
    }
    pub fn real_path(self, root: &Path) -> PathBuf {
        root.join(self.real_file())
    }
}

pub fn has_marker(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else { return false };
    let needle = SHIM_MARKER.as_bytes();
    bytes.windows(needle.len()).any(|w| w == needle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Файл настоящий, шима нет.
    NotInstalled,
    /// Шим на месте, настоящий файл `*.real.exe` рядом.
    Installed,
    /// `*.real.exe` есть, но основной файл снова настоящий: его перезаписал апдейтер.
    NeedsRepair,
    /// Шим есть, а настоящего `*.real.exe` нет или вместо него лежит другой шим: Discord через него не запустить.
    Broken,
    /// Основной файл отсутствует.
    Missing,
}

pub fn state(root: &Path, t: Target) -> State {
    let (main, real) = (t.path(root), t.real_path(root));
    if !main.is_file() {
        return State::Missing;
    }
    match (has_marker(&main), real.is_file()) {
        // `*.real.exe` сам оказался шимом (старая сборка, поставленная поверх себя): запуск через него зациклился бы.
        (true, true) if has_marker(&real) => State::Broken,
        (true, true) => State::Installed,
        (true, false) => State::Broken,
        (false, true) => State::NeedsRepair,
        (false, false) => State::NotInstalled,
    }
}

/// Убирает файл с дороги: запущенный exe нельзя удалить, но можно переименовать.
fn move_aside(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    for n in 0..100 {
        let aside = path.with_extension(format!("old{n}"));
        if !aside.exists() && fs::rename(path, &aside).is_ok() {
            let _ = fs::remove_file(&aside);
            return Ok(());
        }
    }
    bail!("cannot move {} out of the way", path.display())
}

/// Ставит шим на место файла. Возвращает `true`, если что-то изменилось.
pub fn place(root: &Path, t: Target, shim_src: &Path) -> Result<bool> {
    let (main, real) = (t.path(root), t.real_path(root));
    match state(root, t) {
        State::Missing => bail!("{} not found: is Discord installed here?", main.display()),
        State::Broken => {
            bail!(
                "shim is in place but {} is missing or is itself a shim; reinstall Discord or restore the file",
                real.display()
            )
        }
        State::Installed => {
            // Обновляем сам шим, если сборка новее.
            if fs::read(&main)? == fs::read(shim_src)? {
                return Ok(false);
            }
            move_aside(&main)?;
            fs::copy(shim_src, &main).context("copying new shim")?;
            Ok(true)
        }
        State::NotInstalled | State::NeedsRepair => {
            // Основной файл сейчас настоящий (возможно, свежий после обновления Squirrel).
            // Старый `*.real.exe` устарел, rename заменяет его сам. Нельзя удалять его заранее:
            // если основной файл ещё занят писателем, переименование не удастся, и мы бы остались
            // без обоих файлов. Вызывающий повторит попытку.
            fs::rename(&main, &real).with_context(|| format!("renaming {} to {}", t.file(), t.real_file()))?;
            if let Err(e) = fs::copy(shim_src, &main) {
                let _ = fs::rename(&real, &main);
                return Err(e).with_context(|| format!("copying shim; original {} restored", t.file()));
            }
            Ok(true)
        }
    }
}

/// Возвращает настоящий файл на место. Возвращает `true`, если что-то изменилось.
pub fn remove(root: &Path, t: Target) -> Result<bool> {
    let (main, real) = (t.path(root), t.real_path(root));
    match state(root, t) {
        State::NotInstalled | State::Missing => Ok(false),
        State::NeedsRepair => {
            // Основной файл уже настоящий; `*.real.exe` — устаревший остаток.
            move_aside(&real)?;
            Ok(true)
        }
        State::Broken => {
            bail!(
                "shim is in place but {} is missing or is itself a shim; cannot restore automatically",
                real.display()
            )
        }
        State::Installed => {
            move_aside(&main)?;
            fs::rename(&real, &main).with_context(|| format!("restoring {}", t.real_file()))?;
            Ok(true)
        }
    }
}

/// Чинит шим, если апдейтер Squirrel его перезаписал. Вызывать только когда шим должен быть
/// установлен (`Config::shim_installed`): `NotInstalled` здесь значит прерванную починку.
pub fn repair(root: &Path, t: Target, shim_src: &Path) -> Result<bool> {
    if matches!(state(root, t), State::NeedsRepair | State::NotInstalled) {
        place(root, t, shim_src)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shim_in_place_of_the_real_file_is_broken() {
        let dir = std::env::temp_dir().join(format!("dctl-shim-state-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let shim_bytes = format!("MZ..{SHIM_MARKER}..");
        fs::write(Target::Update.path(&dir), &shim_bytes).unwrap();
        fs::write(Target::Update.real_path(&dir), "MZ real squirrel").unwrap();
        assert_eq!(state(&dir, Target::Update), State::Installed);
        // Старая сборка шима, оказавшаяся на месте `Update.real.exe`: через неё запуск зациклится.
        fs::write(Target::Update.real_path(&dir), &shim_bytes).unwrap();
        assert_eq!(state(&dir, Target::Update), State::Broken);
        fs::remove_dir_all(&dir).unwrap();
    }
}
