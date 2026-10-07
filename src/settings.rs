use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui_kit::{
    App, Bounds, Context as ViewContext, Entity, FontWeight, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, IndexPath, Sizable, Theme, TitleBar,
    alert::Alert,
    button::Button,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsPage {
    General,
    Customize,
    Advanced,
    Permissions,
}

impl SettingsPage {
    fn initial(needs_attention: bool) -> Self {
        if needs_attention {
            Self::Permissions
        } else {
            Self::General
        }
    }
}

struct Settings {
    page: SettingsPage,
    #[cfg(test)]
    system_facts: Option<SystemFacts>,
    input: Arc<Context>,
    input_error: Option<String>,
    needs_restart: bool,
    message: Option<Feedback>,
    message_page: SettingsPage,
    feedback_generation: u64,
    timeout_input: Option<Entity<InputState>>,
    timeout_subscription: Option<Subscription>,
    tap_select: Option<Entity<SelectState<SearchableVec<&'static str>>>>,
    hold_select: Option<Entity<SelectState<SearchableVec<&'static str>>>>,
    mapping_subscriptions: Vec<Subscription>,
    viewer_window: Option<gpui_kit::AnyWindowHandle>,
    config: config::Config,
    config_path: Result<std::path::PathBuf, String>,
}

impl Settings {
    fn set_feedback(&mut self, feedback: Option<Feedback>, cx: &mut ViewContext<Self>) {
        self.feedback_generation = self.feedback_generation.wrapping_add(1);
        self.message_page = self.page;
        let transient = matches!(feedback, Some(Feedback::Success(_)));
        self.message = feedback;
        if transient {
            let generation = self.feedback_generation;
            // Only a weak entity survives the delay. Superseded timers cannot
            // dismiss a newer success or an important failure.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(3))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.expire_success(generation);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn expire_success(&mut self, generation: u64) {
        if self.feedback_generation == generation
            && matches!(self.message, Some(Feedback::Success(_)))
        {
            self.message = None;
        }
    }

    fn select_page(&mut self, page: SettingsPage) {
        if self.page != page {
            if matches!(self.message, Some(Feedback::Success(_))) {
                self.feedback_generation = self.feedback_generation.wrapping_add(1);
                self.message = None;
            }
            self.page = page;
        }
    }

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
        self.setup_mapping(window, cx);
    }

    fn setup_mapping(&mut self, window: &mut Window, cx: &mut ViewContext<Self>) {
        for tap in [true, false] {
            let labels: Vec<_> = if tap {
                config::TapKey::ALL.iter().map(|v| v.label()).collect()
            } else {
                config::HoldModifier::ALL
                    .iter()
                    .map(|v| v.label())
                    .collect()
            };
            let selected = if tap {
                self.config.tap_key.label()
            } else {
                self.config.hold_modifier.label()
            };
            let index = labels.iter().position(|v| *v == selected).unwrap();
            let state = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(labels),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
            });
            self.mapping_subscriptions.push(cx.subscribe_in(
                &state,
                window,
                move |this, source, event, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event {
                        if source.read(cx).selected_value() != Some(label) {
                            return; // Ignore a deferred confirmation superseded by reload.
                        }
                        let mut config = this.config.clone();
                        if tap {
                            if let Some(value) = config::TapKey::ALL
                                .into_iter()
                                .find(|v| v.label() == *label)
                            {
                                config.tap_key = value;
                            }
                        } else if let Some(value) = config::HoldModifier::ALL
                            .into_iter()
                            .find(|v| v.label() == *label)
                        {
                            config.hold_modifier = value;
                        }
                        let feedback = this.save(config).err().map(Feedback::Error);
                        this.set_feedback(feedback, cx);
                        this.sync_mapping(window, cx);
                        cx.notify();
                    }
                },
            ));
            if tap {
                self.tap_select = Some(state);
            } else {
                self.hold_select = Some(state);
            }
        }
    }

    fn sync_mapping(&self, window: &mut Window, cx: &mut ViewContext<Self>) {
        for (state, label) in [
            (&self.tap_select, self.config.tap_key.label()),
            (&self.hold_select, self.config.hold_modifier.label()),
        ] {
            if let Some(state) = state {
                state.update(cx, |state, cx| state.set_selected_value(&label, window, cx));
            }
        }
    }

    fn sync_timeout(&self, window: &mut Window, cx: &mut ViewContext<Self>) {
        self.sync_mapping(window, cx);
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
        let feedback = Some(match result {
            Ok(()) => {
                self.sync_timeout(window, cx);
                Feedback::Success("Escape window saved".into())
            }
            Err(error) => Feedback::Error(error),
        });
        self.set_feedback(feedback, cx);
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
        let muted = theme.muted_foreground.opacity(0.85);
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

        div().size_full().flex().flex_col()
            .bg(theme.background).text_color(theme.foreground).text_size(px(13.)).font_family(theme.font_family.clone())
            .child(TitleBar::new().bg(theme.background).border_color(border).pr_4()
                .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child("Capstan"))
                .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(status_color)
                    .child(div().size(px(6.)).rounded_full().bg(status_color)).child(status)))
            .child(div().flex().flex_1().min_h_0()
            .child(Sidebar::new("settings-sidebar").w(px(156.)).collapsible(false)
                .child(SidebarMenu::new().children([
                    (SettingsPage::General, "General", IconName::Settings),
                    (SettingsPage::Customize, "Customize", IconName::Settings2),
                    (SettingsPage::Advanced, "Advanced", IconName::SquareTerminal),
                    (SettingsPage::Permissions, "Permissions", IconName::CircleAlert),
                ].into_iter().filter(|(page, _, _)| *page != SettingsPage::Permissions || !permissions.ready() || self.needs_restart)
                .map(|(page, label, icon)| SidebarMenuItem::new(label).icon(icon).active(self.page == page)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.page != page { window.blur(cx); this.select_page(page); cx.notify(); }
                    }))))))
            .child(div().id("settings-scroll").test_support().flex_1().min_w_0().min_h_0().overflow_y_scroll()
            .child(div().flex().flex_col().gap_4().p_4()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(match self.page {
                    SettingsPage::General => "General", SettingsPage::Customize => "Customize remapping",
                    SettingsPage::Advanced => "Advanced", SettingsPage::Permissions => "Permissions",
                }))
                .when(self.page == SettingsPage::Permissions, |view| view.child(self.permission_panel(permissions, cx)))
                .when_some(self.input_error.clone().filter(|_| self.page == SettingsPage::General), |view, error| view.child(div().id("input-error").test_support().child(Alert::error("input-alert", error))))
                .when_some(self.message.clone().filter(|_| self.message_page == self.page), |view, feedback| view.child(div().id("settings-message").test_support().child(match feedback {
                    Feedback::Success(text) => Alert::success("settings-alert", text),
                    Feedback::Error(text) => Alert::error("settings-alert", text),
                })))
                .when(self.page == SettingsPage::General, |view| view.child(div().id("remap-card").test_support().flex().flex_col().rounded_lg().border_1().border_color(border)
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Remap Caps Lock", "Native Caps Lock is suppressed while enabled.", cx))
                        .child(Switch::new("remapping").small().accessibility_label("Enable remapping").checked(remapping).disabled(!usable)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                let feedback = this.save(config::Config { remapping_enabled: *enabled, ..this.config.clone() }).err().map(Feedback::Error);
                                this.set_feedback(feedback, cx);
                                cx.notify();
                            }))))
                    .child(div().px_3().pb_2().text_size(px(11.)).text_color(muted).child(format!("Tap {} · Hold {} · {} ms", self.config.tap_key.label(), self.config.hold_modifier.label(), self.config.escape_timeout_ms)))
                    ))
                .when(self.page == SettingsPage::Customize, |view| view.child(div().flex().flex_col().rounded_lg().border_1().border_color(border)
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("On tap", "Released alone before the timeout.", cx))
                        .when_some(self.tap_select.clone(), |view, state| view.child(div().w(px(145.)).h(px(28.)).child(Select::new(&state).id("tap-key").small().text_size(px(12.)).accessibility_label("On tap key").disabled(!usable)))))
                    .child(div().h_px().bg(border))
                    .child(div().flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("While held", "Starts immediately. Chords cancel the tap.", cx))
                        .when_some(self.hold_select.clone(), |view, state| view.child(div().w(px(145.)).h(px(28.)).child(Select::new(&state).id("hold-modifier").small().text_size(px(12.)).accessibility_label("While held modifier").disabled(!usable)))))
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
                                .small().text_size(px(12.)).id("escape-timeout").aria_label("Escape window in milliseconds").suffix("ms").w(px(100.)).disabled(!usable)))
                            .child(Button::new("timeout-more").small().icon(IconName::Plus).accessibility_label("Increase Escape window by 50 ms").tooltip("Increase by 50 ms").disabled(!usable)
                                .on_click(cx.listener(|this, _, window, cx| this.step_timeout(true, window, cx))))
                            .child(Button::new("apply-timeout").small().accessibility_label("Apply").child(div().text_size(px(12.)).child("Apply")).outline().disabled(!usable)
                                .on_click(cx.listener(|this, _, window, cx| this.save_timeout(window, cx)))))
                        .child(div().text_size(px(11.)).text_color(muted).child(if usable {
                            "Release Caps before this time to send the tap key. Holding longer cancels it."
                        } else { "Input is unavailable. Check General for startup errors or Permissions for access." })))))
                .when(self.page == SettingsPage::General, |view| view.child(div().id("login-card").test_support().rounded_lg().border_1().border_color(border).flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Launch at login", if bundled { "Start automatically when you sign in." } else { "Open the bundled app to enable this." }, cx))
                        .child(Switch::new("login").small().accessibility_label("Launch at login").checked(login).disabled(!bundled)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                let feedback = mac::set_login_enabled(*enabled).err().map(Feedback::Error);
                                this.set_feedback(feedback, cx);
                                cx.notify();
                            }))))
                    )
                .when(self.page == SettingsPage::Advanced, |view| view.child(div().flex().flex_col().gap_4()
                    .child(div().rounded_lg().border_1().border_color(border).flex().justify_between().items_center().gap_3().p_3()
                        .child(setting_label("Show menu bar icon", "Open Capstan.app to return when hidden.", cx))
                        .child(Switch::new("show-menu-bar").small().accessibility_label("Show menu bar icon").checked(self.config.show_menu_bar_icon)
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                let feedback = this.save(config::Config { show_menu_bar_icon: *enabled, ..this.config.clone() }).err().map(Feedback::Error);
                                this.set_feedback(feedback, cx);
                                cx.notify();
                            }))))
                .child(div().flex().flex_col().gap_2()
                    .child(Button::new("open-viewer").small().accessibility_label("Event Viewer").child(div().text_size(px(12.)).child("Event Viewer")).outline()
                        .on_click(cx.listener(|this, _, _, cx| super::viewer::open(this.input.clone(), !this.needs_restart && this.input_error.is_none(), &mut this.viewer_window, cx))))
                    .child(Button::new("reload-settings").small().accessibility_label("Reload Settings").child(div().text_size(px(12.)).child("Reload Settings")).icon(IconName::RefreshCw).outline()
                        .tooltip("~/Library/Application Support/Capstan/settings.json")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let feedback = Some(match this.reload() {
                                Ok(()) => { this.sync_timeout(window, cx); Feedback::Success("Settings reloaded".into()) },
                                Err(error) => Feedback::Error(format!("{error}. Current settings unchanged.")),
                            });
                            this.set_feedback(feedback, cx);
                            cx.notify();
                        })))))))))
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
                .text_color(cx.theme().muted_foreground.opacity(0.85))
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
                .child(Button::new("input-permission").small().accessibility_label(if permissions.input_monitoring { "Input Monitoring granted" } else { "Input Monitoring" }).child(div().text_size(px(12.)).child(if permissions.input_monitoring { "Input Monitoring granted" } else { "Input Monitoring" }))
                    .outline().icon(IconName::ExternalLink).disabled(permissions.input_monitoring)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let feedback = mac::open_privacy_pane(mac::PrivacyPane::InputMonitoring).err().map(Feedback::Error);
                        this.set_feedback(feedback, cx);
                        cx.notify();
                    })))
                .child(Button::new("accessibility-permission").small().accessibility_label(if permissions.accessibility { "Accessibility granted" } else { "Accessibility" }).child(div().text_size(px(12.)).child(if permissions.accessibility { "Accessibility granted" } else { "Accessibility" }))
                    .outline().icon(IconName::ExternalLink).disabled(permissions.accessibility)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let feedback = mac::open_privacy_pane(mac::PrivacyPane::Accessibility).err().map(Feedback::Error);
                        this.set_feedback(feedback, cx);
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
        let menu_input = input.clone();
        let show_menu_bar_icon = config.show_menu_bar_icon;
        let bounds = Bounds::centered(None, size(px(640.), px(460.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(600.), px(340.))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Capstan".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
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
                        page: SettingsPage::initial(needs_restart),
                        #[cfg(test)]
                        system_facts: None,
                        input,
                        input_error,
                        needs_restart,
                        message: message.map(Feedback::Error),
                        message_page: SettingsPage::General,
                        feedback_generation: 0,
                        config,
                        config_path,
                        timeout_input: None,
                        timeout_subscription: None,
                        tap_select: None,
                        hold_select: None,
                        mapping_subscriptions: vec![],
                        viewer_window: None,
                    };
                    settings.setup_timeout(window, cx);
                    settings
                })
            },
        )
        .expect("failed to open settings window");
        mac::install_status_menu(move || mac::MenuStatus {
            input_available: running,
            enabled: menu_input.enabled.load(Ordering::Acquire),
            timeout_ms: menu_input.escape_timeout_ms.load(Ordering::Acquire),
            login: mac::login_enabled(),
        });
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
    app.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (handle, view) = app
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(640.), px(460.)),
                    })),
                    focus: false,
                    show: false,
                    ..TitleBar::window_options()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut settings = Settings {
                            page: SettingsPage::initial(false),
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
                            message_page: SettingsPage::General,
                            feedback_generation: 0,
                            timeout_input: None,
                            timeout_subscription: None,
                            tap_select: None,
                            hold_select: None,
                            mapping_subscriptions: vec![],
                            viewer_window: None,
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
        for id in [
            "apply-timeout",
            "tap-key",
            "hold-modifier",
            "reload-settings",
            "open-viewer",
            "show-menu-bar",
        ] {
            assert!(
                window.try_find(id).is_none(),
                "{id} must be unmounted on General"
            );
        }
        assert!(window.find("login").visible());
        assert_eq!(view.read(cx).page, SettingsPage::General);
        let remap = window.find("remap-card").bounds();
        let login = window.find("login-card").bounds();
        assert!(
            login.origin.y > remap.bottom(),
            "General has separate cards"
        );
        assert!(login.bottom() < window.find("settings-scroll").bounds().bottom());
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
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
        window.render_frame(cx);
        window.click("0-1", cx);
        assert_eq!(view.read(cx).page, SettingsPage::Customize);
        assert!(window.try_find("login").is_none());
        assert!(window.find("apply-timeout").bounds().size.width > px(0.));
        let viewport = window.find("settings-scroll").bounds();
        let apply = window.find("apply-timeout").bounds();
        assert!(apply.bottom() < viewport.bottom(), "Customize fits");
        assert_eq!(
            window.find("escape-timeout").label(),
            Some("Escape window in milliseconds")
        );
        window.within("tap-key").click("input", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    app.update_window(handle, |_, window, cx| {
        assert_eq!(config::load(&path).unwrap().tap_key, config::TapKey::Tab);
        window.within("hold-modifier").click("input", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    app.update_window(handle, |_, window, cx| {
        assert_eq!(
            config::load(&path).unwrap().hold_modifier,
            config::HoldModifier::Shift
        );
        fs::write(
            path.with_extension(format!("json.{}.tmp", std::process::id())),
            "other writer",
        )
        .unwrap();
        window.within("tap-key").click("input", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    app.update_window(handle, |_, window, cx| {
        assert!(matches!(view.read(cx).message, Some(Feedback::Error(_))));
        assert_eq!(window.find("tap-key").value(), Some("Tab"));
        assert_eq!(
            input.state.lock().unwrap().config.tap_key,
            config::TapKey::Tab
        );
        assert_eq!(config::load(&path).unwrap().tap_key, config::TapKey::Tab);
        fs::remove_file(path.with_extension(format!("json.{}.tmp", std::process::id()))).unwrap();
        view.update(cx, |view, cx| {
            view.message = None;
            cx.notify();
        });
        window.render_frame(cx);
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
        window.click("escape-timeout", cx);
        assert_eq!(window.find("escape-timeout").focused(), Some(true));
        window.click("0-2", cx);
        assert_eq!(view.read(cx).page, SettingsPage::Advanced);
        assert!(window.try_find("escape-timeout").is_none());
        assert!(
            !view
                .read(cx)
                .timeout_input
                .as_ref()
                .unwrap()
                .focus_handle(cx)
                .is_focused(window)
        );
        window.input("999", cx);
        window.press("enter", cx);
        assert_eq!(
            view.read(cx)
                .timeout_input
                .as_ref()
                .unwrap()
                .read(cx)
                .value(),
            "400"
        );
        assert_eq!(config::load(&path).unwrap().escape_timeout_ms, 400);
        window.click("show-menu-bar", cx);
        assert!(!config::load(&path).unwrap().show_menu_bar_icon);
        assert!(input.enabled.load(Ordering::Acquire));
        window.click("show-menu-bar", cx);
        assert!(config::load(&path).unwrap().show_menu_bar_icon);
        window.click("0-0", cx);
        window.click("remapping", cx);
        assert!(!input.enabled.load(Ordering::Acquire));
        assert!(!config::load(&path).unwrap().remapping_enabled);
        window.click("remapping", cx);
        assert!(input.enabled.load(Ordering::Acquire));
        window.click("0-1", cx);
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
        let pending_success = view.read(cx).feedback_generation;
        assert!(matches!(view.read(cx).message, Some(Feedback::Success(_))));
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temp, "other writer").unwrap();
        editor.update(cx, |editor, cx| editor.set_value("750", window, cx));
        window.render_frame(cx);
        window.click("apply-timeout", cx);
        assert_eq!(
            editor.read(cx).value(),
            "750",
            "Failed saves retain the draft"
        );
        assert_eq!(config::load(&path).unwrap().escape_timeout_ms, 650);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 650);
        view.update(cx, |view, cx| {
            view.expire_success(pending_success);
            cx.notify();
        });
        assert!(
            matches!(view.read(cx).message, Some(Feedback::Error(_))),
            "The prior save timer cannot hide a failed save"
        );
        fs::remove_file(temp).unwrap();
        editor.update(cx, |editor, cx| editor.set_value("20", window, cx));
        window.render_frame(cx);
        window.click("apply-timeout", cx);
        assert_eq!(input.escape_timeout_ms.load(Ordering::Acquire), 650);
        assert!(matches!(view.read(cx).message, Some(Feedback::Error(_))));
        window.click("0-2", cx);
        assert!(
            window.try_find("settings-message").is_none(),
            "Errors remain on their originating page"
        );
        window.click("0-1", cx);
        assert!(
            window.find("settings-message").visible(),
            "Returning restores the failure"
        );
        fs::write(&path, "invalid JSON").unwrap();
        window.click("0-2", cx);
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
        let first_reload = view.read(cx).feedback_generation;
        window.click("reload-settings", cx);
        view.update(cx, |view, cx| {
            view.expire_success(first_reload);
            cx.notify();
        });
        assert!(
            matches!(view.read(cx).message, Some(Feedback::Success(_))),
            "The earlier reload timer cannot clear the later success"
        );
        window.render_frame(cx);
        assert!(window.find("settings-message").visible());
        window.click("0-1", cx);
        assert_eq!(window.find("tap-key").value(), Some("Escape"));
        assert_eq!(window.find("hold-modifier").value(), Some("Control"));
        assert!(
            view.read(cx).message.is_none(),
            "Navigation dismisses success feedback"
        );
        assert!(window.try_find("settings-message").is_none());
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
        .save("dist/qa/settings-customize-light.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        window.render_frame(cx);
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-customize-dark.png")
        .unwrap();
    for (mode, name) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
        app.update_window(handle, |_, window, cx| {
            Theme::change(mode, Some(window), cx);
            window.render_frame(cx);
            window.click("0-2", cx);
            let viewport = window.find("settings-scroll").bounds();
            assert!(window.find("reload-settings").bounds().bottom() < viewport.bottom());
        })
        .unwrap();
        app.capture_screenshot(handle)
            .unwrap()
            .save(format!("dist/qa/settings-advanced-{name}.png"))
            .unwrap();
    }
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Light, Some(window), cx);
        view.update(cx, |view, cx| {
            view.input = Arc::new(Context::default());
            view.system_facts.as_mut().unwrap().permissions = mac::Permissions {
                input_monitoring: false,
                accessibility: false,
            };
            view.needs_restart = true;
            view.page = SettingsPage::initial(true);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).page, SettingsPage::Permissions);
        assert!(window.try_find("escape-timeout").is_none());
        let editor = view.read(cx).timeout_input.clone().unwrap();
        assert!(!editor.focus_handle(cx).is_focused(window));
        // Kit does not report the switch's disabled property in this snapshot;
        // the real click below verifies that its controlled value cannot change.
        let permission = window.find("input-permission").bounds();
        let viewport = window.find("settings-scroll").bounds();
        assert!(permission.origin.y < viewport.bottom() && permission.bottom() > viewport.origin.y);
        window.click("0-0", cx);
        window.click("remapping", cx);
        assert!(!view.read(cx).input.enabled.load(Ordering::Acquire));
        assert!(window.find("login").visible());
        window.click("0-1", cx);
        window.click("timeout-more", cx);
        assert_eq!(editor.read(cx).value(), "400");
        window.click("apply-timeout", cx);
        assert!(!view.read(cx).input.enabled.load(Ordering::Acquire));
        window.click("0-3", cx);
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-permissions-light.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        window.render_frame(cx);
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-permissions-dark.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        Theme::change(ThemeMode::Light, Some(window), cx);
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
        window.click("0-2", cx);
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
        window.click("0-0", cx);
        assert!(window.try_find("input-permission").is_none());
        assert!(window.find("input-error").bounds().size.height > px(0.));
        assert!(
            window.try_find("settings-message").is_none(),
            "Reload success must not follow navigation to General"
        );
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(600.))),
            cx,
        );
        let viewport = window.find("settings-scroll").bounds();
        let alert = window.find("input-error").bounds();
        assert!(alert.origin.y < viewport.bottom() && alert.bottom() > viewport.origin.y);
    })
    .unwrap();
    app.capture_screenshot(handle)
        .unwrap()
        .save("dist/qa/settings-input-error.png")
        .unwrap();
    app.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.input = input.clone();
            view.input_error = None;
            cx.notify();
        });
        window.scroll(
            "settings-scroll",
            ScrollDelta::Pixels(point(px(0.), px(-600.))),
            cx,
        );
        window.click("0-2", cx);
        window.click("open-viewer", cx);
    })
    .unwrap();
    let mut viewer_handle = app.update(|cx| view.read(cx).viewer_window);
    let viewer = viewer_handle.unwrap();
    app.update(|cx| super::viewer::open(input.clone(), true, &mut viewer_handle, cx));
    assert_eq!(viewer_handle.unwrap(), viewer, "reuse the existing viewer");
    app.update_window(viewer, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("viewer-record").label(), Some("Record"));
        assert!(window.find("viewer-empty").visible());
        window.click("viewer-record", cx);
        assert_eq!(window.find("viewer-record").label(), Some("Stop"));
        assert!(input.state.lock().unwrap().diagnostics.recording);
        {
            let mut state = input.state.lock().unwrap();
            use super::diagnostics::Event;
            use std::time::Duration;
            for (start, duration, chord) in [
                (46610000, 100, false),
                (46611000, 90, true),
                (46612000, 800, false),
            ] {
                if let Some(action) =
                    state.press(Duration::from_millis(start), Duration::from_millis(300))
                {
                    state
                        .diagnostics
                        .record(Duration::from_secs(1118), Event::Generated(action));
                }
                state
                    .diagnostics
                    .record(Duration::from_secs(1118), Event::CapsSuppressed);
                if chord {
                    state.chord();
                    state
                        .diagnostics
                        .record(Duration::from_secs(1118), Event::Chord);
                }
                for action in state.release(Duration::from_millis(start + duration)) {
                    state.diagnostics.record(
                        Duration::from_secs(1118),
                        if matches!(action, super::Action::Tap(_)) {
                            Event::GenerationFailed(action)
                        } else {
                            Event::Generated(action)
                        },
                    );
                }
            }
            state.press(Duration::from_secs(46614), Duration::from_millis(500));
            if let Some(action) = state.cancel() {
                state
                    .diagnostics
                    .record(Duration::from_secs(1118), Event::Cancelled);
                state
                    .diagnostics
                    .record(Duration::from_secs(1118), Event::Generated(action));
            }
        }
        window.render_frame(cx);
        window.click("viewer-record", cx);
        assert!(!input.state.lock().unwrap().diagnostics.recording);
        let state = input.state.lock().unwrap();
        let rows = state.diagnostics.rows.iter().cloned().collect::<Vec<_>>();
        let gestures = super::diagnostics::gestures(&rows);
        assert_eq!(gestures.len(), 4);
        assert_eq!(gestures[0].result, "Tap");
        assert_eq!(gestures[1].result, "Chord");
        assert_eq!(gestures[2].result, "Hold");
        assert_eq!(gestures[3].result, "Cancelled");
        assert!(window.try_find("viewer-raw").is_none());
        assert!(window.try_find("viewer-details-title").is_none());
        assert!(window.try_find("viewer-cutoff-active").is_none());
        assert_eq!(window.find("viewer-record").label(), Some("Record"));
        for (ix, label) in [
            "Cancelled · — · Control",
            "Hold · 800 ms · Control",
            "Hold (chord) · 90 ms · Control",
            "Tap · 100 ms · Escape (failed)",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(window.find(("gesture", ix)).visible());
            assert_eq!(window.find(("gesture", ix)).label(), Some(label));
        }
        drop(state);
    })
    .unwrap();
    app.capture_screenshot(viewer)
        .unwrap()
        .save("dist/qa/event-viewer.png")
        .unwrap();
    app.update_window(viewer, |_, window, cx| {
        Theme::change(ThemeMode::Dark, Some(window), cx);
        window.render_frame(cx);
    })
    .unwrap();
    app.capture_screenshot(viewer)
        .unwrap()
        .save("dist/qa/event-viewer-dark.png")
        .unwrap();
    app.update_window(viewer, |_, window, cx| {
        window.scroll(
            "viewer-events",
            ScrollDelta::Pixels(point(px(0.), px(600.))),
            cx,
        );
        window.click("viewer-clear", cx);
        assert!(!input.state.lock().unwrap().diagnostics.recording);
        assert!(input.state.lock().unwrap().diagnostics.rows.is_empty());
        assert!(
            input.enabled.load(Ordering::Acquire),
            "viewer controls must not disable input"
        );
        window.click("viewer-record", cx);
        window.click("viewer-clear", cx);
        assert!(
            input.state.lock().unwrap().diagnostics.recording,
            "Clear retains recording state"
        );
        {
            let mut state = input.state.lock().unwrap();
            state.press(
                std::time::Duration::from_secs(46620),
                std::time::Duration::from_millis(300),
            );
        }
        window.render_frame(cx);
        assert_eq!(window.find("viewer-record").label(), Some("Stop"));
        assert_eq!(window.find(("gesture", 0_usize)).label(), Some("Holding…"));
    })
    .unwrap();
    app.capture_screenshot(viewer)
        .unwrap()
        .save("dist/qa/event-viewer-recording-dark.png")
        .unwrap();
    app.update_window(viewer, |_, window, cx| {
        Theme::change(ThemeMode::Light, Some(window), cx);
        window.render_frame(cx);
    })
    .unwrap();
    app.capture_screenshot(viewer)
        .unwrap()
        .save("dist/qa/event-viewer-recording.png")
        .unwrap();
    app.update_window(viewer, |_, window, cx| {
        {
            let mut state = input.state.lock().unwrap();
            state.release(std::time::Duration::from_secs(46621));
            for index in 0..120 {
                let start = std::time::Duration::from_secs(46630 + index);
                state.press(start, std::time::Duration::from_millis(300));
                state.release(start + std::time::Duration::from_millis(100));
            }
            assert_eq!(state.diagnostics.rows.len(), super::diagnostics::CAPACITY);
        }
        window.render_frame(cx);
        let last = input.state.lock().unwrap().diagnostics.gestures().len() - 1;
        assert!(window.find(("gesture", 0_usize)).visible());
        let viewport = window.find("viewer-events").bounds();
        assert!(window.find(("gesture", last)).bounds().origin.y > viewport.bottom());
        window.scroll(
            "viewer-events",
            ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            cx,
        );
        let oldest = window.find(("gesture", last)).bounds();
        assert!(
            oldest.origin.y < viewport.bottom() && oldest.bottom() > viewport.origin.y,
            "Oldest retained row is reachable"
        );
        window.remove_window();
    })
    .unwrap();
    app.update(|_| {});
    assert!(!input.state.lock().unwrap().diagnostics.recording);
    assert!(input.state.lock().unwrap().diagnostics.rows.is_empty());
    let blocked = Arc::new(Context::default());
    let mut blocked_viewer = None;
    app.update(|cx| super::viewer::open(blocked.clone(), false, &mut blocked_viewer, cx));
    app.update_window(blocked_viewer.unwrap(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("viewer-cutoff-active").is_none());
        assert!(window.try_find("viewer-cutoff-unavailable").is_none());
        assert!(window.find("viewer-explainer").visible());
        assert_eq!(window.find("viewer-record").label(), Some("Record"));
        window.remove_window();
    })
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
    fn initial_page_prioritizes_permission_attention_not_input_errors() {
        assert_eq!(SettingsPage::initial(false), SettingsPage::General);
        assert_eq!(SettingsPage::initial(true), SettingsPage::Permissions);
    }

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
            page: SettingsPage::General,
            system_facts: None,
            input: input.clone(),
            input_error: None,
            needs_restart: false,
            message: None,
            message_page: SettingsPage::General,
            feedback_generation: 0,
            timeout_input: None,
            timeout_subscription: None,
            tap_select: None,
            hold_select: None,
            mapping_subscriptions: vec![],
            viewer_window: None,
            config: active.clone(),
            config_path: Ok(path.clone()),
        };
        // Exercise timer invalidation without wall-clock sleeps. The real UI
        // harness separately verifies the listeners enqueue transient feedback.
        settings.message = Some(Feedback::Success("First".into()));
        settings.feedback_generation = 1;
        settings.expire_success(1);
        assert!(settings.message.is_none());
        settings.message = Some(Feedback::Success("Newer".into()));
        settings.feedback_generation = 2;
        settings.expire_success(1);
        assert!(matches!(settings.message, Some(Feedback::Success(_))));
        settings.message = Some(Feedback::Error("Keep failure".into()));
        settings.feedback_generation = 3;
        settings.expire_success(2);
        settings.select_page(SettingsPage::Advanced);
        assert!(matches!(settings.message, Some(Feedback::Error(_))));
        assert_eq!(settings.message_page, SettingsPage::General);
        settings.message = Some(Feedback::Success("Reloaded".into()));
        settings.select_page(SettingsPage::Customize);
        assert!(settings.message.is_none());
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
