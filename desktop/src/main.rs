//! `vexdom-desktop`: the GPUI app (P10, ADR-013). It starts the Bridge as it opens only when
//! Settings says to (`desktop.json`, on by default); otherwise Start does.
//! Closing the window hides the app; it stays in the menu bar until Quit.

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
        ShowOverview,
        ShowBrowsers,
        ShowAgent,
        ShowSettings,
        ShowAbout
    ]
);

/// ⌘1…⌘5 pick a tab wherever the focus is, even with none.
fn on_tab<A: Action>(cx: &mut App, window: WindowHandle<AppView>, tab: Tab) {
    cx.on_action(move |_: &A, cx| {
        let _ = window.update(cx, |view, _, cx| view.show_tab(tab, cx));
    });
}

fn main() {
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
    Application::new().run(move |cx: &mut App| {
        theme::load_fonts(cx);
        tray::set_dock_icon();
        tray::set_notification_sender();
        text_input::bind_keys(cx);
        cx.set_global(theme::Chosen(settings.appearance));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-1", ShowOverview, None),
            KeyBinding::new("cmd-2", ShowBrowsers, None),
            KeyBinding::new("cmd-3", ShowAgent, None),
            KeyBinding::new("cmd-4", ShowSettings, None),
            KeyBinding::new("cmd-5", ShowAbout, None),
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
                    ..Default::default()
                },
                move |window, cx| {
                    theme::paint(window, cx);
                    cx.new(|cx| AppView::new(commands, inbound, inbox, settings, cx))
                },
            )
            .expect("the window opens");
        window
            .update(cx, |_, window, cx| {
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
            })
            .expect("the window is open");
        on_tab::<ShowOverview>(cx, window, Tab::Overview);
        on_tab::<ShowBrowsers>(cx, window, Tab::Browsers);
        on_tab::<ShowAgent>(cx, window, Tab::Agent);
        on_tab::<ShowSettings>(cx, window, Tab::Settings);
        on_tab::<ShowAbout>(cx, window, Tab::About);
        cx.activate(true);
    });
}
