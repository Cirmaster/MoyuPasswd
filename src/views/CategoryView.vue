<!--
  CategoryView.vue - 分类管理页面

  用于管理密码的分类（文件夹/标签）。
  功能：
  1. 查看所有分类及其密码数量
  2. 添加新分类
  3. 删除自定义分类（"全部"和"收藏"不可删除）
  4. 删除分类时，该分类下的密码会自动移到"其他"

  路由路径：/categories

  TODO: 对接 Tauri 后端持久化分类数据
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { usePasswordStore, type Category } from '@/stores/password'
import Toast from '@/components/Toast.vue'

/** 路由实例 */
const router = useRouter()

/** 密码 Store */
const passwordStore = usePasswordStore()
const { categories, passwords } = storeToRefs(passwordStore)
const { addCategory, updateCategory, deleteCategory, loadPasswords, loadCategories } = passwordStore

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 页面加载时从后端获取数据
 */
onMounted(async () => {
  await Promise.all([
    loadPasswords(),
    loadCategories(),
  ])
})

/** app-locked 事件监听清理函数 */
let appLockedUnlisten: UnlistenFn | null = null

// 锁定后跳转解锁页（防止停留在分类页无响应）
onMounted(async () => {
  appLockedUnlisten = await listen('app-locked', () => {
    router.push('/')
  })
})

onUnmounted(() => {
  appLockedUnlisten?.()
  appLockedUnlisten = null
})

/** 控制添加分类弹窗是否显示 */
const showAddDialog = ref(false)

/** 当前正在编辑的分类（null 表示添加模式） */
const editingCategory = ref<Category | null>(null)

/** 新分类名称输入 */
const newCategoryName = ref('')

/**
 * 获取分类下的密码数量
 * @param id - 分类 ID
 * @returns 该分类下的密码数量
 *
 * 特殊分类：
 * - "all": 返回所有密码数量
 * - "favorite": 返回收藏的密码数量
 * - 其他: 返回该分类下的密码数量
 */
const getCategoryCount = (id: string) => {
  if (id === 'all') return passwords.value.length
  if (id === 'favorite') return passwords.value.filter((p) => p.is_favorite).length
  return passwords.value.filter((p) => p.category === id).length
}

/**
 * 打开添加分类弹窗
 */
const openAdd = () => {
  editingCategory.value = null
  newCategoryName.value = ''
  showAddDialog.value = true
}

/**
 * 打开编辑分类弹窗
 * @param category - 要编辑的分类
 */
const openEdit = (category: Category) => {
  editingCategory.value = category
  newCategoryName.value = category.name
  showAddDialog.value = true
}

/**
 * 保存分类（添加或编辑）
 * 1. 验证分类名称不为空
 * 2. 根据 editingCategory 决定调用添加或更新
 * 3. 清空输入框并关闭弹窗
 */
const handleSave = async () => {
  const name = newCategoryName.value.trim()
  if (!name) return

  const isEditing = !!editingCategory.value
  try {
    if (isEditing && editingCategory.value) {
      await updateCategory(editingCategory.value.id, name)
      toast.value = { show: true, type: 'success', message: '分类更新成功' }
    } else {
      await addCategory(name)
      toast.value = { show: true, type: 'success', message: '分类添加成功' }
    }
    // 清空输入框并关闭弹窗
    newCategoryName.value = ''
    editingCategory.value = null
    showAddDialog.value = false
  } catch (e) {
    toast.value = { show: true, type: 'error', message: (isEditing ? '更新' : '添加') + '失败: ' + String(e) }
  }
}

/**
 * 删除分类
 * - "all" 和 "favorite" 是特殊分类，不可删除
 * - 删除前需要用户确认
 * - 删除后，该分类下的密码会自动移到"其他"
 *
 * @param id - 要删除的分类 ID
 */
const handleDelete = async (id: string) => {
  // 特殊分类不可删除
  if (id === 'all' || id === 'favorite') return
  // 确认删除
  if (confirm('确定删除此分类？分类下的密码将移到"其他"')) {
    try {
      await deleteCategory(id)
      toast.value = { show: true, type: 'success', message: '分类删除成功' }
    } catch (e) {
      toast.value = { show: true, type: 'error', message: '删除失败: ' + String(e) }
    }
  }
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <!-- 顶部导航 -->
    <header class="border-b px-4 py-3 flex items-center gap-4">
      <Button variant="ghost" size="icon" @click="router.push('/home')">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          class="h-5 w-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path d="m12 19-7-7 7-7M19 12H5" />
        </svg>
      </Button>
      <h1 class="text-lg font-semibold">分类管理</h1>
    </header>

    <!-- 内容区 -->
    <div class="max-w-2xl mx-auto p-6">
      <div class="flex justify-between items-center mb-6">
        <p class="text-muted-foreground">管理您的密码分类</p>
        <Button @click="openAdd">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            class="h-4 w-4 mr-2"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M5 12h14M12 5v14" />
          </svg>
          添加分类
        </Button>
      </div>

      <div class="space-y-3">
        <Card
          v-for="category in categories"
          :key="category.id"
        >
          <CardContent class="p-4 flex items-center justify-between">
            <div>
              <h3 class="font-medium">{{ category.name }}</h3>
              <p class="text-sm text-muted-foreground">
                {{ getCategoryCount(category.id) }} 个密码
              </p>
            </div>
            <div
              v-if="category.id !== 'all' && category.id !== 'favorite'"
              class="flex items-center gap-1"
            >
              <Button
                variant="ghost"
                size="icon"
                @click="openEdit(category)"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                  <path d="m15 5 4 4" />
                </svg>
              </Button>
              <Button
                variant="ghost"
                size="icon"
                class="text-destructive hover:text-destructive"
                @click="handleDelete(category.id)"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  class="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path d="M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
                </svg>
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>

    <!-- 添加分类弹窗 -->
    <Dialog v-model:open="showAddDialog">
      <DialogContent class="sm:max-w-[300px]">
        <DialogHeader>
          <DialogTitle>{{ editingCategory ? '编辑分类' : '添加分类' }}</DialogTitle>
          <DialogDescription>{{ editingCategory ? '修改分类名称' : '创建一个新的密码分类' }}</DialogDescription>
        </DialogHeader>
        <div class="py-4">
          <Label for="category-name">分类名称</Label>
          <Input
            id="category-name"
            v-model="newCategoryName"
            placeholder="输入分类名称"
            class="mt-2"
          />
        </div>
        <DialogFooter>
          <Button variant="outline" @click="showAddDialog = false">取消</Button>
          <Button @click="handleSave">{{ editingCategory ? '保存' : '添加' }}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- Toast 提示 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
