import { fileURLToPath, URL } from 'node:url'
import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueJsx from '@vitejs/plugin-vue-jsx'
import svgLoader from 'vite-svg-loader'
// import vueDevTools from 'vite-plugin-vue-devtools'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    vueJsx(),
    // .svg 文件作为 Vue 组件导入（src/assets/svg，可直接预览）
    svgLoader(),
    // vueDevTools(),  // 开发时需要可以取消注释
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        'quick-search': resolve(__dirname, 'quick-search.html'),
        'countdown': resolve(__dirname, 'countdown.html'),
      },
    },
  },
})
