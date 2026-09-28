//! Conversion des classeurs Excel de MMC Batch et historique des conversions.
//!
//! Utilise directement par l'appli Tauri (commandes) et par le serveur HTTP
//! optionnel (`src/main.rs`, pour travailler sur l'interface dans un navigateur).

use std::{
    collections::HashMap,
    fmt,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use calamine::{open_workbook_auto_from_rs, Data, Reader};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub const CRPLMT_MONTANT_DEFAUT: &str = "100000";
pub const ADD_ALIAS_TITRE_DEFAUT: &str = "Add Alias Title";
pub const ADD_ALIAS_DESCRIPTION_DEFAUT: &str = "Agent  advance pilote";
const CRPLMT_PREFIXE: &str = "CRPLMT";

// ---------------------------------------------------------------------------
// Erreurs
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Erreur {
    /// Donnees envoyees invalides (fichier, montant...) : a corriger par l'utilisateur.
    Invalide(String),
    /// Entree d'historique ou fichier introuvable.
    Introuvable(String),
    /// Probleme inattendu (disque, base de donnees...).
    Interne(String),
}

impl fmt::Display for Erreur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erreur::Invalide(m) | Erreur::Introuvable(m) | Erreur::Interne(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Erreur {}

impl From<rusqlite::Error> for Erreur {
    fn from(e: rusqlite::Error) -> Self {
        Erreur::Interne(format!("Base de donnees : {e}"))
    }
}

impl From<std::io::Error> for Erreur {
    fn from(e: std::io::Error) -> Self {
        Erreur::Interne(format!("Fichier : {e}"))
    }
}

impl From<zip::result::ZipError> for Erreur {
    fn from(e: zip::result::ZipError) -> Self {
        Erreur::Interne(format!("Creation du zip : {e}"))
    }
}

pub type Resultat<T> = std::result::Result<T, Erreur>;

// ---------------------------------------------------------------------------
// Demande et resultat de conversion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Classeur deja au format `MSISDN,...,Parametre` : ajoute l'entete
    /// `UpdateIndividualRatingParameter` (Init ou Set), un CSV par feuille.
    #[default]
    Standard,
    /// Colonne de numeros : produit le fichier `AddAlias` (.txt) avec une ligne
    /// `MSISDN,"<numero>","CRPLMT_<numero>@<montant>"` par numero.
    Crplmt,
}

#[derive(Debug, Deserialize)]
pub struct Demande {
    pub nom_fichier: String,
    pub contenu: Vec<u8>,
    #[serde(default)]
    pub mode: Mode,
    /// Mode standard : "Init" (defaut) ou "Set".
    #[serde(default)]
    pub style_entete: Option<String>,
    /// Mode CRPLMT.
    #[serde(default)]
    pub montant: Option<String>,
    #[serde(default)]
    pub titre: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Conversion {
    pub nom_zip: String,
    pub nb_feuilles: usize,
    pub historique_id: i64,
    pub zip: Vec<u8>,
}

/// Convertit le classeur, enregistre le zip dans l'historique et le renvoie.
pub fn convertir(historique: &Historique, demande: Demande) -> Resultat<Conversion> {
    if !a_extension_excel(&demande.nom_fichier) {
        return Err(Erreur::Invalide(
            "Le fichier doit etre un .xlsx ou .xls".into(),
        ));
    }
    let base = sans_extension(&demande.nom_fichier);

    let (fichiers, nom_zip, type_entete) = match demande.mode {
        Mode::Standard => {
            let style = if demande.style_entete.as_deref() == Some("Set") {
                "Set"
            } else {
                "Init"
            };
            let fichiers = fichiers_standard(&demande.contenu, style)?;
            (fichiers, format!("{base}_csv.zip"), style.to_string())
        }
        Mode::Crplmt => {
            let montant = valeur_ou_defaut(&demande.montant, CRPLMT_MONTANT_DEFAUT);
            if !montant.chars().all(|c| c.is_ascii_digit()) {
                return Err(Erreur::Invalide(
                    "Le montant doit etre un nombre entier (ex. 100000).".into(),
                ));
            }
            let titre = valeur_ou_defaut(&demande.titre, ADD_ALIAS_TITRE_DEFAUT);
            let description = valeur_ou_defaut(&demande.description, ADD_ALIAS_DESCRIPTION_DEFAUT);
            if titre.contains('"') || description.contains('"') {
                return Err(Erreur::Invalide(
                    "Le titre et la description ne doivent pas contenir de guillemets.".into(),
                ));
            }
            let fichiers =
                fichiers_add_alias(&demande.contenu, base, &montant, &titre, &description)?;
            (fichiers, format!("{base}_AddAlias.zip"), "AddAlias".to_string())
        }
    };

    let zip = construire_zip(&fichiers)?;
    let historique_id =
        historique.enregistrer(&demande.nom_fichier, &nom_zip, &zip, fichiers.len(), &type_entete)?;

    Ok(Conversion {
        nom_zip,
        nb_feuilles: fichiers.len(),
        historique_id,
        zip,
    })
}

/// Valeur d'un champ texte, ou la valeur par defaut s'il est absent ou vide.
/// Les espaces internes sont conserves (ex. "Agent  advance pilote").
fn valeur_ou_defaut(valeur: &Option<String>, defaut: &str) -> String {
    match valeur.as_deref() {
        Some(v) if !v.trim().is_empty() => v.trim_matches(['\r', '\n']).to_string(),
        _ => defaut.to_string(),
    }
}

fn a_extension_excel(nom: &str) -> bool {
    let nom = nom.to_ascii_lowercase();
    nom.ends_with(".xlsx") || nom.ends_with(".xls")
}

fn sans_extension(nom: &str) -> &str {
    nom.rsplit_once('.').map(|(base, _)| base).unwrap_or(nom)
}

// ---------------------------------------------------------------------------
// Lecture Excel
// ---------------------------------------------------------------------------

type Feuille = (String, Vec<Vec<String>>);

/// Lit toutes les feuilles ; les lignes entierement vides sont ignorees.
fn lire_feuilles(contenu: &[u8]) -> Resultat<Vec<Feuille>> {
    let illisible = |e: &dyn fmt::Display| Erreur::Invalide(format!("Fichier Excel illisible : {e}"));

    let mut classeur =
        open_workbook_auto_from_rs(Cursor::new(contenu.to_vec())).map_err(|e| illisible(&e))?;
    let noms = classeur.sheet_names().to_owned();
    if noms.is_empty() {
        return Err(Erreur::Invalide("Aucune feuille trouvee dans le fichier.".into()));
    }

    let mut feuilles = Vec::new();
    for nom in noms {
        let plage = classeur.worksheet_range(&nom).map_err(|e| illisible(&e))?;
        let lignes = plage
            .rows()
            .map(|ligne| ligne.iter().map(nettoyer_cellule).collect::<Vec<_>>())
            .filter(|ligne| ligne.iter().any(|v| !v.trim().is_empty()))
            .collect();
        feuilles.push((nom, lignes));
    }
    Ok(feuilles)
}

fn nettoyer_cellule(cellule: &Data) -> String {
    let valeur = match cellule {
        Data::Empty => String::new(),
        Data::String(v) => v.clone(),
        Data::Float(v) if v.fract() == 0.0 => format!("{v:.0}"),
        Data::Float(v) => v.to_string(),
        Data::Int(v) => v.to_string(),
        Data::Bool(v) => v.to_string(),
        Data::DateTime(v) => v.to_string(),
        Data::DateTimeIso(v) | Data::DurationIso(v) => v.clone(),
        Data::Error(v) => v.to_string(),
    };
    valeur.replace(['\r', '\n'], " ").trim().to_string()
}

fn nom_de_fichier(nom: &str) -> String {
    nom.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-")
}

// ---------------------------------------------------------------------------
// Mode standard : entete UpdateIndividualRatingParameter
// ---------------------------------------------------------------------------

fn fichiers_standard(contenu: &[u8], style: &str) -> Resultat<Vec<(String, Vec<u8>)>> {
    let mut fichiers = Vec::new();
    for (feuille, lignes) in lire_feuilles(contenu)? {
        let parametre = parametre_le_plus_frequent(&lignes).unwrap_or_else(|| feuille.clone());

        let mut csv = entete_standard(lignes.len(), style, &parametre);
        csv.push('\n');
        for ligne in &lignes {
            csv.push_str(&ligne.join(","));
            csv.push('\n');
        }

        // BOM UTF-8, comme les fichiers produits jusqu'ici
        let mut octets = vec![0xEF, 0xBB, 0xBF];
        octets.extend_from_slice(csv.as_bytes());
        fichiers.push((format!("{}.csv", nom_de_fichier(&feuille)), octets));
    }
    Ok(fichiers)
}

fn entete_standard(nb_lignes: usize, style: &str, parametre: &str) -> String {
    if style == "Set" {
        format!(
            "HDR,UpdateIndividualRatingParameter,Set {parametre},ExtId12341,{nb_lignes},{parametre} enable,1"
        )
    } else {
        format!(
            "HDR,\"UpdateIndividualRatingParameter\",\"Init {parametre} TRUE\",\"ExtId12340\",\"{nb_lignes}\",\" {parametre} init\",\"1\""
        )
    }
}

/// Le parametre est la 3e valeur de chaque ligne (`MSISDN,<numero>,<parametre>,...`),
/// y compris quand toute la ligne est dans une seule cellule.
fn parametre_le_plus_frequent(lignes: &[Vec<String>]) -> Option<String> {
    let mut compte: HashMap<String, usize> = HashMap::new();
    for ligne in lignes {
        let complete = ligne.join(",");
        let parametre = complete
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim())
            .nth(2)
            .filter(|p| !p.is_empty());
        if let Some(p) = parametre {
            *compte.entry(p.to_string()).or_insert(0) += 1;
        }
    }
    // A egalite, ordre alphabetique pour un resultat stable
    compte
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(p, _)| p)
}

