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
        .route("/convert/crplmt", post(convert_crplmt))
        .route("/historique", get(history))
        .route("/historique/:id/telecharger", get(download_history_entry))
        .route("/historique/:id", delete(delete_history_entry))
        .layer(DefaultBodyLimit::max(200 * 1024 * 1024))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        anyhow!(
            "Impossible d'ecouter sur {addr} ({e}). Un autre backend (ancienne version ?) \
             utilise deja ce port : arrete le processus mmc-batch-backend puis relance."
        )
    })?;
    println!("API Rust prete sur http://{addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "message": "API prete. Envoie un fichier Excel sur POST /convert"
    }))
}

struct ConvertForm {
    file_name: String,
    bytes: Vec<u8>,
    style_entete: String,
    montant: Option<String>,
    titre: Option<String>,
    description: Option<String>,
}

async fn read_convert_form(mut multipart: Multipart) -> Result<ConvertForm, AppError> {
    let mut file_name: Option<String> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut style_entete = String::from("Init");
    let mut montant: Option<String> = None;
    let mut titre: Option<String> = None;
    let mut description: Option<String> = None;

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
            "montant" => {
                montant = field.text().await.ok();
            }
            "titre" => {
                titre = field.text().await.ok();
            }
            "description" => {
                description = field.text().await.ok();
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

    Ok(ConvertForm {
        file_name,
        bytes,
        style_entete,
        montant,
        titre,
        description,
    })
}

async fn convert(
    State(state): State<Arc<AppState>>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = read_convert_form(multipart).await?;

    let csvs = excel_to_csvs(&form.bytes, &form.style_entete)
        .map_err(|e| AppError::bad_request(format!("Fichier Excel illisible: {e}")))?;
    let zip_name = format!("{}_csv.zip", strip_extension(&form.file_name));
    let nb_feuilles = csvs.len();

    zip_and_respond(&state, &form, &csvs, zip_name, nb_feuilles).await
}

/// Chemin CRPLMT (AddAlias) : une colonne de MSISDN (avec un titre) devient
/// un fichier texte pret a l'import :
/// `HDR,"AddAlias","<titre>","ExtId12340","<nb>","<description>","1"`
/// suivi d'une ligne `MSISDN,"<numero>","CRPLMT_<numero>@<montant>"` par numero.
async fn convert_crplmt(
    State(state): State<Arc<AppState>>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = read_convert_form(multipart).await?;

    let montant = form_value(&form.montant, CRPLMT_MONTANT_DEFAUT);
    if montant.is_empty() || !montant.chars().all(|c| c.is_ascii_digit()) {
        return Err(AppError::bad_request(
            "Le montant doit etre un nombre entier (ex. 100000).",
        ));
    }
    let titre = form_value(&form.titre, ADD_ALIAS_TITRE_DEFAUT);
    let description = form_value(&form.description, ADD_ALIAS_DESCRIPTION_DEFAUT);
    if titre.contains('"') || description.contains('"') {
        return Err(AppError::bad_request(
            "Le titre et la description ne doivent pas contenir de guillemets.",
        ));
    }

    let base_name = strip_extension(&form.file_name);
    let files = excel_to_add_alias_files(&form.bytes, base_name, &montant, &titre, &description)
        .map_err(|e| AppError::bad_request(format!("Fichier Excel invalide: {e}")))?;
    let zip_name = format!("{base_name}_AddAlias.zip");
    let nb_feuilles = files.len();

    zip_and_respond(&state, &form, &files, zip_name, nb_feuilles).await
}

/// Valeur d'un champ texte du formulaire, ou la valeur par defaut s'il est absent/vide.
/// Les espaces internes sont conserves tels quels.
fn form_value(value: &Option<String>, default: &str) -> String {
    match value.as_deref() {
        Some(v) if !v.trim().is_empty() => v.trim_matches(['\r', '\n']).to_string(),
        _ => default.to_string(),
    }
}

async fn zip_and_respond(
    state: &AppState,
    form: &ConvertForm,
    files: &[(String, Vec<u8>)],
    zip_name: String,
    nb_feuilles: usize,
) -> Result<Response, AppError> {
    let zip_bytes = build_zip(files)?;

    let disk_name = format!("{}.zip", Uuid::new_v4().simple());
    let disk_path = state.storage_dir.join(disk_name);
    fs::write(&disk_path, &zip_bytes).await?;

    let history_id = insert_history(
        &state.db_path,
        &form.file_name,
        &zip_name,
        &disk_path.to_string_lossy(),
        nb_feuilles as i64,
        zip_bytes.len() as i64,
        &form.style_entete,
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
        HeaderValue::from_str(&nb_feuilles.to_string())?,
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

const CRPLMT_PREFIXE: &str = "CRPLMT";
const CRPLMT_MONTANT_DEFAUT: &str = "100000";
const ADD_ALIAS_TITRE_DEFAUT: &str = "Add Alias Title";
const ADD_ALIAS_DESCRIPTION_DEFAUT: &str = "Agent  advance pilote";

fn read_sheets(bytes: &[u8]) -> Result<Vec<(String, Vec<Vec<String>>)>> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut workbook = open_workbook_auto_from_rs(cursor)?;
    let sheet_names = workbook.sheet_names().to_owned();

    if sheet_names.is_empty() {
        return Err(anyhow!("Aucune feuille trouvee dans le fichier."));
    }

    let mut sheets = Vec::new();
    for sheet_name in sheet_names {
        let range = workbook.worksheet_range(&sheet_name)?;
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(clean_cell).collect::<Vec<_>>())
            .filter(|row| row.iter().any(|value| !value.trim().is_empty()))
            .collect();
        sheets.push((sheet_name, rows));
    }

    Ok(sheets)
}

fn excel_to_csvs(bytes: &[u8], style_entete: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let mut result = Vec::new();

    for (sheet_name, rows) in read_sheets(bytes)? {
        let parameter_name = find_most_frequent_parameter(&rows).unwrap_or_else(|| sheet_name.clone());
        let lines: Vec<String> = rows.iter().map(|row| row.join(",")).collect();
        let csv = build_csv(&lines, style_entete, &parameter_name);
        result.push((csv_file_name(&sheet_name, ""), with_bom(&csv)));
    }

    Ok(result)
}

/// Un fichier .txt AddAlias par feuille contenant des numeros
/// (fins de ligne Windows, sans BOM, comme le template d'import).
fn excel_to_add_alias_files(
    bytes: &[u8],
    base_name: &str,
    montant: &str,
    titre: &str,
    description: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut sheets = Vec::new();

    for (sheet_name, rows) in read_sheets(bytes)? {
        let msisdns = extract_msisdns(&rows)
            .map_err(|e| anyhow!("feuille \"{sheet_name}\" : {e}"))?;
        if !msisdns.is_empty() {
            sheets.push((sheet_name, msisdns));
        }
    }

    if sheets.is_empty() {
        return Err(anyhow!("Aucun numero trouve dans le fichier."));
    }

    let single = sheets.len() == 1;
    Ok(sheets
        .into_iter()
        .map(|(sheet_name, msisdns)| {
            let name = if single {
                format!("{base_name}.txt")
            } else {
                format!("{base_name}_{}.txt", sheet_name.replace(['/', '\\'], "-"))
            };
            let content = build_add_alias(&msisdns, montant, titre, description);
            (name, content.into_bytes())
        })
        .collect())
}

fn build_add_alias(msisdns: &[String], montant: &str, titre: &str, description: &str) -> String {
    let mut out = format!(
        "HDR,\"AddAlias\",\"{titre}\",\"ExtId12340\",\"{}\",\"{description}\",\"1\"\r\n",
        msisdns.len()
    );
    for msisdn in msisdns {
        out.push_str(&crplmt_line(msisdn, montant));
        out.push_str("\r\n");
    }
    out
}

/// Lit la premiere cellule non vide de chaque ligne. Une premiere ligne
/// non numerique est consideree comme le titre de la colonne et ignoree.
fn extract_msisdns(rows: &[Vec<String>]) -> Result<Vec<String>> {
    let mut msisdns = Vec::new();

    for (index, row) in rows.iter().enumerate() {
        let Some(value) = row.iter().map(|v| v.trim()).find(|v| !v.is_empty()) else {
            continue;
        };
        let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();

        if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
            msisdns.push(value);
        } else if index == 0 {
            // Titre de la colonne
        } else {
            return Err(anyhow!(
                "valeur \"{value}\" invalide a la ligne {} (un numero est attendu)",
                index + 1
            ));
        }
    }

    Ok(msisdns)
}

