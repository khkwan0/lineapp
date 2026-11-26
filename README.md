# lineapp

A small desktop client for the protocol the LINE desktop app speaks: Thrift
TCompact POSTed to `legy-jp.line-apps.com`. This is not the public Bot /
Messaging API.

The window is a TypeScript Tauri app. Login, the chat list, history, send,
letter sealing, stickers, and the rail menu actions run in Rust. A v3 access
token is a JWT and talks to `/S4` in the clear. Older `mid:secret` tokens go
through LINE's encrypted LEGY gateway (`gf.line.naver.jp/enc`).

QR login follows the current desktop flow (`createSession`,
`createQrCodeForSecure`, `checkQrCodeVerified`, PIN or certificate, then
`qrCodeLoginV2ForSecure`). One-to-one sends that LINE rejects with
TalkException `E2EE_RETRY_ENCRYPT` (code 82) are sealed with letter sealing
version 2. Text sealed to a key this client holds opens in the thread.
Messages sealed to another device's key stay labeled "Letter-sealed message."

The app version string defaults to the LINE desktop build installed on this
machine, `26.6.0.4004`.

## Requirements

These packages are for 64-bit x86 Linux. The window needs a graphical session,
X11 or Wayland. A release build needs a few gigabytes of free memory and
several gigabytes free under `src-tauri/target`. The running app needs
outbound HTTPS to `legy-jp.line-apps.com`. Older `mid:secret` tokens also
open `gf.line.naver.jp`. The webview loads profile pictures and stickers from
LINE.

To compile:

- Rust 1.90 or newer. Tauri 2.12 requires that compiler. The `rust-version` field in `src-tauri/Cargo.toml` is still `1.77`.
- Node.js 20 or newer, with npm. Vite 6 also accepts Node.js 18.
- A C toolchain and `pkg-config`. On Debian or Ubuntu install `build-essential`, `pkg-config`, `libwebkit2gtk-4.1-dev`, `libdbus-1-dev`, and `libayatana-appindicator3-dev`. WebKitGTK 4.1 brings GTK 3 with it. The packager looks up `ayatana-appindicator3-0.1` through pkg-config. The runtime library `libayatana-appindicator3-1` does not ship that file; the `-dev` package does. SQLite is bundled, so there is no separate SQLite package.
- An AppImage package also needs `file` and `patchelf` on the build machine. The first AppImage build downloads `linuxdeploy` and `appimagetool`.

To run a development build, the `.deb`, or the AppImage:

- WebKitGTK 4.1 and GTK 3. On Debian or Ubuntu that runtime package is `libwebkit2gtk-4.1-0`. The AppImage does not bundle WebKit or GTK. The `.deb` installs them through apt.
- `libayatana-appindicator3-1` or `libappindicator3-1` for the tray icon.
- `libfuse2` to launch the AppImage.

## Desktop window

Development, from the repo root:

```bash
. "$HOME/.cargo/env"
cd ~/dev/lineapp
npm run tauri dev
```

That starts Vite and opens the window against it. A production build compiles the frontend, then a release binary, then the Linux packages configured in `src-tauri/tauri.conf.json` (`deb` and `appimage`):

```bash
. "$HOME/.cargo/env"
cd ~/dev/lineapp
npm run tauri build -- --bundles deb,appimage
```

Pass one target to build only that package. The packages are written to:

- `src-tauri/target/release/bundle/deb/line_0.1.0_amd64.deb`
- `src-tauri/target/release/bundle/appimage/LINE_0.1.0_amd64.AppImage`

`productName` is `LINE`, so the Debian package name is `line`. Installing it next to the official LINE package will collide. The first AppImage build downloads `linuxdeploy` and `appimagetool`. Running the AppImage needs FUSE.

The session is stored in `lineapp.local.json` (mode 0600) under the app config
directory: `$XDG_CONFIG_HOME/dev.lineapp.desktop`, or
`~/.config/dev.lineapp.desktop` when that variable is unset. The chat database
and tray settings sit in the same directory. A debug build copies a session
left at the repo root into that directory the first time the new file is
missing. A release build does not look at the repo.

## Session

Secrets stay out of the repo. Either export them or put them in
`lineapp.local.json`:

| Variable | Local file field | |
| --- | --- | --- |
| `LINE_AUTH_TOKEN` | `accessToken` | required for profile and send |
| `LINE_REFRESH_TOKEN` | `refreshToken` | stored by QR login |
| `LINE_CERTIFICATE` | `certificate` | skips the PIN on the next QR login |
| `LINE_CONFIG` | | path of the JSON file (default is the app config directory) |
| `LINE_ENDPOINT` | `endpoint` | default `legy-jp.line-apps.com` |

QR login from the window writes `lineapp.local.json` in the app config directory.
