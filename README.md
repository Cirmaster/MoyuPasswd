# moyu-passwd

摸鱼密码 - 密码管理工具

## 开发规范

详见 [.cursorrules](./.cursorrules)，包含注释要求、代码风格、提交规范等。

## 技术栈

### 前端

- **框架**: Vue 3 + TypeScript
- **构建工具**: Vite 8
- **桌面端**: Tauri 2.0
- **UI 组件**: Shadcn Vue
- **样式**: Tailwind CSS v4
- **状态管理**: Pinia
- **路由**: Vue Router 5
- **图标**: Lucide Vue

### 后端

- **语言**: Rust
- **框架**: Tauri 2.0
- **数据库**: SQLite (rusqlite)
- **加密**: AES-256-GCM
- **哈希**: Argon2id

## 开发

```bash
# 安装依赖
pnpm install

# 启动开发服务器
pnpm dev

# 类型检查 + 构建
pnpm build

# 单元测试
pnpm test:unit
```

## 项目结构

```
moyu-passwd-code/
├── src/                           # 前端源码
│   ├── assets/                    # 静态资源、样式
│   ├── components/                # 组件
│   │   ├── ui/                    # Shadcn Vue 组件
│   │   ├── PasswordFormDialog.vue # 密码表单弹窗
│   │   ├── PasswordGenerator.vue  # 密码生成器
│   │   ├── PasswordStrength.vue   # 密码强度指示器
│   │   ├── QuickSearch.vue        # 快速搜索
│   │   ├── QuickAdd.vue           # 快速添加
│   │   ├── Toast.vue              # Toast 提示
│   │   └── ScreenCountdown.vue    # 屏幕倒计时悬浮窗
│   ├── composables/               # 组合式函数
│   │   ├── useTheme.ts            # 主题管理
│   │   ├── useAutoLock.ts         # 自动锁定
│   │   └── useClipboard.ts        # 剪贴板管理
│   ├── lib/                       # 工具函数
│   ├── router/                    # 路由配置
│   ├── stores/                    # Pinia 状态
│   │   ├── password.ts            # 密码数据管理
│   │   └── shortcuts.ts           # 快捷键配置
│   └── views/                     # 页面视图
│       ├── UnlockView.vue         # 解锁页
│       ├── HomeView.vue           # 密码列表页
│       ├── SettingsView.vue       # 设置页
│       └── CategoryView.vue       # 分类管理页
└── src-tauri/                     # Tauri 后端源码
    └── src/
        ├── main.rs                # 应用入口
        ├── lib.rs                 # 模块注册
        ├── db/                    # 数据库模块
        │   └── mod.rs             # 初始化、迁移
        ├── crypto/                # 加密模块
        │   └── mod.rs             # Argon2、AES-256-GCM
        ├── state/                 # 状态管理
        │   └── mod.rs             # AES 密钥、解锁状态
        └── commands/              # Tauri 命令
            ├── mod.rs             # 命令注册
            ├── auth.rs            # 认证命令
            ├── password.rs        # 密码 CRUD
            ├── category.rs        # 分类管理
            └── settings.rs        # 设置管理
```

## 页面进度

### 已完成

| 页面 | 路由 | 说明 | Tauri 适配 |
|------|------|------|------------|
| 解锁页 | `/` | 主密码输入，支持亮暗主题切换 | ⏳ 待对接 |
| 密码列表页 | `/home` | 左侧分类栏 + 右侧密码表格，支持搜索、收藏、复制、删除 | ⏳ 待对接 |
| 密码表单弹窗 | 组件 | 新增/编辑密码，支持调用密码生成器 | ⏳ 待对接 |
| 密码生成器 | 组件 | 可配置长度、字符类型，显示密码强度 | ⏳ 待对接 |
| 快速搜索 | 组件 | Ctrl+K 呼出，键盘导航，快速复制 | ⏳ 待对接 |
| 设置页 | `/settings` | 通用设置、安全设置、修改主密码、数据导入导出 | ⏳ 待对接 |
| 分类管理页 | `/categories` | 查看、添加、删除分类 | ⏳ 待对接 |

## 快捷键（可在设置中自定义）

| 功能 | 默认快捷键 | 说明 |
|------|-----------|------|
| 快速搜索 | `Ctrl+K` | 弹出独立搜索窗口，支持键盘导航 |
| 快速添加 | `Ctrl+Shift+N` | 弹出快速添加密码窗口 |

> 快捷键为全局快捷键，在任何应用中都可以使用。
> 点击设置页面中的快捷键可进行修改。

## 后端开发进度

### 已完成

| 模块 | 文件 | 说明 |
|------|------|------|
| 数据库 | `src/db/mod.rs` | SQLite 初始化、迁移、表结构 |
| 加密 | `src/crypto/mod.rs` | Argon2 哈希、AES-256-GCM 加解密 |
| 状态管理 | `src/state/mod.rs` | AES 密钥存储、解锁状态 |
| 认证命令 | `src/commands/auth.rs` | 主密码设置、验证、修改、锁定 |
| 密码命令 | `src/commands/password.rs` | 密码 CRUD 操作 |
| 分类命令 | `src/commands/category.rs` | 分类管理 |
| 设置命令 | `src/commands/settings.rs` | 应用设置管理 |

### 待实现

- [x] 数据库查询实际实现
- [x] 前端对接 Tauri 命令
- [x] 全局快捷键（Ctrl+K 呼出搜索）
- [x] 全局快捷键（Ctrl+Shift+N 快速添加）
- [x] 系统托盘功能
- [x] 自动锁定功能
- [x] 数据导入导出
- [x] 密码强度指示器
- [x] 剪贴板自动清除
- [x] 快捷键设置页面

---

## 常见问题与解决方案

详见 `docs/errors/` 目录，按语言分类：

| 文件 | 说明 |
|------|------|
| [rust-tauri.md](./docs/errors/rust-tauri.md) | Rust、Tauri、Cargo 相关问题 |
| [vue-frontend.md](./docs/errors/vue-frontend.md) | Vue、Tailwind CSS、Shadcn Vue、pnpm 相关问题 |
