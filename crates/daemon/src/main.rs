//! discordctl-daemon: единственный владелец CDP-канала к Discord.
//!
//! Умеет: запустить Discord с пайпом, переслать аргументы уже запущенному, отдать статус,
//! следить за целостностью шима, поставить хук в основное окно (`cdp` + `hook`).

#![windows_subsystem = "windows"]

mod auth;
mod cdp;
mod hook;
mod launch;
mod watch;

use discordctl_core::{
    args,
    config::Config,
    handler,
    ipc::{self, Listener},
    paths,
    protocol::{Request, Response},
    shim,
};
use std::{
    fs::File,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{WaitForSingleObject, INFINITE},
};

/// Запущенный нами Discord. Пока жива запись, жив и пайп.
struct Managed {
    pid: u32,
    app_dir: PathBuf,
    /// Владеет записывающим концом пайпа: пока жив, Discord не получит EOF и не завершится.
    _cdp: Arc<cdp::Cdp>,
}

static STATE: Mutex<Option<Managed>> = Mutex::new(None);

fn log(msg: &str) {
    paths::log("daemon", msg);
}

fn launch(args_in: Vec<String>) -> Response {
    let extra = match args::sanitize_launch_args(&args_in) {
        Ok(a) => a,
        Err(e) => return Response::err(e),
    };
    let root = Config::load().root();
    let Some(app_dir) = paths::latest_app_dir(&root) else {
        return Response::err(format!("no app-* folder with Discord.exe in {}", root.display()));
    };
    let exe = app_dir.join("Discord.exe");

    let mut state = STATE.lock().unwrap();
    if state.is_some() {
        // Discord уже под нашим управлением: обычный запуск перешлёт аргументы (ссылки, фокус) в работающий экземпляр.
        return match Command::new(&exe).args(&extra).current_dir(&app_dir).spawn() {
            Ok(_) => Response::ok("forwarded to running managed instance"),
            Err(e) => Response::err(format!("cannot forward: {e}")),
        };
    }

    let sp = match launch::spawn_with_cdp(&exe, &app_dir, &extra) {
        Ok(s) => s,
        Err(e) => return Response::err(format!("cannot start Discord: {e}")),
    };
    let pid = sp.pid;
    let process = sp.process as usize;
    let (cdp, events) = cdp::Cdp::start(sp.to_browser, sp.from_browser);
    *state = Some(Managed { pid, app_dir, _cdp: cdp.clone() });
    drop(state);
    log(&format!("started Discord pid {pid}"));

    thread::spawn(move || hook::run(cdp, events));
    // Когда процесс завершился, снимаем отметку «управляется» и закрываем пайп.
    thread::spawn(move || unsafe {
        let h = process as _;
        WaitForSingleObject(h, INFINITE);
        CloseHandle(h);
        let mut st = STATE.lock().unwrap();
        if st.as_ref().is_some_and(|m| m.pid == pid) {
            *st = None;
            hook::reset();
        }
        log(&format!("Discord pid {pid} exited"));
        // После `discordctl uninstall` демон ждал, пока Discord закроют, чтобы не закрыть его самому.
        if !Config::load().shim_installed {
            log("shim is uninstalled and Discord has exited: daemon exits");
            std::process::exit(0);
        }
    });
    Response::ok(format!("started pid {pid}"))
}

fn status() -> Response {
    let cfg = Config::load();
    let managed = STATE.lock().unwrap().as_ref().map(|m| serde_json::json!({ "pid": m.pid, "app_dir": m.app_dir }));
    Response::ok("running").with_data(serde_json::json!({
        "daemon_pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "exe_sha256": std::env::current_exe().ok().and_then(|p| paths::sha256_file(&p)),
        "discord_root": cfg.root(),
        "managed_discord": managed,
        "hook": hook::snapshot(),
        "shim": { "update": shim::state(&cfg.root(), shim::Target::Update), "stub": shim::state(&cfg.root(), shim::Target::Stub) },
    }))
}

/// Запросы приложения (с токеном). Всё, что здесь не перечислено, приложениям недоступно.
fn handle_app(app: &auth::App, req: Request) -> Response {
    match req {
        Request::Ping => Response::ok("pong"),
        Request::Whoami => {
            Response::ok("app").with_data(serde_json::json!({ "app": app.name, "permissions": app.permissions }))
        }
        Request::Call { method, params } => match auth::app_call(app, &method, params) {
            Ok(v) => Response::ok("ok").with_data(v),
            Err(e) => Response::err(e),
        },
        _ => {
            auth::audit(&app.name, "request", None, "denied", "owner-only request");
            Response::err("this request is available to the owner only")
        }
    }
}

