<!--
  PasswordFormDialog.vue - 密码表单弹窗组件

  用于添加新密码或编辑现有密码的弹窗表单。
  支持两种模式：
  1. 添加模式：editItem 为空，表单为空
  2. 编辑模式：editItem 有值，表单填充现有数据

  功能：
  - 名称、用户名、密码、网址、分类、备注等字段
  - 自定义字段（连接地址/端口/连接命令等，支持敏感加密）
  - 密码字段可调用密码生成器
  - 表单验证（必填字段）
  - 提交后自动关闭弹窗并重置表单

  @example
  ```vue
  <PasswordFormDialog
    v-model:open="showDialog"
    :edit-item="editingItem"
    @saved="onSaved"
  />
  ```
-->

<script setup lang="ts">
import { ref, watch, computed } from 'vue'
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
import { usePasswordStore, type PasswordItem, type CustomField } from '@/stores/password'
import PasswordGenerator from './PasswordGenerator.vue'
import CustomFieldsEditor from './CustomFieldsEditor.vue'

/**
 * 组件 Props 接口
 */
interface Props {
  /** 控制弹窗是否打开 */
  open: boolean
  /** 编辑的密码项，为 null 时表示添加模式 */
  editItem?: PasswordItem | null
}

/** 设置默认值 */
const props = withDefaults(defineProps<Props>(), {
  editItem: null,
})

/**
 * 组件事件
 * - update:open: 更新弹窗打开状态（v-model 双向绑定）
 * - saved: 保存成功后触发
 */
const emit = defineEmits<{
  'update:open': [value: boolean]
  saved: []
}>()

/** 密码 Store */
const passwordStore = usePasswordStore()
const { categories, addPassword, updatePassword, getPasswordDetail } = passwordStore

/**
 * 是否为编辑模式
 * 当 editItem 存在时为编辑模式，否则为添加模式
 */
const isEditMode = computed(() => !!props.editItem)

/**
 * 表单数据（响应式）
 * 包含所有表单字段
 */
const form = ref({
  title: '',       // 网站/应用名称
  username: '',    // 用户名或邮箱
  password: '',    // 登录密码
  url: '',         // 网站 URL
  notes: '',       // 备注
  extra_fields: [] as CustomField[], // 自定义字段（连接地址/端口/连接命令等）
  category: 'other', // 分类，默认为"其他"
  is_favorite: false, // 是否收藏
})

/**
 * 重置表单到初始状态
 * 添加成功或取消时调用
 */
const resetForm = () => {
  form.value = {
    title: '',
    username: '',
    password: '',
    url: '',
    notes: '',
    extra_fields: [],
    category: 'other',
    is_favorite: false,
  }
}

/**
 * 备注/敏感字段是否已从详情载入
 *
 * 备注明文与敏感字段值不随列表下发；编辑打开时才拉取详情。
 * 未载入完成前提交的更新不携带对应字段（= 不修改），防止误清空原数据。
 */
const notesLoaded = ref(false)
const fieldsLoaded = ref(false)

/**
 * 监听编辑项变化
 * 当 editItem 变化时，自动填充表单数据
 * immediate: true 表示初始化时也执行一次
 */
watch(
  () => props.editItem,
  (item) => {
    if (item) {
      // 编辑模式：填充除密码外的所有字段（密码不接触明文）
      notesLoaded.value = false
      fieldsLoaded.value = false
      form.value = {
        title: item.title,
        username: item.username,
        password: '', // 编辑时密码留空，提交时为空则保留原密码
        url: item.url || '',
        notes: '', // 备注不随列表下发，稍后从详情载入
        // 敏感字段值不随列表下发，先用列表数据占位（敏感值为空串），详情载入后覆盖
        extra_fields: (item.extra_fields || []).map((f) => ({ ...f })),
        category: item.category,
        is_favorite: item.is_favorite,
      }
      // 按需拉取备注明文与敏感字段值（列表零解密）
      getPasswordDetail(item.id)
        .then((detail) => {
          // 快速切换条目时防止串数据
          if (props.editItem?.id === item.id) {
            form.value.notes = detail.notes || ''
            notesLoaded.value = true
            form.value.extra_fields = (detail.extra_fields || []).map((f) => ({ ...f }))
            fieldsLoaded.value = true
          }
        })
        .catch((e) => {
          console.error('加载备注失败:', e)
        })
    } else {
      // 添加模式：重置表单
      notesLoaded.value = false
      fieldsLoaded.value = false
      resetForm()
    }
  },
  { immediate: true },
)

/** 控制密码生成器弹窗是否显示 */
const showGenerator = ref(false)

/**
 * 处理表单提交
 * 1. 验证必填字段
 * 2. 根据模式调用添加或更新方法
 * 3. 触发 saved 事件
 * 4. 关闭弹窗并重置表单
 */
