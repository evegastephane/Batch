use std::{
    collections::VecDeque,
    fs::OpenOptions,
    io::Write,
    net::TcpListener,
    sync::Mutex,
};

use serde::Serialize;
use tauri::{AppHandle, Manager, RunEvent, State};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

const PORT_PAR_DEFAUT: u16 = 8000;
const LIGNES_JOURNAL: usize = 50;

/// Processus du backend (pour l'arreter a la fermeture) et son etat, lu par l'interface.
#[derive(Default)]
struct Backend {
    processus: Mutex<Option<CommandChild>>,
    etat: Mutex<EtatBackend>,
}

#[derive(Clone, Default, Serialize)]
struct EtatBackend {
    port: u16,
    pret: bool,
    en_marche: bool,
    code_sortie: Option<i32>,
    erreur: Option<String>,
    journal: VecDeque<String>,
    fichier_journal: String,
}

impl EtatBackend {
    fn noter(&mut self, ligne: String) {
        if self.journal.len() >= LIGNES_JOURNAL {
            self.journal.pop_front();
        }
        self.journal.push_back(ligne);
    }
}

/// Appelee par l'interface : port a utiliser, et cause si le backend ne repond pas.
#[tauri::command]
fn etat_backend(backend: State<'_, Backend>) -> EtatBackend {
    backend.etat.lock().unwrap().clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Backend::default())
        .invoke_handler(tauri::generate_handler![etat_backend])
        .setup(|app| {
            // Un ancien backend reste parfois lance (fermeture brutale, ancienne version
            // de l'appli) : il garde le port et le nouveau ne peut pas demarrer.
            arreter_anciens_backends();

            // Une erreur ici ne doit pas fermer l'appli : elle est affichee dans l'interface.
            if let Err(erreur) = lancer_backend(app.handle()) {
                let backend = app.state::<Backend>();
                let mut etat = backend.etat.lock().unwrap();
                etat.erreur = Some(erreur.to_string());
                etat.noter(format!("Lancement du backend impossible : {erreur}"));
                eprintln!("[backend] lancement impossible : {erreur}");
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Erreur lors du lancement de l'application Tauri");

    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            // Arreter le backend avec l'appli, sinon il reste lance en arriere-plan
            if let Some(child) = app_handle.state::<Backend>().processus.lock().unwrap().take() {
                let _ = child.kill();
            }
        }
    });
}

fn lancer_backend(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Donnees dans le profil utilisateur (%APPDATA%\com.mmc.batch) : le dossier
    // d'installation (C:\Program Files\...) n'est pas accessible en ecriture.
    let dossier = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dossier)?;
    let chemin_journal = dossier.join("backend.log");
    let port = choisir_port();

    {
        let backend = app.state::<Backend>();
        let mut etat = backend.etat.lock().unwrap();
        etat.port = port;
        etat.fichier_journal = chemin_journal.display().to_string();
    }

    let (mut rx, child) = app
        .shell()
        .sidecar("mmc-batch-backend")?
        .env("MMC_PORT", port.to_string())
        .env("MMC_STORAGE_DIR", dossier.join("stockage"))
        .env("MMC_DB_PATH", dossier.join("historique.db"))
        .spawn()?;

    {
        let backend = app.state::<Backend>();
        backend.etat.lock().unwrap().en_marche = true;
        *backend.processus.lock().unwrap() = Some(child);
    }

    // Suivre les messages du backend : console de `npm run tauri dev`, fichier
    // backend.log et journal affiche par l'interface en cas d'erreur.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut fichier = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&chemin_journal)
            .ok();

        while let Some(event) = rx.recv().await {
            let (ligne, fin) = match event {
                CommandEvent::Stdout(octets) | CommandEvent::Stderr(octets) => {
                    (String::from_utf8_lossy(&octets).trim_end().to_string(), None)
                }
                CommandEvent::Error(erreur) => (format!("erreur : {erreur}"), None),
                CommandEvent::Terminated(fin) => {
                    (format!("backend arrete (code {:?})", fin.code), Some(fin.code))
                }
                _ => continue,
            };

            eprintln!("[backend] {ligne}");
            if let Some(f) = fichier.as_mut() {
                let _ = writeln!(f, "{ligne}");
            }
            noter_evenement(&app, ligne, fin);
        }
    });

    Ok(())
}

fn noter_evenement(app: &AppHandle, ligne: String, fin: Option<Option<i32>>) {
    let backend = app.state::<Backend>();
    let mut etat = backend.etat.lock().unwrap();
    if ligne.starts_with("API Rust prete") {
        etat.pret = true;
    }
    if let Some(code) = fin {
        etat.en_marche = false;
        etat.pret = false;
        etat.code_sortie = code;
    }
    etat.noter(ligne);
}

/// Le port 8000 s'il est libre, sinon un port libre choisi par le systeme
/// (8000 peut etre pris par un autre programme ou reserve par Windows).
fn choisir_port() -> u16 {
    if TcpListener::bind(("127.0.0.1", PORT_PAR_DEFAUT)).is_ok() {
        return PORT_PAR_DEFAUT;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .unwrap_or(PORT_PAR_DEFAUT)
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
