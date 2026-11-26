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

## Desktop window

```bash
. "$HOME/.cargo/env"
cd ~/dev/lineapp
npm run tauri dev
```

The session is stored in `lineapp.local.json` (mode 0600), next to this file.
Tauri's working directory is `src-tauri`, and the Rust side still reads that
repo-root file.

## Session

Secrets stay out of the repo. Either export them or put them in
`lineapp.local.json`:

| Variable | Local file field | |
| --- | --- | --- |
| `LINE_AUTH_TOKEN` | `accessToken` | required for profile and send |
| `LINE_REFRESH_TOKEN` | `refreshToken` | stored by QR login |
| `LINE_CERTIFICATE` | `certificate` | skips the PIN on the next QR login |
| `LINE_CONFIG` | | path of the JSON file (default `lineapp.local.json`) |
| `LINE_ENDPOINT` | `endpoint` | default `legy-jp.line-apps.com` |

QR login from the window writes `lineapp.local.json`.
