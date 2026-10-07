use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui::{
    App, Application, Bounds, Context as ViewContext, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

use crate::{Context, mac, start_input};

struct Settings {
    input: Arc<Context>,
    input_error: Option<String>,
    needs_restart: bool,
    message: Option<String>,
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
                                    let enabled = !this.input.enabled.load(Ordering::Acquire);
                                    this.input.set_enabled(enabled);
                                    mac::save_remapping_enabled(enabled);
                                    cx.notify();
                                }
                            }))
                            .child("Enable remapping")
                            .child(if remapping { "On" } else { "Off" }),
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
    Application::new().run(move |cx: &mut App| {
        let needs_restart = !mac::permissions().ready();
        let input_error = if needs_restart {
            None
        } else {
            start_input(&input).err()
        };
        if !needs_restart && input_error.is_none() {
            input.set_enabled(mac::remapping_enabled());
        }
        if let Some(error) = &input_error {
            eprintln!("caps-tap: {error}");
        }
        let running = !needs_restart && input_error.is_none();
        let bounds = Bounds::centered(None, size(px(500.), px(470.)), cx);
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
                    message: None,
                })
            },
        )
        .expect("failed to open settings window");
        mac::install_status_menu(running);
        cx.activate(true);
    });
}