const handleSubmit = async () => {
  // 验证必填字段（编辑模式下密码非必填）
  if (!form.value.title || !form.value.username) {
    return
  }
  // 添加模式下密码必填
  if (!isEditMode.value && !form.value.password) {
    return
  }

  try {
    // 根据模式执行不同操作
    if (isEditMode.value && props.editItem) {
      // 编辑模式：更新现有密码（密码为空则保留原密码）
      // 备注/自定义字段未载入完成时不携带对应字段（后端视为不修改），防止误清空原数据
      await updatePassword(props.editItem.id, {
        ...form.value,
        notes: notesLoaded.value ? (form.value.notes || null) : undefined,
        extra_fields: fieldsLoaded.value ? form.value.extra_fields : undefined,
      })
    } else {
      // 添加模式：添加新密码
      await addPassword(form.value)
    }

    // 触发保存成功事件
    emit('saved')
    // 关闭弹窗
    emit('update:open', false)
    // 重置表单
    resetForm()
  } catch (e) {
    // 错误已在 store 中处理
    console.error('保存失败:', e)
  }
}

/**
 * 打开密码生成器弹窗
 */
const openGenerator = () => {
  showGenerator.value = true
}

/**
 * 密码生成器生成密码后的回调
 * @param pwd - 生成的密码
 */
const onPasswordGenerated = (pwd: string) => {
  form.value.password = pwd
}
</script>

<template>
  <!-- 主表单弹窗 -->
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <!-- 高度封顶 + 标题/按钮钉住，中间表单区内部滚动（grid-rows 区分三段） -->
    <DialogContent class="sm:max-w-[560px] max-h-[85vh] grid-rows-[auto_1fr_auto]">
      <!-- 弹窗标题 -->
      <DialogHeader>
        <DialogTitle>{{ isEditMode ? '编辑密码' : '添加密码' }}</DialogTitle>
        <DialogDescription>
          {{ isEditMode ? '修改密码信息' : '添加新的密码到您的密码库' }}
        </DialogDescription>
      </DialogHeader>

      <!-- 表单内容（可滚动区）：短字段两列并排，顺序=身份信息 → 附加字段 → 备注 -->
      <div class="min-h-0 overflow-y-auto">
        <div class="grid gap-4 py-1">
          <!-- 第一行：名称 + 分类 -->
          <div class="grid grid-cols-2 gap-4">
            <div class="grid gap-2">
              <Label for="title">名称 *</Label>
              <Input
                id="title"
                v-model="form.title"
                placeholder="例如：GitHub"
              />
            </div>
            <div class="grid gap-2">
              <Label for="category">分类</Label>
              <select
                id="category"
                v-model="form.category"
                class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              >
                <!-- 过滤掉"全部"和"收藏"这两个特殊分类 -->
                <option
                  v-for="cat in categories.filter(c => c.id !== 'all' && c.id !== 'favorite')"
                  :key="cat.id"
                  :value="cat.id"
                >
                  {{ cat.name }}
                </option>
              </select>
            </div>
          </div>

          <!-- 第二行：用户名 + 密码 -->
          <div class="grid grid-cols-2 gap-4">
            <div class="grid gap-2">
              <Label for="username">用户名 *</Label>
              <Input
                id="username"
                v-model="form.username"
                placeholder="邮箱或用户名"
              />
            </div>
            <div class="grid gap-2">
              <Label for="password">密码 {{ isEditMode ? '' : '*' }}</Label>
              <div class="flex gap-2">
                <Input
                  id="password"
                  v-model="form.password"
                  type="password"
                  :placeholder="isEditMode ? '留空不修改' : '输入密码'"
                  class="flex-1 min-w-0"
                />
                <!-- 打开密码生成器 -->
                <Button variant="outline" class="shrink-0" @click="openGenerator">
                  生成
                </Button>
              </div>
            </div>
          </div>

          <!-- 网址字段（可选，整行） -->
          <div class="grid gap-2">
            <Label for="url">网址</Label>
            <Input
              id="url"
              v-model="form.url"
              placeholder="https://example.com"
            />
          </div>

          <!-- 自定义字段（可选）：连接地址/端口/连接命令等 -->
          <CustomFieldsEditor v-model="form.extra_fields" />

          <!-- 备注字段（可选，整行） -->
          <div class="grid gap-2">
            <Label for="notes">备注</Label>
            <textarea
              id="notes"
              v-model="form.notes"
              placeholder="可选备注信息"
              rows="2"
              class="flex min-h-[56px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            />
          </div>
        </div>
      </div>

      <!-- 底部按钮 -->
      <DialogFooter>
        <!-- 取消按钮 -->
        <Button variant="outline" @click="emit('update:open', false)">
          取消
        </Button>
        <!-- 提交按钮：根据模式显示不同文字 -->
        <Button @click="handleSubmit">
          {{ isEditMode ? '保存' : '添加' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <!-- 密码生成器弹窗（子组件） -->
  <PasswordGenerator
    v-model:open="showGenerator"
    @generated="onPasswordGenerated"
  />
</template>
