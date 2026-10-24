//! The tray icon and its menu.
//!
//! Everything here runs on the event loop's thread. The state is the last scan
//! of listening ports; a scan happens on the tick, after a stop, and when the
//! menu's "Refresh" is used. The menu is only rebuilt when what it would show
//! has changed, because rebuilding closes it on some platforms.

use crate::art::{self, Ink, Rgba};
use crate::config::Config;
use crate::icon::{self, Dot, Face};
use crate::log::{lerror, linfo, lwarn};
use crate::ports::{self, Listener};
use crate::util::{now_unix, span};
use crate::{autostart, notify, program_icon, tech, theme};

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime};
use tao::event_loop::EventLoopProxy;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{
    CheckMenuItem, Icon as MenuIcon, IconMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem,
    PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

const ID_OPEN: &str = "open:";
const ID_COPY: &str = "copy:";
const ID_FOLDER: &str = "folder:";
const ID_COPY_LAN: &str = "copy-lan:";
const ID_STOP: &str = "stop:";
#[cfg(target_os = "macos")]
const ID_KILL: &str = "kill:";
const ID_STOP_ALL: &str = "stop-all";
const ID_COUNT: &str = "show-count";
const ID_SYSTEM: &str = "system";
const ID_REFRESH: &str = "refresh";
const ID_EDIT: &str = "edit-config";
const ID_AUTOSTART: &str = "autostart";
const ID_UPDATES: &str = "check-updates";
const ID_QUIT: &str = "quit";
/// Shared by every read-only row; nothing dispatches on it.
const ID_INERT: &str = "inert";

/// While a rebuild waits for the menu to close.
const MENU_RETRY: Duration = Duration::from_secs(1);

/// A process asked to stop may take a moment to let go of its port. Look again
/// this soon rather than wait for the regular tick.
const AFTER_STOP: Duration = Duration::from_millis(700);

pub enum UserEvent {
    Menu(String),
    /// Once a second, from a thread: time to see whether a look is due.
    Tick,
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
    /// Whether menus are drawn dark, for glyphs and black logos.
    dark_menus: bool,
    /// Everything listening, system or not, as of the last scan.
    all: Vec<Listener>,
    /// This machine's address on the network, as of the last scan.
    lan: Option<IpAddr>,
    /// An early rescan, after a stop.
    soon: Option<Instant>,
    /// When the regular look is due.
    due: Instant,

    /// Pictures by what they are ("app:/path", "tech:nodejs:dark",
    /// "glyph:copy:Text:light"), made once each. `None` remembers that there
    /// was none, so it is not asked again every scan.
    icons: HashMap<String, Option<Rgba>>,

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
            dark_menus: theme::menus_are_dark(),
            all: Vec::new(),
            lan: None,
            soon: None,
            due: Instant::now(),
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
            UserEvent::Tick => {
                if Instant::now() >= self.next_wake() {
                    self.tick();
                }
            }
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
            ID_COUNT => {
                self.cfg.show_count = !self.cfg.show_count;
                self.save();
                self.paint_icon(true);
            }
            ID_STOP_ALL => {
                let pids: Vec<u32> = self.yours().iter().map(|l| l.pid).collect();
                for pid in pids {
                    self.stop(pid, false);
                }
            }
            ID_EDIT => open_config(),
            // The page compares versions; Porta itself stays off the network.
            ID_UPDATES => open_url(concat!(
                "https://josebarbosa.dev/porta/?v=",
                env!("CARGO_PKG_VERSION")
            )),
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
                } else if let Some(port) = other.strip_prefix(ID_COPY) {
                    copy(&format!("http://localhost:{port}"));
                } else if let Some(pid) = other
                    .strip_prefix(ID_COPY_LAN)
                    .and_then(|p| p.parse::<u32>().ok())
                {
                    let url = self.all.iter().find(|l| l.pid == pid);
                    if let Some(url) = url.and_then(|l| l.network_url(self.lan)) {
                        copy(&url);
                    }
                } else if let Some(pid) = other
                    .strip_prefix(ID_FOLDER)
                    .and_then(|p| p.parse::<u32>().ok())
                {
                    if let Some(l) = self.all.iter().find(|l| l.pid == pid) {
                        open_folder(&l.cwd);
                    }
                } else if let Some(pid) = other.strip_prefix(ID_STOP).and_then(|p| p.parse().ok()) {
                    self.stop(pid, false);
                } else {
                    #[cfg(target_os = "macos")]
                    if let Some(pid) = other.strip_prefix(ID_KILL).and_then(|p| p.parse().ok()) {
                        self.stop(pid, true);
                    }
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
        // Rebuilt with the new colours by the scan, as it is in the signature.
        self.dark_menus = theme::menus_are_dark();
        self.scan();
        self.due = Instant::now() + Duration::from_secs(self.cfg.refresh_seconds);
    }

    /// When to look next: the regular tick, or sooner after a stop or while
    /// a rebuild waits for the menu to close.
    fn next_wake(&self) -> Instant {
        self.soon.map_or(self.due, |s| s.min(self.due))
    }

    fn scan(&mut self) {
        let fresh = ports::scan();
        self.lan = ports::lan_address();
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

    /// Ask `pid` to stop, or with `force`, end it (macOS; Windows only has
    /// the forceful kind).
    fn stop(&mut self, pid: u32, force: bool) {
        let Some(l) = self.all.iter().find(|l| l.pid == pid).cloned() else {
            return;
        };
        #[cfg(target_os = "macos")]
        let result = if force {
            ports::force_stop(pid)
        } else {
            ports::stop(pid)
        };
        #[cfg(windows)]
        let result = {
            let _ = force;
            ports::stop(pid)
        };
        match result {
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
        let count = self.yours().len();
        // An empty title rather than `None`: tray-icon 0.24 ignores `None`,
        // which would leave the last count showing after the servers stop.
        #[cfg(target_os = "macos")]
        self.tray
            .set_title(Some(if self.cfg.show_count && count > 0 {
                count.to_string()
            } else {
                String::new()
            }));
        let face = if count > 0 { Face::Open } else { Face::Closed };
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

    /// A picture, made the first time it is asked for.
    fn picture(&mut self, key: String, make: impl FnOnce() -> Option<Rgba>) -> Option<MenuIcon> {
        let (rgba, edge) = self.icons.entry(key).or_insert_with(make).clone()?;
        MenuIcon::from_rgba(rgba, edge, edge).ok()
    }

    fn glyph(&mut self, name: &'static str, ink: Ink) -> Option<MenuIcon> {
        let dark = self.dark_menus;
        self.picture(format!("glyph:{name}:{ink:?}:{dark}"), || {
            art::glyph(name, ink, dark)
        })
    }

    fn logo(&mut self, key: &'static str) -> Option<MenuIcon> {
        let dark = self.dark_menus;
        self.picture(format!("tech:{key}:{dark}"), || art::tech(key, dark))
    }

    /// The icon for a row: the logo of the technology behind it where Porta
    /// knows it ("next dev" gets Next.js'), else the program's own icon,
    /// else a dot.
    fn row_icon(&mut self, l: &Listener) -> Option<MenuIcon> {
        if let Some(t) = tech::of(l) {
            if let Some(logo) = self.logo(t.key) {
                return Some(logo);
            }
        }
        let path = l.path.clone();
        if let Some(own) = self.picture(format!("app:{path}"), || program_icon::load(&path)) {
            return Some(own);
        }
        let kind = if l.from_terminal && !l.system {
            Dot::Yours
        } else {
            Dot::Other
        };
        MenuIcon::from_rgba(icon::dot(kind), 32, 32).ok()
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
            "{header}|{:?}|{:?}|{}{}{autostart_on}{}{:?}",
            rows.iter().map(|(l, s)| (l.pid, s)).collect::<Vec<_>>(),
            other_rows
                .iter()
                .map(|(l, s)| (l.pid, s))
                .collect::<Vec<_>>(),
            self.cfg.show_system,
            self.cfg.show_count,
            self.dark_menus,
            self.lan,
        );
        // Rebuilding closes an open menu on some platforms, so only when
        // something on it actually changed, and on macOS not while it's open:
        // the uptimes alone change every minute. The signature is left as it
        // was, so the next look after it closes does the rebuild.
        if sig == self.menu_sig && !force {
            return;
        }
        if menu_is_open() {
            // Check back soon for the menu closing, so it's never opened stale.
            let retry = Instant::now() + MENU_RETRY;
            self.soon = Some(self.soon.map_or(retry, |s| s.min(retry)));
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
            let _ = menu.append(&IconMenuItem::with_id(
                ID_INERT,
                "Start a dev server and it shows up here",
                false,
                self.glyph("terminal", Ink::Text),
                None,
            ));
        }
        for (l, label) in &rows {
            let sub = self.port_menu(l, label, now);
            let _ = menu.append(&sub);
        }
        if rows.len() > 1 {
            let _ = menu.append(&IconMenuItem::with_id(
                ID_STOP_ALL,
                format!("Stop all {} servers", rows.len()),
                true,
                self.glyph("circle-stop", Ink::Danger),
                None,
            ));
        }

        let _ = menu.append(&PredefinedMenuItem::separator());
        let others = Submenu::new(
            columns("Other apps and services", &other_rows.len().to_string()),
            !other_rows.is_empty(),
        );
        for (l, label) in &other_rows {
            let sub = self.port_menu(l, label, now);
            let _ = others.append(&sub);
        }
        let _ = menu.append(&others);

        let settings = Submenu::new("Settings", true);
        let refresh = IconMenuItem::with_id(
            ID_REFRESH,
            "Refresh now",
            true,
            self.glyph("refresh-cw", Ink::Text),
            shortcut('r'),
        );
        let edit = IconMenuItem::with_id(
            ID_EDIT,
            "Edit settings file\u{2026}",
            true,
            self.glyph("file-cog", Ink::Text),
            shortcut(','),
        );
        let system = CheckMenuItem::with_id(
            ID_SYSTEM,
            "Show system ports",
            true,
            self.cfg.show_system,
            None,
        );
        let login = CheckMenuItem::with_id(ID_AUTOSTART, "Run at login", true, autostart_on, None);
        let count = CheckMenuItem::with_id(
            ID_COUNT,
            "Show count in menu bar",
            true,
            self.cfg.show_count,
            None,
        );
        let mut items: Vec<&dyn IsMenuItem> = vec![&system, &login];
        if cfg!(target_os = "macos") {
            items.push(&count);
        }
        let sep = PredefinedMenuItem::separator();
        let version = MenuItem::with_id(
            ID_INERT,
            concat!("Porta ", env!("CARGO_PKG_VERSION")),
            false,
            None,
        );
        items.extend([&sep as &dyn IsMenuItem, &refresh, &edit]);
        let sep2 = PredefinedMenuItem::separator();
        items.extend([&sep2 as &dyn IsMenuItem, &version]);
        let _ = settings.append_items(&items);
        let _ = menu.append(&settings);
        let _ = menu.append(&MenuItem::with_id(
            ID_UPDATES,
            "Check for updates\u{2026}",
            true,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&IconMenuItem::with_id(
            ID_QUIT,
            "Quit Porta",
            true,
            self.glyph("power", Ink::Text),
            shortcut('q'),
        ));

        self.tray.set_menu(Some(Box::new(menu)));
    }

    /// One port's submenu: what to do with it first, the one with a
    /// consequence set apart, then what exactly it is.
    fn port_menu(&mut self, l: &Listener, label: &str, now: u64) -> Submenu {
        let sub = Submenu::new(label, true);
        sub.set_icon(self.row_icon(l));
        let _ = sub.append(&IconMenuItem::with_id(
            format!("{ID_OPEN}{}", l.port),
            columns("Open in browser", &format!("localhost:{}", l.port)),
            true,
            self.glyph("globe", Ink::Text),
            None,
        ));
        let _ = sub.append(&IconMenuItem::with_id(
            format!("{ID_COPY}{}", l.port),
            "Copy URL",
            true,
            self.glyph("copy", Ink::Text),
            None,
        ));
        // Where other devices reach it, up with the actions: clicking it
        // copies the address, to type into a phone or paste in a chat.
        let network = l.network_url(self.lan);
        if let Some(url) = &network {
            let _ = sub.append(&IconMenuItem::with_id(
                format!("{ID_COPY_LAN}{}", l.pid),
                format!(
                    "Reachable from other devices at {}",
                    url.trim_start_matches("http://")
                ),
                true,
                self.glyph("wifi", Ink::Accent),
                None,
            ));
        }
        if let Some(project) = l.project() {
            let open = if cfg!(target_os = "macos") {
                "Show in Finder"
            } else {
                "Open folder"
            };
            let _ = sub.append(&IconMenuItem::with_id(
                format!("{ID_FOLDER}{}", l.pid),
                columns(open, &menu_text(&project)),
                true,
                self.glyph("folder-open", Ink::Text),
                None,
            ));
        }
        let _ = sub.append(&PredefinedMenuItem::separator());
        // Stopping part of the OS is never the fix, so it is not offered.
        let _ = sub.append(&IconMenuItem::with_id(
            format!("{ID_STOP}{}", l.pid),
            columns(
                &format!("Stop {}", menu_text(&l.name)),
                &format!("pid {}", l.pid),
            ),
            !l.system,
            self.glyph("circle-stop", Ink::Danger),
            None,
        ));
        #[cfg(target_os = "macos")]
        let _ = sub.append(&IconMenuItem::with_id(
            format!("{ID_KILL}{}", l.pid),
            "Force quit",
            !l.system,
            self.glyph("octagon-x", Ink::Danger),
            None,
        ));
        let _ = sub.append(&PredefinedMenuItem::separator());

        let mut info: Vec<(Option<MenuIcon>, String)> = Vec::new();
        if let Some(t) = tech::of(l) {
            let what = if t.label.eq_ignore_ascii_case(&l.name) {
                t.label.to_string()
            } else {
                format!("{} \u{00b7} {}", t.label, menu_text(&l.name))
            };
            info.push((self.logo(t.key), what));
        }
        if !l.command.is_empty() {
            let mut c: String = l.command.chars().take(70).collect();
            if c.len() < l.command.len() {
                c.push('\u{2026}');
            }
            info.push((self.glyph("terminal", Ink::Text), menu_text(&c)));
        }
        if let Some(s) = l.started {
            info.push((
                self.glyph("clock", Ink::Text),
                format!("Running for {}", span(now.saturating_sub(s))),
            ));
        }
        // With an address, reachability is the clickable row above.
        if !l.exposed {
            info.push((
                self.glyph("house", Ink::Text),
                "Only reachable from this computer".into(),
            ));
        } else if network.is_none() {
            info.push((
                self.glyph("wifi", Ink::Accent),
                "Reachable from other devices on your network".into(),
            ));
        }
        for (pic, line) in info {
            let _ = sub.append(&IconMenuItem::with_id(ID_INERT, line, false, pic, None));
        }
        sub
    }
}

/// What a row is called: the command-line hint where there is one ("next dev",
/// "manage.py runserver"), which says more than "node" or "python" does.
///
/// The project folder comes first where there is one: "juanalist" says which
/// server it is better than "next dev" does once two projects use Next.js.
fn title(l: &Listener) -> String {
    match (l.project(), l.hint()) {
        (Some(p), Some(h)) => format!("{p} \u{00b7} {h}"),
        (Some(p), None) => p,
        (None, h) => h.unwrap_or_else(|| l.name.clone()),
    }
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

/// Whether a menu is open right now. On macOS the run loop is in event
/// tracking mode for exactly as long as a menu is being shown.
#[cfg(target_os = "macos")]
fn menu_is_open() -> bool {
    use objc2_app_kit::NSEventTrackingRunLoopMode;
    use objc2_foundation::NSRunLoop;
    let mode = NSRunLoop::currentRunLoop().currentMode();
    mode.is_some_and(|m| *m == *unsafe { NSEventTrackingRunLoopMode })
}

/// Windows has no such signal. Its tray menu runs a modal loop of its own,
/// and this problem hasn't been seen there.
#[cfg(windows)]
fn menu_is_open() -> bool {
    false
}

/// ⌘ and a key, on macOS. Windows tray menus can't take keyboard shortcuts.
fn shortcut(key: char) -> Option<Accelerator> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    use tray_icon::menu::accelerator::{Code, Modifiers};
    let code = match key {
        'r' => Code::KeyR,
        'q' => Code::KeyQ,
        ',' => Code::Comma,
        _ => return None,
    };
    Some(Accelerator::new(Some(Modifiers::SUPER), code))
}

/// Windows menus treat `&` as an accelerator marker.
fn menu_text(s: &str) -> String {
    if cfg!(windows) {
        s.replace('&', "&&")
    } else {
        s.to_string()
    }
}

/// Show a folder in Finder or Explorer.
fn open_folder(dir: &str) {
    if dir.is_empty() {
        return;
    }
    #[cfg(windows)]
    let spawned = std::process::Command::new("explorer.exe").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("/usr/bin/open").arg(dir).spawn();
    if let Err(e) = spawned {
        lwarn!("could not open {dir}: {e}");
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

/// Put `text` on the clipboard.
fn copy(text: &str) {
    use std::io::Write;
    #[cfg(windows)]
    let child = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let child = std::process::Command::new("/usr/bin/pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn();
    let done = child.and_then(|mut c| {
        c.stdin
            .take()
            .map(|mut i| i.write_all(text.as_bytes()))
            .transpose()?;
        c.wait().map(|_| ())
    });
    if let Err(e) = done {
        lwarn!("could not copy {text}: {e}");
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
            cwd: String::new(),
            started: Some(1_000),
            exposed: true,
            bound: None,
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

    #[test]
    fn network_urls() {
        let lan: IpAddr = "192.168.1.110".parse().unwrap();
        assert_eq!(
            l().network_url(Some(lan)).as_deref(),
            Some("http://192.168.1.110:3000")
        );
        let pinned = Listener {
            bound: Some("10.0.0.7".parse().unwrap()),
            ..l()
        };
        assert_eq!(
            pinned.network_url(Some(lan)).as_deref(),
            Some("http://10.0.0.7:3000")
        );
        let v6 = Listener {
            bound: Some("fe80::1".parse().unwrap()),
            ..l()
        };
        assert_eq!(
            v6.network_url(None).as_deref(),
            Some("http://[fe80::1]:3000")
        );
        let local = Listener {
            exposed: false,
            ..l()
        };
        assert_eq!(local.network_url(Some(lan)), None);
        assert_eq!(l().network_url(None), None, "offline: no address to give");
    }

    #[test]
    fn the_project_folder_leads() {
        let next = Listener {
            command: "next-server (v16.1.5)".into(),
            cwd: "/Users/me/projects/juanalist".into(),
            ..l()
        };
        assert_eq!(title(&next), "juanalist \u{00b7} next-server");
        let bare = Listener {
            command: String::new(),
            ..next
        };
        assert_eq!(title(&bare), "juanalist");
    }
}
