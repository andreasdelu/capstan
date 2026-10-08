use super::{Context, diagnostics};
use gpui_kit::component::{
    ActiveTheme, Sizable, TitleBar,
    accordion::Accordion,
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
    details_open: bool,
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
    fn new(input: Arc<Context>, cx: &mut ViewContext<Self>) -> Self {
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
            details_open: false,
        }
    }
}

impl Render for EventViewer {
    fn render(&mut self, _: &mut Window, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let (recording, rows, timeout) = {
            let state = self.input.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.diagnostics.recording,
                state.diagnostics.rows.iter().cloned().collect::<Vec<_>>(),
                state.config.escape_timeout_ms,
            )
        };
        let gestures = diagnostics::gestures(&rows);
        let theme = cx.theme();
        div().size_full().flex().flex_col().bg(theme.background).text_color(theme.foreground).font_family(theme.font_family.clone()).text_size(px(13.))
            .child(TitleBar::new().bg(theme.background).border_color(theme.border).child("Capstan Event Viewer"))
            .child(div().flex().flex_col().gap_3().p_4()
                .child(div().text_size(px(11.)).text_color(theme.muted_foreground).child("Caps only · 200 events in memory · no export. Translations are not OS delivery proof."))
                .child(div().child(format!("Current tap cutoff: {timeout} ms · Each press keeps its original cutoff.")))
                .child(div().flex().items_center().gap_2()
                    .child(Button::new("viewer-record").small().label(if recording { "Pause" } else { "Record" }).primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let mut state = this.input.state.lock().unwrap_or_else(|e| e.into_inner());
                            let recording = !state.diagnostics.recording;
                            state.diagnostics.set_recording(recording);
                            cx.notify();
                        })))
                    .child(Button::new("viewer-clear").small().label("Clear").outline()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.input.state.lock().unwrap_or_else(|e| e.into_inner()).diagnostics.clear();
                            cx.notify();
                        })))
                    .child(div().text_color(theme.muted_foreground).child(format!("{} · {} / 200", if recording { "Recording" } else { "Paused" }, rows.len())))))
            .child(div().id("viewer-events").test_support().flex_1().min_h_0().overflow_y_scroll().px_4().pb_4()
                .child(div().flex().flex_col().gap_2()
                    .when(rows.is_empty(), |view| view.child("Press Record, then try Caps Lock. Closing this window stops and clears capture."))
                    .children(gestures.iter().rev().enumerate().map(|(ix, gesture)| div().id(("gesture", ix)).test_support().flex_none().text_size(px(12.)).py_2().border_b_1().border_color(theme.border).child(gesture.label())))
                    .child(Accordion::new("viewer-details").small().h_auto().flex_none()
                        .item(|item| item.open(self.details_open).title(div().id("viewer-details-title").test_support().child("Details"))
                            .when(self.details_open, |item| item.child(div().id("viewer-raw").test_support().flex().flex_col().gap_2()
                                .children(rows.iter().enumerate().map(|(ix, row)| div().text_size(px(11.)).child(format!("{} · raw source timestamp {:.3} s · {}", ix + 1, row.timestamp.as_secs_f64(), row.label())))))))
                        .on_toggle_click(cx.listener(|this, indices: &[usize], _, cx| { this.details_open = !indices.is_empty(); cx.notify(); })))))
    }
}

pub fn open(input: Arc<Context>, existing: &mut Option<AnyWindowHandle>, cx: &mut App) {
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
                cx.new(|cx| EventViewer::new(input, cx))
            },
        )
        .expect("failed to open event viewer")
        .0,
    );
}
