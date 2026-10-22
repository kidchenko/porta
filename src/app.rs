//! The tray icon and its menu.
//!
//! Everything here runs on the event loop's thread. The state is the last scan
//! of listening ports; a scan happens on the tick, after a stop, and when the
//! menu's "Refresh" is used. The menu is only rebuilt when what it would show
//! has changed, because rebuilding closes it on some platforms.

use crate::config::Config;
use crate::icon::{self, Dot, Face};
use crate::log::{lerror, linfo, lwarn};
use crate::ports::{self, Listener};
use crate::util::{now_unix, span};
use crate::{autostart, notify, program_icon, theme};

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use tao::event_loop::EventLoopProxy;
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem,
    Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const ID_OPEN: &str = "open:";
const ID_STOP: &str = "stop:";
const ID_SYSTEM: &str = "system";
const ID_REFRESH: &str = "refresh";
const ID_EDIT: &str = "edit-config";
const ID_AUTOSTART: &str = "autostart";
const ID_QUIT: &str = "quit";
/// Shared by every read-only row; nothing dispatches on it.
const ID_INERT: &str = "inert";

/// A process asked to stop may take a moment to let go of its port. Look again
/// this soon rather than wait for the regular tick.
const AFTER_STOP: Duration = Duration::from_millis(700);

pub enum UserEvent {
    Menu(String),
    /// A second launch of the app.
    Summon,
}

#[derive(PartialEq, Eq)]
pub enum Flow {
    Continue,
    Exit,
}

pub struct App {
    cfg: Config,
    tray: TrayIcon,
    dark_taskbar: bool,
    /// Everything listening, system or not, as of the last scan.
    all: Vec<Listener>,
    /// An early rescan, after a stop.
    soon: Option<Instant>,

    /// Program icons by executable path, read once each. `None` remembers
    /// that there was none, so it is not asked again every scan.
    icons: HashMap<String, Option<(Vec<u8>, u32)>>,

    // What is on screen, so a scan only repaints what changed.
    face: Face,
    menu_sig: String,
    cfg_mtime: Option<SystemTime>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        forward_menu_events(&proxy);

        let dark_taskbar = theme::taskbar_is_dark();
        let tray = TrayIconBuilder::new()
            .with_tooltip("Porta")
            .with_icon(tray_image(Face::Closed, dark_taskbar)?)
            // Either button opens the menu: it is the whole interface.
            .with_menu_on_left_click(true)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .build()
            .map_err(|e| format!("could not create the tray icon: {e}"))?;

