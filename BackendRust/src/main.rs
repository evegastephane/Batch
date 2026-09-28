use std::{
    collections::HashMap,
    io::{Cursor, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use calamine::{open_workbook_auto_from_rs, Data, Reader};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

#[derive(Clone)]
struct AppState {
    storage_dir: PathBuf,
    db_path: PathBuf,
}

#[derive(Debug, Serialize)]
struct HistoryEntry {
    id: i64,
    nom_original: String,
    nom_zip: String,
    chemin_zip: String,
    nb_feuilles: i64,
    taille_octets: i64,
    style_entete: String,
    date_creation: String,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limite: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let base_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or(std::env::current_dir()?);

    let storage_dir = std::env::var_os("MMC_STORAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| base_dir.join("stockage"));
    let db_path = std::env::var_os("MMC_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| base_dir.join("historique.db"));

    fs::create_dir_all(&storage_dir).await?;

    let state = Arc::new(AppState {
        storage_dir,
        db_path,
    });
    init_db(&state.db_path)?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any)
        .expose_headers([
            header::CONTENT_DISPOSITION,
            HeaderNameExt::x_nb_feuilles(),
            HeaderNameExt::x_historique_id(),
        ]);

    let app = Router::new()
        .route("/", get(root))
        .route("/convert", post(convert))
        .route("/historique", get(history))
        .route("/historique/:id/telecharger", get(download_history_entry))
        .route("/historique/:id", delete(delete_history_entry))
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("API Rust prete sur http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "message": "API prete. Envoie un fichier Excel sur POST /convert"
    }))
}

async fn convert(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    let mut file_name: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut style_entete = String::from("Init");

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "fichier" => {
                file_name = field.file_name().map(ToString::to_string);
                file_bytes = Some(field.bytes().await?.to_vec());
            }
            "style_entete" => {
                style_entete = field.text().await.unwrap_or_else(|_| String::from("Init"));
            }
            _ => {}
        }
    }

    if style_entete != "Set" {
        style_entete = String::from("Init");
    }

    let file_name = file_name.ok_or_else(|| AppError::bad_request("Fichier manquant."))?;
    let bytes = file_bytes.ok_or_else(|| AppError::bad_request("Fichier manquant."))?;

    if !has_excel_extension(&file_name) {
        return Err(AppError::bad_request(
            "Le fichier doit etre un .xlsx ou .xls",
        ));
    }

    let csvs = excel_to_csvs(&bytes, &style_entete)
        .map_err(|e| AppError::bad_request(format!("Fichier Excel illisible: {e}")))?;
    let zip_bytes = build_zip(&csvs)?;
    let zip_name = format!("{}_csv.zip", strip_extension(&file_name));

    let disk_name = format!("{}.zip", Uuid::new_v4().simple());
    let disk_path = state.storage_dir.join(disk_name);
    fs::write(&disk_path, &zip_bytes).await?;

    let history_id = insert_history(
        &state.db_path,
        &file_name,
        &zip_name,
        &disk_path.to_string_lossy(),
        csvs.len() as i64,
        zip_bytes.len() as i64,
        &style_entete,
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename={zip_name}"))?,
    );
    headers.insert(
        "X-Nb-Feuilles",
        HeaderValue::from_str(&csvs.len().to_string())?,
    );
    headers.insert(
        "X-Historique-Id",
        HeaderValue::from_str(&history_id.to_string())?,
    );

    Ok((headers, Body::from(zip_bytes)).into_response())
}

