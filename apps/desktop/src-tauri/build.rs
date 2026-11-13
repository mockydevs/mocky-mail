//! Generates the Tauri context (config, icons, permissions) at build time.

fn main() {
    tauri_build::build();
}
