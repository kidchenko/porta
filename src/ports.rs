//! What is listening on which TCP port, and stopping it.
//!
//! One `Listener` per (port, process): IPv4 and IPv6 sockets for the same
//! server are folded together. Each carries what a person needs to recognise
//! it: the process name, a hint from its command line ("vite", "manage.py",
//! "http.server"), how long it has been up, and whether it can be reached from
//! the network or only from this machine.
//!
//! It also says whether the process was started from a terminal: whether a
//! shell or terminal app is among its ancestors. That is what separates the dev
//! server you forgot about from the dozen ports desktop apps keep open for
//! themselves (Spotify, VS Code, OneDrive...).
//!
//! | platform | sockets                         | process details                    |
//! |----------|---------------------------------|------------------------------------|
//! | Windows  | `GetExtendedTcpTable`           | Toolhelp, `GetProcessTimes`, the command line via `NtQueryInformationProcess` |
//! | macOS    | `lsof -iTCP -sTCP:LISTEN`       | `ps`                               |

use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Listener {
    pub port: u16,
    pub pid: u32,
    /// Executable name without `.exe`: "node", "python", "postgres".
    pub name: String,
    /// The full command line, where the OS lets us read it.
    pub command: String,
    /// The executable's full path, for its icon. Empty where unknown.
    pub path: String,
    /// The directory it runs in, which is usually the project's. Empty where
    /// unknown (always, so far, on Windows).
    pub cwd: String,
    /// When the process started, as Unix time.
    pub started: Option<u64>,
    /// Bound to something other than loopback: other machines can connect.
    pub exposed: bool,
    /// The one network address it is bound to, where it is bound to one
    /// rather than to all of them (`0.0.0.0`, `::`).
    pub bound: Option<IpAddr>,
    /// Part of the OS rather than something you started.
    pub system: bool,
    /// A shell or terminal is among its ancestors: you started it by hand.
    pub from_terminal: bool,
}

impl Listener {
    pub fn hint(&self) -> Option<String> {
        hint(&self.command, &self.name)
    }

    /// The project it serves, by its folder's name: "juanalist" for a server
    /// started in ~/projects/juanalist. `None` for the home folder or `/`,
    /// where daemons and apps run and the name says nothing.
    pub fn project(&self) -> Option<String> {
        project(&self.cwd)
    }

    /// The address other devices reach it at: the one it is bound to, or,
    /// listening on all of them, this machine's address on the LAN (`lan`).
    /// `None` when it only listens on loopback.
    pub fn network_url(&self, lan: Option<IpAddr>) -> Option<String> {
        if !self.exposed {
            return None;
        }
        Some(match self.bound.or(lan)? {
            IpAddr::V4(a) => format!("http://{a}:{}", self.port),
            IpAddr::V6(a) => format!("http://[{a}]:{}", self.port),
        })
    }
}

/// This machine's address on the network: the source address of its default
/// route. Connecting a UDP socket only picks a route; nothing is sent.
pub fn lan_address() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("192.0.2.1", 9)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn project(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        return None;
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if !home.is_empty() && trimmed == home.trim_end_matches(['/', '\\']) {
        return None;
    }
    let name = base_name(trimmed);
    // A system folder isn't a project.
    if ["system32", "tmp", "private", "usr", "bin", "sbin"]
        .contains(&name.to_ascii_lowercase().as_str())
    {
        return None;
    }
    Some(name.to_string())
}

#[cfg(target_os = "macos")]
pub use imp::force_stop;
pub use imp::{scan, stop};

/// One socket as the OS reports it, before folding.
#[derive(Clone, Debug)]
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
struct Socket {
    port: u16,
    pid: u32,
    addr: IpAddr,
}

/// One (port, pid) after folding its sockets together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Folded {
    port: u16,
    pid: u32,
    /// Any of its sockets is reachable from the network.
    exposed: bool,
    /// The specific address it is reachable at, unless one of its sockets
    /// listens on all of them.
    bound: Option<IpAddr>,
}

