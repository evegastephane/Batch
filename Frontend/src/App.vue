<script setup>
import { ref, computed, onMounted } from 'vue'
import {
  FileSpreadsheet,
  UploadCloud,
  Loader2,
  CircleCheck,
  Download,
  X,
  TriangleAlert,
  RotateCcw,
  AlignLeft,
  Rows3,
  Table2,
  Phone
} from 'lucide-vue-next'
import EnTete from './components/EnTete.vue'
import BarreLaterale from './components/BarreLaterale.vue'
import NotificationToast from './components/NotificationToast.vue'
import { convertir, listerHistorique, lireZipHistorique, supprimerHistorique } from './api'

// --- Options de conversion ---------------------------------------------------

const modes = [
  {
    valeur: 'standard',
    label: 'Standard',
    description: 'Classeur déjà au format MSISDN,…,Paramètre',
    icone: Table2
  },
  {
    valeur: 'crplmt',
    label: 'CRPLMT',
    description: 'Colonne de numéros → fichier AddAlias .txt (MSISDN,"numéro","CRPLMT_numéro@montant")',
    icone: Phone
  }
]

const options = [
  { valeur: 'Init', label: 'Init', description: 'Initiation', icone: AlignLeft },
  { valeur: 'Set', label: 'Set', description: 'Modification', icone: Rows3 }
]

const modeConversion = ref('standard') // standard | crplmt
const styleEntete = ref('Init')
const montant = ref('100000')
const titreAlias = ref('Add Alias Title')
const descriptionAlias = ref('Agent  advance pilote')

const montantValide = computed(() => /^\d+$/.test(montant.value.trim()))

// --- État de l'écran -----------------------------------------------------------

const etat = ref('repos') // repos | survol | conversion | termine | erreur
const fichier = ref(null)
const inputFichier = ref(null)
const nomZip = ref('')
const tailleZipKo = ref(0)
const nbFeuilles = ref(0)
const messageErreur = ref('')
const urlTelechargement = ref(null)
const progression = ref(0)
const statutProgression = ref('')

const formatKo = (octets) => (octets / 1024).toFixed(0)
const pourcentageRestant = computed(() => Math.max(0, 100 - progression.value))
const libelleZone = computed(() =>
  etat.value === 'survol' ? 'Relâche pour déposer' : 'Glisse ton fichier ici, ou clique pour parcourir'
)

// --- Historique ------------------------------------------------------------------

const sidebarOuvert = ref(false)
const historique = ref([])
const chargementHistorique = ref(false)

async function chargerHistorique() {
  chargementHistorique.value = true
  try {
    historique.value = await listerHistorique()
  } catch {
    // l'historique est secondaire : la conversion reste possible
  } finally {
    chargementHistorique.value = false
  }
}

async function telechargerDepuisHistorique(entree) {
  try {
    enregistrerFichier(await lireZipHistorique(entree.id), entree.nom_zip)
    afficherNotification(entree.nom_zip, 'Téléchargement depuis l\'historique lancé.')
  } catch (erreur) {
    afficherNotification(entree.nom_zip, erreur.message)
  }
}

async function supprimerDeHistorique(entree) {
  historique.value = historique.value.filter((e) => e.id !== entree.id)
  try {
    await supprimerHistorique(entree.id)
  } catch {
    chargerHistorique()
  }
}

onMounted(chargerHistorique)

// --- Notification ------------------------------------------------------------------

const notificationVisible = ref(false)
const notificationFichier = ref('')
const notificationMessage = ref('')
let timerNotification = null

function afficherNotification(nomFichier, message) {
  notificationFichier.value = nomFichier
  notificationMessage.value = message
  notificationVisible.value = true
  clearTimeout(timerNotification)
  timerNotification = setTimeout(() => {
    notificationVisible.value = false
  }, 4000)
}

function fermerNotification() {
  notificationVisible.value = false
  clearTimeout(timerNotification)
}

// --- Sélection du fichier -----------------------------------------------------------

function ouvrirSelecteur() {
  inputFichier.value?.click()
}

function onSurvolEntree(e) {
  e.preventDefault()
  if (etat.value === 'repos') etat.value = 'survol'
}

function onSurvolSortie(e) {
  e.preventDefault()
  if (etat.value === 'survol') etat.value = 'repos'
}

