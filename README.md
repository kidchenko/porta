<div align="center">

<img src="assets/icon-256.png" width="112" height="112" alt="Porta">

# Porta

See what's listening on your ports, from the tray, and stop it.<br>
On Windows and macOS.

<sub><i>porta</i>: Portuguese for <i>door</i>, and close enough to <i>port</i></sub>

</div>

"Port 3000 is already in use." Then comes the hunt: `netstat -ano`, a pid,
Task Manager, and a guess at which of six `node.exe` processes it is. Porta is
a door in the tray. Its menu lists every server you started from a terminal,
with its port, what it was started with and how long it's been up. Open one in
the browser, or stop it, in two clicks.

It's a single 0.5 MB binary with no runtime dependencies. In the tray it uses
about 2 MB of memory and no measurable CPU.

## The door

| | |
|---|---|
| **Shut** | Nothing you started from a terminal is listening. |
| **Ajar** | At least one of your servers is up. |

Click it, with either button, to open the menu.

## The menu

```
     2 servers from your terminals
     ──────────────────────────────────────────
[js] :3000  next dev                 2h 05m  ▸
[py] :8000  manage.py runserver  LAN · 1h 10m ▸
     ──────────────────────────────────────────
     Other apps and services             11  ▸
     Settings                                ▸
     ──────────────────────────────────────────
     Quit Porta
```

Each row has the program's own icon, the port, and what it was started as:
a hint from its command line, like "next dev" or "manage.py runserver", or
the program's name when there's nothing more telling. On the right, **LAN**
means it's reachable from other devices on your network, not just this
computer, and then how long it has been running. On macOS rows get a dot
instead of the program's icon.

Each port opens a submenu:

| | |
|---|---|
| **Open in browser** | `http://localhost:<port>` in your default browser. |
| **Stop node** | Ends the process. On macOS it gets a SIGTERM, so it can shut down cleanly. |
| The command line, the program and its uptime, and whether it's reachable from your network | For telling two `node` processes apart. |

"From your terminals" means a shell or terminal app is among the process's
parents: Windows Terminal, WezTerm, PowerShell, bash, zsh, VS Code's terminal
and so on. Ports that desktop apps keep open for themselves, like Spotify,
VS Code's extension host or OneDrive, go under **Other apps and services**,
along with services such as a local Postgres or nginx.

**Settings** holds the rest:

| | |
|---|---|
| **Show system ports** | Also list the OS's own listeners. Porta never offers to stop those. |
| **Run at login** | Per-user, no admin. |
| **Refresh now** | Look again now. Porta looks every 5 seconds anyway. |
| **Edit settings file…** | Opens `config.json`. |

Running `porta` again while it's in the tray makes it tell you what's
listening.

## From a terminal

```
porta list          # what's listening, with pid, uptime and LAN/local
porta list --all    # including the OS's own
porta stop 3000     # stop whatever is listening on 3000
```

## Install

### Windows

```powershell
winget install kidchenko.Porta
# or
choco install porta
```

Or grab [**Porta-Setup.exe**](https://github.com/kidchenko/porta/releases/latest/download/Porta-Setup.exe)
directly. Each release also ships a portable `porta-x.y.z-x64.exe`.

The binary isn't code-signed, so SmartScreen warns on first run: click **More
info → Run anyway**.

### macOS

```bash
brew install --cask kidchenko/tap/porta
```

Or download [**Porta.dmg**](https://github.com/kidchenko/porta/releases/latest/download/Porta.dmg)
(Apple silicon and Intel) and drag Porta into **Applications**. The build isn't
notarized: the Homebrew cask clears Gatekeeper for you, or run
`xattr -dr com.apple.quarantine /Applications/Porta.app` once.

## How it works

| | Sockets | Process details |
|---|---|---|
| Windows | `GetExtendedTcpTable` | Toolhelp for names and parents, `GetProcessTimes` for uptime, and the command line from `NtQueryInformationProcess` |
| macOS | `lsof -iTCP -sTCP:LISTEN` | `ps` |

A scan takes around 100 ms on Windows. IPv4 and IPv6 sockets of the same
process on the same port are shown as one. On macOS, `lsof` only sees your own
processes without root, which is the set you can stop anyway.

## Configuration

`%APPDATA%\Porta\config.json` on Windows,
`~/Library/Application Support/Porta/config.json` on macOS.

```json
{
  "showSystem": false,
  "refreshSeconds": 5
}
```

## Build

Rust 1.77 or newer.

```powershell
.\build.ps1           # Windows: tests, release exe, installer if Inno Setup is installed
.\build.ps1 -Icons    # ... and regenerate the icons
```

```bash
./build.sh            # macOS: tests, release binary, dist/Porta.app
./build.sh --dmg      # ... and a disk image
```

## Releasing

1. Bump `version` in `Cargo.toml` and add a section to `CHANGELOG.md`.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`

The `release` workflow builds the DMG and the installer, publishes them to a
GitHub Release, and fills the new version and checksums into the Chocolatey,
winget and Homebrew manifests. Chocolatey is pushed when the `CHOCO_API_KEY`
secret is set; submit the `manifest-winget` artifact to
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), and copy
`manifest-homebrew` to `Casks/porta.rb` in `kidchenko/homebrew-tap`.

## Not yet

- UDP ports.
- The project folder a server runs in, next to its command line.
- Stopping a process another account or an elevated terminal started; that
  needs Porta itself to run elevated.

## License

MIT
