<!--
  QuickAdd.vue - 快速添加密码弹窗

  全局快捷键 Ctrl+Shift+N 呼出，快速保存当前网站密码。
  简化版的密码添加表单，只包含必要字段。

  @example
  ```vue
  <QuickAdd v-model:open="showQuickAdd" />
  ```
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { usePasswordStore } from '@/stores/password'
import Toast from './Toast.vue'

/**
 * 组件 Props
 */
interface Props {
  /** 控制弹窗是否打开 */
  open: boolean
}

const props = defineProps<Props>()

/**
 * 组件事件
 */
const emit = defineEmits<{
  'update:open': [value: boolean]
}>()

/** 密码 Store */
const passwordStore = usePasswordStore()
const { addPassword } = passwordStore

/** 表单数据 */
const form = ref({
  title: '',
  username: '',
  password: '',
  url: '',
})

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/** 加载状态 */
const loading = ref(false)

/**
 * 重置表单
 */
const resetForm = () => {
  form.value = {
    title: '',
    username: '',
    password: '',
    url: '',
  }
}

/**
 * 保存密码
 */
const handleSave = async () => {
  if (!form.value.title || !form.value.password) {
    toast.value = { show: true, type: 'error', message: '请填写名称和密码' }
    return
  }

  loading.value = true

  try {
    await addPassword({
      title: form.value.title,
      username: form.value.username || '',
      password: form.value.password,
      url: form.value.url || undefined,
      notes: undefined,
      category: 'other',
      is_favorite: false,
    })

    toast.value = { show: true, type: 'success', message: '密码添加成功' }
    resetForm()
    emit('update:open', false)
  } catch (e) {
    toast.value = { show: true, type: 'error', message: '添加失败: ' + String(e) }
  } finally {
    loading.value = false
  }
}

/**
 * 监听弹窗打开状态
 */
const handleOpenChange = (open: boolean) => {
  if (open) {
    resetForm()
  }
  emit('update:open', open)
}

/**
 * 注册全局快捷键
 */
onMounted(async () => {
  const { listen } = await import('@tauri-apps/api/event')

  // 监听全局快捷键事件
  await listen('show-quick-add', () => {
    emit('update:open', true)
  })
})
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-[400px]">
      <DialogHeader>
        <DialogTitle>快速添加密码</DialogTitle>
        <DialogDescription>快速保存密码到您的密码库</DialogDescription>
      </DialogHeader>

      <div class="grid gap-4 py-4">
        <div class="grid gap-2">
          <Label for="quick-title">名称 *</Label>
          <Input
            id="quick-title"
            v-model="form.title"
            placeholder="例如：GitHub"
            autofocus
          />
        </div>

        <div class="grid gap-2">
          <Label for="quick-username">用户名</Label>
          <Input
            id="quick-username"
            v-model="form.username"
            placeholder="邮箱或用户名"
          />
        </div>

        <div class="grid gap-2">
          <Label for="quick-password">密码 *</Label>
          <Input
            id="quick-password"
            v-model="form.password"
            type="password"
            placeholder="输入密码"
          />
        </div>

        <div class="grid gap-2">
          <Label for="quick-url">网址</Label>
          <Input
            id="quick-url"
            v-model="form.url"
            placeholder="https://example.com"
          />
        </div>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="emit('update:open', false)">
          取消
        </Button>
        <Button @click="handleSave" :disabled="loading">
          {{ loading ? '保存中...' : '保存' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <!-- Toast 提示 -->
  <Toast
    v-model:show="toast.show"
    :type="toast.type"
    :message="toast.message"
  />
</template>