/// Fold sockets into one entry per (port, pid), sorted by port.
fn fold(sockets: &[Socket]) -> Vec<Folded> {
    let mut by: BTreeMap<(u16, u32), (bool, bool, Option<IpAddr>)> = BTreeMap::new();
    for s in sockets {
        let (exposed, everywhere, bound) = by.entry((s.port, s.pid)).or_default();
        if s.addr.is_loopback() {
            continue;
        }
        *exposed = true;
        if s.addr.is_unspecified() {
            *everywhere = true;
        } else if bound.is_none() || s.addr.is_ipv4() {
            // IPv4 first: it's the one people type.
            *bound = Some(s.addr);
        }
    }
    by.into_iter()
        .map(|((port, pid), (exposed, everywhere, bound))| Folded {
            port,
            pid,
            exposed,
            bound: if everywhere { None } else { bound },
        })
        .collect()
}

/// Processes that are part of the OS. Their ports are hidden unless asked for:
/// nobody started them, and stopping them is never the fix.
const SYSTEM: &[&str] = &[
    // Windows
    "system",
    "idle",
    "svchost",
    "lsass",
    "services",
    "wininit",
    "spoolsv",
    "smss",
    "csrss",
    "msmpeng",
    "nissrv",
    // macOS
    "launchd",
    "controlce",
    "controlcenter",
    "rapportd",
    "sharingd",
    "remoted",
    "mdnsresponder",
    "identityservicesd",
];

fn is_system_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SYSTEM.contains(&n.as_str())
}

/// Shells and terminal apps. A process with one of these above it was started
/// from a command line.
const SHELLS: &[&str] = &[
    "cmd",
    "powershell",
    "pwsh",
    "bash",
    "zsh",
    "fish",
    "sh",
    "nu",
    "dash",
    "tcsh",
    "csh",
    "ksh",
    "login",
    "tmux",
    "screen",
    "wsl",
    "wslhost",
    "mintty",
    "windowsterminal",
    "openconsole",
    "wezterm-gui",
    "wezterm",
    "alacritty",
    "kitty",
    "ghostty",
    "hyper",
    "warp",
    "tabby",
    "terminal",
    "iterm2",
];

/// Lower-case executable name without directory or `.exe`.
fn exe_key(name: &str) -> String {
    let n = base_name(name).to_ascii_lowercase();
    n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
}

/// Whether a shell or terminal is among `pid`'s ancestors in `parents`
/// (pid → (parent pid, name)). Pure, so it can be tested on a made-up tree.
fn under_shell(parents: &std::collections::HashMap<u32, (u32, String)>, pid: u32) -> bool {
    let mut cur = parents.get(&pid).map(|(p, _)| *p);
    // A cycle or a recycled pid can make the chain loop; real ones are short.
    for _ in 0..24 {
        let Some(p) = cur else { return false };
        let Some((parent, name)) = parents.get(&p) else {
            return false;
        };
        if SHELLS.contains(&exe_key(name).as_str()) {
            return true;
        }
        if *parent == p || *parent == 0 {
            return false;
        }
        cur = Some(*parent);
    }
    false
}

