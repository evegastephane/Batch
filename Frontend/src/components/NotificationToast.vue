<script setup>
import { Download, X, CheckCircle2 } from 'lucide-vue-next'

defineProps({
  visible: Boolean,
  nomFichier: {
    type: String,
    default: ''
  },
  message: {
    type: String,
    default: 'Le fichier a été enregistré dans vos téléchargements.'
  }
})

const emit = defineEmits(['fermer'])
</script>

<template>
  <Transition
    enter-active-class="transform ease-out duration-300 transition"
    enter-from-class="translate-y-4 opacity-0 sm:translate-y-0 sm:translate-x-4"
    enter-to-class="translate-y-0 opacity-100 sm:translate-x-0"
    leave-active-class="transition ease-in duration-200"
    leave-from-class="opacity-100"
    leave-to-class="opacity-0"
  >
    <div
      v-if="visible"
      class="fixed bottom-5 right-5 z-50 max-w-sm w-full bg-paper border border-line rounded-lg shadow-xl p-4 flex items-start gap-3 pointer-events-auto"
      role="alert"
    >
      <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-yellow text-yellow-ink font-semibold">
        <Download :size="18" :stroke-width="2.25" />
      </div>

      <div class="min-w-0 flex-1 pt-0.5">
        <div class="flex items-center gap-1.5">
          <p class="text-sm font-semibold text-ink">Téléchargement lancé</p>
          <span class="inline-flex items-center px-1.5 py-0.2 rounded text-[10px] font-medium bg-success/15 text-success">
            Prêt
          </span>
        </div>
        <p v-if="nomFichier" class="font-mono text-xs text-ink-soft truncate mt-0.5" :title="nomFichier">
          {{ nomFichier }}
        </p>
        <p class="text-xs text-ink-soft mt-1 leading-snug">
          {{ message }}
        </p>
      </div>

      <button
        type="button"
        @click="emit('fermer')"
        class="shrink-0 rounded p-1 text-ink-soft hover:text-ink hover:bg-paper-dim transition-colors"
        aria-label="Fermer la notification"
      >
        <X :size="15" :stroke-width="2" />
      </button>
    </div>
  </Transition>
</template>
