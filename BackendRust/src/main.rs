//! Serveur HTTP optionnel autour de `mmc_batch_core`.
//!
//! L'appli Tauri n'en a pas besoin (elle appelle la bibliotheque directement).
//! Il sert a travailler sur l'interface dans un navigateur (`npm run dev`) :
//!
//! ```powershell
//! cd BackendRust
//! cargo run
//! ```

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use mmc_batch_core::{convertir, Demande, EntreeHistorique, Erreur, Historique, Mode};
use serde::Deserialize;
use tower_http::cors::{Any, CorsLayer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dossier = dossier_donnees();
    let dossier_zips = std::env::var_os("MMC_STORAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dossier.join("stockage"));
    let base = std::env::var_os("MMC_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| dossier.join("historique.db"));
    let historique = Arc::new(Historique::avec_chemins(dossier_zips, base)?);
    println!(
        "Donnees : {} et {}",
        historique.dossier_zips().display(),
        historique.base().display()
    );

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any)
        .expose_headers([
            header::CONTENT_DISPOSITION,
            HeaderName::from_static("x-nb-feuilles"),
            HeaderName::from_static("x-historique-id"),
        ]);

    let app = Router::new()
        .route("/", get(accueil))
        .route("/convert", post(convertir_standard))
        .route("/convert/crplmt", post(convertir_crplmt))
        .route("/historique", get(lister_historique))
        .route("/historique/:id/telecharger", get(telecharger_historique))
        .route("/historique/:id", delete(supprimer_historique))
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
        .layer(cors)
        .with_state(historique);

    let port = std::env::var("MMC_PORT")
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .unwrap_or(8000);
    let adresse = SocketAddr::from(([127, 0, 0, 1], port));
    let ecoute = tokio::net::TcpListener::bind(adresse).await.map_err(|e| {
        anyhow::anyhow!("Impossible d'ecouter sur {adresse} ({e}) : le port est deja utilise.")
    })?;
    println!("API Rust prete sur http://{adresse}");
    axum::serve(ecoute, app).await?;
    Ok(())
}

/// A cote du binaire si on peut y ecrire, sinon dans le profil utilisateur.
fn dossier_donnees() -> PathBuf {
    let dossier_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    if let Some(dossier) = dossier_exe {
        let test = dossier.join(".mmc-ecriture-test");
        if std::fs::write(&test, b"ok").is_ok() {
            let _ = std::fs::remove_file(&test);
            return dossier;
        }
    }
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(|d| PathBuf::from(d).join("MMC Batch"))
        .unwrap_or_else(|| std::env::temp_dir().join("MMC Batch"))
}

type Etat = State<Arc<Historique>>;

async fn accueil() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "message": "API prete", "version": env!("CARGO_PKG_VERSION") }))
}

async fn convertir_standard(historique: Etat, multipart: Multipart) -> Result<Response, ErreurHttp> {
    convertir_formulaire(historique, multipart, Mode::Standard).await
}

async fn convertir_crplmt(historique: Etat, multipart: Multipart) -> Result<Response, ErreurHttp> {
    convertir_formulaire(historique, multipart, Mode::Crplmt).await
}

async fn convertir_formulaire(
    State(historique): Etat,
    mut multipart: Multipart,
    mode: Mode,
) -> Result<Response, ErreurHttp> {
    let mut demande = Demande {
        nom_fichier: String::new(),
        contenu: Vec::new(),
        mode,
        style_entete: None,
        montant: None,
        titre: None,
        description: None,
    };

    while let Some(champ) = multipart.next_field().await.map_err(ErreurHttp::invalide)? {
        match champ.name().unwrap_or("") {
            "fichier" => {
                demande.nom_fichier = champ.file_name().unwrap_or("").to_string();
                demande.contenu = champ.bytes().await.map_err(ErreurHttp::invalide)?.to_vec();
            }
            "style_entete" => demande.style_entete = champ.text().await.ok(),
            "montant" => demande.montant = champ.text().await.ok(),
            "titre" => demande.titre = champ.text().await.ok(),
            "description" => demande.description = champ.text().await.ok(),
            _ => {}
        }
    }
    if demande.nom_fichier.is_empty() {
        return Err(ErreurHttp(Erreur::Invalide("Fichier manquant.".into())));
    }

    let conversion = tokio::task::spawn_blocking(move || convertir(&historique, demande))
        .await
        .map_err(|e| ErreurHttp(Erreur::Interne(e.to_string())))??;

    let mut entetes = zip_entetes(&conversion.nom_zip)?;
    entetes.insert("x-nb-feuilles", HeaderValue::from(conversion.nb_feuilles));
    entetes.insert("x-historique-id", HeaderValue::from(conversion.historique_id));
    Ok((entetes, Body::from(conversion.zip)).into_response())
}

#[derive(Deserialize)]
struct Limite {
    limite: Option<i64>,
}

async fn lister_historique(
    State(historique): Etat,
    Query(query): Query<Limite>,
) -> Result<Json<Vec<EntreeHistorique>>, ErreurHttp> {
    Ok(Json(historique.lister(query.limite.unwrap_or(50))?))
}

async fn telecharger_historique(
    State(historique): Etat,
    AxumPath(id): AxumPath<i64>,
) -> Result<Response, ErreurHttp> {
    let (nom, octets) = historique.lire_zip(id)?;
    Ok((zip_entetes(&nom)?, Body::from(octets)).into_response())
}

async fn supprimer_historique(
    State(historique): Etat,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, ErreurHttp> {
    historique.supprimer(id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn zip_entetes(nom: &str) -> Result<HeaderMap, ErreurHttp> {
    let mut entetes = HeaderMap::new();
    entetes.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    let disposition = HeaderValue::from_str(&format!("attachment; filename={nom}"))
        .map_err(|e| ErreurHttp(Erreur::Interne(e.to_string())))?;
    entetes.insert(header::CONTENT_DISPOSITION, disposition);
    Ok(entetes)
}

struct ErreurHttp(Erreur);

impl ErreurHttp {
    fn invalide(e: impl std::fmt::Display) -> Self {
        ErreurHttp(Erreur::Invalide(e.to_string()))
    }
}

impl From<Erreur> for ErreurHttp {
    fn from(e: Erreur) -> Self {
        ErreurHttp(e)
    }
}

impl IntoResponse for ErreurHttp {
    fn into_response(self) -> Response {
        let statut = match self.0 {
            Erreur::Invalide(_) => StatusCode::BAD_REQUEST,
            Erreur::Introuvable(_) => StatusCode::NOT_FOUND,
            Erreur::Interne(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (statut, Json(serde_json::json!({ "detail": self.0.to_string() }))).into_response()
    }
}
