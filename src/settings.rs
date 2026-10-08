use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui_kit::{
    App, Bounds, Context as ViewContext, Entity, FontWeight, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable, Theme,
    alert::Alert,
    button::Button,
    input::{Input, InputEvent, InputState},
    switch::Switch,
};

use super::{Context, config, mac, start_input};

#[derive(Clone)]
enum Feedback {
    Success(String),
    Error(String),
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct SystemFacts {
    permissions: mac::Permissions,
    bundled: bool,
    login: bool,
}

struct Settings {
    #[cfg(test)]
    system_facts: Option<SystemFacts>,
    input: Arc<Context>,
    input_error: Option<String>,
    needs_restart: bool,
    message: Option<Feedback>,
    timeout_input: Option<Entity<InputState>>,
    timeout_subscription: Option<Subscription>,
    config: config::Config,
    config_path: Result<std::path::PathBuf, String>,
}

impl Settings {
    fn setup_timeout(&mut self, window: &mut Window, cx: &mut ViewContext<Self>) {
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(self.config.escape_timeout_ms.to_string())
                .step(50.)
                .min(config::MIN_TIMEOUT_MS as f64)
                .max(config::MAX_TIMEOUT_MS as f64)
                .validate(|text, _| text.chars().all(|ch| ch.is_ascii_digit()))
        });
        self.timeout_subscription =
            Some(
                cx.subscribe_in(&state, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.save_timeout(window, cx);
                    }
                }),
            );
        self.timeout_input = Some(state);
    }

    fn sync_timeout(&self, window: &mut Window, cx: &mut ViewContext<Self>) {
        if let Some(state) = &self.timeout_input {
            state.update(cx, |state, cx| {
                state.set_value(self.config.escape_timeout_ms.to_string(), window, cx)
            });
        }
    }

    fn step_timeout(&mut self, increment: bool, window: &mut Window, cx: &mut ViewContext<Self>) {
        if let Some(state) = &self.timeout_input {
            let draft = state
                .read(cx)
                .value()
                .parse::<u64>()
                .unwrap_or(self.config.escape_timeout_ms);
            let value = if increment {
                draft.saturating_add(50)
            } else {
                draft.saturating_sub(50)
            }
            .clamp(config::MIN_TIMEOUT_MS, config::MAX_TIMEOUT_MS);
            state.update(cx, |state, cx| {
                state.set_value(value.to_string(), window, cx)
            });
        }
    }

    fn save_timeout(&mut self, window: &mut Window, cx: &mut ViewContext<Self>) {
        let Some(state) = &self.timeout_input else {
            return;
        };
        let text = state.read(cx).value();
        let result = text
            .parse::<u64>()
            .map_err(|_| "Enter a whole number of milliseconds".into())
            .and_then(|escape_timeout_ms| {
                self.save(config::Config {
                    escape_timeout_ms,
                    ..self.config.clone()
                })
            });
        self.message = Some(match result {
            Ok(()) => {
                self.sync_timeout(window, cx);
                Feedback::Success("Escape window saved".into())
            }
            Err(error) => Feedback::Error(error),
        });
        cx.notify();
    }

    fn apply(&mut self, config: config::Config) {
        if !self.needs_restart && self.input_error.is_none() {
            self.input.apply_config(&config);
        }
        #[cfg(not(test))]
        mac::set_menu_bar_visible(config.show_menu_bar_icon);
        self.config = config;
    }

    fn save(&mut self, config: config::Config) -> Result<(), String> {
        let path = self.config_path.as_ref().map_err(Clone::clone)?;
        config::save(path, &config)?;
        self.apply(config);
        Ok(())
    }

    fn reload(&mut self) -> Result<(), String> {
        let path = self.config_path.as_ref().map_err(Clone::clone)?;
        let config = config::load(path)?;
        self.apply(config);
        Ok(())
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let remapping = self.input.enabled.load(Ordering::Acquire);
        #[cfg(test)]
        let facts = self.system_facts.unwrap_or_else(|| SystemFacts {
            permissions: mac::permissions(),
            bundled: mac::bundled(),
            login: mac::login_enabled(),
        });
        #[cfg(test)]
        let (permissions, bundled, login) = (facts.permissions, facts.bundled, facts.login);
        #[cfg(not(test))]
        let (permissions, bundled, login) =
            (mac::permissions(), mac::bundled(), mac::login_enabled());
        let usable = permissions.ready() && !self.needs_restart && self.input_error.is_none();
        let status = if !usable {
            "Needs attention"
        } else if remapping {
            "On"
        } else {
            "Off"
        };
        let status_color = if !usable {
            theme.warning
        } else if remapping {
            theme.success
        } else {
            muted
        };

        div().id("settings-scroll").test_support().size_full().overflow_y_scroll()
            .bg(theme.background).text_color(theme.foreground).text_size(px(13.)).font_family(theme.font_family.clone())
            .child(div().flex().flex_col().gap_4().p_4()
                .child(div().flex().justify_between().items_center()
                    .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Capstan"))
                    .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(status_color)
                        .child(div().size(px(6.)).rounded_full().bg(status_color)).child(status)))
                .when(!permissions.ready() || self.needs_restart, |view| view.child(self.permission_panel(permissions, cx)))
                .when_some(self.input_error.clone(), |view, error| view.child(div().id("input-error").test_support().child(Alert::error("input-alert", error))))
                .when_some(self.message.clone(), |view, feedback| view.child(div().id("settings-message").test_support().child(match feedback {
                    Feedback::Success(text) => Alert::success("settings-alert", text),
                    Feedback::Error(text) => Alert::error("settings-alert", text),
                })))
                .child(div().flex().flex_col().rounded_lg().border_1().border_color(border)
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Remap Caps Lock", "Tap → Escape · Hold → Control", cx))
                        .child(Switch::new("remapping").small().accessibility_label("Enable remapping").checked(remapping).disabled(!usable)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                this.message = this.save(config::Config { remapping_enabled: *enabled, ..this.config.clone() }).err().map(Feedback::Error);
                                cx.notify();
                            }))))
                    .child(div().h_px().bg(border))
                    .child(div().flex().flex_col().gap_2().p_3()
                        .child(div().flex().items_center().justify_between().gap_2()
                            .child(div().font_weight(FontWeight::MEDIUM).child("Tap timeout"))
                            .child(div().text_size(px(11.)).text_color(muted).child(if usable {
                                format!("Active: {} ms", self.config.escape_timeout_ms)
                            } else { format!("Saved: {} ms", self.config.escape_timeout_ms) })))
                        .child(div().flex().items_center().gap_2()
                            .child(Button::new("timeout-less").small().icon(IconName::Minus).accessibility_label("Decrease Escape window by 50 ms").tooltip("Decrease by 50 ms").disabled(!usable)
                                .on_click(cx.listener(|this, _, window, cx| this.step_timeout(false, window, cx))))
                            .when_some(self.timeout_input.clone(), |view, state| view.child(Input::new(&state)
                                .small().id("escape-timeout").aria_label("Escape window in milliseconds").suffix("ms").w(px(100.)).disabled(!usable)))
                            .child(Button::new("timeout-more").small().icon(IconName::Plus).accessibility_label("Increase Escape window by 50 ms").tooltip("Increase by 50 ms").disabled(!usable)
                                .on_click(cx.listener(|this, _, window, cx| this.step_timeout(true, window, cx))))
                            .child(Button::new("apply-timeout").small().label("Apply").outline().disabled(!usable)
                                .on_click(cx.listener(|this, _, window, cx| this.save_timeout(window, cx)))))
                        .child(div().text_size(px(11.)).text_color(muted).child(if usable {
                            "Hold longer to cancel Escape. Changes apply to the next press."
                        } else { "Available after restart once input permissions are ready." })))
                    .child(div().h_px().bg(border))
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Launch at login", if bundled { "Start automatically when you sign in." } else { "Open the bundled app to enable this." }, cx))
                        .child(Switch::new("login").small().accessibility_label("Launch at login").checked(login).disabled(!bundled)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                this.message = mac::set_login_enabled(*enabled).err().map(Feedback::Error);
                                cx.notify();
                            }))))
                    .child(div().h_px().bg(border))
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Show menu bar icon", "Open Capstan.app to return when hidden.", cx))
                        .child(Switch::new("show-menu-bar").small().accessibility_label("Show menu bar icon").checked(self.config.show_menu_bar_icon)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                this.message = this.save(config::Config { show_menu_bar_icon: *enabled, ..this.config.clone() }).err().map(Feedback::Error);
                                cx.notify();
                            })))))
                .child(div().flex().justify_between().items_center().gap_3()
                    .child(div().text_size(px(11.)).text_color(muted).child("Edited settings.json?"))
                    .child(Button::new("reload-settings").small().label("Reload Settings").icon(IconName::RefreshCw).outline()
                        .tooltip("~/Library/Application Support/Capstan/settings.json")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.message = Some(match this.reload() {
                                Ok(()) => { this.sync_timeout(window, cx); Feedback::Success("Settings reloaded".into()) },
                                Err(error) => Feedback::Error(format!("{error}. Current settings unchanged.")),
                            });
                            cx.notify();
                        })))))
    }
}