/// Запросы владельца (CLI без токена).
fn handle_owner(req: Request) -> (Response, bool) {
    let resp = match req {
        Request::Ping => Response::ok("pong"),
        Request::Whoami => Response::ok("owner"),
        Request::Status => status(),
        Request::Launch { args } => launch(args),
        Request::HookReload { payload_path } => match hook::reload(payload_path.as_deref()) {
            Ok(()) => Response::ok("payload reloaded"),
            Err(e) => Response::err(e),
        },
        Request::Call { method, params } => match hook::call(&method, params) {
            Ok(v) => Response::ok("ok").with_data(v),
            Err(e) => Response::err(e),
        },
        Request::AuthGrant { name, permissions } => match auth::grant(&name, permissions) {
            Ok(token) => Response::ok("granted").with_data(serde_json::json!({ "token": token })),
            Err(e) => Response::err(e),
        },
        Request::AuthRevoke { name } => match auth::revoke(&name) {
            Ok(true) => Response::ok("revoked"),
            Ok(false) => Response::err(format!("no app named {name}")),
            Err(e) => Response::err(e),
        },
        Request::AuthList => Response::ok("apps").with_data(auth::list()),
        Request::AuthAudit { limit } => Response::ok("audit").with_data(auth::audit_tail(limit)),
        Request::Approvals => Response::ok("pending").with_data(auth::pending()),
        Request::Decide { id, allow } => {
            if auth::decide(id, allow) {
                Response::ok(if allow { "allowed" } else { "rejected" })
            } else {
                Response::err("no such pending request (already decided or timed out)")
            }
        }
        Request::Shutdown { force } => {
            if !force && STATE.lock().unwrap().is_some() {
                return (
                    Response::err("managed Discord is running; closing the pipe would close Discord (use force)"),
                    false,
                );
            }
            return (Response::ok("bye"), true);
        }
    };
    (resp, false)
}

fn handle(conn: File) {
    let env = match ipc::read_request(&conn) {
        Ok(r) => r,
        Err(e) => {
            let _ = ipc::write_response(&conn, &Response::err(format!("bad request: {e}")));
            return;
        }
    };
    let (resp, exit) = match env.token {
        // Неверный токен не превращается в доступ владельца.
        Some(token) => match auth::identify(&token) {
            Some(app) => (handle_app(&app, env.req), false),
            None => {
                auth::audit("?", "auth", None, "denied", "invalid or revoked token");
                (Response::err("invalid or revoked token"), false)
            }
        },
        None => handle_owner(env.req),
    };
    let _ = ipc::write_response(&conn, &resp);
    if exit {
        log("shutdown requested");
        std::process::exit(0);
    }
}

/// Один проход починки. Возвращает `true`, если все цели в порядке.
/// Писатель (Squirrel) может ещё держать файл, поэтому вызывающий повторяет попытки.
fn repair_once() -> bool {
    let cfg = Config::load();
    if !cfg.shim_installed {
        return true;
    }
    let src = paths::bin_dir().join("discordctl-shim.exe");
    let mut all_ok = true;
    for t in shim::TARGETS {
        match shim::repair(&cfg.root(), t, &src) {
            Ok(true) => log(&format!("{} was overwritten by the updater; shim repaired", t.file())),
            Ok(false) => {}
            Err(e) => {
                all_ok = false;
                log(&format!("shim repair of {} failed: {e:#}", t.file()));
            }
        }
    }
    all_ok
}

/// Чинит сразу после изменения файлов в корне Discord, с короткими повторами.
fn repair_with_retries() {
    for _ in 0..20 {
        if repair_once() {
            return;
        }
        thread::sleep(Duration::from_millis(150));
    }
}

/// Страховка: раз в полминуты проверяет шим, даже если событие файловой системы пропущено.
/// Возвращает обработчик `discord://` на заглушку и запоминает, что стояло до нас.
fn apply_handler() {
    let cfg = Config::load();
    match handler::apply(&cfg) {
        Ok(handler::Applied::Written { previous }) => {
            log("discord:// handler pointed back at the stub");
            if let Some(prev) = previous {
                // Свою же команду и версионный путь Discord как «оригинал» не запоминаем: откатываться к ним бессмысленно.
                let mut cfg = Config::load();
                if cfg.handler_original.is_none() {
                    cfg.handler_original = Some(prev);
                    let _ = cfg.save();
                }
            }
        }
        Ok(_) => {}
        Err(e) => log(&format!("discord:// handler: {e}")),
    }
}

/// Discord переписывает обработчик при каждом запуске; ждём изменений ключа и чиним сразу, с короткой паузой,
/// пока писатель закончит. Если ключа ещё нет, проверяем раз в 30 секунд.
fn handler_loop() {
    loop {
        apply_handler();
        if handler::wait_change().is_ok() {
            thread::sleep(Duration::from_millis(300));
        } else {
            thread::sleep(Duration::from_secs(30));
        }
    }
}

fn repair_loop() {
    loop {
        thread::sleep(Duration::from_secs(30));
        repair_once();
    }
}

fn main() {
    let mut listener = match Listener::bind(&paths::pipe_name()) {
        Ok(l) => l,
        Err(e) => {
            // Скорее всего демон уже запущен.
            log(&format!("cannot bind pipe (already running?): {e}"));
            return;
        }
    };
    log(&format!("daemon started pid {}", std::process::id()));
    // Шим мог быть затёрт, пока демон не работал: проверяем сразу, а не через 30 секунд.
    thread::spawn(repair_with_retries);
    thread::spawn(repair_loop);
    thread::spawn(handler_loop);
    thread::spawn(|| watch::watch_root(&Config::load().root(), repair_with_retries));
    loop {
        match listener.accept() {
            Ok(conn) => {
                thread::spawn(move || handle(conn));
            }
            Err(e) => {
                log(&format!("accept failed: {e}"));
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
