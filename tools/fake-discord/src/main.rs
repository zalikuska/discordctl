//! Тестовая подделка. В зависимости от имени exe притворяется:
//!   Update.exe / Update.real.exe: логирует аргументы, на `--processStart` запускает app-*/Discord.exe без пайпа;
//!   Discord.exe: если есть --remote-debugging-io-pipes, пишет READY в пайп и ждёт его закрытия (как Chromium).
//! Всё пишется в fake.log рядом с exe родительской папки.

use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::io::{FromRawHandle, RawHandle},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

fn log_path(me: &Path) -> PathBuf {
    // <root>/fake.log, где root = папка с Update.exe (для Discord.exe это родитель app-*)
    let dir = me.parent().unwrap();
    let root = if dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with("app-")) {
        dir.parent().unwrap()
    } else {
        dir
    };
    root.join("fake.log")
}

fn log(me: &Path, msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path(me)) {
        let _ = writeln!(f, "{msg}");
    }
}

fn main() {
    let me = env::current_exe().unwrap();
    let stem = me.file_stem().unwrap().to_string_lossy().to_ascii_lowercase();
    let args: Vec<String> = env::args().skip(1).collect();

    if stem.starts_with("update") {
        log(&me, &format!("{} args={:?}", stem, args));
        if args.first().map(|s| s.eq_ignore_ascii_case("--processStart")).unwrap_or(false) {
            let root = me.parent().unwrap();
            let app = std::fs::read_dir(root)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("Discord.exe").is_file())
                .max();
            if let Some(app) = app {
                #[allow(clippy::zombie_processes)] // подделка намеренно не ждёт «Discord»: он живёт, пока жив тест
                Command::new(app.join("Discord.exe")).spawn().unwrap();
            }
        }
        return;
    }

    // Корневая заглушка Squirrel (Discord.exe / Discord.real.exe в корне): как настоящая,
    // запускает app-*/Discord.exe напрямую, минуя Update.exe.
    let in_app = me.parent().and_then(|d| d.file_name()).is_some_and(|n| n.to_string_lossy().starts_with("app-"));
    if !in_app {
        log(&me, &format!("{} args={:?}", if stem == "discord" { "stub" } else { "stub.real" }, args));
        let root = me.parent().unwrap();
        let app = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("Discord.exe").is_file())
            .max();
        if let Some(app) = app {
            #[allow(clippy::zombie_processes)] // подделка намеренно не ждёт «Discord»: он живёт, пока жив тест
            Command::new(app.join("Discord.exe")).spawn().unwrap();
        }
        return;
    }

    // Discord.exe внутри app-*.
    // Как настоящий Squirrel: при каждом запуске восстанавливает оригинальную корневую заглушку,
    // затирая шим. Пауза нужна, чтобы шим успел завершиться (запущенный exe не перезаписать).
    let restorer = {
        let me2 = me.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(800));
            if env::var_os("FAKE_NO_RESTORE").is_some() {
                return; // тесты без демона: чинить некому, поэтому заглушку не трогаем
            }
            let root = me2.parent().unwrap().parent().unwrap();
            let _ = std::fs::copy(&me2, root.join("Discord.exe"));
        })
    };
    let pipes = args.iter().find_map(|a| a.strip_prefix("--remote-debugging-io-pipes="));
    match pipes {
        None => {
            log(&me, &format!("discord PLAIN pid={} args={:?}", std::process::id(), args));
            thread::sleep(Duration::from_millis(300));
            let _ = restorer.join();
        }
        Some(p) => {
            let (r, w) = p.split_once(',').expect("r,w");
            let (r, w): (usize, usize) = (r.parse().unwrap(), w.parse().unwrap());
            let mut rd = unsafe { File::from_raw_handle(r as RawHandle) };
            let mut wr = unsafe { File::from_raw_handle(w as RawHandle) };
            log(&me, &format!("discord CDP pid={} args={:?}", std::process::id(), args));
            wr.write_all(b"READY\0").unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            // Читаем до закрытия пайпа, как Chromium: после EOF процесс завершается.
            loop {
                match rd.read(&mut buf) {
                    Ok(n) if n > 0 => got.extend_from_slice(&buf[..n]),
                    _ => break,
                }
            }
            log(&me, &format!("discord pid={} pipe closed, exiting", std::process::id()));
            let _ = restorer.join();
        }
    }
}
