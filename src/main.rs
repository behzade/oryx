mod app;
mod assets;
mod audio;
mod keybindings;
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
    if let Err(error) = app::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
