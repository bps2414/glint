# Changelog

## Unreleased

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
