<!--
  QuickAdd.vue - 快速添加密码弹窗

  全局快捷键 Ctrl+Shift+N 呼出，快速保存当前网站密码。
  简化版的密码添加表单，只包含必要字段（含可选自定义字段、分类与标签）。

  @example
  ```vue
  <QuickAdd v-model:open="showQuickAdd" />
  ```
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { storeToRefs } from 'pinia'
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
import { usePasswordStore, type CustomField } from '@/stores/password'
import CustomFieldsEditor from './CustomFieldsEditor.vue'
import TagInput from './TagInput.vue'
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
const { categories, categoryTree, tags } = storeToRefs(passwordStore)
const { addPassword } = passwordStore

/** 标签建议列表：来自全局标签（供 TagInput 匹配提示） */
const tagSuggestions = computed(() => tags.value.map((t) => t.name))

/**
 * 分类下拉选项
 * 优先使用分类树（按 depth 缩进展示层级）；分类树尚未加载时退回扁平分类列表
 */
const categoryOptions = computed(() => {
  if (categoryTree.value.length > 0) {
    return categoryTree.value.map((node) => ({
      id: node.id,
      name: node.name,
      depth: node.depth,
    }))
  }
  // 兼容分类树尚未加载的场景（过滤掉"全部"/"收藏"两个虚拟分类）
  return categories.value
    .filter((c) => c.id !== 'all' && c.id !== 'favorite')
    .map((c) => ({ id: c.id, name: c.name, depth: 0 }))
})

/** 表单数据 */
const form = ref({
  title: '',
  username: '',
  password: '',
  url: '',
  extra_fields: [] as CustomField[],
  category: 'other', // 分类，默认为"其他"
  tags: [] as string[], // 标签（0~20 个）
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
    extra_fields: [],
    category: 'other',
    tags: [],
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
      extra_fields: form.value.extra_fields,
      category: form.value.category,
      tags: [...form.value.tags],
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
 * 打开与关闭都重置表单：明文密码/自定义字段值不驻留内存
 */
const handleOpenChange = (open: boolean) => {
  resetForm()
  emit('update:open', open)
}

// 注：'show-quick-add' 全局快捷键事件由 HomeView 统一监听并打开本弹窗，
// 组件内部不再重复监听（旧写法 listen 返回的 unlisten 被丢弃，反复挂载会泄漏）
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <!-- 高度封顶 + 标题/按钮钉住，中间表单区内部滚动 -->
    <DialogContent class="sm:max-w-[480px] max-h-[85vh] grid-rows-[auto_1fr_auto]">
      <DialogHeader>
        <DialogTitle>快速添加密码</DialogTitle>
        <DialogDescription>快速保存密码到您的密码库</DialogDescription>
      </DialogHeader>

      <!-- 表单内容（可滚动区） -->
      <div class="min-h-0 overflow-y-auto">
        <div class="grid gap-4 py-1">
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

          <!-- 网址 + 分类：两列并排 -->
          <div class="grid grid-cols-2 gap-4">
            <div class="grid gap-2">
              <Label for="quick-url">网址</Label>
              <Input
                id="quick-url"
                v-model="form.url"
                placeholder="https://example.com"
              />
            </div>
            <div class="grid gap-2">
              <Label for="quick-category">分类</Label>
              <select
                id="quick-category"
                v-model="form.category"
                class="flex h-10 w-full rounded-md border border-input bg-card px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              >
                <!-- 分类树按层级缩进（depth 前缀），层级不限 -->
                <option
                  v-for="opt in categoryOptions"
                  :key="opt.id"
                  :value="opt.id"
                >
                  {{ '— '.repeat(opt.depth) }}{{ opt.name }}
                </option>
              </select>
            </div>
          </div>

          <!-- 标签字段（可选）：可输入新标签，也可从已有标签中选择 -->
          <div class="grid gap-2">
            <Label>标签</Label>
            <TagInput
              v-model="form.tags"
              :suggestions="tagSuggestions"
              placeholder="输入标签后回车，可添加多个"
            />
          </div>

          <!-- 自定义字段（可选）：连接地址/端口/连接命令等 -->
          <CustomFieldsEditor v-model="form.extra_fields" />
        </div>
      </div>

      <DialogFooter>
        <Button variant="outline" @click="handleOpenChange(false)">
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
