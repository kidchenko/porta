# Changelog

## [0.0.1] - 2026-10-23

First release.

- Tray app for Windows and macOS that lists what's listening on TCP ports: the
  port, the process, a hint from its command line ("vite", "next dev",
  "manage.py runserver"), how long it has been up, and whether it's reachable
  from the network.
- Servers started from a terminal are listed first; desktop apps and services
  go under "Other apps and services", and the OS's own ports are hidden unless
  asked for.
- Open a port in the browser, copy its URL, or stop its process, from the
  menu; on macOS also force quit it. "Stop all" when two or more of your
  servers are up.
- Each row carries the logo of the technology behind it (Next.js, Django,
  Postgres, Docker, Node, Python... about 90, from Devicon), or the app's own
  icon, on macOS too. Submenus get Lucide glyphs, and dark menus get light
  versions of black logos.
- Rows lead with the project folder a server runs in ("juanalist · next dev")
  on macOS, and its submenu can show that folder in Finder.
- Servers reachable from the network show the address to reach them at;
  clicking it copies it.
- macOS: the number of your servers beside the door in the menu bar
  (`showCount`), and ⌘R, ⌘, and ⌘Q in the menu.
- The door in the tray is ajar while any of your servers is up.
- `porta list` and `porta stop <port>` from a terminal.
- Run at login.
- Windows installer (Inno Setup), Chocolatey and winget manifests, a universal
  macOS DMG, and a Homebrew cask.
