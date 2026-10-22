# Changelog

## [Unreleased]

First release, planned as 0.0.1.

- Tray app for Windows and macOS that lists what's listening on TCP ports: the
  port, the process, a hint from its command line ("vite", "next dev",
  "manage.py runserver"), how long it has been up, and whether it's reachable
  from the network.
- Servers started from a terminal are listed first; desktop apps and services
  go under "Other apps and services", and the OS's own ports are hidden unless
  asked for.
- Open a port in the browser, or stop its process, from the menu.
- The door in the tray is ajar while any of your servers is up.
- `porta list` and `porta stop <port>` from a terminal.
- Run at login.
- Windows installer (Inno Setup), Chocolatey and winget manifests, a universal
  macOS DMG, and a Homebrew cask.
