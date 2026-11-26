# lineapp

Line App Client for Linux

**BETA.** This app is an unfinished beta. It is incomplete, it breaks, and it changes without warning. Do not rely on it as a stable LINE client.

## Implemented

Sign-in and the window:

- QR login. The phone app scans the code, confirms a PIN, and a saved certificate skips that PIN the next time.
- The session stays in the app config directory. Log out clears it. Quit exits.
- A tray icon shows the unread count. Closing the window can leave the app in the tray.
- About shows the build number and the time it was built.

Chats:

- The chat list covers friends, groups, rooms, and official accounts. Tabs and a name search filter that list. New activity arrives while the window is open.
- Opening a chat loads the latest 40 messages and marks the chat read. Your own messages show Read, or Read and a count when more than one person has read them.
- The composer sends text, including a reply that quotes another message. A failed send can be retried.
- One-to-one text that LINE refuses with `E2EE_RETRY_ENCRYPT` is sent with letter sealing. Text sealed to a key this client holds opens in the thread. Text sealed to another device's key stays labeled "Letter-sealed message."
- Stickers, photos, videos, and short animated images already in a chat are shown. Photos and videos open in a viewer.
- A call that already happened shows as a duration. The app does not place the call.
- Reactions use a fixed set: like, cool, love, angry, please, cry, and wow.
- The message menu copies text, shares that text into another chat, unsends your own message, and reports someone else's.
- Each chat can mute the notification sound on this machine. A chat LINE already muted stays quiet. A new message plays a chime and updates the tray badge.
- Group system lines cover joins, leaves, removals, invites, renames, and picture changes.

People and groups:

- The rail shows your profile photo.
- A profile window shows a person's name, picture, and status. From there you can favorite them, rename how they appear to you, open the chat, add them, block or unblock them, or report them. Someone who is not a friend gets a fraud warning.
- A group window lists members and can search them, invite friends, remove a member, rename the group, and turn automatic join on or off.

## What the official LINE app does that this does not

Sending and history:

- You can send text only. Photos, videos, voice messages, files, stickers, locations, and contact cards cannot be sent. Received stickers, photos, and videos are shown. Voice messages, files, locations, and contact cards are only a label in the thread.
- There is no search inside a chat or across message text. Search covers chat names.
- History stops at the latest 40 messages.
- Letter sealing on send is one-to-one text. A message this client cannot unseal stays a label.
- There is no typing indicator, no way to announce a message, and no albums, notes, or Keep.

Calls:

- There are no voice or video calls. Settings lists speakers, microphones, and cameras, and those lists do nothing. This app has no LINE VoIP.

People, groups, and the rest of LINE:

- Friends is a filter of chats you already have. There is no friend directory, and no way to add someone by LINE ID, phone number, or QR.
- You cannot create a group or a room, leave a group, change the group picture, or manage an invite link.
- You cannot change your own name, status message, or profile picture.
- Official accounts appear in the list. Rich menus, coupons, and shop actions do not.
- There is no OpenChat, LINE VOOM, Keep, albums, notes, LINE Pay, sticker shop, or theme.
- There is one account. Chats cannot be pinned, hidden, or archived. Notification settings on LINE's servers are not edited here. The sound mute is local.
- Help opens a note that help is not in this window yet. There is no desktop notification popup, only the chime and the tray badge.

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

To run a development build or the `.deb`:

- WebKitGTK 4.1 and GTK 3. On Debian or Ubuntu that runtime package is `libwebkit2gtk-4.1-0`. The `.deb` installs it through apt.
- `libayatana-appindicator3-1` or `libappindicator3-1` for the tray icon.

The AppImage carries WebKitGTK 4.1, GTK 3, their modules and schemas, the WebKit helper processes, and the tray library. It still uses the host C library, libstdc++, the GPU libraries (`libGL`, `libEGL`, `libdrm`), X11 or Wayland, and fontconfig. AppImage leaves those out: a bundled GPU driver only works on the build machine, and the format cannot carry glibc. Mounting the image needs `libfuse2`. `APPIMAGE_EXTRACT_AND_RUN=1` skips FUSE and unpacks it instead. The image runs where glibc and libstdc++ are at least as new as on the machine that built it.

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

`productName` is `LINE`, so the Debian package name is `line`. Installing it next to the official LINE package will collide. The first AppImage build downloads `linuxdeploy` and `appimagetool`. `linuxdeploy` copies the libraries the binary links, including WebKit and GTK, and its GTK plugin copies the modules and schemas. Mounting the finished image needs FUSE.

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
