import { fileURLToPath } from 'node:url'
import { mergeConfig, defineConfig, configDefaults } from 'vitest/config'
import type { ConfigEnv, UserConfig } from 'vite'
import viteConfigExport from './vite.config'

// vite.config 导出函数形式（build 时剥离 console），这里按 serve 形态展开成对象再合并
const resolveViteConfig = async (env: ConfigEnv): Promise<UserConfig> =>
  typeof viteConfigExport === 'function' ? await viteConfigExport(env) : viteConfigExport

export default defineConfig(async (env) =>
  mergeConfig(
    await resolveViteConfig(env),
    defineConfig({
      test: {
        environment: 'jsdom',
        exclude: [...configDefaults.exclude, 'e2e/**'],
        root: fileURLToPath(new URL('./', import.meta.url)),
      },
    }),
  ),
)
