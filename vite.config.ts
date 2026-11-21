import { fileURLToPath, URL } from 'node:url'
import { resolve } from 'node:path'
import { defineConfig, type UserConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueJsx from '@vitejs/plugin-vue-jsx'
import svgLoader from 'vite-svg-loader'
// import vueDevTools from 'vite-plugin-vue-devtools'

// https://vite.dev/config/
export default defineConfig(({ command }): UserConfig => ({
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
  server: {
    // 必须显式钉住 IPv4：Vite 默认 host 是 'localhost'，Node 17+ 按 verbatim 顺序解析，
    // 多数机器上先拿到 ::1，于是只监听 [::1]:5173；而 Tauri 的 WebView2 走 Happy Eyeballs
    // 先试 127.0.0.1，连不上就直接白屏（且不会有任何报错）。
    // 两端都用字面量 127.0.0.1（与 tauri.conf.json 的 devUrl 保持一致），不依赖解析顺序。
    host: '127.0.0.1',
    port: 5173,
    // 端口被占用时直接报错，而不是静默换端口——换端口后 devUrl 失配，现象同样是白屏
    strictPort: true,
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
}))
