//! The tray icon and its menu.
//!
//! Everything here runs on the event loop's thread. The state is the last scan
//! of listening ports; a scan happens on the tick, after a stop, and when the
//! menu's "Refresh" is used. The menu is only rebuilt when what it would show
//! has changed, because rebuilding closes it on some platforms.

use crate::config::Config;
use crate::icon::{self, Face};
use crate::log::{lerror, linfo, lwarn};
use crate::ports::{self, Listener};
use crate::util::{now_unix, span};
use crate::{autostart, notify, theme};

use std::time::{Duration, Instant, SystemTime};
use tao::event_loop::EventLoopProxy;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
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
        if sig == self.menu_sig && !force {
            return;
        }
        self.menu_sig = sig;

        let tip: Vec<String> = rows
            .iter()
            .take(8)
            .map(|(l, _)| format!(":{} {}", l.port, l.name))
            .collect();
        let tooltip = if tip.is_empty() {
            format!("Porta: {header}")
        } else {
            format!("Porta: {header}\n{}", tip.join("\n"))
        };
        let _ = self.tray.set_tooltip(Some(tooltip));

        let menu = Menu::new();
        let _ = menu.append(&MenuItem::with_id(ID_INERT, &header, false, None));
        if !rows.is_empty() {
            let _ = menu.append(&PredefinedMenuItem::separator());
            for (l, label) in &rows {
                let _ = menu.append(&port_menu(l, label, now));
            }
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        let others = Submenu::new(
            format!("Other apps and services ({})", other_rows.len()),
            !other_rows.is_empty(),
        );
        for (l, label) in &other_rows {
            let _ = others.append(&port_menu(l, label, now));
        }
        let _ = menu.append(&others);
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_SYSTEM,
            "Show system ports",
            true,
            self.cfg.show_system,
            None,
        ));
        let _ = menu.append(&MenuItem::with_id(ID_REFRESH, "Refresh", true, None));
        let _ = menu.append(&MenuItem::with_id(
            ID_EDIT,
            "Edit settings\u{2026}",
            true,
            None,
        ));
        let _ = menu.append(&CheckMenuItem::with_id(
            ID_AUTOSTART,
            "Run at login",
            true,
            autostart_on,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Quit Porta", true, None));

        self.tray.set_menu(Some(Box::new(menu)));
    }
}

/// ":3000  node · vite · up 2h 05m · LAN". Minutes only, so the label (and
/// with it the menu) changes at most once a minute.
fn row_label(l: &Listener, now: u64) -> String {
    let mut parts = vec![menu_text(&l.name)];
    if let Some(h) = l.hint() {
        parts.push(menu_text(&h));
    }
    if let Some(s) = l.started {
        let up = now.saturating_sub(s);
        parts.push(if up < 60 {
            "just started".into()
        } else {
            format!("up {}", span(up))
        });
    }
    if l.exposed {
        parts.push("LAN".into());
    }
    format!(":{}  {}", l.port, parts.join(" \u{00b7} "))
}

/// One port's submenu: open it, what exactly it is, and stop it.
fn port_menu(l: &Listener, label: &str, now: u64) -> Submenu {
    let sub = Submenu::new(label, true);
    let _ = sub.append(&MenuItem::with_id(
        format!("{ID_OPEN}{}", l.port),
        format!("Open http://localhost:{}", l.port),
        true,
        None,
    ));
    let _ = sub.append(&PredefinedMenuItem::separator());
    if !l.command.is_empty() {
        let mut c: String = l.command.chars().take(70).collect();
        if c.len() < l.command.len() {
            c.push('\u{2026}');
        }
        let _ = sub.append(&MenuItem::with_id(ID_INERT, menu_text(&c), false, None));
    }
    let mut facts = vec![format!("pid {}", l.pid)];
    if let Some(s) = l.started {
        facts.push(format!("running {}", span(now.saturating_sub(s))));
    }
    let _ = sub.append(&MenuItem::with_id(
        ID_INERT,
        facts.join(" \u{00b7} "),
        false,
        None,
    ));
    let _ = sub.append(&MenuItem::with_id(
        ID_INERT,
        if l.exposed {
            "Reachable from other devices on your network"
        } else {
            "Only reachable from this computer"
        },
        false,
        None,
    ));
    let _ = sub.append(&PredefinedMenuItem::separator());
    // Stopping part of the OS is never the fix, so it is not offered.
    let _ = sub.append(&MenuItem::with_id(
        format!("{ID_STOP}{}", l.pid),
        format!("Stop {}", menu_text(&l.name)),
        !l.system,
        None,
    ));
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
            started: Some(1_000),
            exposed: true,
            system: false,
            from_terminal: true,
        }
    }

    #[test]
    fn row_labels_say_what_and_how_long() {
        assert_eq!(
            row_label(&l(), 1_000 + 7_500),
            ":3000  node \u{00b7} vite \u{00b7} up 2h 05m \u{00b7} LAN"
        );
        let quiet = Listener {
            command: String::new(),
            started: None,
            exposed: false,
            ..l()
        };
        assert_eq!(row_label(&quiet, 0), ":3000  node");
    }
}
