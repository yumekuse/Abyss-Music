# Changelog

Each `## <version>` section becomes the release notes for that version and shows up in the app's "what's new" window.

## 0.2.1
- Test update: same app as 0.2.0, just a new version number to check that updating works. Nothing else changed.

## 0.2.0
- **Rebuilt in Rust.** One ~6 MB app instead of Electron: the window appears in about 0.35 s and home is filled in under a second.
- **Instant start:** home, your picks, library and likes show right away from the last session, then refresh by themselves.
- **Auto-update:** new versions are found, downloaded, checked (SHA-256) and installed for you.
- **Announcements:** news from the abyss team appears at the top of the app.
- **Downloads work again**, with a progress panel (playlist / album / song, songs done, % per song, cancel).
- **Music only:** playlists hide videos that aren't music and tell you how many.
- **Linux:** .deb (Debian / Ubuntu / Kali), AppImage, tarball and a PKGBUILD for Arch / CachyOS.
- Romaji for Japanese lyrics now reads kanji correctly.