/// Split a command line into words, honouring double quotes the way both
/// Windows and a POSIX shell would for an ordinary path.
fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in command.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// What tells this process apart from others with the same name, read off its
/// command line: "vite", "next dev", "manage.py runserver", "http.server".
/// `None` when the command line adds nothing to the name.
pub fn hint(command: &str, name: &str) -> Option<String> {
    let w = words(command);

    // A process that renamed itself: Next.js shows up as
    // "next-server (v16.1.5)", with no arguments left to read. The new name,
    // without its version, is the best there is.
    if let Some(first) = w.first() {
        if !first.contains(['/', '\\']) && exe_key(first) != exe_key(name) {
            let title: String = command
                .split('(')
                .next()
                .unwrap_or(command)
                .trim()
                .to_string();
            if !title.is_empty() && exe_key(&title) != exe_key(name) {
                return Some(shorten(title));
            }
        }
    }
    let args = w.get(1..).unwrap_or(&[]);

    // node .../node_modules/vite/bin/vite.js → "vite"; also @scope/pkg, and
    // .bin shims, where the next segment is the tool.
    for a in args {
        let a = a.replace('\\', "/");
        if let Some(i) = a.rfind("node_modules/") {
            let rest: Vec<&str> = a[i + "node_modules/".len()..].split('/').collect();
            let pkg = match rest.as_slice() {
                [".bin", tool, ..] => tool.to_string(),
                [scope, pkg, ..] if scope.starts_with('@') => format!("{scope}/{pkg}"),
                [pkg, ..] => pkg.to_string(),
                [] => continue,
            };
            return Some(with_subcommand(pkg, args, &a));
        }
    }

    // python -m http.server → "http.server"
    if let Some(i) = args.iter().position(|a| a == "-m") {
        if let Some(module) = args.get(i + 1) {
            return Some(module.clone());
        }
    }

    // The first real argument: a script's file name, or a subcommand with its
    // own argument ("run dev"). A word right after a short flag is that flag's
    // value ("-D data", "-p 8080"), not something to name the process by.
    let mut it = args
        .iter()
        .enumerate()
        .filter(|(i, a)| {
            let after_short_flag = *i > 0 && {
                let prev = &args[i - 1];
                prev.len() == 2 && prev.starts_with('-')
            };
            !a.starts_with('-') && !after_short_flag
        })
        .map(|(_, a)| a);
    let first = it.next()?;
    let first_name = base_name(first);
    if first_name.eq_ignore_ascii_case(name) {
        return None;
    }
    let mut h = first_name.to_string();
    if matches!(first_name, "run" | "exec" | "start" | "serve" | "dev") {
        if let Some(next) = it.next() {
            h = format!("{h} {}", base_name(next));
        }
    } else if first_name.ends_with(".py") {
        if let Some(next) = it.next() {
            h = format!("{h} {next}");
        }
    }
    Some(shorten(h))
}

/// "next" + the "dev" that follows its script → "next dev".
fn with_subcommand(pkg: String, args: &[String], script: &str) -> String {
    let mut seen = false;
    for a in args {
        if seen && !a.starts_with('-') {
            return shorten(format!("{pkg} {a}"));
        }
        if a.replace('\\', "/") == script {
            seen = true;
        }
    }
    shorten(pkg)
}

