mod app;
mod applications;
mod config;
mod icons;
mod launcher;
mod layout;
mod niri;
mod niri_config;
mod session;
mod ui;

fn main() -> cosmic::iced::Result {
    // Initialize standard logging with default level for our crate
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("cosmic_ext_niri_windows=info"),
    )
    .init();

    // Start the COSMIC Applet event loop
    cosmic::applet::run::<app::AppModel>(())
}
