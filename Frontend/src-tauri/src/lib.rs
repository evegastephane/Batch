use tauri_plugin_shell::ShellExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Lancer le sidecar backend Rust (mmc-batch-backend)
            let sidecar_command = app
                .shell()
                .sidecar("mmc-batch-backend")
                .expect("Impossible de trouver le sidecar mmc-batch-backend");

            let (_rx, _child) = sidecar_command
                .spawn()
                .expect("Echec du lancement du sidecar backend");

            // Donner le temps au backend de démarrer
            std::thread::sleep(std::time::Duration::from_millis(500));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Erreur lors du lancement de l'application Tauri");
}