function onDepot(e) {
  e.preventDefault()
  const f = e.dataTransfer?.files?.[0]
  if (f) traiterFichier(f)
  else if (etat.value === 'survol') etat.value = 'repos'
}

function onSelection(e) {
  const f = e.target.files?.[0]
  if (f) traiterFichier(f)
}

// --- Conversion ------------------------------------------------------------------------

function afficherErreur(message) {
  messageErreur.value = message
  etat.value = 'erreur'
}

async function traiterFichier(f) {
  if (!/\.(xlsx|xls)$/i.test(f.name)) {
    return afficherErreur('Format non pris en charge. Dépose un fichier .xlsx ou .xls.')
  }
  if (modeConversion.value === 'crplmt' && !montantValide.value) {
    return afficherErreur('Le montant doit être un nombre entier (ex. 100000).')
  }

  fichier.value = f
  etat.value = 'conversion'
  const arreterProgression = demarrerProgression()

  try {
    const resultat = await convertir({
      fichier: f,
      mode: modeConversion.value,
      styleEntete: styleEntete.value,
      montant: montant.value.trim(),
      titre: titreAlias.value,
      description: descriptionAlias.value
    })
    await arreterProgression(true)

    nomZip.value = resultat.nomZip
    nbFeuilles.value = resultat.nbFeuilles
    tailleZipKo.value = resultat.blob.size
    urlTelechargement.value = URL.createObjectURL(resultat.blob)
    etat.value = 'termine'
    chargerHistorique()
  } catch (erreur) {
    await arreterProgression(false)
    afficherErreur(erreur.message || 'La conversion a échoué. Réessaie.')
  }
}

// Barre de progression indicative pendant la conversion (qui ne donne pas
// d'avancement réel) ; complétée jusqu'à 100 % quand le résultat arrive.
function demarrerProgression() {
  progression.value = 1
  statutProgression.value = 'Lecture du fichier'

  const timer = setInterval(() => {
    if (progression.value < 25) {
      progression.value += 1
      statutProgression.value = 'Lecture du fichier'
    } else if (progression.value < 55) {
      progression.value += 1
      statutProgression.value = 'Analyse des données'
    } else if (progression.value < 85) {
      progression.value += 1
      statutProgression.value = 'Conversion des feuilles'
    } else if (progression.value < 96) {
      if (Math.random() > 0.35) progression.value += 1
      statutProgression.value = 'Compression du zip'
    }
  }, 45)

  return async (reussi) => {
    clearInterval(timer)
    if (!reussi) return
    statutProgression.value = 'Finalisation'
    while (progression.value < 100) {
      progression.value += 1
      await new Promise((r) => setTimeout(r, 20))
    }
    statutProgression.value = 'Terminé'
    await new Promise((r) => setTimeout(r, 350))
  }
}

// --- Téléchargement ---------------------------------------------------------------------

function enregistrerFichier(blob, nom) {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = nom
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 60_000)
}

function telecharger() {
  const a = document.createElement('a')
  a.href = urlTelechargement.value
  a.download = nomZip.value
  a.click()
  afficherNotification(nomZip.value, 'Téléchargement de l\'archive ZIP lancé.')
}

function reinitialiser() {
  if (urlTelechargement.value) URL.revokeObjectURL(urlTelechargement.value)
  fichier.value = null
  urlTelechargement.value = null
  nomZip.value = ''
  messageErreur.value = ''
  progression.value = 0
  statutProgression.value = ''
  etat.value = 'repos'
  if (inputFichier.value) inputFichier.value.value = ''
}
</script>