// ---------------------------------------------------------------------------
// Mode CRPLMT : fichier AddAlias
// ---------------------------------------------------------------------------

/// Un .txt AddAlias par feuille contenant des numeros (CRLF, sans BOM,
/// comme le template d'import).
fn fichiers_add_alias(
    contenu: &[u8],
    base: &str,
    montant: &str,
    titre: &str,
    description: &str,
) -> Resultat<Vec<(String, Vec<u8>)>> {
    let mut feuilles = Vec::new();
    for (feuille, lignes) in lire_feuilles(contenu)? {
        let numeros = extraire_numeros(&lignes)
            .map_err(|e| Erreur::Invalide(format!("Feuille \"{feuille}\" : {e}")))?;
        if !numeros.is_empty() {
            feuilles.push((feuille, numeros));
        }
    }

    if feuilles.is_empty() {
        return Err(Erreur::Invalide("Aucun numero trouve dans le fichier.".into()));
    }

    let une_seule = feuilles.len() == 1;
    Ok(feuilles
        .into_iter()
        .map(|(feuille, numeros)| {
            let nom = if une_seule {
                format!("{}.txt", nom_de_fichier(base))
            } else {
                format!("{}_{}.txt", nom_de_fichier(base), nom_de_fichier(&feuille))
            };
            (nom, add_alias(&numeros, montant, titre, description).into_bytes())
        })
        .collect())
}

