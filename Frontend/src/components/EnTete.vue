<script setup>
import { PanelLeft, PanelLeftClose } from 'lucide-vue-next'

defineProps({
  sidebarOuvert: Boolean
})
defineEmits(['basculer-sidebar'])
</script>

<template>
  <header class="h-14 shrink-0 border-b border-line flex items-center gap-3 px-4 bg-paper">
    <button
      type="button"
      @click="$emit('basculer-sidebar')"
      class="flex h-8 w-8 items-center justify-center rounded text-ink-soft hover:bg-paper-dim hover:text-ink transition-colors"
      :aria-label="sidebarOuvert ? 'Masquer l\'historique' : 'Afficher l\'historique'"
    >
      <component :is="sidebarOuvert ? PanelLeftClose : PanelLeft" :size="18" :stroke-width="1.75" />
    </button>

    <div class="flex items-center gap-2.5">
      <img
        src="/logo_mtn.png"
        alt="MTN Logo"
        class="h-7 w-7 rounded object-cover shadow-sm border border-line"
      />
      <div class="flex items-baseline gap-2">
        <span class="font-display text-[15px] font-semibold tracking-tight text-ink">MMC Batch</span>
        <span class="font-mono text-[11px] text-ink-soft">/ conversion par lot</span>
      </div>
    </div>

    <div class="ml-auto h-2 w-2 rounded-sm bg-yellow" aria-hidden="true" />
  </header>
</template>
