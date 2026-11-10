mod automation;
mod header;
mod shell;
use tessel_editor_core::state;
mod theme;
mod workstation;

use gpui_kit::*;
use shell::EditorShell;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "ui") {
        if let Err(error) = automation::run_cli(&args[1..]) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if !args.is_empty() {
        eprintln!("Usage: tessel-editor [ui --help]");
        std::process::exit(2);
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            workstation::install_perf_meter(cx);
            theme::apply(cx, true);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // Below this the timeline would have no room for a monitor.
                window_min_size: Some(size(px(960.0), px(640.0))),
                titlebar: Some(TitlebarOptions {
                    appears_transparent: true,
                    // Centered in the 40 px header strip.
                    traffic_light_position: Some(point(px(12.0), px(13.0))),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let (window, shell) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| EditorShell::new(window, cx))
            })
            .expect("Failed to open the Tessel Editor window");
            if let Err(error) = automation::start(window, shell, cx) {
                eprintln!("Cannot start UI automation: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
