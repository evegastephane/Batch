//! Tests de bout en bout de `convertir`, avec la demande au format JSON envoye
//! par l'interface a la commande Tauri `convertir`.

use std::io::{Cursor, Read};

use mmc_batch_core::{convertir, Demande, Historique};

fn historique_temporaire(nom: &str) -> Historique {
    let dossier = std::env::temp_dir().join(format!("mmc-batch-test-{nom}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dossier);
    Historique::ouvrir(&dossier).unwrap()
}

fn demande(json: serde_json::Value, fixture: &str) -> Demande {
    let mut json = json;
    let contenu = std::fs::read(format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    json["contenu"] = serde_json::json!(contenu);
    serde_json::from_value(json).unwrap()
}

fn fichiers_du_zip(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut fichier = archive.by_index(i).unwrap();
            let mut octets = Vec::new();
            fichier.read_to_end(&mut octets).unwrap();
            (fichier.name().to_string(), octets)
        })
        .collect()
}

#[test]
fn crplmt_produit_le_fichier_add_alias() {
    let historique = historique_temporaire("crplmt");
    let conversion = convertir(
        &historique,
        demande(
            serde_json::json!({
                "nom_fichier": "MoMo_Test.xlsx",
                "mode": "crplmt",
                "style_entete": "Init",
                "montant": "100000",
                "titre": "Add Alias Title",
                "description": "Agent  advance pilote"
            }),
            "msisdn.xlsx",
        ),
    )
    .unwrap();

    assert_eq!(conversion.nom_zip, "MoMo_Test_AddAlias.zip");
    assert_eq!(conversion.nb_feuilles, 1);

    let fichiers = fichiers_du_zip(&conversion.zip);
    assert_eq!(fichiers.len(), 1);
    assert_eq!(fichiers[0].0, "MoMo_Test.txt");
    assert_eq!(
        String::from_utf8(fichiers[0].1.clone()).unwrap(),
        "HDR,\"AddAlias\",\"Add Alias Title\",\"ExtId12340\",\"3\",\"Agent  advance pilote\",\"1\"\r\n\
         MSISDN,\"237600000001\",\"CRPLMT_237600000001@100000\"\r\n\
         MSISDN,\"237600000002\",\"CRPLMT_237600000002@100000\"\r\n\
         MSISDN,\"237600000003\",\"CRPLMT_237600000003@100000\"\r\n"
    );

    // Historique : listee, relisible puis supprimable
    let entrees = historique.lister(50).unwrap();
    assert_eq!(entrees.len(), 1);
    assert_eq!(entrees[0].id, conversion.historique_id);
    assert_eq!(entrees[0].style_entete, "AddAlias");
    let (nom, octets) = historique.lire_zip(conversion.historique_id).unwrap();
    assert_eq!(nom, "MoMo_Test_AddAlias.zip");
    assert_eq!(octets, conversion.zip);
    historique.supprimer(conversion.historique_id).unwrap();
    assert!(historique.lister(50).unwrap().is_empty());
}

#[test]
fn crplmt_valeurs_par_defaut_si_champs_absents() {
    let historique = historique_temporaire("defauts");
    let conversion = convertir(
        &historique,
        demande(serde_json::json!({ "nom_fichier": "liste.xlsx", "mode": "crplmt" }), "msisdn.xlsx"),
    )
    .unwrap();
    let texte = String::from_utf8(fichiers_du_zip(&conversion.zip)[0].1.clone()).unwrap();
    assert!(texte.starts_with(
        "HDR,\"AddAlias\",\"Add Alias Title\",\"ExtId12340\",\"3\",\"Agent  advance pilote\",\"1\"\r\n"
    ));
    assert!(texte.contains("CRPLMT_237600000001@100000"));
}

#[test]
fn crplmt_montant_invalide_refuse() {
    let historique = historique_temporaire("montant");
    let erreur = convertir(
        &historique,
        demande(
            serde_json::json!({ "nom_fichier": "liste.xlsx", "mode": "crplmt", "montant": "10a" }),
            "msisdn.xlsx",
        ),
    )
    .unwrap_err();
    assert!(erreur.to_string().contains("montant"));
}

#[test]
fn standard_ajoute_l_entete_update_individual_rating_parameter() {
    let historique = historique_temporaire("standard");
    let conversion = convertir(
        &historique,
        demande(
            serde_json::json!({ "nom_fichier": "regions.xlsx", "mode": "standard", "style_entete": "Set" }),
            "standard.xlsx",
        ),
    )
    .unwrap();

    assert_eq!(conversion.nom_zip, "regions_csv.zip");
    let fichiers = fichiers_du_zip(&conversion.zip);
    assert_eq!(fichiers[0].0, "Regions.csv");
    assert_eq!(
        String::from_utf8(fichiers[0].1.clone()).unwrap(),
        "\u{feff}HDR,UpdateIndividualRatingParameter,Set AgentFloatRegion,ExtId12341,2,AgentFloatRegion enable,1\n\
         MSISDN,237600000001,AgentFloatRegion,West\n\
         MSISDN,237600000002,AgentFloatRegion,North\n"
    );
}

#[test]
fn fichier_non_excel_refuse() {
    let historique = historique_temporaire("extension");
    let erreur = convertir(
        &historique,
        demande(serde_json::json!({ "nom_fichier": "notes.txt" }), "msisdn.xlsx"),
    )
    .unwrap_err();
    assert!(erreur.to_string().contains(".xlsx"));
}
