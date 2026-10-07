use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui::{
    App, Application, Bounds, Context as ViewContext, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

use crate::{Context, config, mac, start_input};

struct Settings {
    input: Arc<Context>,
    input_error: Option<String>,
    needs_restart: bool,
    message: Option<String>,
    config: config::Config,
    config_path: Result<std::path::PathBuf, String>,
}

impl Settings {
    fn apply(&mut self, config: config::Config) {
        if !self.needs_restart && self.input_error.is_none() {
            self.input.apply_config(&config);
        }
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
        let remapping = self.input.enabled.load(Ordering::Acquire);
        let login = mac::login_enabled();
        let permissions = mac::permissions();
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_6()
            .gap_4()
            .bg(rgb(0x1b2230))
            .text_color(rgb(0xe5edf7))
            .child(div().text_xl().child("Caps Tap"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x91a4ba))
                    .child("Caps → Control · tap alone → Escape"),
            )
            .when(
                permissions.ready() && !self.needs_restart && self.input_error.is_none(),
                |view| {
                    view.child(
                        div()
                            .id("remapping")
                            .flex()
                            .justify_between()
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(0x2a3444))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.input_error.is_none() && !this.needs_restart {
                                    let config = config::Config {
                                        remapping_enabled: !this.config.remapping_enabled,
                                        ..this.config.clone()
                                    };
                                    this.message = this.save(config).err();
                                    cx.notify();
                                }
                            }))
                            .child("Enable remapping")
                            .child(if remapping { "On" } else { "Off" }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(0x2a3444))
                            .child(format!(
                                "Escape window: {} ms",
                                self.config.escape_timeout_ms
                            ))
                            .child(
                                div()
                                    .id("timeout-less")
                                    .px_3()
                                    .cursor_pointer()
                                    .child("−")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let config = config::Config {
                                            escape_timeout_ms: this
                                                .config
                                                .escape_timeout_ms
                                                .saturating_sub(50)
                                                .max(config::MIN_TIMEOUT_MS),
                                            ..this.config.clone()
                                        };
                                        this.message = this.save(config).err();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("timeout-more")
                                    .px_3()
                                    .cursor_pointer()
                                    .child("+")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let config = config::Config {
                                            escape_timeout_ms: (this.config.escape_timeout_ms + 50)
                                                .min(config::MAX_TIMEOUT_MS),
                                            ..this.config.clone()
                                        };
                                        this.message = this.save(config).err();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("login")
                            .flex()
                            .justify_between()
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(0x2a3444))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.message = mac::set_login_enabled(!mac::login_enabled()).err();
                                cx.notify();
                            }))
                            .child("Launch at login")
                            .child(if login { "On" } else { "Off" }),
                    )
                },
            )
            .when(!permissions.ready() || self.needs_restart, |view| {
                view.child(div().text_sm().text_color(rgb(0x91a4ba)).child(
                    if permissions.ready() {
                        "Permissions granted · quit and reopen to activate"
                    } else {
                        "Grant the missing permissions, then quit and reopen"
                    },
                ))
                .child(
                    div()
                        .id("input-permission")
                        .flex()
                        .justify_between()
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(0x2a3444))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.message =
                                mac::open_privacy_pane(mac::PrivacyPane::InputMonitoring).err();
                            cx.notify();
                        }))
                        .child("Input Monitoring")
                        .child(if permissions.input_monitoring {
                            "Granted"
                        } else {
                            "Open Settings →"
                        }),
                )
                .child(
                    div()
                        .id("accessibility-permission")
                        .flex()
                        .justify_between()
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(0x2a3444))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.message =
                                mac::open_privacy_pane(mac::PrivacyPane::Accessibility).err();
                            cx.notify();
                        }))
                        .child("Accessibility")
                        .child(if permissions.accessibility {
                            "Granted"
                        } else {
                            "Open Settings →"
                        }),
                )
            })
            .child(
                div()
                    .id("reload-settings")
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(0x2a3444))
                    .cursor_pointer()
                    .child("Reload Settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.message = Some(match this.reload() {
                            Ok(()) => "Settings reloaded".into(),
                            Err(error) => format!("{error}. Current settings unchanged."),
                        });
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x91a4ba))
                    .child("JSON: ~/Library/Application Support/Caps Tap/settings.json"),
            )
            .when(
                self.input_error.is_some() || self.message.is_some(),
                |view| {
                    view.child(
                        div().text_sm().text_color(rgb(0xf2ae87)).child(
                            self.input_error
                                .clone()
                                .or_else(|| self.message.clone())
                                .unwrap_or_default(),
                        ),
                    )
                },
            )
    }
}

pub fn run(input: Arc<Context>) {
    let config_path = config::path();
    let loaded = config_path
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|path| config::load_or_create(path, mac::remapping_enabled()));
    let message = loaded.as_ref().err().cloned();
    // A broken file must not silently activate remapping or be overwritten.
    let config = loaded.unwrap_or(config::Config {
        remapping_enabled: false,
        ..config::Config::default()
    });
    Application::new().run(move |cx: &mut App| {
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
        let bounds = Bounds::centered(None, size(px(600.), px(650.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                // Hide rather than close: the menu bar reopens the window while the
                // HID listener keeps running.
                window.on_window_should_close(cx, |_, _| {
                    mac::hide_settings();
                    false
                });
                cx.new(|_| Settings {
                    input,
                    input_error,
                    needs_restart,
                    message,
                    config,
                    config_path,
                })
            },
        )
        .expect("failed to open settings window");
        mac::install_status_menu(running);
        cx.activate(true);
    });
}
