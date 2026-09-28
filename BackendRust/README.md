# Backend Rust MMC Batch

Backend compatible avec le frontend existant. Il remplace l'API FastAPI en exposant les memes routes sur `127.0.0.1:8000`.

## Lancer

```powershell
cd BackendRust
cargo run
```

Routes principales :

- `POST /convert` avec `multipart/form-data` : champs `fichier` et `style_entete`.
- `GET /historique`
- `GET /historique/{id}/telecharger`
- `DELETE /historique/{id}`

Par defaut, les zips et la base SQLite sont crees a cote du binaire. Tu peux changer ces chemins avec :

```powershell
$env:MMC_STORAGE_DIR = "C:\chemin\stockage"
$env:MMC_DB_PATH = "C:\chemin\historique.db"
cargo run
```
