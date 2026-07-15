mod utils;
mod app;
mod niri;

fn main() -> cosmic::iced::Result {
    // Initialize standard logging
    env_logger::init();

    // Start the COSMIC Applet event loop
    cosmic::applet::run::<app::AppModel>(())
}