/// Premiere cellule non vide de chaque ligne. Une premiere ligne non numerique
/// est le titre de la colonne ; toute autre valeur non numerique est refusee.
fn extraire_numeros(lignes: &[Vec<String>]) -> std::result::Result<Vec<String>, String> {
    let mut numeros = Vec::new();
    for (index, ligne) in lignes.iter().enumerate() {
        let Some(valeur) = ligne.iter().map(|v| v.trim()).find(|v| !v.is_empty()) else {
            continue;
        };
        let valeur: String = valeur.chars().filter(|c| !c.is_whitespace()).collect();

        if !valeur.is_empty() && valeur.chars().all(|c| c.is_ascii_digit()) {
            numeros.push(valeur);
        } else if index > 0 {
            return Err(format!(
                "valeur \"{valeur}\" invalide a la ligne {} (un numero est attendu)",
                index + 1
            ));
        }
    }
    Ok(numeros)
}

fn add_alias(numeros: &[String], montant: &str, titre: &str, description: &str) -> String {
    let mut texte = format!(
        "HDR,\"AddAlias\",\"{titre}\",\"ExtId12340\",\"{}\",\"{description}\",\"1\"\r\n",
        numeros.len()
    );
    for numero in numeros {
        texte.push_str(&format!(
            "MSISDN,\"{numero}\",\"{CRPLMT_PREFIXE}_{numero}@{montant}\"\r\n"
        ));
    }
    texte
}