fn setting_label(title: &'static str, detail: &'static str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        )
}

impl Settings {
    fn permission_panel(
        &self,
        permissions: mac::Permissions,
        cx: &mut ViewContext<Self>,
    ) -> impl IntoElement {
        div().flex().flex_col().gap_3()
            .child(Alert::warning("permission-guidance", if permissions.ready() {
                "Permissions granted. Quit and reopen Capstan to activate."
            } else { "Grant the missing permissions in System Settings, then quit and reopen Capstan." }))
            .child(div().flex().gap_2()
                .child(Button::new("input-permission").label(if permissions.input_monitoring { "Input Monitoring granted" } else { "Input Monitoring" })
                    .outline().icon(IconName::ExternalLink).disabled(permissions.input_monitoring)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.message = mac::open_privacy_pane(mac::PrivacyPane::InputMonitoring).err().map(Feedback::Error);
                        cx.notify();
                    })))
                .child(Button::new("accessibility-permission").label(if permissions.accessibility { "Accessibility granted" } else { "Accessibility" })
                    .outline().icon(IconName::ExternalLink).disabled(permissions.accessibility)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.message = mac::open_privacy_pane(mac::PrivacyPane::Accessibility).err().map(Feedback::Error);
                        cx.notify();
                    }))))
    }
}