<template>
  <div class="h-screen flex flex-col">
    <EnTete :sidebar-ouvert="sidebarOuvert" @basculer-sidebar="sidebarOuvert = !sidebarOuvert" />

    <div class="flex flex-1 overflow-hidden">
      <BarreLaterale
        :ouvert="sidebarOuvert"
        :historique="historique"
        :chargement="chargementHistorique"
        @telecharger="telechargerDepuisHistorique"
        @supprimer="supprimerDeHistorique"
      />

      <main class="flex-1 overflow-y-auto">
        <div class="max-w-lg mx-auto px-6 py-16">
          <h1 class="font-display text-3xl font-semibold tracking-tight text-ink mb-2 leading-tight">
            Chaque feuille devient<br />un CSV autonome.
          </h1>
          <p class="text-sm leading-relaxed text-ink-soft max-w-[46ch] mb-10">
            Dépose un classeur Excel. Chaque feuille en ressort avec son propre titre généré
            et son propre fichier, regroupés dans un zip.
          </p>

          <!-- Sélecteur du type de fichier d'entrée -->
          <div v-if="etat === 'repos' || etat === 'survol'" :class="modeConversion === 'crplmt' ? 'mb-6' : 'mb-4'">
            <div class="flex items-center gap-1 border border-line rounded-md p-0.5 w-fit">
              <button
                v-for="mode in modes"
                :key="mode.valeur"
                type="button"
                @click="modeConversion = mode.valeur"
                :class="[
                  'flex items-center gap-1.5 rounded px-3 py-1.5 text-xs font-medium transition-colors',
                  modeConversion === mode.valeur
                    ? 'bg-blue text-white'
                    : 'text-ink-soft hover:text-ink'
                ]"
              >
                <component :is="mode.icone" :size="13" :stroke-width="2.25" />
                {{ mode.label }}
              </button>
            </div>
            <p class="mt-2 font-mono text-[11px] text-ink-soft">
              {{ modes.find(m => m.valeur === modeConversion)?.description }}
            </p>

            <div v-if="modeConversion === 'crplmt'" class="mt-3 grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2 max-w-sm">
              <label for="champ-montant" class="text-xs font-medium text-ink">Montant</label>
              <input
                id="champ-montant"
                v-model="montant"
                type="text"
                inputmode="numeric"
                placeholder="100000"
                :class="[
                  'w-32 rounded-md border bg-paper px-2 py-1 font-mono text-xs text-ink outline-none focus:border-blue',
                  montantValide ? 'border-line' : 'border-danger'
                ]"
              />
              <label for="champ-titre" class="text-xs font-medium text-ink">Titre</label>
              <input
                id="champ-titre"
                v-model="titreAlias"
                type="text"
                class="rounded-md border border-line bg-paper px-2 py-1 font-mono text-xs text-ink outline-none focus:border-blue"
              />
              <label for="champ-description" class="text-xs font-medium text-ink">Description</label>
              <input
                id="champ-description"
                v-model="descriptionAlias"
                type="text"
                class="rounded-md border border-line bg-paper px-2 py-1 font-mono text-xs text-ink outline-none focus:border-blue"
              />
            </div>
          </div>

          <!-- Sélecteur du style d'en-tête (process standard uniquement) -->
          <div v-if="(etat === 'repos' || etat === 'survol') && modeConversion === 'standard'" class="mb-6">
            <div class="flex items-center gap-1 border border-line rounded-md p-0.5 w-fit">
              <button
                v-for="opt in options"
                :key="opt.valeur"
                type="button"
                @click="styleEntete = opt.valeur"
                :class="[
                  'flex items-center gap-1.5 rounded px-3 py-1.5 text-xs font-medium transition-colors',
                  styleEntete === opt.valeur
                    ? 'bg-blue text-white'
                    : 'text-ink-soft hover:text-ink'
                ]"
              >
                <component :is="opt.icone" :size="13" :stroke-width="2.25" />
                {{ opt.label }}
              </button>
            </div>
            <p class="mt-2 font-mono text-[11px] text-ink-soft">
              {{ options.find(o => o.valeur === styleEntete)?.description }}
            </p>
          </div>

          <!-- Zone de dépôt : bande structurelle, pas une carte flottante -->
          <div
            v-if="etat === 'repos' || etat === 'survol'"
            @dragover="onSurvolEntree"
            @dragenter="onSurvolEntree"
            @dragleave="onSurvolSortie"
            @drop="onDepot"
            @click="ouvrirSelecteur"
            role="button"
            tabindex="0"
            @keydown.enter="ouvrirSelecteur"
            :class="[
              'flex items-center gap-4 rounded-md border px-6 py-8 cursor-pointer transition-colors',
              etat === 'survol' ? 'border-yellow bg-blue-dim' : 'border-line bg-paper-dim hover:border-blue/40'
            ]"
          >
            <input
              ref="inputFichier"
              type="file"
              accept=".xlsx,.xls"
              class="hidden"
              @change="onSelection"
            />
            <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-yellow text-yellow-ink">
              <UploadCloud :size="20" :stroke-width="1.75" />
            </div>
            <div class="min-w-0">
              <p class="text-sm font-medium text-ink">{{ libelleZone }}</p>
              <p class="font-mono text-[11px] text-ink-soft">.xlsx / .xls</p>
            </div>
          </div>

          <!-- Conversion en cours -->
          <div v-else-if="etat === 'conversion'" class="rounded-md border border-line bg-paper-dim px-6 py-6">
            <div class="flex items-center gap-4">
              <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-blue-dim text-blue">
                <Loader2 :size="20" :stroke-width="2" class="animate-spin" />
              </div>
              <div class="min-w-0 flex-1">
                <div class="flex items-center justify-between gap-3">
                  <p class="text-sm font-medium text-ink">Conversion en cours</p>
                  <p class="font-mono text-[11px] text-ink-soft shrink-0">{{ pourcentageRestant }}% restant</p>
                </div>
                <p class="font-mono text-[11px] text-ink-soft truncate">{{ fichier?.name }}</p>
              </div>
            </div>

            <div class="mt-5">
              <div class="h-2 w-full overflow-hidden rounded bg-line" role="progressbar" :aria-valuenow="progression" aria-valuemin="0" aria-valuemax="100">
                <div
                  class="h-full rounded bg-yellow transition-[width] duration-300 ease-out"
                  :style="{ width: `${progression}%` }"
                ></div>
              </div>
              <div class="mt-2 flex items-center justify-between gap-3 font-mono text-[11px] text-ink-soft">
                <span>{{ statutProgression }}</span>
                <span>{{ progression }}%</span>
              </div>
            </div>
          </div>

          <!-- Terminé -->
          <div v-else-if="etat === 'termine'" class="rounded-md border border-line bg-paper-dim px-6 py-6">
            <div class="flex items-center gap-4 mb-5">
              <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-success/10 text-success">
                <CircleCheck :size="20" :stroke-width="2" />
              </div>
              <div class="min-w-0">
                <p class="text-sm font-medium text-ink">Conversion terminée</p>
                <p class="font-mono text-[11px] text-ink-soft truncate">
                  {{ nomZip }} — {{ formatKo(tailleZipKo) }} Ko<span v-if="nbFeuilles"> — {{ nbFeuilles }} feuilles</span>
                </p>
              </div>
            </div>

            <div class="flex items-center gap-2">
              <button
                @click="telecharger"
                class="inline-flex items-center gap-2 rounded-md bg-yellow text-yellow-ink text-sm font-semibold py-2 px-4 transition-transform hover:brightness-95 active:translate-y-px"
              >
                <Download :size="15" :stroke-width="2.25" />
                Télécharger le zip
              </button>
              <button
                @click="reinitialiser"
                class="inline-flex items-center gap-1.5 text-xs font-medium text-ink-soft hover:text-blue transition-colors px-2"
              >
                <RotateCcw :size="13" :stroke-width="2.25" />
                Nouveau fichier
              </button>
            </div>
          </div>

          <!-- Erreur -->
          <div v-else-if="etat === 'erreur'" class="rounded-md border border-danger/30 bg-danger/5 px-6 py-6">
            <div class="flex items-start gap-4 mb-5">
              <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-danger/10 text-danger">
                <TriangleAlert :size="20" :stroke-width="2" />
              </div>
              <div class="min-w-0">
                <p class="text-sm font-medium text-ink">Échec de la conversion</p>
                <p class="text-xs text-ink-soft whitespace-pre-line break-words select-text">{{ messageErreur }}</p>
              </div>
            </div>
            <button
              @click="reinitialiser"
              class="inline-flex items-center gap-2 rounded-md bg-ink text-paper text-sm font-semibold py-2 px-4 transition-transform hover:brightness-110"
            >
              <X :size="15" :stroke-width="2.25" />
              Réessayer
            </button>
          </div>

          <div v-if="fichier && etat === 'conversion'" class="mt-3 flex items-center gap-1.5 font-mono text-[11px] text-ink-soft">
            <FileSpreadsheet :size="12" :stroke-width="2" />
            <span>{{ (fichier.size / 1024).toFixed(0) }} Ko</span>
          </div>
        </div>
      </main>
    </div>

    <!-- Notification Toast lors du téléchargement -->
    <NotificationToast
      :visible="notificationVisible"
      :nom-fichier="notificationFichier"
      :message="notificationMessage"
      @fermer="fermerNotification"
    />
  </div>
</template>