fn shorten(s: String) -> String {
    if s.chars().count() <= 32 {
        return s;
    }
    let mut t: String = s.chars().take(31).collect();
    t.push('\u{2026}');
    t
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use super::{fold, is_system_name, under_shell, Folded, Listener, Socket};
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use windows::core::PWSTR;
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, UNICODE_STRING};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    /// The listener table for one address family, as raw bytes. Asked twice:
    /// once for the size, once for the data, retrying if it grew between.
    fn table(family: u32) -> Vec<u64> {
        let mut size = 0u32;
        for _ in 0..4 {
            // u64s so the buffer is aligned for the structs read out of it.
            let mut buf = vec![0u64; (size as usize).div_ceil(8).max(1)];
            let rc = unsafe {
                GetExtendedTcpTable(
                    Some(buf.as_mut_ptr().cast()),
                    &mut size,
                    false,
                    family,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if rc == 0 {
                return buf;
            }
            // ERROR_INSUFFICIENT_BUFFER: `size` now holds what is needed.
            if rc != 122 {
                return Vec::new();
            }
        }
        Vec::new()
    }

    fn sockets() -> Vec<Socket> {
        let mut out = Vec::new();

        let v4 = table(AF_INET.0 as u32);
        if !v4.is_empty() {
            let t = v4.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
            let rows: &[MIB_TCPROW_OWNER_PID] = unsafe {
                std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize)
            };
            for r in rows {
                out.push(Socket {
                    // Network byte order in the low 16 bits.
                    port: u16::from_be(r.dwLocalPort as u16),
                    pid: r.dwOwningPid,
                    // Already in network order as laid out in memory.
                    addr: IpAddr::V4(Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes())),
                });
            }
        }

        let v6 = table(AF_INET6.0 as u32);
        if !v6.is_empty() {
            let t = v6.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
            let rows: &[MIB_TCP6ROW_OWNER_PID] = unsafe {
                std::slice::from_raw_parts((*t).table.as_ptr(), (*t).dwNumEntries as usize)
            };
            for r in rows {
                out.push(Socket {
                    port: u16::from_be(r.dwLocalPort as u16),
                    pid: r.dwOwningPid,
                    addr: IpAddr::V6(Ipv6Addr::from(r.ucLocalAddr)),
                });
            }
        }
        out
    }

    /// pid → (parent pid, executable name).
    fn processes() -> HashMap<u32, (u32, String)> {
        let mut out = HashMap::new();
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            out.insert(
                e.th32ProcessID,
                (
                    e.th32ParentProcessID,
                    String::from_utf16_lossy(&e.szExeFile[..len]),
                ),
            );
            ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
        }
        unsafe {
            let _ = CloseHandle(snap);
        }
        out
    }

    /// Closes on drop, so every early return lets go of the process.
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn unix_from(ft: FILETIME) -> u64 {
        let ticks = (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64;
        // 100 ns ticks since 1601 → seconds since 1970.
        (ticks / 10_000_000).saturating_sub(11_644_473_600)
    }

    fn started(p: &Process) -> Option<u64> {
        let (mut c, mut e, mut k, mut u) = Default::default();
        unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) }.ok()?;
        Some(unix_from(c))
    }

    fn image_path(p: &Process) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
        }
        .ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    /// ProcessCommandLineInformation (60, Windows 8.1+) hands back the command
    /// line directly, with no reading of the other process's memory.
    fn command_line(p: &Process) -> Option<String> {
        const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);
        let mut len = 0u32;
        unsafe {
            let _ = NtQueryInformationProcess(
                p.0,
                PROCESS_COMMAND_LINE_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut len,
            );
        }
        if len == 0 || len > 64 * 1024 {
            return None;
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let status = unsafe {
            NtQueryInformationProcess(
                p.0,
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr().cast(),
                len,
                &mut len,
            )
        };
        if status.is_err() {
            return None;
        }
        // The buffer starts with a UNICODE_STRING pointing into itself.
        let us = unsafe { &*(buf.as_ptr() as *const UNICODE_STRING) };
        if us.Buffer.is_null() || us.Length == 0 {
            return None;
        }
        let chars = unsafe { std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2) };
        Some(String::from_utf16_lossy(chars))
    }

    pub fn scan() -> Vec<Listener> {
        let procs = processes();
        let me = std::process::id();
        fold(&sockets())
            .into_iter()
            .filter(|f| f.pid != me)
            .map(
                |Folded {
                     port,
                     pid,
                     exposed,
                     bound,
                 }| {
                    let exe = procs.get(&pid).map(|(_, n)| n.clone()).unwrap_or_default();
                    let name = exe
                        .strip_suffix(".exe")
                        .or_else(|| exe.strip_suffix(".EXE"))
                        .unwrap_or(&exe)
                        .to_string();
                    let opened =
                        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
                            .ok()
                            .map(Process);
                    let (command, path, started) = match &opened {
                        Some(p) => (
                            command_line(p).unwrap_or_default(),
                            image_path(p).unwrap_or_default(),
                            started(p),
                        ),
                        None => (String::new(), String::new(), None),
                    };
                    Listener {
                        port,
                        pid,
                        // Pids 0 and 4 are the kernel's; a process we may not even
                        // look at belongs to another account, often SYSTEM.
                        system: pid <= 4 || opened.is_none() || is_system_name(&name),
                        name: if name.is_empty() {
                            format!("pid {pid}")
                        } else {
                            name
                        },
                        command,
                        path,
                        // Another process's working directory is only in its
                        // PEB; not read yet.
                        cwd: String::new(),
                        started,
                        exposed,
                        bound,
                        from_terminal: under_shell(&procs, pid),
                    }
                },
            )
            .collect()
    }

    pub fn stop(pid: u32) -> Result<(), String> {
        let p = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }
            .map(Process)
            .map_err(|e| format!("not allowed to stop it ({e})"))?;
        unsafe { TerminateProcess(p.0, 1) }.map_err(|e| format!("Windows refused ({e})"))
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
//
// `lsof` and `ps` rather than libproc: both ship with macOS, answer in tens of
// milliseconds, and keep FFI nobody here can test without a Mac to a minimum.
// `lsof` only sees this user's processes without root, which is the useful set
// anyway.
#[cfg(target_os = "macos")]
mod imp {
    use super::{fold, is_system_name, under_shell, Folded, Listener, Socket};
    use std::collections::HashMap;
    use std::net::IpAddr;

