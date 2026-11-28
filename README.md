# lineapp

Line App Client for Linux and Mac

This app does not support VoIP or video calls. LINE places those calls through a separate call service and a native media stack that ships inside the official desktop app. This client only speaks the messaging gateway, so it never opens a call, negotiates audio or video, or sends call media. A call that already happened is stored as a chat message, and the thread can show how long it lasted. The microphone records voice typing and voice messages. Those are files in the chat, not a live call.

**BETA.** This app is an unfinished beta. It is incomplete, it breaks, and it changes without warning. Do not rely on it as a stable LINE client.

## Implemented

Sign-in and the window:

- QR login. The phone app scans the code, confirms a PIN, and a saved certificate skips that PIN the next time.
- The session stays in the app config directory. Log out clears it. Quit exits.
- The tray icon shows a red dot while anything is unread. The unread count is on the chat button in the window. Closing the window can leave the app in the tray. The window can stay above other windows. A new message can raise the window and ask the taskbar for attention. A muted chat does neither.
- Settings covers language, font, and text size. The languages are English, Japanese, Korean, simplified and traditional Chinese, Thai, Vietnamese, Indonesian, Malay, Spanish, Portuguese, French, German, Italian, Russian, Arabic, Turkish, Greek, and Hindi, or the device language. Arabic is right to left. Fonts are Noto Sans, the system font, a serif, and a monospace face, at five sizes. Characters a chosen font does not include still use Noto Sans.
- The notification sound has a volume and fourteen tones. Each chat can mute that sound on this machine. A chat LINE already muted stays quiet. The red tray dot still appears.
- The microphone chosen in Settings is used for voice typing, voice messages, and a test that plays the recording back and shows what the speech engine heard. Speakers and cameras are listed for reference.
- About shows the build number and the time it was built. The chat list pane can be resized.

Chats:

- The chat list covers friends, groups, rooms, and official accounts. Tabs and a name search filter that list. New activity arrives while the window is open.
- Opening a chat loads the latest 40 messages, scrolls to the first unread one, and marks the chat read. Your own messages show Read, or Read and a count when more than one person has read them. A long message collapses until you expand it.
- The composer sends text, including a reply that quotes another message. The quote scrolls to that message when it is in the loaded history. A failed send can be retried.
- Photos, videos, audio, and other files can be attached or dropped onto the thread. Up to ten photos go out together and show as one grid. The send follows the desktop object-storage flow: a content-typed message, then the bytes uploaded to LINE. A chat that requires letter sealing encrypts the file first. A sticker button sends a short published set, five still stickers and eleven animated ones.
- One-to-one text that LINE refuses with `E2EE_RETRY_ENCRYPT` is sent with letter sealing. Photos, videos, audio, and files get the same retry, including in a group or a room when this client has that chat's shared key. Text and files sealed to a key this client holds open in the thread. Anything sealed to another device's key stays labeled "Letter-sealed message."
- Stickers, photos, videos, voice messages, and short animated images already in a chat are shown. Photos and videos open in a viewer. A received file can be saved. A link that is the whole message gets a preview card and opens in the browser.
- Voice typing writes into the composer. Holding the voice button records a message, and sliding away cancels it. Playback draws a waveform. On Linux the recording is encoded with `ffmpeg`. On macOS it uses `afconvert`.
- A call that already happened shows as a duration. The app does not place the call.
- Reactions use a fixed set: like, cool, love, angry, please, cry, and wow.
- The message menu copies text, shares that text into another chat, unsends your own message, and reports someone else's.
- Group system lines cover joins, leaves, removals, invites, renames, and picture changes.

People and groups:

- The rail shows your profile photo. Friends is a directory, with favorites first and a name search. Choosing someone opens their chat.
- Add friends searches a LINE ID or a phone number and lists LINE's recommendations. From a result you can add the person.
- Create a group takes a name and the friends you pick.
- A profile window shows a person's name, picture, cover, and status. From there you can favorite them, rename how they appear to you, open the chat, add them, block or unblock them, or report them. Someone who is not a friend gets a fraud warning. Your own profile is view only.
- A group window lists members and can search them, invite friends, remove a member, rename the group, and turn automatic join on or off. A member's row opens their profile.

Agent:

- The agent answers questions about chats already saved on this computer. It can search those chats by name or message text, list them, and read the recent messages in one of them. It cannot send, delete, or change anything.
- Settings points it at an OpenAI-compatible endpoint, Anthropic, or Gemini, with a model, an API key, and optional temperature, top-p, max tokens, thinking level, and extra JSON. The text it reads from your chats is sent to that endpoint. The endpoint and key are stored in `lineapp.settings.json` next to the session file.

## What the official LINE app does that this does not

Sending and history:

