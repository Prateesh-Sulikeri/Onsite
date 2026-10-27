fn main() {
    // tauri-build embeds the icons (exe resource + window icon) but doesn't watch them,
    // so a new logo kept showing the old one in the taskbar until a clean build.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
