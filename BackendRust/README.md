# Backend Rust MMC Batch

Backend compatible avec le frontend existant. Il remplace l'API FastAPI en exposant les memes routes sur `127.0.0.1:8000`.

## Lancer

```powershell
cd BackendRust
cargo run
```

Routes principales :

- `POST /convert` avec `multipart/form-data` : champs `fichier` et `style_entete`.
- `POST /convert/crplmt` avec `multipart/form-data` : champs `fichier`, `style_entete` et `montant` (defaut `100000`).
  Le fichier contient une seule colonne (un titre puis un numero par ligne). Chaque numero devient
  `MSISDN,"237653282055","CRPLMT_237653282055@100000"`, puis le process habituel ajoute l'entete HDR
  avec le parametre `CRPLMT`. Le zip contient `<feuille>.csv` (final) et `<feuille>_intermediaire.csv`.
- `GET /historique`
- `GET /historique/{id}/telecharger`
- `DELETE /historique/{id}`

Par defaut, les zips et la base SQLite sont crees a cote du binaire. Tu peux changer ces chemins avec :

```powershell
$env:MMC_STORAGE_DIR = "C:\chemin\stockage"
$env:MMC_DB_PATH = "C:\chemin\historique.db"
cargo run
```