- Locations and contact cards cannot be sent. A received location or contact card is a label in the thread. Photos, videos, voice messages, and files you send show in the thread. The sticker button is that short published set, not the sticker shop.
- There is no search box inside a chat. The chat list searches names. The agent can search message text that is already in the local database.
- The thread shows the latest 40 messages. The local database keeps the latest 80, which is what the agent can read.
- Letter sealing on send is one-to-one text, plus photos, videos, audio, and files when LINE asks for it. A message this client cannot unseal stays a label.
- Holding the send button offers a send without notification. LINE still notifies the recipient, so that choice does not send.
- There is no typing indicator, no way to announce a message, and no LINE albums, notes, or Keep. Several photos can be sent together. That is not the albums feature.

Calls:

- Settings lists speakers and cameras, and those lists do nothing. The microphone is for voice typing, voice messages, and the mic test.

People, groups, and the rest of LINE:

- You can add someone by LINE ID or phone number. You cannot add them by scanning a QR code in this window.
- You cannot create a room, leave a group, change the group picture, or manage an invite link.
- You cannot change your own name, status message, or profile picture.
- Official accounts appear in the list. Rich menus, coupons, and shop actions do not.
- There is no OpenChat, LINE VOOM, Keep, albums, notes, LINE Pay, sticker shop, or LINE theme. Font and text size are local.
- There is one account. Chats cannot be pinned, hidden, or archived. Notification settings on LINE's servers are not edited here. The sound mute, the tone, and the volume are local.
- Help opens a note that help is not in this window yet. There is no desktop notification popup. A new message plays the chime, shows the red tray dot, and can raise the window.

A small desktop client for the protocol the LINE desktop app speaks: Thrift
TCompact POSTed to `legy-jp.line-apps.com`. This is not the public Bot /
Messaging API.

The window is a TypeScript Tauri app. Login, the chat list, history, send,
letter sealing, photos, videos, audio, files, stickers, friends, groups, voice,
link previews, and the agent run in Rust. A v3 access
token is a JWT and talks to `/S4` in the clear. Older `mid:secret` tokens go
through LINE's encrypted LEGY gateway (`gf.line.naver.jp/enc`).

QR login follows the current desktop flow (`createSession`,
`createQrCodeForSecure`, `checkQrCodeVerified`, PIN or certificate, then
`qrCodeLoginV2ForSecure`). Sends that LINE rejects with
TalkException `E2EE_RETRY_ENCRYPT` (code 82) are sealed with letter sealing
version 2. One-to-one text uses the peer's public key. Photos, videos, audio,
and files use that key, or the group shared key in a group or a room. Text and
files sealed to a key this client holds open in the thread. Messages sealed to
another device's key stay labeled "Letter-sealed message."

The app version string defaults to the LINE desktop build installed on this
machine, `26.6.0.4004`.

## Requirements

A release build needs a few gigabytes of free memory and several gigabytes free under `src-tauri/target`. The running app needs outbound HTTPS to `legy-jp.line-apps.com`. Older `mid:secret` tokens also open `gf.line.naver.jp`. The webview loads profile pictures and stickers from LINE.

To compile on either system:

- Rust 1.90 or newer. Tauri 2.12 requires that compiler. The `rust-version` field in `src-tauri/Cargo.toml` is still `1.77`.
- Node.js 20 or newer, with npm. Vite 6 also accepts Node.js 18.

### Linux

These packages are for 64-bit x86 Linux. The window needs a graphical session, X11 or Wayland.

To compile:

- A C toolchain and `pkg-config`. On Debian or Ubuntu install `build-essential`, `pkg-config`, `libwebkit2gtk-4.1-dev`, `libdbus-1-dev`, and `libayatana-appindicator3-dev`. WebKitGTK 4.1 brings GTK 3 with it. The packager looks up `ayatana-appindicator3-0.1` through pkg-config. The runtime library `libayatana-appindicator3-1` does not ship that file; the `-dev` package does. SQLite is bundled, so there is no separate SQLite package.
- An AppImage package also needs `file` and `patchelf` on the build machine. The first AppImage build downloads `linuxdeploy` and `appimagetool`.
- A Flatpak also needs `flatpak` on the build machine, with Flathub available. The first build installs `org.flatpak.Builder` and the GNOME 51 SDK for the current user. That SDK is about a gigabyte. The package is built from the `.deb`, then packed as a single-file `.flatpak`.

To run a development build or the `.deb`:

- WebKitGTK 4.1 and GTK 3. On Debian or Ubuntu that runtime package is `libwebkit2gtk-4.1-0`. The `.deb` installs it through apt.
- `libayatana-appindicator3-1` or `libappindicator3-1` for the tray icon.
- `ffmpeg` on `PATH` to send a voice message. Voice typing uses `whisper-cli` from `src-tauri/speech` or from `PATH`. The first transcription downloads the tiny Whisper model when that file is not already there.