fn startup_config(loaded: Result<config::Config, String>) -> (config::Config, Option<String>) {
    match loaded {
        Ok(config) => (config, None),
        // A broken file must not silently activate remapping or be overwritten.
        Err(error) => (
            config::Config {
                remapping_enabled: false,
                ..config::Config::default()
            },
            Some(error),
        ),
    }
}

pub fn run(input: Arc<Context>) {
    let config_path = config::path();
    let loaded = config_path
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|path| config::load_or_create(path, mac::remapping_enabled()));
    let (config, message) = startup_config(loaded);
    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    application.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        mac::install_reopen_handler();
        Theme::sync_system_appearance(None, cx);
        let needs_restart = !mac::permissions().ready();
        let input_error = if needs_restart {
            None
        } else {
            start_input(&input).err()
        };
        if !needs_restart && input_error.is_none() {
            input.apply_config(&config);
        }
        if let Some(error) = &input_error {
            eprintln!("caps-tap: {error}");
        }
        let running = !needs_restart && input_error.is_none();
        let show_menu_bar_icon = config.show_menu_bar_icon;
        let bounds = Bounds::centered(None, size(px(440.), px(430.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(420.), px(340.))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Capstan".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                // Hide rather than close: the menu bar reopens the window while the
                // HID listener keeps running.
                window.on_window_should_close(cx, |_, _| {
                    mac::hide_settings();
                    false
                });
                cx.new(|cx| {
                    let mut settings = Settings {
                        #[cfg(test)]
                        system_facts: None,
                        input,
                        input_error,
                        needs_restart,
                        message: message.map(Feedback::Error),
                        config,
                        config_path,
                        timeout_input: None,
                        timeout_subscription: None,
                    };
                    settings.setup_timeout(window, cx);
                    settings
                })
            },
        )
        .expect("failed to open settings window");
        mac::install_status_menu(running);
        mac::set_menu_bar_visible(show_menu_bar_icon);
        cx.activate(true);
    });
}

