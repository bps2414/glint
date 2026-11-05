# Architecture

Glint is a Cargo workspace plus a small Cloudflare Worker. The design goal is that **nothing the user does
in the UI can cost frames in the game**, so recording and the interface are separate programs.

## Processes

| Binary | Crate | Role |
|---|---|---|
| `glint-recorder` | `app-recorder` | Headless daemon in the tray. Detects the game, captures, encodes, keeps the replay buffer, saves clips on F8 and F9 (kill), shows the optional Discord Rich Presence. Idle (EcoQoS) when no game is running. |
| `glint` | `app-ui` | The library, editor, player and settings (GPUI + gpui-component). Started on demand; when it closes, it is gone. During a match its cost is zero. |

The two talk over a local socket (`app-ipc`: named pipe on Windows, Unix socket elsewhere) with typed
`postcard` messages. The protocol is synchronous request/response: the UI asks for the recorder status when
it needs it, and a `clips_saved` counter tells it when to reload the library. Both share the SQLite library
(`library.db`, WAL) and the config file (TOML).

## Crates

| Crate | Responsibility |
|---|---|
| `app-core` | Domain with no OS code: replay buffer, library database, config, game-detection rules, disk retention, naming, migration. Defines the platform traits. |
| `app-media` | FFmpeg (LGPL, dynamically linked): HW encode, remux/mux, decode for the player, thumbnails, `export_mp4` (trim + H.264/AAC re-encode, optional watermark). |
| `app-platform-win` | **All Windows code**: Windows Graphics Capture, D3D11, WASAPI, hotkeys, foreground/game detection, tray and tray menu, credentials, autostart. |
| `app-ipc` | Recorder ↔ UI protocol. |
| `app-cloud` | Client for the optional server: Discord login (loopback + PKCE), chunked upload with resume, update check. |
| `app-recorder`, `app-ui` | The two binaries above. |
| `server/` | Cloudflare Worker (TypeScript, Hono) with D1 and R2. See below. |
| `tools/make-icons` | Generates icons from `docs/brand/*.svg`. |

OS-specific code lives only in `app-platform-*` behind traits (`CaptureBackend`, `EncoderBackend`,
`AudioBackend`, `HotkeyBackend`, `TrayBackend`), so a Linux backend (PipeWire, VAAPI) is a new crate, not a
rewrite.

## Capture and encode pipeline

```text
WGC frame (D3D11 texture, GPU) -> "latest frame" slot
        | video thread, constant frame rate
        v
D3D11 VideoProcessor: BGRA -> NV12 (+ scale), still on the GPU
        v
hardware encoder via FFmpeg: AMD AMF / NVIDIA NVENC / Intel QSV (HEVC or H.264)
        v
compressed packets -> ring buffer in RAM  <- AAC audio (game and microphone, separate tracks)
```

- **Windows Graphics Capture** captures at the compositor level. It does not inject into, hook or read the
  memory of the game process, and draws no overlay, so it is safe with anti-cheat such as Vanguard.
- Frames never reach the CPU. Color conversion uses the fixed-function video processor, and the encoder
  is a dedicated block of the GPU, separate from the shaders that draw the game.
- Game audio uses WASAPI process loopback (only the game, not Discord or Spotify); the microphone is its own
  track.

## Replay buffer and saving

The buffer is a sliding window of **already-encoded packets** (default 60 s, 15-120 s). It always starts at a
keyframe, so pressing F8 saves by **remuxing** to MP4, with no re-encode (~150 ms). A background saver then
writes the thumbnail, records the clip in SQLite and applies the disk limit (oldest first; favorites and
published clips are never deleted).

**Kill clips** use a second hotkey (F9, `[kill]` in `config.toml`). Each press stamps `clock::now_us()` (QPC,
the same clock as the packets) and plays a short tick; the daemon keeps one pending clip
(`first_us`, `last_us`, `presses`, `deadline`). A press before the deadline extends it, so consecutive kills
become one clip; the deadline is `max(combo_secs, post_secs)` after the last press. When it expires (or the
session is about to be dropped) `ReplayBuffer::snapshot_range(first - pre, last + post)` is remuxed like any
other clip and stored with `kind = kill` and the press count (x2, x3...). A combo that would outgrow the
buffer is flushed and a new one started. The video starts at the keyframe before the requested start, so
the clip can have up to one GOP (1 s) extra at the beginning. The two hotkeys register independently: a key
taken by another app disables only itself.

## Player and editor

`app_media::Player` runs a decoder thread (D3D11VA when available, software fallback) that keeps a short
queue of frames ahead. The UI owns the clock and picks which frame to show; seeks are exact and coalesced
while scrubbing. GPUI has no native video surface on Windows, so frames are handed over as BGRA images at the
size they are displayed. Trimming is non-destructive: the in-/out-points are stored and applied on export.

## Publishing and the server

Publishing runs in a background thread: `export_mp4` (trimmed part, H.264 + AAC, `+faststart`) -> upload in
8 MiB parts through the Worker (resumable per part) -> complete. The server is optional and self-hosted:
each person or group deploys their own on Cloudflare's free plan.

- **Worker:** Hono + zod, D1 for metadata, R2 for video and thumbnails. Serves the clip page with OpenGraph
  tags (so Discord embeds the player), the MP4 with `Range` support, the invite/admin pages and the JSON API.
- **Auth:** Discord OAuth only. The app logs in through a loopback redirect with PKCE and receives a Bearer
  token; the browser uses a `__Host-` session cookie with CSRF protection. Tokens are stored as hashes.
- **Access:** the first sign-in on a new server becomes admin; after that an invite is required. Clips are
  `private` (signed, expiring URLs), `unlisted` (link) or `public`.
- **Contract:** [`API.md`](API.md) is the source of truth for the HTTP API (`/api/v1`).

## Design choices worth knowing

- **Rust + GPUI** for a light, fully custom UI, with the recorder as a separate headless process. Iced was
  the fallback if the video player had failed.
- **FFmpeg LGPL, shared** so one integration covers AMF, NVENC and QSV (and VAAPI later) while keeping the
  license permissive.
- **Cloudflare Workers + R2** because R2 has no egress fees and the Worker can stream video with Range, so a
  small group can host clips for free. Uploads go through the Worker in parts instead of presigned URLs: no
  S3 key to leak, resumable, and testable locally.
- **Windowed liquid-glass look** is done with the Windows acrylic backdrop plus CPU blur where glass sits over
  video, because GPUI on Windows has no in-window backdrop filter.