async fn history(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryEntry>>, AppError> {
    let limit = query.limite.unwrap_or(50).clamp(1, 500);
    Ok(Json(list_history(&state.db_path, limit)?))
}

async fn download_history_entry(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Response, AppError> {
    let entry = get_history_entry(&state.db_path, id)?
        .ok_or_else(|| AppError::not_found("Entree introuvable."))?;
    let bytes = fs::read(&entry.chemin_zip)
        .await
        .map_err(|_| AppError::not_found("Le fichier n'existe plus sur le disque."))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename={}", entry.nom_zip))?,
    );
    Ok((headers, Body::from(bytes)).into_response())
}

async fn delete_history_entry(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let entry = get_history_entry(&state.db_path, id)?
        .ok_or_else(|| AppError::not_found("Entree introuvable."))?;

    if Path::new(&entry.chemin_zip).exists() {
        let _ = fs::remove_file(&entry.chemin_zip).await;
    }

    delete_history(&state.db_path, id)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn excel_to_csvs(bytes: &[u8], style_entete: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut workbook = open_workbook_auto_from_rs(cursor)?;
    let sheet_names = workbook.sheet_names().to_owned();

    if sheet_names.is_empty() {
        return Err(anyhow!("Aucune feuille trouvee dans le fichier."));
    }

    let mut result = Vec::new();

    for sheet_name in sheet_names {
        let range = workbook.worksheet_range(&sheet_name)?;
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(clean_cell).collect::<Vec<_>>())
            .filter(|row| row.iter().any(|value| !value.trim().is_empty()))
            .collect();

        let parameter_name = find_most_frequent_parameter(&rows).unwrap_or_else(|| sheet_name.clone());

        let mut csv = String::new();
        csv.push_str(&generate_hdr(rows.len(), style_entete, &parameter_name));
        csv.push('\n');

        for row in rows {
            csv.push_str(&row.join(","));
            csv.push('\n');
        }

        let name = format!("{}.csv", sheet_name.replace(['/', '\\'], "-"));
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(csv.as_bytes());
        result.push((name, with_bom));
    }

    Ok(result)
}

fn clean_cell(cell: &Data) -> String {
    let value = match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.clone(),
        Data::Float(value) => {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::DateTime(value) => value.to_string(),
        Data::DateTimeIso(value) => value.clone(),
        Data::DurationIso(value) => value.clone(),
        Data::Error(value) => value.to_string(),
    };

    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn generate_hdr(row_count: usize, style_entete: &str, parameter_name: &str) -> String {
    if style_entete == "Set" {
        return format!(
            "HDR,UpdateIndividualRatingParameter,Set {parameter_name},ExtId12341,{row_count},{parameter_name} enable,1"
        );
    }

    format!(
        "HDR,\"UpdateIndividualRatingParameter\",\"Init {parameter_name} TRUE\",\"ExtId12340\",\"{row_count}\",\" {parameter_name} init\",\"1\""
    )
}

fn find_most_frequent_parameter(rows: &[Vec<String>]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for row in rows {
        if let Some(param) = extract_parameter_from_row(row) {
            *counts.entry(param).or_insert(0) += 1;
        }
    }

    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(val, _)| val)
}

fn extract_parameter_from_row(row: &[String]) -> Option<String> {
    let full_line = row.join(",");
    let parts: Vec<&str> = full_line
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim())
        .collect();

    if parts.len() >= 3 {
        let candidate = parts[2];
        if !candidate.is_empty() {
            return Some(candidate.to_string());
        }
    }

    None
}

fn build_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for (name, data) in files {
        writer.start_file(name, options)?;
        writer.write_all(data)?;
    }

    Ok(writer.finish()?.into_inner())
}

fn init_db(db_path: &Path) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS historique (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            nom_original TEXT NOT NULL,
            nom_zip TEXT NOT NULL,
            chemin_zip TEXT NOT NULL,
            nb_feuilles INTEGER NOT NULL,
            taille_octets INTEGER NOT NULL,
            style_entete TEXT NOT NULL,
            date_creation TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
        [],
    )?;
    Ok(())
}

