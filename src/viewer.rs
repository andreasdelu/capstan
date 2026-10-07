use super::Context;
use gpui_kit::component::{
    ActiveTheme, Sizable, TitleBar,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context as ViewContext, TestSupportExt, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use std::sync::Arc;
use std::time::Duration;

pub struct EventViewer {
    input: Arc<Context>,
    input_available: bool,
}

impl Drop for EventViewer {
    fn drop(&mut self) {
        self.input
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .stop();
    }
}

impl EventViewer {
    fn new(input: Arc<Context>, input_available: bool, cx: &mut ViewContext<Self>) -> Self {
        // The task holds only a weak entity between ticks; it cannot keep a closed
        // window alive. Snapshot at render, never perform UI work in input callbacks.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            input,
            input_available,
        }
    }
}

impl Render for EventViewer {
    fn render(&mut self, _: &mut Window, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let (recording, mut gestures, holding) = {
            let state = self.input.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.diagnostics.recording,
                state.diagnostics.gestures(),
                state.pressed_at.is_some(),
            )
        };
        for gesture in &mut gestures {
            gesture.active &= recording && holding;
        }
        let theme = cx.theme();
        div().size_full().flex().flex_col().bg(theme.background).text_color(theme.foreground).font_family(theme.font_family.clone()).text_size(px(13.))
            .child(TitleBar::new().bg(theme.background).border_color(theme.border).child("Capstan Event Viewer"))
            .child(div().flex().flex_col().gap_2().px_4().pt_4().pb_2()
                .child(div().flex().items_center().gap_2()
                    .child(Button::new("viewer-record").small().accessibility_label(if recording { "Stop" } else { "Record" })
                        .child(div().text_size(px(12.)).child(if recording { "Stop" } else { "Record" }))
                        .when(recording, |button| button.danger()).when(!recording, |button| button.primary())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let mut state = this.input.state.lock().unwrap_or_else(|e| e.into_inner());
                            let recording = !state.diagnostics.recording;
                            state.diagnostics.set_recording(recording);
                            cx.notify();
                        })))
                    .child(Button::new("viewer-clear").small().accessibility_label("Clear").child(div().text_size(px(12.)).child("Clear")).outline()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.input.state.lock().unwrap_or_else(|e| e.into_inner()).diagnostics.clear();
                            cx.notify();
                        }))))
                .child(div().id("viewer-explainer").test_support().text_size(px(11.)).text_color(theme.muted_foreground.opacity(0.85)).child(if self.input_available {
                    "Caps timing and generated keys, not OS delivery. Closing clears capture."
                } else { "Input unavailable. Caps timing and generated keys, not OS delivery. Closing clears capture." })))
            .child(div().id("viewer-events").test_support().flex_1().min_h_0().overflow_y_scroll().px_4().pb_4()
                .child(div().flex().flex_col().gap_2()
                    .when(gestures.is_empty(), |view| view.child(div().id("viewer-empty").test_support().text_size(px(12.)).text_color(theme.muted_foreground.opacity(0.85)).child(if recording { "Try Caps Lock." } else { "Press Record, then try Caps Lock." })))
                    .children(gestures.iter().rev().enumerate().map(|(ix, gesture)| div().id(("gesture", ix)).aria_label(gesture.label()).test_support().flex_none().text_size(px(12.)).py_2().border_b_1().border_color(theme.border).child(gesture.label())))))
    }
}

pub fn open(
    input: Arc<Context>,
    input_available: bool,
    existing: &mut Option<AnyWindowHandle>,
    cx: &mut App,
) {
    if let Some(handle) = existing
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let close_input = input.clone();
    let bounds = Bounds::centered(None, size(px(760.), px(500.)), cx);
    // A new window is a new capture session, even if a platform removed the
    // previous window without routing the native close callback.
    input
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .diagnostics
        .stop();
    *existing = Some(
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("Capstan Event Viewer".into()),
                    ..TitleBar::title_bar_options()
                }),
                ..TitleBar::window_options()
            },
            cx,
            move |window, cx| {
                window.on_window_should_close(cx, move |_, _| {
                    close_input
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .diagnostics
                        .stop();
                    true
                });
                cx.new(|cx| EventViewer::new(input, input_available, cx))
            },
        )
        .expect("failed to open event viewer")
        .0,
    );
}
