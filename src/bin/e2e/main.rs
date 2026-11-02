// E2E test tool — Linux/uinput only (dev helper, not the product).

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() -> std::io::Result<()> {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("e2e is a Linux/uinput test tool");
    std::process::exit(1);
}
