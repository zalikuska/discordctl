use super::*;
use std::collections::HashSet;

#[test]
fn captures_follow_the_order_of_the_english_text() {
    let (segs, _) = split_groups("{} of {loaded} loaded members");
    assert_eq!(segs, vec!["", " of ", " loaded members"]);
    assert_eq!(capture(&segs, "5 of 40 loaded members").unwrap(), vec!["5", "40"]);
    assert!(capture(&segs, "5 loaded members").is_none());
}

#[test]
fn translation_fills_in_the_values() {
    assert_eq!(
        translate(r"Discord found: app-1.0.9265 (C:\x)").as_deref(),
        Some(r"Discord найден: app-1.0.9265 (C:\x)")
    );
    assert_eq!(
        translate("3 checks: 1 problem(s), 2 warning(s)").as_deref(),
        Some("проверок: 3, проблем: 1, предупреждений: 2")
    );
    // Значение может быть цветным текстом с управляющими кодами: оно подставляется как есть.
    assert_eq!(
        translate("daemon running \u{1b}[2m(v1, pid 2)\u{1b}[0m").as_deref(),
        Some("демон работает \u{1b}[2m(v1, pid 2)\u{1b}[0m")
    );
}

#[test]
fn explicit_numbers_reorder_the_values() {
    let (_, groups) = split_groups("{2} из {1}");
    assert_eq!(groups, vec![Some(2), Some(1)]);
}

#[test]
fn indents_and_padding_survive() {
    set(Lang::Ru);
    assert_eq!(tr("  found on this PC"), "  найден на этом ПК");
    let padded = tr("Antivirus      ");
    assert!(padded.starts_with("Антивирус") && padded.chars().count() == 15, "{padded:?}");
    assert_eq!(tr("no such text anywhere"), "no such text anywhere");
    set(Lang::En);
    assert_eq!(tr("Antivirus"), "Antivirus", "English is left alone");
}

#[test]
fn language_words_are_understood() {
    assert_eq!(Choice::parse("RU"), Some(Choice::Ru));
    assert_eq!(Choice::parse(" english "), Some(Choice::En));
    assert_eq!(Choice::parse("auto"), Some(Choice::Auto));
    assert_eq!(Choice::parse("fr"), None);
}

/// Исходники в том виде, в каком строки попадают в программу: продолжение строки через `\` и экранирование убраны.
fn sources() -> String {
    let mut all = String::new();
    for s in [
        include_str!("ui.rs"),
        include_str!("main.rs"),
        include_str!("ops.rs"),
        include_str!("setup.rs"),
        include_str!("api.rs"),
        include_str!("approve.rs"),
        include_str!("render.rs"),
        include_str!("doctor.rs"),
        include_str!("mcp.rs"),
        include_str!("update.rs"),
        include_str!("cli.rs"),
        include_str!("tui/wizard.rs"),
        include_str!("tui/mod.rs"),
        include_str!("../../core/src/shim.rs"),
    ] {
        all.push_str(&s.replace("\r\n", "\n"));
    }
    let mut out = String::new();
    let mut chars = all.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some('n') => out.push('\n'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => {}
        }
    }
    out
}

#[test]
fn every_catalog_entry_is_consistent_and_still_in_the_code() {
    let code = sources();
    let decode = |s: &str| {
        // Ключ записан в каталоге как строка Rust; приводим его к тому же виду, что и исходники выше.
        s.to_owned()
    };
    let mut seen = HashSet::new();
    for (en, ru) in crate::i18n_ru::RU.iter().chain(crate::i18n_ru_help::RU_HELP) {
        assert!(seen.insert(*en), "duplicate key {en:?}");
        let (segs, groups) = split_groups(en);
        let (_, ru_groups) = split_groups(ru);
        assert_eq!(groups.len(), ru_groups.len(), "different number of values: {en:?}");
        if !groups.is_empty() {
            assert!(segs.iter().map(|s| s.len()).sum::<usize>() >= 3, "pattern is too loose: {en:?}");
        }
        // Ключ стоит в коде как есть (или частями между значениями): иначе строку поменяли, а перевод забыли.
        for part in segs.iter().map(|s| decode(s)).filter(|p| p.trim().len() >= 3) {
            let first_line = part.lines().find(|l| l.trim().len() >= 3).unwrap_or(&part).trim_end();
            assert!(code.contains(first_line), "the text is gone from the code: {first_line:?} (key {en:?})");
        }
    }
}
