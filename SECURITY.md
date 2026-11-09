# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue: on GitHub, open the repository's **Security** tab →
**Report a vulnerability**. Describe what is affected, how to reproduce it and which version you used (`rgbtray --diagnose` shows it).

Once a fix is released, the report can be made public, with credit if you want it.

Only the latest release is supported.

## What is worth reporting

RGB Tray runs as the signed-in user, without administrator rights. The parts that matter most:

- **Updates.** Once a day the app asks GitHub (`api.github.com`, HTTPS) for the latest release of this repository, downloads its
  setup and runs it. The setup is taken only from this repository's release assets and must match the SHA-256 digest GitHub publishes
  for it. A way to make the app download or run anything else is a vulnerability.
- **Tuya bulbs.** The app talks to bulbs on the local network with their local keys (stored in `%APPDATA%\rgbtray\tuya.toml`) and
  looks for them on its own /24 subnet.
- **Files it reads.** Settings, imported settings and presets, and device profiles (`%APPDATA%\rgbtray\devices\*.toml`). A file that
  makes the app crash, hang or write outside its own folder is worth reporting.
- **Devices.** The app must never send firmware update, bootloader or "save to flash" commands. A profile or code path that does is a bug.

The installer is not code-signed yet.
