# BackendRust — bibliothèque `mmc_batch_core`

Conversion des classeurs Excel et historique, utilisée directement par l'appli Tauri
(`Frontend/src-tauri`). Point d'entrée : `mmc_batch_core::convertir(&historique, demande)`.

## Serveur HTTP optionnel

Pour utiliser l'interface dans un navigateur (`npm run dev`), sans Tauri :

```powershell
cd BackendRust
cargo run
```

Routes :

- `POST /convert` (`multipart/form-data`) : `fichier`, `style_entete` (`Init` ou `Set`).
- `POST /convert/crplmt` : `fichier`, `montant` (défaut `100000`), `titre` (défaut `Add Alias Title`),
  `description` (défaut `Agent  advance pilote`).
- `GET /historique`, `GET /historique/{id}/telecharger`, `DELETE /historique/{id}`

Données à côté du binaire s'il est accessible en écriture, sinon dans `%LOCALAPPDATA%\MMC Batch`.
Variables : `MMC_STORAGE_DIR`, `MMC_DB_PATH`, `MMC_PORT` (défaut `8000`).

## Tests

```powershell
cargo test
```
