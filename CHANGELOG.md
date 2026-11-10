# Changelog

## 1.1.0

- **New setup:** a full-screen setup on first open (and once after updating, keeping your account and
  settings): hotkeys you set by pressing them, with a live test that lights up when the recorder hears
  F8/F9 and warns if another app owns the key; quality levels (Light, Balanced, Maximum) recommended
  from what your graphics card can actually encode, plus game sound and microphone with a level meter;
  joining a server by pasting the invite; and Windows options. **Skip setup** keeps the recommended
  values.
- **Launcher:** opening Glint with everything closed shows a small splash that checks for an update
  (5 s at most, skipped offline), installs it, starts the recorder and opens the library with the
  thumbnails already loaded.
- **Background updates:** the recorder downloads new versions on its own and installs them only when
  nothing is happening (no game, no recording, Glint window closed). Settings > System > Check for
  updates automatically turns this off.
- **Join with an invite:** an invite link becomes your account on a friend's server, no Discord sign-in
  needed; **I have a code from another PC** connects a second PC to the same account. Admins invite,
  list members and remove devices from Settings > Server.
- **Create your server from the app:** a wizard deploys the server to your free Cloudflare account and
  connects you as admin; it also updates the server when the app is newer.
- **Scroll bars:** the library, Settings, the setup and the publish panel show a scroll bar you can click
  and drag.
- **Windows 10:** recording no longer shows the yellow capture border.
- **Tray:** Quit always closes the recorder, even if a capture is stuck.
- **Report a bug / send feedback:** Settings > Help (and **Report bug** in the tray) sends a message
  straight to the maintainer, no GitHub account needed. The message and the PC info (app version,
  Windows build, GPU, capture backend, language) become a public issue; the optional contact and the
  recorder logs stay private. Logs are opt-in, cleaned of your user folder and tokens, and
  **See what will be sent** shows the exact text first. **Leave a star** opens the repository.

## 1.0.0-rc.3

- **Kill clips:** a second hotkey (**F9** by default) saves a short clip around the play instead of the
  full 60 s: 8 s before the press and 2 s after. Pressing it again within 6 s extends the same clip, so
  a quick double or triple kill becomes one clip marked "x2"/"x3". Each press plays a short tick; the
  clip is saved when the combo window closes. Kill clips get their own "Kills" section in the library
  and a badge on the card. Times and hotkey are in Settings > Kill clip.
- **Hotkeys:** if another app already uses one of the hotkeys, the other keeps working (before, one busy
  key disabled both); the tray says which one is busy.
- **Clip page:** on phones, leaving fullscreen on the video no longer leaves the page zoomed out with a
  long empty scroll.

## 1.0.0-rc.2

- **Discord Rich Presence:** your Discord profile shows "Clipping VALORANT with Glint" while a game is
  being recorded ("Clipping with Glint" otherwise), the time since Glint started and a "Get Glint"
  button. On by default; Settings > System > "Show on Discord" turns it off. No server or sign-in
  needed.

## 1.0.0-rc.1

First public release candidate.

- Saves the last seconds of gameplay with a hotkey; recorder runs separately in the tray.
- Library with hover preview, trimming, favorites and multi-select publishing.
- Optional watermark and Export MP4.
- Publish to your own free Cloudflare server (see docs/DEPLOY.md); uploads pause while a game is open.
- English and Brazilian Portuguese, following the Windows language.
- Auto-update from GitHub Releases.
