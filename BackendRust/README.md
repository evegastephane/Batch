# Backend Rust MMC Batch

Backend compatible avec le frontend existant. Il remplace l'API FastAPI en exposant les memes routes sur `127.0.0.1:8000`.

## Lancer

```powershell
cd BackendRust
cargo run
```

Routes principales :

- `POST /convert` avec `multipart/form-data` : champs `fichier` et `style_entete`.
- `POST /convert/crplmt` avec `multipart/form-data` : champs `fichier`, `montant` (defaut `100000`),
  `titre` (defaut `Add Alias Title`) et `description` (defaut `Agent  advance pilote`).
  Le fichier Excel contient une seule colonne (un titre puis un numero par ligne). Sortie : un zip avec
  `<nom>.txt` (CRLF, sans BOM) :
  `HDR,"AddAlias","Add Alias Title","ExtId12340","<nb>","Agent  advance pilote","1"` puis une ligne
  `MSISDN,"237653282055","CRPLMT_237653282055@100000"` par numero.
- `GET /historique`
- `GET /historique/{id}/telecharger`
- `DELETE /historique/{id}`

Par defaut, les zips et la base SQLite sont crees a cote du binaire s'il est accessible en ecriture,
sinon dans `%LOCALAPPDATA%\MMC Batch`. Dans l'appli Tauri, ils sont dans `%APPDATA%\com.mmc.batch`
(avec `backend.log`), et le port est 8000 s'il est libre, sinon un port libre. Variables disponibles :

```powershell
$env:MMC_STORAGE_DIR = "C:\chemin\stockage"
$env:MMC_DB_PATH = "C:\chemin\historique.db"
$env:MMC_PORT = "8000"
cargo run
```
