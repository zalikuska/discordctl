//! Шим вместо Squirrel `Update.exe` и вместо корневой заглушки `Discord.exe`.
//! Роль определяется по имени собственного файла.
//!
//! Запуск Discord -> просим демон запустить его с CDP-пайпом.
//! Всё остальное (установка, обновление) и любой сбой -> передаём настоящему `*.real.exe`.
//! Главное правило: шим не должен мешать запуску Discord ни при каких ошибках.

#![windows_subsystem = "windows"]

use discordctl_core::{
    args, client, paths,
    protocol::Request,
    shim::{self, SHIM_MARKER},
};
use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitCode},
    time::Duration,
};

fn log(msg: &str) {
    paths::log("shim", msg);
}

fn managed_launch(extra: Vec<String>) -> Result<(), String> {
    let resp = client::call(&Request::Launch { args: extra }, Duration::from_secs(10))?;
    if resp.ok {
        log(&format!("managed launch: {}", resp.msg.unwrap_or_default()));
        Ok(())
    } else {
        Err(resp.msg.unwrap_or_else(|| "daemon refused".into()))
    }
}

fn forward(real: &PathBuf, args: &[OsString]) -> ExitCode {
    // Страховка от бесконечной рекурсии: если «настоящий» файл сам шим, запуск порождал бы копии без конца.
    if shim::has_marker(real) {
        log(&format!("refusing to run {}: it is a shim, not the real file", real.display()));
        return ExitCode::from(1);
    }
    match Command::new(real).args(args).status() {
        // Код возврата отдаём как есть: `ExitCode::from(u8)` обрезал бы коды Squirrel больше 255,
        // а отрицательные (аварийное завершение, 0xC0000005) превратил бы в «успех».
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            log(&format!("cannot run {}: {e}", real.display()));
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    // Не даёт линкеру выбросить маркер: по нему установщик отличает шим от настоящего файла.
    std::hint::black_box(SHIM_MARKER);

    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let Ok(me) = env::current_exe() else { return ExitCode::from(1) };
    let is_stub = me.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("discord"));
    log(&format!("start {} args={:?}", me.display(), args));

    let (real, launch) = if is_stub {
        (me.with_file_name("Discord.real.exe"), args::parse_stub_launch(&args))
    } else {
        (me.with_file_name("Update.real.exe"), args::parse_process_start(&args))
    };

    if let Some(extra) = launch {
        match args::sanitize_launch_args(&extra).and_then(managed_launch) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => log(&format!("fallback to {}: {e}", real.display())),
        }
    }
    forward(&real, &args)
}
