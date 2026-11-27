//! `vexdom-desktop`: the GPUI app (P10, ADR-013). It starts in the menu bar, which is its main
//! surface; the window is Settings, opened from the menu (and shown by itself only on the very
//! first launch). It starts the Bridge as it opens only when Settings says to (`desktop.json`, on
//! by default); otherwise Start does. Closing the window hides it; the app stays in the menu bar
//! until Quit.

mod bridge;
mod log_tail;
mod login_item;
mod model;
mod preflight;
mod settings;
mod shell;
mod text_input;
mod theme;
mod tray;
mod ui;

use gpui::{
    Action, App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, actions, point, prelude::*, px, size,
};

use ui::{AppView, Tab};

actions!(
    vexdom,
    [
        Quit,
        ShowGeneral,
        ShowBrowsers,
        ShowAgent,
        ShowSkills,
        ShowActivity,
        ShowLogs,
        ShowAbout
    ]
);

/// The Settings window, for a launch of the app while it runs.
struct MainWindow(WindowHandle<AppView>);

impl gpui::Global for MainWindow {}

/// ⌘1…⌘7 pick a tab wherever the focus is, even with none.
fn on_tab<A: Action>(cx: &mut App, window: WindowHandle<AppView>, tab: Tab) {
    cx.on_action(move |_: &A, cx| {
        let _ = window.update(cx, |view, _, cx| view.show_tab(tab, cx));
    });
}

fn main() {
    // No settings file yet: the very first launch, which shows the window.
    let first_launch = !settings::path().exists();
    let settings = settings::load();
    let (inbox, inbound) = tokio::sync::mpsc::unbounded_channel();
    let from_bridge = inbox.clone();
    let launch = bridge::Launch {
        start_bridge: settings.start_bridge_on_launch,
        finish_update: settings.finish_update_on_launch,
    };
    let commands = bridge::spawn(launch, move |update| {
        let _ = from_bridge.send(ui::Inbound::Bridge(update));
    });
    let app = Application::new();
    // Opening the app again (Finder, Spotlight, `open -a`) brings the window back.
    app.on_reopen(|cx: &mut App| {
        if let Some(handle) = cx.try_global::<MainWindow>().map(|main| main.0) {
            let _ = handle.update(cx, |view, window, cx| view.reveal(None, window, cx));
        }
    });
    app.run(move |cx: &mut App| {
        theme::load_fonts(cx);
        tray::set_notification_sender();
        text_input::bind_keys(cx);
        cx.set_global(theme::Chosen(settings.appearance));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-1", ShowGeneral, None),
            KeyBinding::new("cmd-2", ShowBrowsers, None),
            KeyBinding::new("cmd-3", ShowAgent, None),
            KeyBinding::new("cmd-4", ShowSkills, None),
            KeyBinding::new("cmd-5", ShowActivity, None),
            KeyBinding::new("cmd-6", ShowLogs, None),
            KeyBinding::new("cmd-7", ShowAbout, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(760.), px(600.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Vexdom".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(14.), px(13.))),
                    }),
                    window_min_size: Some(size(px(680.), px(520.))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                move |window, cx| {
                    theme::paint(window, cx);
                    cx.new(|cx| AppView::new(commands, inbound, inbox, settings, window, cx))
                },
            )
            .expect("the window opens");
        cx.set_global(MainWindow(window));
        window
            .update(cx, |view, window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.hide();
                    false
                });
                // Light or dark follows System Settings, live, unless Settings picked one.
                window
                    .observe_window_appearance(|window, cx| {
                        theme::paint(window, cx);
                        window.refresh();
                    })
                    .detach();
                view.launch(first_launch, window, cx);
            })
            .expect("the window is open");
        on_tab::<ShowGeneral>(cx, window, Tab::General);
        on_tab::<ShowBrowsers>(cx, window, Tab::Browsers);
        on_tab::<ShowAgent>(cx, window, Tab::Agent);
        on_tab::<ShowSkills>(cx, window, Tab::Skills);
        on_tab::<ShowActivity>(cx, window, Tab::Activity);
        on_tab::<ShowLogs>(cx, window, Tab::Logs);
        on_tab::<ShowAbout>(cx, window, Tab::About);
    });
}
