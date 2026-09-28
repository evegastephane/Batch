//! Appli MMC Batch. La conversion tourne dans l'appli elle-meme (commandes
//! Tauri qui appellent `mmc_batch_core`) : pas de serveur a lancer, pas de port,
//! pas de binaire separe qui pourrait rester dans une ancienne version.

use mmc_batch_core::{Conversion, Demande, EntreeHistorique, Historique};
use serde::Serialize;
use tauri::{Manager, State};

#[derive(Serialize)]
struct FichierZip {
    nom: String,
    contenu: Vec<u8>,
}

#[tauri::command]
async fn convertir(
    historique: State<'_, Historique>,
    demande: Demande,
) -> Result<Conversion, String> {
    let historique = historique.inner().clone();
    // Lecture Excel et zip hors du fil de l'interface
    tauri::async_runtime::spawn_blocking(move || mmc_batch_core::convertir(&historique, demande))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn lister_historique(
    historique: State<'_, Historique>,
) -> Result<Vec<EntreeHistorique>, String> {
    historique.lister(50).map_err(|e| e.to_string())
}

#[tauri::command]
async fn lire_zip_historique(
    historique: State<'_, Historique>,
    id: i64,
) -> Result<FichierZip, String> {
    let (nom, contenu) = historique.lire_zip(id).map_err(|e| e.to_string())?;
    Ok(FichierZip { nom, contenu })
}

#[tauri::command]
async fn supprimer_historique(historique: State<'_, Historique>, id: i64) -> Result<(), String> {
    historique.supprimer(id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Historique dans le profil utilisateur (%APPDATA%\com.mmc.batch) :
            // le dossier d'installation n'est pas accessible en ecriture.
            let dossier = app.path().app_data_dir()?;
            app.manage(Historique::ouvrir(&dossier)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            convertir,
            lister_historique,
            lire_zip_historique,
            supprimer_historique
        ])
        .run(tauri::generate_context!())
        .expect("Erreur lors du lancement de l'application Tauri");
}
