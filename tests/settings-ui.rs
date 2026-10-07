// Render the real Settings implementation with a native Metal headless window.
// No event tap, login-item registration, real config writes, or running-app restart.
// Reuse binary-private modules rather than duplicating the settings screen.
#[allow(dead_code, unused_imports)]
#[path = "../src/main.rs"]
mod app;

fn main() {
    app::check_settings_ui();
}