// ---------------------------------------------------------------------------
// Zip
// ---------------------------------------------------------------------------

fn construire_zip(fichiers: &[(String, Vec<u8>)]) -> Resultat<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (nom, octets) in fichiers {
        zip.start_file(nom.as_str(), options)?;
        zip.write_all(octets)?;
    }
    Ok(zip.finish()?.into_inner())
}

// ---------------------------------------------------------------------------
// Historique (SQLite + zips sur disque)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EntreeHistorique {
    pub id: i64,
    pub nom_original: String,
    pub nom_zip: String,
    pub chemin_zip: String,
    pub nb_feuilles: i64,
    pub taille_octets: i64,
    pub style_entete: String,
    pub date_creation: String,
}

#[derive(Debug, Clone)]
pub struct Historique {
    dossier_zips: PathBuf,
    base: PathBuf,
}

impl Historique {
    /// Historique dans `dossier` : `dossier/stockage/*.zip` et `dossier/historique.db`.
    pub fn ouvrir(dossier: &Path) -> Resultat<Self> {
        Self::avec_chemins(dossier.join("stockage"), dossier.join("historique.db"))
    }

    pub fn avec_chemins(dossier_zips: PathBuf, base: PathBuf) -> Resultat<Self> {
        std::fs::create_dir_all(&dossier_zips).map_err(|e| {
            Erreur::Interne(format!(
                "Impossible de creer le dossier {} : {e}",
                dossier_zips.display()
            ))
        })?;
        if let Some(parent) = base.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let historique = Historique { dossier_zips, base };
        historique.connexion()?.execute(
            "CREATE TABLE IF NOT EXISTS historique (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                nom_original TEXT NOT NULL,
                nom_zip TEXT NOT NULL,
                chemin_zip TEXT NOT NULL,
                nb_feuilles INTEGER NOT NULL,
                taille_octets INTEGER NOT NULL,
                style_entete TEXT NOT NULL,
                date_creation TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;
        Ok(historique)
    }

    pub fn dossier_zips(&self) -> &Path {
        &self.dossier_zips
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    fn connexion(&self) -> Resultat<Connection> {
        Connection::open(&self.base).map_err(|e| {
            Erreur::Interne(format!(
                "Impossible d'ouvrir la base {} : {e}",
                self.base.display()
            ))
        })
    }

    fn enregistrer(
        &self,
        nom_original: &str,
        nom_zip: &str,
        zip: &[u8],
        nb_feuilles: usize,
        style_entete: &str,
    ) -> Resultat<i64> {
        let chemin = self
            .dossier_zips
            .join(format!("{}.zip", Uuid::new_v4().simple()));
        std::fs::write(&chemin, zip)?;

        let connexion = self.connexion()?;
        connexion.execute(
            "INSERT INTO historique
             (nom_original, nom_zip, chemin_zip, nb_feuilles, taille_octets, style_entete)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                nom_original,
                nom_zip,
                chemin.to_string_lossy(),
                nb_feuilles as i64,
                zip.len() as i64,
                style_entete
            ],
        )?;
        Ok(connexion.last_insert_rowid())
    }

    pub fn lister(&self, limite: i64) -> Resultat<Vec<EntreeHistorique>> {
        let connexion = self.connexion()?;
        let mut requete = connexion
            .prepare("SELECT * FROM historique ORDER BY date_creation DESC, id DESC LIMIT ?1")?;
        let entrees = requete
            .query_map(params![limite.clamp(1, 500)], entree_depuis_ligne)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entrees)
    }

