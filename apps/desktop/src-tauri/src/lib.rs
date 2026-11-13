//! Tauri shell for Mocky Mail.
//!
//! This crate only wires the window and the IPC commands. All mail rules
//! live in `mail-core`, where they are tested without a GUI.

mod commands;

/// Build and run the app.
///
/// Tauri reports a failed start (for example, no display available) by
/// returning an error; we print it and exit non-zero instead of panicking.
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::sanitize_email_html,
            commands::parse_message,
            commands::check_message_size,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Mocky Mail could not start: {error}");
        std::process::exit(1);
    }
}
