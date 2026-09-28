use std::sync::Mutex;

use tauri::{Manager, RunEvent};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

/// Processus du backend, garde pour pouvoir l'arreter a la fermeture de l'appli.
struct Backend(Mutex<Option<CommandChild>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Backend(Mutex::new(None)))
        .setup(|app| {
            // Un ancien backend reste parfois lance (fermeture brutale, ancienne version
            // de l'appli) : il garde le port 8000 et le nouveau ne peut pas demarrer.
            arreter_anciens_backends();

            // Lancer le sidecar backend Rust (mmc-batch-backend)
            let (mut rx, child) = app.shell().sidecar("mmc-batch-backend")?.spawn()?;
            *app.state::<Backend>().0.lock().unwrap() = Some(child);

            // Afficher les messages du backend dans la console de `npm run tauri dev`
            tauri::async_runtime::spawn(async move {
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(ligne) => {
                            println!("[backend] {}", String::from_utf8_lossy(&ligne).trim_end())
                        }
                        CommandEvent::Stderr(ligne) => {
                            eprintln!("[backend] {}", String::from_utf8_lossy(&ligne).trim_end())
                        }
                        CommandEvent::Error(erreur) => eprintln!("[backend] erreur : {erreur}"),
                        CommandEvent::Terminated(fin) => {
                            eprintln!("[backend] arrete (code {:?})", fin.code)
                        }
                        _ => {}
                    }
                }
            });

            // Donner le temps au backend de démarrer
            std::thread::sleep(std::time::Duration::from_millis(500));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Erreur lors du lancement de l'application Tauri");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            // Arreter le backend avec l'appli, sinon il reste lance en arriere-plan
            if let Some(child) = app_handle.state::<Backend>().0.lock().unwrap().take() {
                let _ = child.kill();
            }
        }
    });
}

#[cfg(windows)]
fn arreter_anciens_backends() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "mmc-batch-backend.exe"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn arreter_anciens_backends() {}
