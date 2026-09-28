import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
  ],

  // Tauri : port fixe, ne pas effacer le terminal
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Ignorer le dossier src-tauri pour éviter les rechargements inutiles
      ignored: ['**/src-tauri/**'],
    },
  },
})
