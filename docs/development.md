# Development

## Settings

`~/Library/Application Support/Capstan/settings.json`:

```json
{
  "remapping_enabled": true,
  "escape_timeout_ms": 300,
  "show_menu_bar_icon": true,
  "tap_key": "escape",
  "hold_modifier": "control"
}
```

Tap keys: `escape`, `tab`, `return`, `backspace`, `space`.
Held modifiers: `control`, `shift`, `option`, `command`.
Timeout: 50–2000 ms. Launch at login is managed by macOS, not JSON.

Missing fields use defaults. Invalid reloads leave current settings intact. Invalid startup settings leave remapping off without overwriting the file. Saves use same-directory atomic rename. Mapping and timeout changes apply to the next press, so an active gesture releases its original modifier.

If the new settings file is absent, valid legacy `Caps Tap/settings.json` settings are imported without removing the old file. An existing destination always wins.

## Checks

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
scripts/build-app
codesign --verify --deep --strict dist/Capstan.app
```

Tests cover gesture policy, persistence, diagnostic capture, native window recovery and real GPUI Kit component interactions. Component tests use temporary settings and controlled input/status fixtures; they do not start an input tap or change Login Items. They produce previews in `dist/qa/`.

Tests do not establish physical key delivery, Finder event delivery, permission continuity or installed-app behavior. Secure input, sleep/wake and tap interruptions still need live checks. Other remappers with exclusive keyboard access cannot reliably coexist with Capstan.

## Packaging

The app keeps bundle identifier `dev.andreasdeleuran.capstap` and the local `Caps Tap Local` certificate to avoid unnecessary identity changes. Changed builds can still require new permission grants. Never commit or distribute certificate private keys.

`assets/capstan.icon` is the app icon source. `actool` compiles its layered artwork to `Assets.car` and a legacy `.icns` fallback; compiler-generated icon plist keys are merged into the bundle. The menu icon is a separate monochrome template, `assets/MenuBarIcon.png`.

The release profile uses size optimization, ThinLTO and one codegen unit. `scripts/build-app` strips local symbols from the packaged executable before signing, leaving the Cargo artifact intact. Signing before stripping would invalidate the signature.

A controlled same-source comparison saved about 0.9% in the signed executable with this profile. Compare signed artifacts, not raw Cargo binaries; sizes depend on source, toolchain and icon resources.

The distributed test executable is currently arm64 with a macOS 14 minimum. The build script's local bundle plist still declares 13.0; the executable's actual minimum takes precedence. The friend-test copy corrects its plist to 14.0. Older macOS and Intel builds are not validated.

## Diagnostics

Capture is opt-in, bounded to 200 Caps-only records and kept in memory. Other keycodes and characters are not stored. Stop preserves rows, Clear removes them without changing capture state, and closing the viewer stops and clears the session.

Gesture summaries use record sequence, not timestamp sorting: HID and callback timestamps have different references. Missing starts and recording boundaries produce partial captures rather than invented pairings. An unfinished press displays `Holding…`; event-creation failures remain visible. Generated output is not confirmation of OS delivery.