    fn run(cmd: &str, args: &[&str]) -> String {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// `lsof -F pcn` output: a `p` line per process, then a `c` (command) and
    /// one `n` (address) line per socket.
    pub(super) fn parse_lsof(out: &str) -> (Vec<Socket>, HashMap<u32, String>) {
        let mut sockets = Vec::new();
        let mut names = HashMap::new();
        let mut pid = 0u32;
        for line in out.lines() {
            let (tag, rest) = line.split_at(line.len().min(1));
            match tag {
                "p" => pid = rest.parse().unwrap_or(0),
                "c" => {
                    names.insert(pid, rest.to_string());
                }
                "n" => {
                    // "*:3000", "127.0.0.1:5432", "[::1]:8080"
                    let Some((host, port)) = rest.rsplit_once(':') else {
                        continue;
                    };
                    let Ok(port) = port.parse() else { continue };
                    let host = host.trim_start_matches('[').trim_end_matches(']');
                    let addr = if host == "*" {
                        IpAddr::from([0u8, 0, 0, 0])
                    } else {
                        match host.parse() {
                            Ok(a) => a,
                            Err(_) => continue,
                        }
                    };
                    sockets.push(Socket { port, pid, addr });
                }
                _ => {}
            }
        }
        (sockets, names)
    }

    /// `lsof -d cwd -Fpn` → pid → working directory.
    pub(super) fn parse_cwds(out: &str) -> HashMap<u32, String> {
        let mut cwds = HashMap::new();
        let mut pid = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix('p') {
                pid = p.parse().ok();
            } else if let (Some(dir), Some(p)) = (line.strip_prefix('n'), pid) {
                cwds.insert(p, dir.to_string());
            }
        }
        cwds
    }

    /// "[[dd-]hh:]mm:ss" → seconds.
    pub(super) fn parse_etime(s: &str) -> Option<u64> {
        let (days, rest) = match s.split_once('-') {
            Some((d, r)) => (d.parse::<u64>().ok()?, r),
            None => (0, s),
        };
        let mut secs = 0u64;
        for part in rest.split(':') {
            secs = secs * 60 + part.parse::<u64>().ok()?;
        }
        Some(days * 86_400 + secs)
    }

