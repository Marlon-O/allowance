# Setup guide

## macOS

1. Download the DMG matching your Mac architecture. The currently tested build is Apple Silicon (`arm64`).
2. Open the DMG and drag **Allowance** to **Applications**.
3. Open Allowance. Its gauge icon appears in the menu bar.
4. Because the build is unsigned, macOS may block the first launch. Open **System Settings → Privacy & Security**, verify that the blocked app is Allowance from this project, and choose **Open Anyway**. Publicly distributed builds should be Developer ID signed and notarized.

## Windows

1. Build or download the Windows x64 NSIS installer.
2. Run the installer for the current user, then open Allowance from the Start menu.
3. The gauge icon appears in the system tray. Windows may warn about the unsigned installer; publicly distributed builds should be Authenticode signed.

The Windows x64 NSIS installer has been cross-built and structurally verified on Apple Silicon. It has not yet been run on a Windows machine. Native Windows installations are supported; WSL-only provider logins are not.

## Connect Codex

1. Install the official Codex CLI and sign in with the account whose subscription allowance you want to see.
2. Open Allowance and choose Refresh after signing in. Codex is enabled by default.
3. Confirm the account label in the card, because the CLI account can differ from an account selected in the Codex desktop app.

API-key-only logins do not expose subscription allowance through this integration.

## Connect Claude Code

1. Install the official Claude Code CLI and run `claude auth login` if needed.
2. In Allowance, choose **Connect Claude Code**. Read the notice, then choose **Approve & connect**.
3. Use Claude normally. Allowance prefers the official Desktop app's local plan-usage history and cached usage response, and uses Claude Code's status-line data as a CLI fallback. Restart open CLI sessions if the fallback is needed.

Allowance backs up the current user-level `statusLine`, chains its output, and restores it on disconnect only while it still owns the configured value. Project or managed settings can override the user-level status line. A GUI-launched app might not inherit a terminal-only `CLAUDE_CONFIG_DIR`.

Claude Desktop refreshes its local usage response during normal use and records plan-usage history every 12–15 minutes. Remote/cloud sessions run away from this computer and cannot update local files.

If Claude shows no data:

- Confirm `claude auth status --json` reports a signed-in subscription account.
- Restart every Claude Code session after connecting.
- Use Claude Code normally once so it can emit a new status-line observation.
- In Claude Desktop, open the usage ring once if its local cache has not been created yet.
- Check that project or managed Claude settings do not override `statusLine`.
- Confirm the Desktop session environment is **Local**, not Remote.
- Quit and reopen Allowance after upgrading so it can atomically switch Claude to the helper bundled with the new build.
- Disconnect and reconnect after moving Allowance's data directory so the helper path is refreshed.

## Daily use

- Click the tray icon to show the compact usage popover beside the macOS menu bar or Windows system tray.
- Click the icon again, press Escape, or click elsewhere to hide it.
- Closing the popover keeps the tray process running; use **Preferences → Quit Allowance** to exit.
- Launch at login and low-allowance notifications are disabled by default.
- Disconnect Claude before uninstalling or deleting Allowance data.
