//! Чтение файлов из zip-архива релиза (методы «без сжатия» и deflate; zip64 не нужен: архив около 1 МБ).
//!
//! Имена из архива на диск не попадают: вызывающий просит файл по имени и сам решает, куда его положить.
//! Так подделанный путь вида `..\..\x.exe` ничего не запишет за пределами нужной папки.

use anyhow::{bail, Context, Result};

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
/// Потолок размера одного распакованного файла: защита от архива-бомбы.
const MAX_FILE: usize = 64 * 1024 * 1024;

fn u16_at(b: &[u8], at: usize) -> Result<usize> {
    let s = b.get(at..at + 2).context("the archive is truncated")?;
    Ok(u16::from_le_bytes([s[0], s[1]]) as usize)
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).context("the archive is truncated")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Последний компонент пути; в архивах из Windows PowerShell 5.1 разделитель — обратная косая черта.
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Файл верхнего уровня архива по имени (без учёта регистра). `Ok(None)`, если такого нет.
pub fn read_root_file(zip: &[u8], wanted: &str) -> Result<Option<Vec<u8>>> {
    // Конец центрального каталога: после него может идти комментарий до 64 КБ.
    let from = zip.len().saturating_sub(22 + 65535);
    let eocd = (from..=zip.len().saturating_sub(22))
        .rev()
        .find(|&i| u32_at(zip, i).is_ok_and(|s| s == EOCD))
        .context("this is not a zip archive")?;
    let entries = u16_at(zip, eocd + 10)?;
    let mut at = u32_at(zip, eocd + 16)? as usize;
    for _ in 0..entries {
        if u32_at(zip, at)? != CENTRAL {
            bail!("the archive directory is damaged");
        }
        let method = u16_at(zip, at + 10)?;
        let comp = u32_at(zip, at + 20)? as usize;
        let size = u32_at(zip, at + 24)? as usize;
        let (n, x, c) = (u16_at(zip, at + 28)?, u16_at(zip, at + 30)?, u16_at(zip, at + 32)?);
        let local = u32_at(zip, at + 42)? as usize;
        let name_bytes = zip.get(at + 46..at + 46 + n).context("the archive is truncated")?;
        at += 46 + n + x + c;
        let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
        // Только файлы верхнего уровня: имя без папок и точно то, что просили.
        if name.contains('/') || !base_name(&name).eq_ignore_ascii_case(wanted) {
            continue;
        }
        if size > MAX_FILE {
            bail!("{wanted} is too large for a release file");
        }
        if u32_at(zip, local)? != LOCAL {
            bail!("the archive entry for {wanted} is damaged");
        }
        let start = local + 30 + u16_at(zip, local + 26)? + u16_at(zip, local + 28)?;
        let raw = zip.get(start..start + comp).context("the archive is truncated")?;
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, MAX_FILE)
                .map_err(|e| anyhow::anyhow!("cannot unpack {wanted}: {e:?}"))?,
            m => bail!("{wanted} uses an unsupported compression method ({m})"),
        };
        if data.len() != size {
            bail!("{wanted} unpacked to {} bytes, expected {size}", data.len());
        }
        return Ok(Some(data));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Собирает zip: (имя, содержимое, сжимать ли).
    fn build(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = vec![];
        let mut central = vec![];
        for (name, data, deflate) in files {
            let packed = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            for chunk in [
                &LOCAL.to_le_bytes()[..],
                &[20, 0, 0, 0],
                &method.to_le_bytes(),
                &[0; 4], // время и дата
                &[0; 4], // crc не проверяем: целостность держит SHA-256 всего архива
                &(packed.len() as u32).to_le_bytes(),
                &(data.len() as u32).to_le_bytes(),
                &(name.len() as u16).to_le_bytes(),
                &[0, 0],
                name.as_bytes(),
                &packed,
            ] {
                out.extend_from_slice(chunk);
            }
            for chunk in [
                &CENTRAL.to_le_bytes()[..],
                &[20, 0, 20, 0, 0, 0],
                &method.to_le_bytes(),
                &[0; 8],
                &(packed.len() as u32).to_le_bytes(),
                &(data.len() as u32).to_le_bytes(),
                &(name.len() as u16).to_le_bytes(),
                &[0; 12],
                &offset.to_le_bytes(),
                name.as_bytes(),
            ] {
                central.extend_from_slice(chunk);
            }
        }
        let cd_at = out.len() as u32;
        out.extend_from_slice(&central);
        for chunk in [
            &EOCD.to_le_bytes()[..],
            &[0; 4],
            &(files.len() as u16).to_le_bytes(),
            &(files.len() as u16).to_le_bytes(),
            &(central.len() as u32).to_le_bytes(),
            &cd_at.to_le_bytes(),
            &[0, 0],
        ] {
            out.extend_from_slice(chunk);
        }
        out
    }

    #[test]
    fn reads_stored_and_deflated_files_by_name() {
        let body = b"MZ binary ".repeat(500);
        let zip = build(&[("discordctl.exe", &body, true), ("LICENSE", b"MIT", false)]);
        assert_eq!(read_root_file(&zip, "discordctl.exe").unwrap().unwrap(), body);
        assert_eq!(read_root_file(&zip, "license").unwrap().unwrap(), b"MIT");
        assert!(read_root_file(&zip, "missing.exe").unwrap().is_none());
    }

    #[test]
    fn only_top_level_files_count_and_backslashes_are_folders() {
        let zip = build(&[("examples\\discordctl.exe", b"evil", false), ("../discordctl.exe", b"evil", false)]);
        assert!(read_root_file(&zip, "discordctl.exe").unwrap().is_none(), "files in folders must not match");
    }

    #[test]
    fn broken_archives_are_errors_not_panics() {
        assert!(read_root_file(b"not a zip", "x").is_err());
        let mut zip = build(&[("a.exe", b"data", false)]);
        let cut = zip.len() - 30;
        zip.truncate(cut);
        assert!(read_root_file(&zip, "a.exe").is_err());
        // Разрыв внутри данных: размеры в каталоге больше, чем есть в файле.
        let mut zip = build(&[("a.exe", b"data data data", false)]);
        let cd_at = u32_at(&zip, zip.len() - 22 + 16).unwrap() as usize;
        zip[cd_at + 21] = 0x7f; // сжатый размер в каталоге: старший байт
        assert!(read_root_file(&zip, "a.exe").is_err());
    }
}
