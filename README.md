# Caps Tap

Experimental macOS Caps Lock remapper with a GPUI Kit settings window. Caps acts as Control immediately; if released alone within the configured window (300 ms by default), Escape is sent. A chord or longer hold sends no Escape, matching Andreas's Karabiner rule.

## Build and try

1. Quit Karabiner-Elements completely; it grabs the keyboard even when its Caps rule is disabled.
2. Create and trust a local **Code Signing** identity named `Caps Tap Local` in Keychain Access (`security find-identity -v -p codesigning` must list it). Run `scripts/build-app` to build and sign the bundle (or set `CAPS_TAP_SIGNING_IDENTITY` to a different valid identity). Then move `dist/Caps Tap.app` to a **stable location** (such as `/Applications`) before using Launch at Login. Open the `.app` from there. `cargo run` works for UI testing, but cannot enable Launch at Login.
3. Until both permissions are granted, the GPUI settings window lists **Input Monitoring** and **Accessibility** instead of the two main toggles. Once they're usable after relaunch, the permissions section disappears. Caps Tap does not auto-prompt: click each missing permission to open its System Settings pane, grant Caps Tap there, then quit and reopen Caps Tap. macOS may not apply Input Monitoring until relaunch, so there is no in-process Refresh button. If an older build is still listed, remove its old grant before enabling the rebuilt app. The window reports startup errors rather than pretending remapping is active.
4. Toggle **Enable remapping** (stored in the JSON settings file) or **Launch at login** (macOS Login Items). macOS may require approval in System Settings → General → Login Items. The `Caps` menu-bar item shows running status, Open Settings, and Quit. Settings has a Dock icon while open. Closing it hides the window and Dock icon while remapping stays active; Open Settings in the menu bar brings both back.

`scripts/build-app` signs the finished bundle with the same local certificate on every build. `codesign -d -r- 'dist/Caps Tap.app'` should show `identifier "dev.andreasdeleuran.capstap" and certificate leaf = …`, **not** `cdhash`. The switch from the old ad-hoc signature may require one new grant; permission survival after a changed-binary rebuild still needs a live test. Do not commit or export the certificate's private key. Builds and tests do **not** verify macOS input delivery, menu-bar behavior, or login-item registration. No login item is registered by building the app. Run `CAPS_TAP_DEBUG=1 'path/to/Caps Tap.app/Contents/MacOS/caps-tap'` to log physical Caps up/down, chord event type/keycode, measured HID duration, and the Escape/chord/hold decision. Timing uses the HID event timestamps rather than callback delivery times, so a delayed callback does not change the tap window. Modifier chords require a real modifier keycode; state-only flags notifications (observed as keycode 255 around Caps transitions) do not cancel an alone tap. These logs contain keycodes, not typed text; do not share traces containing sensitive input.

## Settings

Caps Tap creates `~/Library/Application Support/Caps Tap/settings.json` on first launch:

```json
{
  "remapping_enabled": true,
  "escape_timeout_ms": 300
}
```

The existing macOS remapping preference is imported only when this file is absent. The app's remapping toggle and Escape window ± buttons save and apply changes immediately. You can also edit the JSON and click **Reload Settings** without restarting or rebuilding. A reload validates the entire file before applying it; malformed JSON, unknown keys, or a timeout outside 50–2000 ms show an error and leave active settings unchanged. Invalid startup settings leave remapping off until corrected and reloaded, without overwriting the file. Missing fields use defaults. Launch at Login remains an OS-managed Login Item, not a JSON option.

Timeout changes affect the next Caps press, not an in-progress gesture. Control starts immediately; an unchorded release at or beyond the window cancels Escape. Settings are written by same-directory rename so an interrupted write cannot truncate the live file. There is no file watcher: external edits need explicit Reload Settings.

While remapping is enabled, Caps Tap clears native Caps Lock on activation, processed Caps transitions, and physical release, and strips the Caps flag from forwarded input. Dropping the processed event alone is insufficient because macOS has already updated its lock state. Disabling remapping restores normal Caps behavior; it does not restore a previously active lock.

This is still a spike: login screen, secure input, sleep/wake, physical Caps Lock state/LED (including the native-state reset), tap interruptions, and stuck modifiers need real-device testing. A disabled event tap is re-enabled and a synthetic Control release is attempted, but lost input cannot be recovered. It cannot reliably coexist with a remapper that has exclusive access to the same keyboard.