fn crplmt_line(msisdn: &str, montant: &str) -> String {
    format!("MSISDN,\"{msisdn}\",\"{CRPLMT_PREFIXE}_{msisdn}@{montant}\"")
}

fn build_csv(lines: &[String], style_entete: &str, parameter_name: &str) -> String {
    let mut csv = String::new();
    csv.push_str(&generate_hdr(lines.len(), style_entete, parameter_name));
    csv.push('\n');

    for line in lines {
        csv.push_str(line);
        csv.push('\n');
    }

    csv
}

fn csv_file_name(sheet_name: &str, suffix: &str) -> String {
    format!("{}{suffix}.csv", sheet_name.replace(['/', '\\'], "-"))
}

fn with_bom(csv: &str) -> Vec<u8> {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(csv.as_bytes());
    bytes
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

    #[test]
    fn test_crplmt_extract_msisdns_skips_title() {
        let rows = vec![
            vec!["Numeros".to_string()],
            vec!["237653282055".to_string()],
            vec!["237 681 178 408".to_string()],
        ];

        let msisdns = extract_msisdns(&rows).unwrap();
        assert_eq!(msisdns, vec!["237653282055", "237681178408"]);
    }

    #[test]
    fn test_crplmt_extract_msisdns_rejects_invalid_value() {
        let rows = vec![
            vec!["MSISDN".to_string()],
            vec!["237653282055".to_string()],
            vec!["abc".to_string()],
        ];

        assert!(extract_msisdns(&rows).is_err());
    }

    #[test]
    fn test_add_alias_matches_template() {
        let msisdns = vec!["237653282055".to_string(), "237679447586".to_string()];
        let out = build_add_alias(
            &msisdns,
            CRPLMT_MONTANT_DEFAUT,
            ADD_ALIAS_TITRE_DEFAUT,
            ADD_ALIAS_DESCRIPTION_DEFAUT,
        );
        assert_eq!(
            out,
            "HDR,\"AddAlias\",\"Add Alias Title\",\"ExtId12340\",\"2\",\"Agent  advance pilote\",\"1\"\r\n\
             MSISDN,\"237653282055\",\"CRPLMT_237653282055@100000\"\r\n\
             MSISDN,\"237679447586\",\"CRPLMT_237679447586@100000\"\r\n"
        );
    }
}