fn insert_history(
    db_path: &Path,
    original_name: &str,
    zip_name: &str,
    zip_path: &str,
    sheet_count: i64,
    byte_size: i64,
    header_style: &str,
) -> Result<i64> {
    let conn = Connection::open(db_path)?;
    conn.execute(
        r#"
        INSERT INTO historique
        (nom_original, nom_zip, chemin_zip, nb_feuilles, taille_octets, style_entete)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            original_name,
            zip_name,
            zip_path,
            sheet_count,
            byte_size,
            header_style
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn list_history(db_path: &Path, limit: i64) -> Result<Vec<HistoryEntry>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM historique ORDER BY date_creation DESC LIMIT ?1")?;
    let rows = stmt.query_map(params![limit], history_from_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn get_history_entry(db_path: &Path, id: i64) -> Result<Option<HistoryEntry>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM historique WHERE id = ?1")?;
    match stmt.query_row(params![id], history_from_row) {
        Ok(entry) => Ok(Some(entry)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn delete_history(db_path: &Path, id: i64) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute("DELETE FROM historique WHERE id = ?1", params![id])?;
    Ok(())
}

fn history_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get("id")?,
        nom_original: row.get("nom_original")?,
        nom_zip: row.get("nom_zip")?,
        chemin_zip: row.get("chemin_zip")?,
        nb_feuilles: row.get("nb_feuilles")?,
        taille_octets: row.get("taille_octets")?,
        style_entete: row.get("style_entete")?,
        date_creation: row.get("date_creation")?,
    })
}

fn has_excel_extension(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".xlsx") || lower.ends_with(".xls")
}

fn strip_extension(file_name: &str) -> &str {
    file_name
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(file_name)
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "detail": self.message
            })),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.into().to_string(),
        }
    }
}

struct HeaderNameExt;

impl HeaderNameExt {
    fn x_nb_feuilles() -> header::HeaderName {
        header::HeaderName::from_static("x-nb-feuilles")
    }

    fn x_historique_id() -> header::HeaderName {
        header::HeaderName::from_static("x-historique-id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_most_frequent_parameter() {
        let rows = vec![
            vec!["ID".to_string(), "14824152".to_string(), "EleCashback".to_string()],
            vec!["ID".to_string(), "14824153".to_string(), "EleCashback".to_string()],
            vec!["ID".to_string(), "14824154".to_string(), "EleCashback".to_string()],
        ];

        let param = find_most_frequent_parameter(&rows);
        assert_eq!(param.as_deref(), Some("EleCashback"));

        let hdr = generate_hdr(rows.len(), "Set", param.as_deref().unwrap());
        assert_eq!(
            hdr,
            "HDR,UpdateIndividualRatingParameter,Set EleCashback,ExtId12341,3,EleCashback enable,1"
        );
    }

    #[test]
    fn test_find_most_frequent_parameter_agent_float_region() {
        let rows = vec![
            vec!["MSISDN".to_string(), "237681178408".to_string(), "AgentFloatRegion".to_string(), "West".to_string()],
            vec!["MSISDN".to_string(), "237681178409".to_string(), "AgentFloatRegion".to_string(), "North".to_string()],
        ];

        let param = find_most_frequent_parameter(&rows);
        assert_eq!(param.as_deref(), Some("AgentFloatRegion"));

        let hdr = generate_hdr(rows.len(), "Set", param.as_deref().unwrap());
        assert_eq!(
            hdr,
            "HDR,UpdateIndividualRatingParameter,Set AgentFloatRegion,ExtId12341,2,AgentFloatRegion enable,1"
        );
    }

    #[test]
    fn test_find_most_frequent_parameter_single_cell_csv() {
        // Format quand toute la ligne CSV est stockée dans une seule cellule Excel (colonne A)
        let rows = vec![
            vec!["MSISDN,237674497307,AgentFloatRegion,SouthWest".to_string()],
            vec!["MSISDN,237674931273,AgentFloatRegion,West".to_string()],
            vec!["MSISDN,237652748363,AgentFloatRegion,LittoralII".to_string()],
        ];

        let param = find_most_frequent_parameter(&rows);
        assert_eq!(param.as_deref(), Some("AgentFloatRegion"));

        let hdr = generate_hdr(rows.len(), "Init", param.as_deref().unwrap());
        assert_eq!(
            hdr,
            "HDR,\"UpdateIndividualRatingParameter\",\"Init AgentFloatRegion TRUE\",\"ExtId12340\",\"3\",\" AgentFloatRegion init\",\"1\""
        );
    }
}
