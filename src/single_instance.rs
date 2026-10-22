//! One copy in the tray, and a use for the second launch.
//!
//! Launching Porta while it is already running makes the running copy say
//! what is listening, then steps aside. Pin it, bind it to a hotkey, or run
//! `porta` from a terminal for a quick answer.
//!
//! Windows carries the signal on a named auto-reset event; macOS on a Unix
//! domain socket in the config directory.

pub use imp::acquire;

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod imp {
    use crate::log::{linfo, lwarn};
    use crate::util::{pcwstr, wide};
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };

    // "Local\" scopes both to the logon session, so two users signed in at once
    // each get their own tray.
    const MUTEX_NAME: &str = r"Local\Porta.SingleInstance";
    const EVENT_NAME: &str = r"Local\Porta.Summon";

    /// Held by the sole running instance: the event other launches signal.
    pub struct Signal(isize);

    /// Returns `None` if another instance is already running, in which case it
    /// has been summoned and this process should exit.
    pub fn acquire() -> Option<Signal> {
        let mutex_name = wide(MUTEX_NAME);
        let existing = unsafe {
            match CreateMutexW(None, true, pcwstr(&mutex_name)) {
                Ok(handle) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        let _ = CloseHandle(handle);
                        true
                    } else {
                        // Never closed: it must outlive `main`, and Windows
                        // releases it on exit anyway.
                        false
                    }
                }
                // Cannot tell: assume we are alone rather than refuse to start.
                Err(_) => false,
            }
        };
        if existing {
            linfo!("another instance is running; summoning it");
            summon();
            return None;
        }
        let event_name = wide(EVENT_NAME);
        match unsafe { CreateEventW(None, false, false, pcwstr(&event_name)) } {
            Ok(handle) => Some(Signal(handle.0 as isize)),
            Err(e) => {
                // Not fatal: losing relaunch-to-summon beats refusing to start.
                lwarn!("could not create the summon event ({e})");
                Some(Signal(0))
            }
        }
    }

    fn summon() {
        let n = wide(EVENT_NAME);
        unsafe {
            if let Ok(h) = OpenEventW(EVENT_MODIFY_STATE, false, pcwstr(&n)) {
                let _ = SetEvent(h);
                let _ = CloseHandle(h);
            }
        }
    }

    impl Signal {
        /// Run `on_signal` every time another launch summons us. The thread
        /// blocks for the life of the process, which tears it down on exit.
        pub fn watch<F>(self, on_signal: F)
        where
            F: Fn() + Send + 'static,
        {
            let raw = self.0;
            if raw == 0 {
                return;
            }
            let _ = std::thread::Builder::new()
                .name("porta-instance".into())
                .spawn(move || {
                    let handle = HANDLE(raw as *mut std::ffi::c_void);
                    loop {
                        // Anything but WAIT_OBJECT_0 means the handle is gone,
                        // and looping would spin at full tilt.
                        if unsafe { WaitForSingleObject(handle, INFINITE) }.0 != 0 {
                            lwarn!("summon listener stopping");
                            break;
                        }
                        on_signal();
                    }
                });
        }
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod imp {
    use crate::log::{linfo, lwarn};
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    fn socket_path() -> Option<PathBuf> {
        // Alongside the config, well within the ~104-char sun_path limit.
        crate::config::Config::dir().map(|d| d.join("porta.sock"))
    }

    pub struct Signal(Option<UnixListener>);

    /// Returns `None` if another instance is already running, in which case it
    /// has been summoned and this process should exit.
    pub fn acquire() -> Option<Signal> {
        let Some(path) = socket_path() else {
            return Some(Signal(None));
        };
        if summon(&path) {
            linfo!("another instance is running; summoned it");
            return None;
        }
        // Nothing answered: a stale socket file, or none. Take it over.
        let _ = std::fs::remove_file(&path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match UnixListener::bind(&path) {
            Ok(l) => Some(Signal(Some(l))),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                // Lost a startup race; the winner is up now.
                let _ = summon(&path);
                None
            }
            Err(e) => {
                lwarn!("could not bind the instance socket ({e})");
                Some(Signal(None))
            }
        }
    }

    fn summon(path: &Path) -> bool {
        match UnixStream::connect(path) {
            Ok(mut s) => {
                let _ = s.write_all(b"s");
                true
            }
            Err(_) => false,
        }
    }

    impl Signal {
        pub fn watch<F>(self, on_signal: F)
        where
            F: Fn() + Send + 'static,
        {
            let Some(listener) = self.0 else { return };
            let _ = std::thread::Builder::new()
                .name("porta-instance".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        match stream {
                            Ok(mut s) => {
                                let mut buf = [0u8; 1];
                                let _ = s.read(&mut buf);
                                on_signal();
                            }
                            Err(e) => {
                                lwarn!("summon accept failed ({e}); listener stopping");
                                break;
                            }
                        }
                    }
                });
        }
    }
}