    fn entree(&self, id: i64) -> Resultat<EntreeHistorique> {
        let connexion = self.connexion()?;
        let mut requete = connexion.prepare("SELECT * FROM historique WHERE id = ?1")?;
        match requete.query_row(params![id], entree_depuis_ligne) {
            Ok(entree) => Ok(entree),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Err(Erreur::Introuvable("Entree introuvable.".into()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Nom et contenu du zip d'une conversion passee.
    pub fn lire_zip(&self, id: i64) -> Resultat<(String, Vec<u8>)> {
        let entree = self.entree(id)?;
        let octets = std::fs::read(&entree.chemin_zip).map_err(|_| {
            Erreur::Introuvable("Le fichier n'existe plus sur le disque.".into())
        })?;
        Ok((entree.nom_zip, octets))
    }

    pub fn supprimer(&self, id: i64) -> Resultat<()> {
        let entree = self.entree(id)?;
        let _ = std::fs::remove_file(&entree.chemin_zip);
        self.connexion()?
            .execute("DELETE FROM historique WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn entree_depuis_ligne(ligne: &rusqlite::Row<'_>) -> rusqlite::Result<EntreeHistorique> {
    Ok(EntreeHistorique {
        id: ligne.get("id")?,
        nom_original: ligne.get("nom_original")?,
        nom_zip: ligne.get("nom_zip")?,
        chemin_zip: ligne.get("chemin_zip")?,
        nb_feuilles: ligne.get("nb_feuilles")?,
        taille_octets: ligne.get("taille_octets")?,
        style_entete: ligne.get("style_entete")?,
        date_creation: ligne.get("date_creation")?,
    })
}

// ---------------------------------------------------------------------------
// Tests unitaires
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn lignes(valeurs: &[&[&str]]) -> Vec<Vec<String>> {
        valeurs
            .iter()
            .map(|l| l.iter().map(|v| v.to_string()).collect())
            .collect()
    }

    #[test]
    fn parametre_en_troisieme_colonne() {
        let l = lignes(&[
            &["ID", "14824152", "EleCashback"],
            &["ID", "14824153", "EleCashback"],
        ]);
        assert_eq!(parametre_le_plus_frequent(&l).as_deref(), Some("EleCashback"));
        assert_eq!(
            entete_standard(l.len(), "Set", "EleCashback"),
            "HDR,UpdateIndividualRatingParameter,Set EleCashback,ExtId12341,2,EleCashback enable,1"
        );
    }

    #[test]
    fn parametre_ligne_dans_une_seule_cellule() {
        let l = lignes(&[
            &["MSISDN,237674497307,AgentFloatRegion,SouthWest"],
            &["MSISDN,237674931273,AgentFloatRegion,West"],
        ]);
        assert_eq!(parametre_le_plus_frequent(&l).as_deref(), Some("AgentFloatRegion"));
        assert_eq!(
            entete_standard(l.len(), "Init", "AgentFloatRegion"),
            "HDR,\"UpdateIndividualRatingParameter\",\"Init AgentFloatRegion TRUE\",\"ExtId12340\",\"2\",\" AgentFloatRegion init\",\"1\""
        );
    }

    #[test]
    fn numeros_titre_ignore_et_espaces_retires() {
        let l = lignes(&[&["msisdn"], &["237653282055"], &["237 681 178 408"]]);
        assert_eq!(
            extraire_numeros(&l).unwrap(),
            vec!["237653282055", "237681178408"]
        );
    }

    #[test]
    fn numeros_valeur_invalide_refusee() {
        let l = lignes(&[&["msisdn"], &["237653282055"], &["abc"]]);
        assert!(extraire_numeros(&l).unwrap_err().contains("ligne 3"));
    }

    #[test]
    fn add_alias_identique_au_template() {
        let numeros = vec!["237653282055".to_string(), "237679447586".to_string()];
        assert_eq!(
            add_alias(
                &numeros,
                CRPLMT_MONTANT_DEFAUT,
                ADD_ALIAS_TITRE_DEFAUT,
                ADD_ALIAS_DESCRIPTION_DEFAUT
            ),
            "HDR,\"AddAlias\",\"Add Alias Title\",\"ExtId12340\",\"2\",\"Agent  advance pilote\",\"1\"\r\n\
             MSISDN,\"237653282055\",\"CRPLMT_237653282055@100000\"\r\n\
             MSISDN,\"237679447586\",\"CRPLMT_237679447586@100000\"\r\n"
        );
    }
}