The AppImage carries WebKitGTK 4.1, GTK 3, their modules and schemas, the WebKit helper processes, and the tray library. It still uses the host C library, libstdc++, the GPU libraries (`libGL`, `libEGL`, `libdrm`), X11 or Wayland, and fontconfig. AppImage leaves those out: a bundled GPU driver only works on the build machine, and the format cannot carry glibc. Mounting the image needs `libfuse2`. `APPIMAGE_EXTRACT_AND_RUN=1` skips FUSE and unpacks it instead. The image runs where glibc and libstdc++ are at least as new as on the machine that built it.

The Flatpak runs on the GNOME 51 runtime from Flathub. That runtime supplies WebKitGTK 4.1, GTK 3, `ffmpeg`, `pw-record`, and `pactl`, so a voice message does not need those tools on the host. The tray library is built inside the SDK and shipped in the app. The sandbox can reach the network, show a window on Wayland or X11, play and record audio, register a tray icon, open files through the desktop portal, and ask KWin to raise the window or keep it above others. Installing the `.flatpak` also installs that runtime from Flathub. The binary is the one in the `.deb`, so the machine that built it needs a glibc older than the runtime's. GNOME 51 currently has glibc 2.44.

### macOS

The Mac build needs the Xcode command-line tools (`xcode-select --install`). There is no WebKit or GTK package: the window is the system WebKit. Voice typing records with CoreAudio and asks for the microphone. Voice notes are encoded with `/usr/bin/afconvert`. Transcription still needs `whisper-cli` on `PATH` or in the app's speech directory, the same as on Linux. Opening a link uses `open`.

`src-tauri/tauri.macos.conf.json` selects the `app` and `dmg` packages and merges `Info.plist`, which carries the microphone and camera usage strings. The first time voice typing runs, macOS shows that prompt. A development binary embeds the same plist so the prompt is available from `tauri dev`.

`src-tauri/.cargo/config.toml` forces `PKG_CONFIG_PATH` at this machine's Linux WebKit sysroot. Cargo reads that file on every operating system. The Mac build links the system frameworks and does not use those libraries. If a Mac build stops in `pkg-config` because a Homebrew package is missing, comment out that `[env]` table so the Linux path does not replace `PKG_CONFIG_PATH`.

## Desktop window

Development, from the repo root:

```bash
. "$HOME/.cargo/env"
cd ~/dev/lineapp
npm run tauri dev
```

That starts Vite and opens the window against it. A production build compiles the frontend, then a release binary, then the packages for the machine you are on. Linux uses `deb` and `appimage` from `src-tauri/tauri.conf.json`. macOS uses `app` and `dmg` from `src-tauri/tauri.macos.conf.json`.

```bash
. "$HOME/.cargo/env"
cd ~/dev/lineapp
npm run tauri build -- --bundles deb,appimage
npm run flatpak
```

On macOS:

```bash
npm run tauri build -- --bundles app,dmg
```

`npm run tauri build` with no bundle list uses the config for the current system. Pass one target to build only that package. The packages are written to:

- `src-tauri/target/release/bundle/deb/line_0.1.0_amd64.deb`
- `src-tauri/target/release/bundle/appimage/LINE_0.1.0_amd64.AppImage`
- `src-tauri/target/release/bundle/flatpak/LINE_0.1.0_amd64.flatpak`
- `src-tauri/target/release/bundle/macos/LINE.app`
- `src-tauri/target/release/bundle/dmg/LINE_0.1.0_aarch64.dmg` (or `x64` on an Intel Mac)

`productName` is `LINE`, so the Debian package name is `line`. Installing it next to the official LINE package will collide. The first AppImage build downloads `linuxdeploy` and `appimagetool`. `linuxdeploy` copies the libraries the binary links, including WebKit and GTK, and its GTK plugin copies the modules and schemas. Mounting the finished image needs FUSE.

`npm run flatpak` packs the `.deb` into `LINE_0.1.0_amd64.flatpak`. It reuses a `.deb` already in the bundle directory, and builds one when that file is missing. Voice typing is included when `src-tauri/speech/whisper-cli` is present; the engine is installed under `/app/share/lineapp/speech`. Someone who receives the file installs it with:

```bash
flatpak install --user src-tauri/target/release/bundle/flatpak/LINE_0.1.0_amd64.flatpak
flatpak run dev.lineapp.desktop
```

`npm run release` on Linux publishes that file with the `.deb` and the AppImage. The app id inside Flatpak is `dev.lineapp.desktop`, the same id as the config directory. Flatpak stores that directory in the sandbox, at `~/.var/app/dev.lineapp.desktop/config/dev.lineapp.desktop`, so a Flatpak install does not read the session saved by the `.deb`.

The session is stored in `lineapp.local.json` (mode 0600) under the app config
directory. On Linux that is `$XDG_CONFIG_HOME/dev.lineapp.desktop`, or
`~/.config/dev.lineapp.desktop` when that variable is unset. On macOS it is
`~/Library/Application Support/dev.lineapp.desktop`. The chat database
and tray settings sit in the same directory. Photos and videos are cached under
the system cache directory (`~/.cache/lineapp` on Linux,
`~/Library/Caches/lineapp` on macOS). A debug build copies a session
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
