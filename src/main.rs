#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod assets;
mod audio;
mod keybindings;
mod launch;
mod library;
mod metadata;
mod model;
mod pathing;
mod platform;
mod progressive;
mod provider;
mod theme;
mod transfer;
mod url_media;

fn main() {
    let launch_options = match launch::prepare() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = app::run(launch_options) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