#[cfg(test)]
pub fn check_ui() {
    use gpui_kit::component::ThemeMode;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Focusable, HeadlessAppContext, ScrollDelta, point};
    use std::fs;

    let dir = std::env::temp_dir().join(format!("caps-tap-ui-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::create_dir_all("dist/qa").unwrap();
    let path = dir.join("settings.json");
    let active = config::Config {
        remapping_enabled: true,
        escape_timeout_ms: 400,
        ..config::Config::default()
    };
    config::save(&path, &active).unwrap();
    let input = Arc::new(Context::default());
    input.apply_config(&active); // No native connection or event tap in this fixture.
    let mut app = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    app.update(gpui_kit::init);
    let (handle, view) = app
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(440.), px(430.)),
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut settings = Settings {
                            system_facts: Some(SystemFacts {
                                permissions: mac::Permissions {
                                    input_monitoring: true,
                                    accessibility: true,
                                },
                                bundled: true,
                                login: false,
                            }),
                            input: input.clone(),
                            input_error: None,
                            needs_restart: false,
                            message: None,
                            timeout_input: None,
                            timeout_subscription: None,
                            config: active.clone(),
                            config_path: Ok(path.clone()),
                        };
                        settings.setup_timeout(window, cx);
                        settings
                    })
                },
            )
        })
        .unwrap();

    app.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("remapping").checked(), Some(true));
        assert!(window.find("apply-timeout").bounds().size.width > px(0.));
        let viewport = window.find("settings-scroll").bounds();
        let reload = window.find("reload-settings").bounds();
        assert!(reload.origin.y >= viewport.origin.y);
        assert!(
            reload.bottom() <= viewport.bottom(),
            "Reload must fit the normal compact window"
        );
        assert_eq!(
            window.find("escape-timeout").label(),
            Some("Escape window in milliseconds")
        );
        window.click("timeout-more", cx);
        assert_eq!(
            view.read(cx)
                .timeout_input
                .as_ref()
                .unwrap()
                .read(cx)
                .value(),
            "450"
        );
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 400);
        window.click("timeout-less", cx);
        window.click("show-menu-bar", cx);
        assert!(!config::load(&path).unwrap().show_menu_bar_icon);
        assert!(input.enabled.load(Ordering::Acquire));
        window.click("show-menu-bar", cx);
        assert!(config::load(&path).unwrap().show_menu_bar_icon);
        window.click("remapping", cx);
        assert!(!input.enabled.load(Ordering::Acquire));
        assert!(!config::load(&path).unwrap().remapping_enabled);
        window.click("remapping", cx);
        assert!(input.enabled.load(Ordering::Acquire));
        window.click("escape-timeout", cx);
        window.press("cmd-a", cx);
        window.input("550", cx);
    })
    .unwrap();
    app.update_window(handle, |_, window, cx| {
        let editor = view.read(cx).timeout_input.clone().unwrap();
        assert_eq!(editor.read(cx).value(), "550");
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 400);
        assert_eq!(config::load(&path).unwrap().escape_timeout_ms, 400);
        window.click("apply-timeout", cx);
        assert_eq!(config::load(&path).unwrap().escape_timeout_ms, 550);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 550);
        editor.update(cx, |editor, cx| editor.set_value("650", window, cx));
        window.focus(&editor.focus_handle(cx), cx);
        window.render_frame(cx);
        assert_eq!(window.find("escape-timeout").focused(), Some(true));
        window.press("tab", cx);
        assert_eq!(window.find("timeout-more").focused(), Some(true));
        window.press("shift-tab", cx);
        assert_eq!(window.find("escape-timeout").focused(), Some(true));
        window.press("enter", cx);
    })
    .unwrap();
    // Event subscriptions are deferred until the window update completes.
    app.update_window(handle, |_, window, cx| {
        let editor = view.read(cx).timeout_input.clone().unwrap();
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 650);
        editor.update(cx, |editor, cx| editor.set_value("20", window, cx));
        window.render_frame(cx);
        window.click("apply-timeout", cx);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 650);
        assert!(matches!(view.read(cx).message, Some(Feedback::Error(_))));
        fs::write(&path, "invalid JSON").unwrap();
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(-600.))),
            cx,
        );
        window.click("reload-settings", cx);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 650);
        let reloaded = config::Config {
            escape_timeout_ms: 400,
            ..active.clone()
        };
        config::save(&path, &reloaded).unwrap();
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(-600.))),
            cx,
        );
        window.click("reload-settings", cx);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 400);
        assert_eq!(editor.read(cx).value(), "400");
        assert!(matches!(view.read(cx).message, Some(Feedback::Success(_))));
        view.update(cx, |view, cx| {
            view.message = None;
            cx.notify();
        });
        window.render_frame(cx);
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(600.))),
            cx,
        );
    })
    .unwrap();
    app.capture_screenshot(handle)
        .expect("Metal rendering unavailable")
        .save("dist/qa/settings-light.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        window.render_frame(cx);
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-dark.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Light, Some(window), cx);
        view.update(cx, |view, cx| {
            view.input = Arc::new(Context::default());
            view.system_facts.as_mut().unwrap().permissions = mac::Permissions {
                input_monitoring: false,
                accessibility: false,
            };
            view.needs_restart = true;
            cx.notify();
        });
        window.render_frame(cx);
        // Kit does not report the switch's disabled property in this snapshot;
        // the real click below verifies that its controlled value cannot change.
        let permission = window.find("input-permission").bounds();
        let viewport = window.find("settings-scroll").bounds();
        assert!(permission.origin.y < viewport.bottom() && permission.bottom() > viewport.origin.y);
        window.click("remapping", cx);
        assert!(!view.read(cx).input.enabled.load(Ordering::Acquire));
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-permissions.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.system_facts.as_mut().unwrap().permissions = mac::Permissions {
                input_monitoring: true,
                accessibility: true,
            };
            view.needs_restart = false;
            view.input_error = Some("Could not create event tap: test fixture".into());
            cx.notify();
        });
        config::save(
            &path,
            &config::Config {
                escape_timeout_ms: 500,
                ..active
            },
        )
        .unwrap();
        window.render_frame(cx);
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(-600.))),
            cx,
        );
        window.click("reload-settings", cx);
        assert_eq!(view.read(cx).config.escape_timeout_ms, 500);
        assert!(!view.read(cx).input.enabled.load(Ordering::Acquire));
        assert_eq!(
            view.read(cx)
                .input
                .escape_timeout_ms
                .load(Ordering::Acquire),
            0
        );
        assert!(window.find("input-error").bounds().size.height > px(0.));
        assert!(window.find("settings-message").bounds().size.height > px(0.));
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(600.))),
            cx,
        );
        let viewport = window.find("settings-scroll").bounds();
        for id in ["input-error", "settings-message"] {
            let alert = window.find(id).bounds();
            assert!(alert.origin.y < viewport.bottom() && alert.bottom() > viewport.origin.y);
        }
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-input-error.png")
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    println!(
        "Settings UI: real switch/apply/reload interactions and light/dark/permission Metal renders passed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn failed_save_and_reload_preserve_active_settings_and_startup_is_safe() {
        let dir =
            std::env::temp_dir().join(format!("caps-tap-settings-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let active = config::Config {
            remapping_enabled: true,
            escape_timeout_ms: 500,
            ..config::Config::default()
        };
        config::save(&path, &active).unwrap();
        let input = Arc::new(Context::default());
        input.apply_config(&active);
        let mut settings = Settings {
            system_facts: None,
            input: input.clone(),
            input_error: None,
            needs_restart: false,
            message: None,
            timeout_input: None,
            timeout_subscription: None,
            config: active.clone(),
            config_path: Ok(path.clone()),
        };
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temp, "another write").unwrap();
        let updated = config::Config {
            remapping_enabled: false,
            escape_timeout_ms: 300,
            ..config::Config::default()
        };
        assert!(settings.save(updated.clone()).is_err());
        assert_eq!(settings.config, active);
        assert_eq!(config::load(&path).unwrap(), active);
        assert!(input.enabled.load(Ordering::Acquire));
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 500);
        fs::remove_file(temp).unwrap();
        fs::write(&path, "invalid JSON").unwrap();
        assert!(settings.reload().is_err());
        assert_eq!(settings.config, active);
        assert!(input.enabled.load(Ordering::Acquire));
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 500);
        let (startup, error) = startup_config(config::load_or_create(&path, true));
        assert!(!startup.remapping_enabled);
        assert!(error.is_some());
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid JSON");
        config::save(&path, &updated).unwrap();
        settings.reload().unwrap();
        assert_eq!(settings.config, updated);
        assert!(!input.enabled.load(Ordering::Acquire));
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 300);
        fs::remove_dir_all(dir).unwrap();
    }
}
