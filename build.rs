use std::env;

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/app/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        compile_windows_resources();
    }
}

#[cfg(windows)]
fn compile_windows_resources() {
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icons/app/icon.ico");
    resource
        .compile()
        .expect("failed to compile Windows icon resources");
}

#[cfg(not(windows))]
fn compile_windows_resources() {}