        let mut app = Self {
            cfg: Config::load(),
            tray,
            dark_taskbar,
            all: Vec::new(),
            soon: None,
            icons: HashMap::new(),
            face: Face::Closed,
            menu_sig: String::new(),
            cfg_mtime: Config::modified(),
        };
        app.scan();
        app.paint_icon(true);
        Ok(app)
    }

    pub fn handle(&mut self, event: UserEvent) -> Flow {
        match event {
            UserEvent::Summon => {
                self.scan();
                self.announce_summary();
            }
            UserEvent::Menu(id) => return self.menu(&id),
        }
        Flow::Continue
    }

    fn menu(&mut self, id: &str) -> Flow {
        match id {
            ID_QUIT => {
                linfo!("quit requested from the tray");
                return Flow::Exit;
            }
            ID_REFRESH => self.scan(),
            ID_SYSTEM => {
                self.cfg.show_system = !self.cfg.show_system;
                self.save();
            }
            ID_EDIT => open_config(),
            ID_AUTOSTART => {
                let want = !autostart::is_enabled();
                match autostart::set(want) {
                    Ok(()) => linfo!("run at login {}", if want { "on" } else { "off" }),
                    Err(e) => lerror!("{e}"),
                }
            }
            other => {
                if let Some(port) = other.strip_prefix(ID_OPEN) {
                    open_url(&format!("http://localhost:{port}"));
                } else if let Some(pid) = other.strip_prefix(ID_STOP).and_then(|p| p.parse().ok()) {
                    self.stop(pid);
                }
            }
        }
        self.rebuild_menu(true);
        Flow::Continue
    }

    pub fn tick(&mut self) {
        self.soon = None;
        self.reload_if_edited();
        let dark = theme::taskbar_is_dark();
        if dark != self.dark_taskbar {
            self.dark_taskbar = dark;
            self.paint_icon(true);
        }
        self.scan();
    }

    pub fn next_wake(&self) -> Instant {
        let tick = Instant::now() + Duration::from_secs(self.cfg.refresh_seconds);
        self.soon.map_or(tick, |s| s.min(tick))
    }

    fn scan(&mut self) {
        let fresh = ports::scan();
        let key = |l: &Listener| (l.port, l.pid);
        if fresh.iter().map(key).ne(self.all.iter().map(key)) {
            let ports: Vec<String> = fresh
                .iter()
                .filter(|l| !l.system)
                .map(|l| format!("{} {}", l.port, l.name))
                .collect();
            linfo!("listening: {}", ports.join(", "));
        }
        self.all = fresh;
        self.paint_icon(false);
        self.rebuild_menu(false);
    }

    /// Servers you started from a terminal: the top of the menu.
    fn yours(&self) -> Vec<&Listener> {
        self.all
            .iter()
            .filter(|l| l.from_terminal && !l.system)
            .collect()
    }

    /// Everything else the menu lists, under "Other apps and services".
    fn others(&self) -> Vec<&Listener> {
        self.all
            .iter()
            .filter(|l| !(l.from_terminal && !l.system))
            .filter(|l| self.cfg.show_system || !l.system)
            .collect()
    }

    fn stop(&mut self, pid: u32) {
        let Some(l) = self.all.iter().find(|l| l.pid == pid).cloned() else {
            return;
        };
        match ports::stop(pid) {
            Ok(()) => linfo!("stopped {} (pid {pid}) on port {}", l.name, l.port),
            Err(e) => {
                lwarn!("could not stop {} (pid {pid}): {e}", l.name);
                notify::show(
                    &format!("Couldn't stop {}", l.name),
                    &format!("Port {}: {e}", l.port),
                    None,
                );
            }
        }
        self.soon = Some(Instant::now() + AFTER_STOP);
        self.scan();
    }

    fn announce_summary(&self) {
        let yours = self.yours();
        if yours.is_empty() {
            notify::show(
                "Porta",
                "Nothing you started from a terminal is listening.",
                None,
            );
            return;
        }
        let list: Vec<String> = yours
            .iter()
            .take(6)
            .map(|l| format!("{} {}", l.port, l.name))
            .collect();
        notify::show(&self.header(), &list.join(", "), None);
    }

    fn save(&mut self) {
        match self.cfg.save() {
            Ok(()) => self.cfg_mtime = Config::modified(),
            Err(e) => lwarn!("could not save config: {e}"),
        }
    }

    fn reload_if_edited(&mut self) {
        let mtime = Config::modified();
        if mtime.is_none() || mtime == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = mtime;
        if let Some(fresh) = Config::reload() {
            if fresh != self.cfg {
                linfo!("config edited on disk");
                self.cfg = fresh;
                self.rebuild_menu(true);
            }
        }
    }

    fn paint_icon(&mut self, force: bool) {
        let face = if !self.yours().is_empty() {
            Face::Open
        } else {
            Face::Closed
        };
        if face == self.face && !force {
            return;
        }
        self.face = face;
        match tray_image(face, self.dark_taskbar) {
            Ok(img) => {
                let _ = self.tray.set_icon(Some(img));
                #[cfg(target_os = "macos")]
                self.tray.set_icon_as_template(true);
            }
            Err(e) => lerror!("{e}"),
        }
    }

    fn header(&self) -> String {
        match self.yours().len() {
            0 => "Nothing from your terminals".into(),
            1 => "1 server from your terminals".into(),
            n => format!("{n} servers from your terminals"),
        }
    }

    /// The icon for a row: the program's own where there is one, else a dot.
    fn row_icon(&mut self, l: &Listener) -> Option<MenuIcon> {
        let program = self
            .icons
            .entry(l.path.clone())
            .or_insert_with(|| program_icon::load(&l.path))
            .clone();
        let (rgba, edge) = program.unwrap_or_else(|| {
            let kind = if l.from_terminal && !l.system {
                Dot::Yours
            } else {
                Dot::Other
            };
            (icon::dot(kind), 32)
        });
        MenuIcon::from_rgba(rgba, edge, edge).ok()
    }

    fn rebuild_menu(&mut self, force: bool) {
        let now = now_unix();
        let labelled = |ls: Vec<&Listener>| -> Vec<(Listener, String)> {
            ls.into_iter()
                .map(|l| (l.clone(), row_label(l, now)))
                .collect()
        };
        let rows = labelled(self.yours());
        let other_rows = labelled(self.others());
        let header = self.header();
        let autostart_on = autostart::is_enabled();
        let sig = format!(
            "{header}|{:?}|{:?}|{}{autostart_on}",
            rows.iter().map(|(l, s)| (l.pid, s)).collect::<Vec<_>>(),
            other_rows
                .iter()
                .map(|(l, s)| (l.pid, s))
                .collect::<Vec<_>>(),
            self.cfg.show_system
        );
        // Rebuilding closes an open menu on some platforms, so only when
        // something on it actually changed.
        if sig == self.menu_sig && !force {
            return;
        }
        self.menu_sig = sig;

        let tip: Vec<String> = rows
            .iter()
            .take(8)
            .map(|(l, _)| format!(":{} {}", l.port, title(l)))
            .collect();
        let tooltip = if tip.is_empty() {
            format!("Porta: {header}")
        } else {
            format!("Porta: {header}\n{}", tip.join("\n"))
        };
        let _ = self.tray.set_tooltip(Some(tooltip));

        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id(ID_INERT, &header, false, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
        if rows.is_empty() {
            let _ = menu.append(&MenuItem::with_id(
                ID_INERT,
                "Start a dev server and it shows up here",
                false,
                None,
            ));
        }
        for (l, label) in &rows {
            let sub = port_menu(l, label, now);
            sub.set_icon(self.row_icon(l));
            let _ = menu.append(&sub);
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        let others = Submenu::new(
            columns("Other apps and services", &other_rows.len().to_string()),
            !other_rows.is_empty(),
        );
        for (l, label) in &other_rows {
            let sub = port_menu(l, label, now);
            sub.set_icon(self.row_icon(l));
            let _ = others.append(&sub);
        }
        let _ = menu.append(&others);

        let settings = Submenu::new("Settings", true);
        let _ = settings.append_items(&[
            &CheckMenuItem::with_id(
                ID_SYSTEM,
                "Show system ports",
                true,
                self.cfg.show_system,
                None,
            ),
            &CheckMenuItem::with_id(ID_AUTOSTART, "Run at login", true, autostart_on, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(ID_REFRESH, "Refresh now", true, None),
            &MenuItem::with_id(ID_EDIT, "Edit settings file\u{2026}", true, None),
        ]);
        let _ = menu.append(&settings);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Quit Porta", true, None));

        self.tray.set_menu(Some(Box::new(menu)));
    }
}

/// What a row is called: the command-line hint where there is one ("next dev",
/// "manage.py runserver"), which says more than "node" or "python" does.
fn title(l: &Listener) -> String {
    l.hint().unwrap_or_else(|| l.name.clone())
}

/// Left text, with right text in a column against the menu's edge. Windows
/// menus right-align whatever follows a tab, the way they show shortcuts;
/// macOS ones don't, so there the two are just spaced apart.
fn columns(left: &str, right: &str) -> String {
    if right.is_empty() {
        left.to_string()
    } else if cfg!(windows) {
        format!("{left}\t{right}")
    } else {
        format!("{left}    {right}")
    }
}

/// ":3000  next dev" on the left, "LAN · 2h 05m" on the right. Minutes only,
/// so the label (and with it the menu) changes at most once a minute.
fn row_label(l: &Listener, now: u64) -> String {
    let mut right = Vec::new();
    if l.exposed {
        right.push("LAN".to_string());
    }
    if let Some(s) = l.started {
        let up = now.saturating_sub(s);
        right.push(if up < 60 { "now".into() } else { span(up) });
    }
    columns(
        &format!(":{}  {}", l.port, menu_text(&title(l))),
        &right.join(" \u{00b7} "),
    )
}

/// One port's submenu: the two actions first, then what exactly it is.
fn port_menu(l: &Listener, label: &str, now: u64) -> Submenu {
    let sub = Submenu::new(label, true);
    let _ = sub.append(&MenuItem::with_id(
        format!("{ID_OPEN}{}", l.port),
        columns("Open in browser", &format!("localhost:{}", l.port)),
        true,
        None,
    ));
    // Stopping part of the OS is never the fix, so it is not offered.
    let stop_label = columns(
        &format!("Stop {}", menu_text(&l.name)),
        &format!("pid {}", l.pid),
    );
    let _ = sub.append(&IconMenuItem::with_id(
        format!("{ID_STOP}{}", l.pid),
        stop_label,
        !l.system,
        MenuIcon::from_rgba(icon::stop_mark(), 32, 32).ok(),
        None,
    ));
    let _ = sub.append(&PredefinedMenuItem::separator());

    let mut info = Vec::new();
    if !l.command.is_empty() {
        let mut c: String = l.command.chars().take(70).collect();
        if c.len() < l.command.len() {
            c.push('\u{2026}');
        }
        info.push(menu_text(&c));
    }
    match l.started {
        Some(s) => info.push(format!(
            "{} \u{00b7} running {}",
            menu_text(&l.name),
            span(now.saturating_sub(s))
        )),
        None => info.push(menu_text(&l.name)),
    }
    info.push(
        if l.exposed {
            "Reachable from other devices on your network"
        } else {
            "Only reachable from this computer"
        }
        .into(),
    );
    for line in info {
        let _ = sub.append(&MenuItem::with_id(ID_INERT, line, false, None));
    }
    sub
}

/// Windows menus treat `&` as an accelerator marker.
fn menu_text(s: &str) -> String {
    if cfg!(windows) {
        s.replace('&', "&&")
    } else {
        s.to_string()
    }
}

fn open_url(url: &str) {
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // `start` hands the URL to the default browser; the empty title
        // argument keeps it from being mistaken for one.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    if let Err(e) = spawned {
        lwarn!("could not open {url}: {e}");
    }
}

/// Open config.json in a text editor, creating it first if need be.
fn open_config() {
    let Some(path) = Config::path() else { return };
    if !path.exists() {
        let _ = Config::load().save();
    }
    #[cfg(windows)]
    let spawned = std::process::Command::new("notepad.exe").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("/usr/bin/open")
        .arg("-t")
        .arg(&path)
        .spawn();
    if let Err(e) = spawned {
        lwarn!("could not open {}: {e}", path.display());
    }
}

fn tray_image(face: Face, dark: bool) -> Result<Icon, String> {
    Icon::from_rgba(icon::render(face, dark, icon::EDGE), icon::EDGE, icon::EDGE)
        .map_err(|e| format!("could not build the tray image: {e}"))
}

fn forward_menu_events(proxy: &EventLoopProxy<UserEvent>) {
    let proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = proxy.send_event(UserEvent::Menu(e.id.0));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l() -> Listener {
        Listener {
            port: 3000,
            pid: 42,
            name: "node".into(),
            command: "node /app/node_modules/vite/bin/vite.js".into(),
            path: String::new(),
            started: Some(1_000),
            exposed: true,
            system: false,
            from_terminal: true,
        }
    }

    #[test]
    fn row_labels_say_what_and_how_long() {
        let gap = if cfg!(windows) { "\t" } else { "    " };
        assert_eq!(
            row_label(&l(), 1_000 + 7_500),
            format!(":3000  vite{gap}LAN \u{00b7} 2h 05m")
        );
        let quiet = Listener {
            command: String::new(),
            started: None,
            exposed: false,
            ..l()
        };
        assert_eq!(row_label(&quiet, 0), ":3000  node");
        assert_eq!(title(&quiet), "node");
    }
}