    pub fn scan() -> Vec<Listener> {
        let (sockets, names) = parse_lsof(&run(
            "/usr/sbin/lsof",
            &["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"],
        ));
        let folded = fold(&sockets);
        let me = std::process::id();

        // Every process's parent, for "was it started from a terminal".
        let mut parents: HashMap<u32, (u32, String)> = HashMap::new();
        if !folded.is_empty() {
            for line in run("/bin/ps", &["-A", "-o", "pid=,ppid=,comm="]).lines() {
                let mut it = line.split_whitespace();
                let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
                    continue;
                };
                let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) else {
                    continue;
                };
                parents.insert(pid, (ppid, it.collect::<Vec<_>>().join(" ")));
            }
        }

        let mut details: HashMap<u32, (Option<u64>, String)> = HashMap::new();
        let pids: Vec<String> = folded.iter().map(|f| f.pid.to_string()).collect();
        if !pids.is_empty() {
            let now = crate::util::now_unix();
            let out = run(
                "/bin/ps",
                &["-o", "pid=,etime=,command=", "-p", &pids.join(",")],
            );
            for line in out.lines() {
                let mut it = line.split_whitespace();
                let (Some(pid), Some(etime)) = (it.next(), it.next()) else {
                    continue;
                };
                let Ok(pid) = pid.parse::<u32>() else {
                    continue;
                };
                let command = it.collect::<Vec<_>>().join(" ");
                let started = parse_etime(etime).map(|e| now.saturating_sub(e));
                details.insert(pid, (started, command));
            }
        }

        let mut cwds = if pids.is_empty() {
            HashMap::new()
        } else {
            parse_cwds(&run(
                "/usr/sbin/lsof",
                &["-a", "-d", "cwd", "-Fpn", "-p", &pids.join(",")],
            ))
        };

        folded
            .into_iter()
            .filter(|f| f.pid != me)
            .map(
                |Folded {
                     port,
                     pid,
                     exposed,
                     bound,
                 }| {
                    let path = exe_path(pid);
                    // lsof cuts names at 15 characters ("com.docker.back"); the
                    // path has the whole of it.
                    let name = match path.rsplit('/').next() {
                        Some(n) if !n.is_empty() => n.to_string(),
                        _ => names
                            .get(&pid)
                            .cloned()
                            .unwrap_or_else(|| format!("pid {pid}")),
                    };
                    let (started, command) = details.remove(&pid).unwrap_or_default();
                    Listener {
                        port,
                        pid,
                        system: is_system_name(&name),
                        name,
                        command,
                        path,
                        cwd: cwds.remove(&pid).unwrap_or_default(),
                        started,
                        exposed,
                        bound,
                        from_terminal: under_shell(&parents, pid),
                    }
                },
            )
            .collect()
    }

    /// The executable's full path; empty for another user's process.
    fn exe_path(pid: u32) -> String {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let n = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if n <= 0 {
            return String::new();
        }
        buf.truncate(n as usize);
        String::from_utf8_lossy(&buf).into_owned()
    }

    pub fn stop(pid: u32) -> Result<(), String> {
        // SIGTERM: a dev server gets to shut down cleanly.
        signal(pid, libc::SIGTERM)
    }

    /// SIGKILL, for a server that ignores being asked.
    pub fn force_stop(pid: u32) -> Result<(), String> {
        signal(pid, libc::SIGKILL)
    }

    fn signal(pid: u32, sig: libc::c_int) -> Result<(), String> {
        if unsafe { libc::kill(pid as libc::pid_t, sig) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lsof_output() {
            let (s, n) =
                parse_lsof("p501\ncnode\nn*:3000\nn[::1]:3000\np77\ncpostgres\nn127.0.0.1:5432\n");
            assert_eq!(s.len(), 3);
            assert_eq!(n[&501], "node");
            assert!(s[0].addr.is_unspecified());
            assert!(s[2].addr.is_loopback());
        }

        #[test]
        fn cwds() {
            let c = parse_cwds("p41859\nfcwd\nn/Users/me/projects/juanalist\np7\nfcwd\nn/\n");
            assert_eq!(c[&41859], "/Users/me/projects/juanalist");
            assert_eq!(c[&7], "/");
        }

        #[test]
        fn etime() {
            assert_eq!(parse_etime("05:07"), Some(307));
            assert_eq!(parse_etime("01:00:00"), Some(3600));
            assert_eq!(parse_etime("2-00:00:01"), Some(172_801));
            assert_eq!(parse_etime("x"), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(port: u16, pid: u32, addr: &str) -> Socket {
        Socket {
            port,
            pid,
            addr: addr.parse().unwrap(),
        }
    }

    #[test]
    fn folds_v4_and_v6_and_flags_exposure() {
        let f = fold(&[
            s(3000, 1, "127.0.0.1"),
            s(3000, 1, "::1"),
            s(5173, 2, "::"),
            s(5173, 2, "127.0.0.1"),
            s(80, 3, "192.168.1.5"),
        ]);
        let summary: Vec<_> = f
            .iter()
            .map(|f| (f.port, f.pid, f.exposed, f.bound))
            .collect();
        assert_eq!(
            summary,
            vec![
                (80, 3, true, Some("192.168.1.5".parse().unwrap())),
                (3000, 1, false, None),
                (5173, 2, true, None),
            ]
        );
    }

    #[test]
    fn projects_are_folders_that_say_something() {
        assert_eq!(
            project("/Users/me/projects/juanalist").as_deref(),
            Some("juanalist")
        );
        assert_eq!(project(r"C:\code\api\").as_deref(), Some("api"));
        assert_eq!(project("/"), None);
        assert_eq!(project(""), None);
        assert_eq!(project(r"C:\"), None);
        assert_eq!(project(r"C:\Windows\System32"), None);
    }

    #[test]
    fn hints_from_command_lines() {
        let h = |c: &str, n: &str| hint(c, n);
        assert_eq!(
            h(
                r#""C:\Program Files\nodejs\node.exe" C:\app\node_modules\vite\bin\vite.js"#,
                "node"
            )
            .as_deref(),
            Some("vite")
        );
        assert_eq!(
            h("node /app/node_modules/.bin/next dev -p 3000", "node").as_deref(),
            Some("next dev")
        );
        assert_eq!(
            h("node /app/node_modules/@angular/cli/bin/ng serve", "node").as_deref(),
            Some("@angular/cli serve")
        );
        assert_eq!(
            h("python -m http.server 8000", "python").as_deref(),
            Some("http.server")
        );
        assert_eq!(
            h("python manage.py runserver", "python").as_deref(),
            Some("manage.py runserver")
        );
        assert_eq!(h("node server.js", "node").as_deref(), Some("server.js"));
        assert_eq!(
            h("next-server (v16.1.5) ", "node").as_deref(),
            Some("next-server")
        );
        assert_eq!(h("node", "node"), None);
        assert_eq!(h("npm run dev", "npm").as_deref(), Some("run dev"));
        assert_eq!(h("postgres", "postgres"), None);
        assert_eq!(h(r"C:\x\postgres.exe -D data", "postgres"), None);
        assert_eq!(h("ruby -p 4000 app.rb", "ruby").as_deref(), Some("app.rb"));
        assert_eq!(h("", "x"), None);
    }

    #[test]
    fn started_from_a_terminal_or_not() {
        let tree: std::collections::HashMap<u32, (u32, String)> = [
            (50, (40, "node.exe")),
            (40, (30, "pwsh.exe")),
            (30, (1, "WindowsTerminal.exe")),
            (60, (1, "Spotify.exe")),
            (1, (0, "explorer.exe")),
            (70, (80, "Code Helper")),
            (80, (90, "/bin/zsh")),
            (90, (1, "/Applications/iTerm.app/Contents/MacOS/iTerm2")),
        ]
        .into_iter()
        .map(|(k, (p, n))| (k, (p, n.to_string())))
        .collect();
        assert!(under_shell(&tree, 50));
        assert!(!under_shell(&tree, 60));
        assert!(under_shell(&tree, 70), "full paths, as macOS reports them");
        assert!(!under_shell(&tree, 999));
    }

    #[test]
    fn system_names() {
        assert!(is_system_name("svchost"));
        assert!(is_system_name("ControlCenter"));
        assert!(!is_system_name("node"));
    }
}
