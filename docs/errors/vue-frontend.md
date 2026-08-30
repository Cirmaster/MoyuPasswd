# Vue / 前端常见问题与解决方案

## Tailwind CSS

### PostCSS 插件已迁移

**报错信息：**
```
[postcss] It looks like you're trying to use `tailwindcss` directly as a PostCSS plugin.
The PostCSS plugin has moved to a separate package.
```

**原因：**
Tailwind CSS v4 将 PostCSS 插件分离到了 `@tailwindcss/postcss` 包。

**解决方案：**
安装 `@tailwindcss/postcss` 并更新配置。

```bash
pnpm add -D @tailwindcss/postcss
```

```javascript
// postcss.config.js
export default {
  plugins: {
    '@tailwindcss/postcss': {},  // v4 写法
  },
}
```

```css
/* src/assets/main.css */
@import "tailwindcss";  /* v4 写法，替代 @tailwind base/components/utilities */
```

---

## Shadcn Vue

### 找不到 import alias

**报错信息：**
```
No import alias found in your tsconfig.json file.
```

**原因：**
tsconfig.json 缺少路径别名配置。

**解决方案：**
在 `tsconfig.json` 中添加 `paths` 配置。

```json
// tsconfig.json
{
  "compilerOptions": {
    "paths": {
      "@/*": ["./src/*"]
    }
  }
}
```

---

## Vue 3

### Cannot access before initialization

**报错信息：**
```
ReferenceError: Cannot access 'xxx' before initialization
```

**原因：**
在 `watch` 的 `immediate: true` 回调中调用了后面才定义的函数。

**解决方案：**
将函数定义移到 `watch` 之前。

```typescript
// ❌ 错误写法
watch(
  () => props.editItem,
  (item) => {
    if (item) {
      // ...
    } else {
      resetForm()  // resetForm 还未定义
    }
  },
  { immediate: true },
)

const resetForm = () => { ... }

// ✅ 正确写法
const resetForm = () => { ... }  // 先定义

watch(
  () => props.editItem,
  (item) => {
    if (item) {
      // ...
    } else {
      resetForm()  // 现在可以调用
    }
  },
  { immediate: true },
)
```

---

## pnpm

### 存储路径不匹配

**报错信息：**
```
[ERR_PNPM_UNEXPECTED_STORE] Unexpected store location
```

**原因：**
node_modules 使用的 pnpm 存储路径与当前配置不一致。

**解决方案：**
重新安装依赖。

```bash
pnpm install
```

### 构建脚本被阻止

**报错信息：**
```
[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts
```

**原因：**
pnpm 默认阻止依赖包的安装后脚本，需要手动批准。

**解决方案：**
批准构建脚本。

```bash
pnpm approve-builds
```

---

*最后更新: 2024*
