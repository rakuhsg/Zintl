mod scenarios;

use std::env;

use zintl_ui_desktop::{App, AppError};

fn main() -> Result<(), AppError> {
    let scenario = scenario_argument().unwrap_or_else(|| "text-field".to_owned());

    match scenario.as_str() {
        "text-field" => App::new(scenarios::text_field::MainView::new()).run()?,
        "hstack" => App::new(scenarios::hstack::MainView).run()?,
        "full-width-stack" => App::new(scenarios::full_width_stack::MainView).run()?,
        "window-close" => App::new(scenarios::window_close::MainView).run()?,
        unknown => {
            eprintln!("unknown UI test scenario: {unknown}");
            std::process::exit(2);
        }
    }

    if let Some(path) = env::var_os("ZINTL_UI_TEST_EXIT_MARKER") {
        std::fs::write(path, "ok").expect("the UI test exit marker must be writable");
    }
    Ok(())
}

fn scenario_argument() -> Option<String> {
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--scenario" {
            return arguments.next();
        }
    }
    None
}
