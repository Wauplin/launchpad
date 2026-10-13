//! WebKitGTK can omit in-memory screenshots from DOM paste events.

use base64::Engine;

#[tauri::command]
pub async fn read_clipboard_image(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    // The UI is served by the daemon; never expose the clipboard to another origin.
    let url = window.url().map_err(|e| e.to_string())?;
    let port = if std::env::var("LAUNCHPAD_PROFILE").as_deref() == Ok("dev") { 7318 } else { 7317 };
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.port() != Some(port) {
        return Err("Clipboard access is only available to the local Launchpad UI.".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    window.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.request_image(move |_, image| {
            let result = image.map(|image| {
                let bytes = image.save_to_bufferv("png", &[]).map_err(|e| e.to_string())?;
                if bytes.len() > 5 * 1024 * 1024 {
                    return Err("Each attachment must be 5 MB or smaller.".into());
                }
                Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
            }).transpose();
            let _ = tx.send(result);
        });
    }).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "Could not read the clipboard image.".to_string())?
    }).await.map_err(|e| e.to_string())?
}
