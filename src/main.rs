// No console window: this is a tray app. No-op off Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! Porta ("door", and "port") shows in the tray what is listening on which
//! port, and stops it, on both Windows and macOS. For "port 3000 is already in
//! use" and "which forgotten dev server is this?".
//!
//! | module            | responsibility                                       |
//! |-------------------|------------------------------------------------------|
//! | `ports`           | listening sockets → processes; stopping one          |
//! | `config`          | the persisted preferences                            |
//! | `icon`            | draws the door, and the dots beside menu rows        |
//! | `art`             | the menu's logos and glyphs (Devicon, Lucide)        |
//! | `tech`            | which framework, server or runtime is behind a port  |
//! | `program_icon`    | the icon of the app behind a port                    |
//! | `notify`          | toasts                                               |
//! | `app`             | the tray icon and its menu                           |
//! | `single_instance` | one copy at a time; relaunching summons it           |
//! | `autostart`       | run at login                                         |

mod app;
mod art;
mod autostart;
mod config;
mod icon;
mod log;
mod notify;
mod ports;
mod program_icon;
mod single_instance;
mod tech;
mod theme;
mod util;

use app::{App, Flow, UserEvent};
use log::{lerror, linfo};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

const USAGE: &str = "\
Porta: see what is listening on your ports, and stop it.

usage:
  porta               run in the tray (or summon the running copy)
  porta list [--all]  print what is listening; --all includes the OS's own
  porta stop <port>   stop whatever is listening on <port>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => tray(),
        Some("--dump-icons") => {
            let dir = args.get(1).map(String::as_str).unwrap_or("target/icons");
            std::process::exit(match icon::dump(dir) {
                Ok(()) => 0,
                Err(_) => 1,
            });
        }
        Some("--dump-art") => {
            let dir = args.get(1).map(String::as_str).unwrap_or("target/art");
            std::process::exit(match art::dump(dir) {
                Ok(()) => 0,
                Err(_) => 1,
            });
        }
        Some(_) => {
            util::attach_console();
            log::init(false);
            std::process::exit(cli(&args));
        }
    }
}

fn cli(args: &[String]) -> i32 {
    match args[0].as_str() {
        "list" | "ls" => {
            let all = args.iter().any(|a| a == "--all" || a == "-a");
            let now = util::now_unix();
            let found: Vec<_> = ports::scan()
                .into_iter()
                .filter(|l| all || !l.system)
                .collect();
            let width = found
                .iter()
                .map(|l| l.name.len())
                .max()
                .unwrap_or(4)
                .clamp(4, 24);
            if found.is_empty() {
                println!("Nothing listening.");
                return 0;
            }
            for l in &found {
                let up = l
                    .started
                    .map(|s| util::span(now.saturating_sub(s)))
                    .unwrap_or_default();
                println!(
                    "{:>6}  {:<width$}  {:>7}  {:<8}  {:<5}  {:<8}  {}",
                    l.port,
                    l.name,
                    l.pid,
                    up,
                    if l.exposed { "LAN" } else { "local" },
                    if l.from_terminal { "terminal" } else { "" },
                    l.project().unwrap_or_default()
                );
            }
            0
        }
        "stop" | "kill" => {
            let Some(port) = args
                .get(1)
                .and_then(|p| p.trim_start_matches(':').parse::<u16>().ok())
            else {
                eprintln!("porta: stop needs a port number, like `porta stop 3000`");
                return 2;
            };
            let targets: Vec<_> = ports::scan()
                .into_iter()
                .filter(|l| l.port == port)
                .collect();
            if targets.is_empty() {
                println!("Nothing is listening on {port}.");
                return 0;
            }
            let mut code = 0;
            for l in targets {
                if l.system {
                    eprintln!(
                        "porta: {} (pid {}) is part of the OS; not stopping it",
                        l.name, l.pid
                    );
                    code = 1;
                    continue;
                }
                match ports::stop(l.pid) {
                    Ok(()) => println!("Stopped {} (pid {}) on {port}.", l.name, l.pid),
                    Err(e) => {
                        eprintln!("porta: could not stop {} (pid {}): {e}", l.name, l.pid);
                        code = 1;
                    }
                }
            }
            code
        }
        "--version" | "-V" => {
            println!("porta {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            0
        }
        other => {
            eprintln!("porta: unknown command '{other}'\n\n{USAGE}");
            2
        }
    }
}

fn tray() {
    log::init(true);
    linfo!(
        "--- Porta {} starting (pid {}) ---",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );

    // A second launch summons the running instance and exits.
    let Some(signal) = single_instance::acquire() else {
        return;
    };

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    // Menu-bar agent, no Dock icon. tao resets the activation policy as it
    // starts, overriding LSUIElement, so it has to be asked for here too.
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        event_loop.set_activate_ignoring_other_apps(false);
    }
    let proxy = event_loop.create_proxy();

    // The loop's own timer can be lost while a menu is open on macOS (tao
    // doesn't wake in the menu's run loop mode), and then Porta would never
    // look again. A thread's nudge every second always gets through; the app
    // decides whether a look is due.
    let tick_proxy = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if tick_proxy.send_event(UserEvent::Tick).is_err() {
            return;
        }
    });

    let summon_proxy = proxy.clone();
    signal.watch(move || {
        let _ = summon_proxy.send_event(UserEvent::Summon);
    });

    // The tray is built once the loop is running: on macOS the status item
    // cannot be created before the application has finished launching.
    let mut app: Option<App> = None;

    event_loop.run(move |event, _target, control_flow| {
        match event {
            Event::NewEvents(StartCause::Init) => {
                #[cfg(target_os = "macos")]
                drop_launch_activation();
                match App::new(proxy.clone()) {
                    Ok(a) => app = Some(a),
                    Err(e) => {
                        lerror!("{e}");
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
            }
            Event::UserEvent(e) => {
                if let Some(a) = app.as_mut() {
                    if a.handle(e) == Flow::Exit {
                        app = None;
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
            }
            _ => {}
        }

        if app.is_some() {
            *control_flow = ControlFlow::Wait;
        }
    });
}

/// tao asks macOS to activate the app as it finishes launching. A menu bar
/// app has nothing to bring forward, and on macOS 26 the request stays
/// pending until the first click on the door, then completes while the menu
/// is open and closes it. Withdraw it before anything is shown.
#[cfg(target_os = "macos")]
fn drop_launch_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).deactivate();
    }
}
