// Accès à la conversion et à l'historique.
//
// - Dans l'appli Tauri : commandes Rust (`src-tauri/src/lib.rs`), la conversion
//   tourne dans l'appli elle-même, sans serveur ni port.
// - Dans un navigateur (`npm run dev`) : serveur HTTP de BackendRust (`cargo run`).
import { invoke, isTauri } from '@tauri-apps/api/core'

const URL_SERVEUR = import.meta.env.VITE_API_URL || 'http://127.0.0.1:8000'
const dansTauri = isTauri()

// Tauri rejette avec le message d'erreur Rust (une chaîne) : on le transforme en Error.
async function appeler(commande, args) {
  try {
    return await invoke(commande, args)
  } catch (erreur) {
    throw new Error(typeof erreur === 'string' ? erreur : erreur?.message || String(erreur))
  }
}

async function requeteServeur(chemin, options) {
  let reponse
  try {
    reponse = await fetch(`${URL_SERVEUR}${chemin}`, options)
  } catch {
    throw new Error(
      `Impossible de joindre le serveur ${URL_SERVEUR}. Lance-le avec « cargo run » dans BackendRust.`
    )
  }
  if (!reponse.ok) {
    let detail = ''
    try {
      detail = (await reponse.json())?.detail || ''
    } catch {
      // réponse sans JSON
    }
    throw new Error(detail || `Erreur serveur (${reponse.status})`)
  }
  return reponse
}

function zipEnBlob(octets) {
  return new Blob([new Uint8Array(octets)], { type: 'application/zip' })
}

/**
 * Convertit un classeur Excel.
 * @returns {Promise<{ nomZip: string, nbFeuilles: number, blob: Blob }>}
 */
export async function convertir({ fichier, mode, styleEntete, montant, titre, description }) {
  if (dansTauri) {
    const contenu = Array.from(new Uint8Array(await fichier.arrayBuffer()))
    const resultat = await appeler('convertir', {
      demande: {
        nom_fichier: fichier.name,
        contenu,
        mode,
        style_entete: styleEntete,
        montant,
        titre,
        description
      }
    })
    return {
      nomZip: resultat.nom_zip,
      nbFeuilles: resultat.nb_feuilles,
      blob: zipEnBlob(resultat.zip)
    }
  }

  const formulaire = new FormData()
  formulaire.append('fichier', fichier)
  formulaire.append('style_entete', styleEntete)
  if (mode === 'crplmt') {
    formulaire.append('montant', montant)
    formulaire.append('titre', titre)
    formulaire.append('description', description)
  }
  const chemin = mode === 'crplmt' ? '/convert/crplmt' : '/convert'
  const reponse = await requeteServeur(chemin, { method: 'POST', body: formulaire })
  const disposition = reponse.headers.get('content-disposition') || ''
  return {
    nomZip: /filename=([^;]+)/.exec(disposition)?.[1]?.trim() || 'export.zip',
    nbFeuilles: Number(reponse.headers.get('x-nb-feuilles')) || 0,
    blob: await reponse.blob()
  }
}

export async function listerHistorique() {
  if (dansTauri) return appeler('lister_historique')
  return (await requeteServeur('/historique')).json()
}

/** @returns {Promise<Blob>} le zip d'une conversion passée */
export async function lireZipHistorique(id) {
  if (dansTauri) return zipEnBlob((await appeler('lire_zip_historique', { id })).contenu)
  return (await requeteServeur(`/historique/${id}/telecharger`)).blob()
}

export async function supprimerHistorique(id) {
  if (dansTauri) return appeler('supprimer_historique', { id })
  await requeteServeur(`/historique/${id}`, { method: 'DELETE' })
}
