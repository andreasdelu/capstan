<img src="assets/CapstanIcon.png" width="80" alt="Capstan app icon">

# Capstan

A small macOS app that gives Caps Lock two jobs:

- **Tap:** Escape.
- **Hold:** Control, immediately.

A solo tap must be shorter than 300 ms. Holding longer or pressing another key cancels Escape. Change the keys and timeout in **Customize**.

## A quick look

<img src="docs/images/settings.png" width="640" alt="Capstan settings with General, Customize and Advanced navigation">

<img src="docs/images/event-viewer.png" width="640" alt="Event Viewer showing Caps press results, durations and mapped keys">

*UI previews rendered from the real components with test data.*

## Try it

The current build is for **Apple Silicon Macs running macOS 14 or newer**.

1. Move `Capstan.app` to **Applications**.
2. Quit Karabiner-Elements or other keyboard remappers.
3. Open Capstan, grant **Input Monitoring** and **Accessibility**, then quit and reopen it.

**General** controls remapping and Launch at login. **Customize** changes the tap key, held modifier and timeout. **Advanced** contains menu-bar visibility, Event Viewer and Reload Settings.

Closing settings keeps remapping running. Open Capstan again to return, even if its menu icon is hidden.

### Event Viewer

Press **Record**, then try Caps Lock. Each row shows the action, duration and mapped key. **Stop** ends capture; **Clear** removes it. Closing the viewer stops and clears capture.

Only Caps-related events are retained, in memory, up to 200 events. The viewer reports generated translations, not proof that another app received them.

## Build locally

You need Rust and a recent full Xcode installation with Icon Composer support.

Create a trusted local **Code Signing** certificate named `Caps Tap Local` in Keychain Access, then run:

```sh
scripts/build-app
```

The signed app is written to `dist/Capstan.app`. Set `CAPS_TAP_SIGNING_IDENTITY` to use a different signing certificate.

## Sharing

Local and ad hoc signatures are **not** Apple-verified distribution trust. Gatekeeper or company device management may block these builds.

For wider distribution, use a **Developer ID certificate and notarization**, which require Apple Developer Program membership. Managed Macs may still need IT approval.

## Settings and development

Settings live in `~/Library/Application Support/Capstan/settings.json`. UI changes save automatically; external edits need **Advanced → Reload Settings**.

See [development notes](docs/development.md) for the JSON format, tests and packaging details.

Capstan is experimental. Secure input, sleep/wake and interrupted keyboard access need further real-device testing.
