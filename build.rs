#[cfg(windows)]
fn main() {
    println!("cargo:rerun-if-changed=assets/logo.ico");
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/logo.ico");
    res.compile().expect("failed to compile Windows resources");
}

#[cfg(not(windows))]
fn main() {}
