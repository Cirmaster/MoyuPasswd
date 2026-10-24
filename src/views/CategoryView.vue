<!--
  CategoryView.vue - 分类管理页面

  用于管理密码的分类树与全局标签。
  功能：
  1. 分类按 categoryTree 深度优先展示并缩进（缩进最多按 8 级封顶显示，层级本身不限），
     显示直接数量与含子分类总数
  2. 添加/编辑分类，可设置父分类（编辑时排除自身及其后代，避免形成环）
  3. 删除分类时，该分类下的密码移到"其他"，子分类上移一级
  4. 标签管理：查看、重命名、删除全局标签

  路由路径：/categories
-->

<script setup lang="ts">
import { ref, computed, onMounted, type Component } from 'vue'
import { useRouter } from 'vue-router'
import { storeToRefs } from 'pinia'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Card, CardContent } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  IconArrowLeft,
  IconArrowUp,
  IconChevronRight,
  IconGrid,
  IconPencil,
  IconPlus,
  IconTag,
  IconTrash,
} from '@/components/icons'
import ContextMenu from '@/components/ContextMenu.vue'
import { useCategoryDragSort } from '@/composables/useCategoryDragSort'
import {
  usePasswordStore,
  type CategoryTreeNode,
  type TagInfo,
} from '@/stores/password'
import Toast from '@/components/Toast.vue'

/** 路由实例 */
const router = useRouter()

/** 密码 Store */
const passwordStore = usePasswordStore()
const { categoryTree, tags, allCount, favoriteCount } = storeToRefs(passwordStore)
const {
  addCategory,
  updateCategory,
  deleteCategory,
  loadPasswords,
  loadStats,
  loadCategories,
  loadTags,
  renameTag,
  deleteTag,
  reorderCategories,
} = passwordStore

/** Toast 提示状态 */
const toast = ref({
  show: false,
  type: 'success' as 'success' | 'error' | 'info',
  message: '',
})

/**
 * 页面加载时从后端获取数据
 * 分类树来自 loadCategories，标签来自 loadTags，密码用于智能视图计数
 */
onMounted(async () => {
  await Promise.all([
    loadPasswords(),
    loadStats(),
    loadCategories(),
    loadTags(),
  ])
})

// app-locked 锁定清理与跳转由 App.vue 全局统一处理，此处不再重复监听

/**
 * 智能视图（全部 / 收藏）概览
 * 这两个是虚拟分类，只读展示，不参与分类树的编辑与删除
 */
const smartViews = computed(() => [
  { id: 'all', name: '全部', count: allCount.value },
  { id: 'favorite', name: '收藏', count: favoriteCount.value },
])

/**
 * 计算分类行的左内边距（按层级缩进）
 * 层级不限，但缩进最多按 8 级封顶，避免深层分类把内容挤到右侧
 * @param depth - 分类层级（顶级为 0）
 */
const indentStyle = (depth: number) => ({
  paddingLeft: `${16 + Math.min(depth, 8) * 12}px`,
})

/**
 * 分类拖拽排序（按住名称直接拖拽，无手柄；仅同父级排序）
 * 拖拽中显示插入指示线，排序失败回滚并 Toast 提示。
 */
const {
  startDrag: startCategoryDrag,
  isDragging: isCategoryDragging,
  dropIndicator: categoryDropIndicator,
} = useCategoryDragSort({
  categoryTree,
  reorderCategories,
  // 拖到目标行中间时把分类移动到目标下（父级变化）
  moveCategory: async (id, parentId) => {
    await updateCategory(id, undefined, parentId)
  },
  onError: (message) => {
    toast.value = { show: true, type: 'error', message: '操作失败: ' + message }
  },
})

/** 控制添加/编辑分类弹窗是否显示 */
const showAddDialog = ref(false)

/** 当前正在编辑的分类节点（null 表示添加模式） */
const editingCategory = ref<CategoryTreeNode | null>(null)

/** 分类名称输入 */
const newCategoryName = ref('')

/** 父分类选择（下拉框值）：空串表示「无（顶级分类）」，避免 select v-model 使用 null 的兼容风险 */
const parentId = ref<string>('')

/** 正在编辑的分类 ID（添加模式为 null） */
const editingCategoryId = computed(() => editingCategory.value?.id ?? null)

/**
 * 编辑中的分类的全部后代 ID 集合
 * 父分类下拉需要排除「自身及其后代」，否则会形成环
 */
const descendantIds = computed<Set<string>>(() => {
  const rootId = editingCategoryId.value
  if (!rootId) return new Set<string>()

  // 先按 parentId 建立「父 → 子」索引
  const childrenMap = new Map<string, string[]>()
  for (const node of categoryTree.value) {
    if (!node.parentId) continue
    const siblings = childrenMap.get(node.parentId)
    if (siblings) {
      siblings.push(node.id)
    } else {
      childrenMap.set(node.parentId, [node.id])
    }
  }

  // 迭代遍历收集所有后代（避免深层递归）
  const result = new Set<string>()
  const stack: string[] = [rootId]
  while (stack.length > 0) {
    const current = stack.pop()
    if (current === undefined) continue
    const children = childrenMap.get(current)
    if (!children) continue
    for (const childId of children) {
      if (result.has(childId)) continue
      result.add(childId)
      stack.push(childId)
    }
  }
  return result
})

/**
 * 父分类下拉选项
 * 树形顺序（深度优先）+ 缩进；编辑时排除自身及其后代
 */
const parentOptions = computed(() => {
  const selfId = editingCategoryId.value
  const excluded = descendantIds.value
  return categoryTree.value.filter(
    (node) => node.id !== selfId && !excluded.has(node.id),
  )
})

/**
 * 打开添加分类弹窗
 */
const openAdd = () => {
  editingCategory.value = null
  newCategoryName.value = ''
  parentId.value = ''
  showAddDialog.value = true
}

/**
 * 打开新增子分类弹窗（预设父分类）
 * @param parentCategoryId - 父分类 ID
 */
const openAddChild = (parentCategoryId: string) => {
  editingCategory.value = null
  newCategoryName.value = ''
  parentId.value = parentCategoryId
  showAddDialog.value = true
}

/**
 * 打开编辑分类弹窗
 * @param node - 要编辑的分类树节点
 */
const openEdit = (node: CategoryTreeNode) => {
  editingCategory.value = node
  newCategoryName.value = node.name
  parentId.value = node.parentId ?? ''
  showAddDialog.value = true
}

/**
 * 保存分类（添加或编辑）
 * 1. 验证分类名称不为空
 * 2. 编辑时显式提交 parentId（null = 置为顶级分类，禁止选中自身及后代）
 * 3. 关闭弹窗并提示结果
 */
const handleSave = async () => {
  const name = newCategoryName.value.trim()
  if (!name) return

  const target = editingCategory.value
  // 下拉值空串 = 无父分类（顶级）；Store 约定 null 表示置顶级，undefined 表示不修改
  const selectedParentId = parentId.value ? parentId.value : null
  try {
    if (target) {
      // 编辑：名称与父分类一起提交；selectedParentId 为 null 表示置为顶级分类
      await updateCategory(target.id, name, selectedParentId)
    } else {
      // 添加：selectedParentId 为 null 表示创建顶级分类
      await addCategory(name, selectedParentId)
    }
    // 清空输入框并关闭弹窗
    newCategoryName.value = ''
    editingCategory.value = null
    parentId.value = ''
    showAddDialog.value = false
  } catch (e) {
    toast.value = {
      show: true,
      type: 'error',
      message: (target ? '更新' : '添加') + '失败: ' + String(e),
    }
  }
}

/**
 * 删除分类
 * - 删除前需要用户确认
 * - 该分类下的密码移到"其他"，子分类上移一级（后端处理）
 * - 删除后重新加载分类与密码，保证分类树父子关系、各分类计数与后端一致
 *
 * @param id - 要删除的分类 ID
 */
const handleDelete = async (id: string) => {
  // 智能视图（虚拟分类）与内置「其他」不可删除
  //（「其他」是删除分类时的兜底归入目标，删除入口与 HomeView 右键菜单保持一致）
  if (id === 'all' || id === 'favorite' || id === 'other') return
  // 确认删除
  if (!confirm('确定删除此分类？该分类下的密码将移到“其他”，子分类将上移一级')) return
  try {
    await deleteCategory(id)
  } catch (e) {
    toast.value = { show: true, type: 'error', message: '删除失败: ' + String(e) }
  }
}

// ==================== 分类右键菜单（快捷管理） ====================

/** 分类右键菜单状态（categoryId 为 null 表示空白处：新增顶级分类） */
const categoryMenu = ref({
  open: false,
  x: 0,
  y: 0,
  categoryId: null as string | null,
})

/** 右键目标分类节点（null 表示空白处） */
const categoryMenuNode = computed<CategoryTreeNode | null>(() => {
  const id = categoryMenu.value.categoryId
  if (!id) return null
  return categoryTree.value.find((node) => node.id === id) ?? null
})

/** 右键菜单项 */
const categoryMenuItems = computed<
  Array<{
    key: string
    label: string
    icon?: Component
    danger?: boolean
    disabled?: boolean
    divided?: boolean
  }>
>(() => {
  const node = categoryMenuNode.value
  const items: Array<{
    key: string
    label: string
    icon?: Component
    danger?: boolean
    disabled?: boolean
    divided?: boolean
  }> = []

  if (!node) {
    items.push({ key: 'add-root', label: '新增顶级分类', icon: IconPlus })
  } else {
    items.push({ key: 'add-child', label: `在「${node.name}」下新增子分类`, icon: IconPlus })
    items.push({ key: 'rename', label: '重命名分类', icon: IconPencil })
    if (node.parentId) {
      items.push({ key: 'make-top', label: '置为顶级分类', icon: IconArrowUp })
    }
    items.push({
      key: 'delete',
      label: '删除分类',
      icon: IconTrash,
      danger: true,
      divided: true,
      disabled: node.id === 'other',
    })
  }
  return items
})

/**
 * 打开分类右键菜单
 * @param event - 鼠标事件（用于定位菜单）
 * @param categoryId - 目标分类 ID；null 表示空白处
 */
const openCategoryMenu = (event: MouseEvent, categoryId: string | null) => {
  event.preventDefault()
  categoryMenu.value = { open: true, x: event.clientX, y: event.clientY, categoryId }
}

/**
 * 处理分类右键菜单动作
 * @param key - 菜单项 key
 */
const handleCategoryMenuSelect = async (key: string) => {
  const node = categoryMenuNode.value
  categoryMenu.value = { ...categoryMenu.value, open: false }

  switch (key) {
    case 'add-root':
      openAdd()
      break
    case 'add-child':
      if (node) openAddChild(node.id)
      break
    case 'rename':
      if (node) openEdit(node)
      break
    case 'make-top':
      if (node && node.parentId) {
        if (!confirm(`将「${node.name}」置为顶级分类？`)) return
        try {
          await updateCategory(node.id, undefined, null)
        } catch (e) {
          toast.value = { show: true, type: 'error', message: '操作失败: ' + String(e) }
        }
      }
      break
    case 'delete':
      if (node) await handleDelete(node.id)
      break
  }
}

// ==================== 标签管理 ====================

/** 控制重命名标签弹窗是否显示 */
const showRenameTagDialog = ref(false)

/** 当前正在重命名的标签 */
const renamingTag = ref<TagInfo | null>(null)

/** 标签新名称输入 */
const renameTagName = ref('')

/**
 * 打开重命名标签弹窗
 * @param tag - 要重命名的标签
 */
const openRenameTag = (tag: TagInfo) => {
  renamingTag.value = tag
  renameTagName.value = tag.name
  showRenameTagDialog.value = true
}

/**
 * 提交标签重命名
 * 重命名到已存在的标签时后端会执行合并（两个标签都存在的密码去重）
 */
const handleRenameTag = async () => {
  const target = renamingTag.value
  if (!target) return
  const newName = renameTagName.value.trim()
  if (!newName) {
    toast.value = { show: true, type: 'error', message: '标签名称不能为空' }
    return
  }
  // 名称未变化，直接关闭弹窗
  if (newName === target.name) {
    showRenameTagDialog.value = false
    return
  }
  try {
    await renameTag(target.name, newName)
    showRenameTagDialog.value = false
    renamingTag.value = null
    renameTagName.value = ''
    toast.value = { show: true, type: 'success', message: '标签重命名成功' }
  } catch (e) {
    toast.value = { show: true, type: 'error', message: '重命名失败: ' + String(e) }
  }
}

/**
 * 删除标签
 * 删除后该标签会从所有密码中移除
 * @param name - 标签名称
 */
const handleDeleteTag = async (name: string) => {
  if (!confirm(`确定删除标签「${name}」？该标签将从所有密码中移除`)) return
  try {
    await deleteTag(name)
    toast.value = { show: true, type: 'success', message: '标签删除成功' }
  } catch (e) {
    toast.value = { show: true, type: 'error', message: '删除失败: ' + String(e) }
  }
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <!-- 顶部导航 -->
    <header class="border-b px-4 py-3 flex items-center gap-4">
      <Button variant="ghost" size="icon" @click="router.push('/home')">
        <IconArrowLeft class="h-5 w-5" />
      </Button>
      <h1 class="text-lg font-semibold">分类管理</h1>
    </header>

    <!-- 内容区 -->
    <div class="max-w-2xl mx-auto p-6">
      <div class="flex justify-between items-center mb-6">
        <p class="text-muted-foreground">管理您的密码分类与标签</p>
        <Button @click="openAdd">
          <IconPlus class="h-4 w-4 mr-2" />
          添加分类
        </Button>
      </div>

      <!-- 智能视图概览（虚拟分类，只读） -->
      <div class="flex items-center gap-2 mb-3">
        <IconGrid class="h-4 w-4 text-muted-foreground" />
        <h2 class="text-sm font-medium">智能视图</h2>
        <span class="text-xs text-muted-foreground">（只读概览）</span>
      </div>
      <div class="space-y-3">
        <Card v-for="view in smartViews" :key="view.id">
          <CardContent class="p-4 flex items-center justify-between">
            <div>
              <h3 class="font-medium">{{ view.name }}</h3>
              <p class="text-sm text-muted-foreground">{{ view.count }} 个密码</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <!-- 分类树（深度优先，按层级缩进） -->
      <div class="flex items-center gap-2 mt-8 mb-3">
        <IconChevronRight class="h-4 w-4 text-muted-foreground" />
        <h2 class="text-sm font-medium">分类</h2>
        <span class="text-xs text-muted-foreground">共 {{ categoryTree.length }} 个</span>
      </div>

      <Card v-if="categoryTree.length === 0">
        <CardContent class="p-6 text-center text-sm text-muted-foreground">
          还没有分类，点击右上角「添加分类」创建
        </CardContent>
      </Card>

      <div v-else class="space-y-3" @contextmenu.prevent="openCategoryMenu($event, null)">
        <Card
          v-for="node in categoryTree"
          :key="node.id"
          :class="[
            'relative',
            isCategoryDragging(node.id) ? 'opacity-50' : '',
            categoryDropIndicator(node.id) === 'into' ? 'ring-2 ring-primary ring-inset' : '',
          ]"
          :data-category-id="node.id"
          @contextmenu.prevent.stop="openCategoryMenu($event, node.id)"
        >
          <!-- 拖拽插入指示线（上/下边缘） -->
          <span
            v-if="categoryDropIndicator(node.id) === 'before'"
            class="pointer-events-none absolute left-0 right-0 top-0 h-0.5 rounded bg-primary"
          />
          <span
            v-if="categoryDropIndicator(node.id) === 'after'"
            class="pointer-events-none absolute left-0 right-0 bottom-0 h-0.5 rounded bg-primary"
          />
          <CardContent
            class="p-4 flex items-center justify-between"
            :style="indentStyle(node.depth)"
          >
            <div class="min-w-0">
              <!-- 名称：按住即可拖拽排序（无拖拽手柄，自定义指针拖拽） -->
              <h3
                class="font-medium truncate cursor-grab touch-none select-none active:cursor-grabbing"
                :title="`${node.name}（按住名称可拖拽排序）`"
                @pointerdown="startCategoryDrag($event, node)"
              >{{ node.name }}</h3>
              <p class="text-sm text-muted-foreground">
                {{ node.directCount }} 个密码<span v-if="node.hasChildren"> · 含子分类共 {{ node.totalCount }} 个密码</span>
              </p>
            </div>
            <div class="flex items-center gap-1 shrink-0">
              <Button
                variant="ghost"
                size="icon"
                title="编辑分类"
                @click="openEdit(node)"
              >
                <IconPencil class="h-4 w-4" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                class="text-destructive hover:text-destructive"
                title="删除分类"
                :disabled="node.id === 'other'"
                @click="handleDelete(node.id)"
              >
                <IconTrash class="h-4 w-4" />
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>

      <!-- 标签管理 -->
      <div class="flex items-center justify-between mt-10 mb-3">
        <div class="flex items-center gap-2">
          <IconTag class="h-4 w-4 text-muted-foreground" />
          <h2 class="text-sm font-medium">标签管理</h2>
          <span class="text-xs text-muted-foreground">共 {{ tags.length }} 个</span>
        </div>
      </div>

      <Card v-if="tags.length === 0">
        <CardContent class="p-6 text-center text-sm text-muted-foreground">
          还没有标签，可在添加/编辑密码时创建
        </CardContent>
      </Card>

      <Card v-else>
        <CardContent class="p-0 divide-y">
          <div
            v-for="tag in tags"
            :key="tag.name"
            class="flex items-center justify-between gap-3 px-4 py-3"
          >
            <div class="flex items-center gap-2 min-w-0">
              <span class="text-sm font-medium truncate" :title="tag.name">{{ tag.name }}</span>
              <span class="shrink-0 text-xs text-muted-foreground">{{ tag.count }} 个密码</span>
            </div>
            <div class="flex items-center gap-1 shrink-0">
              <Button
                variant="ghost"
                size="icon"
                title="重命名标签"
                @click="openRenameTag(tag)"
              >
                <IconPencil class="h-4 w-4" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                class="text-destructive hover:text-destructive"
                title="删除标签"
                @click="handleDeleteTag(tag.name)"
              >
                <IconTrash class="h-4 w-4" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>

    <!-- 添加/编辑分类弹窗 -->
    <Dialog v-model:open="showAddDialog">
      <DialogContent class="sm:max-w-[360px]">
        <DialogHeader>
          <DialogTitle>{{ editingCategory ? '编辑分类' : '添加分类' }}</DialogTitle>
          <DialogDescription>
            {{ editingCategory ? '修改分类名称或父分类' : '创建一个新的密码分类' }}
          </DialogDescription>
        </DialogHeader>
        <div class="py-4 grid gap-4">
          <div class="grid gap-2">
            <Label for="category-name">分类名称</Label>
            <Input
              id="category-name"
              v-model="newCategoryName"
              placeholder="输入分类名称"
              @keydown.enter="handleSave"
            />
          </div>
          <div class="grid gap-2">
            <Label for="category-parent">父分类</Label>
            <select
              id="category-parent"
              v-model="parentId"
              class="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            >
              <option value="">无（顶级分类）</option>
              <option
                v-for="node in parentOptions"
                :key="node.id"
                :value="node.id"
              >
                {{ '— '.repeat(node.depth) }}{{ node.name }}
              </option>
            </select>
            <p v-if="editingCategory && descendantIds.size > 0" class="text-xs text-muted-foreground">
              已自动排除该分类自身及其子分类
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" @click="showAddDialog = false">取消</Button>
          <Button :disabled="!newCategoryName.trim()" @click="handleSave">
            {{ editingCategory ? '保存' : '添加' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- 重命名标签弹窗 -->
    <Dialog v-model:open="showRenameTagDialog">
      <DialogContent class="sm:max-w-[360px]">
        <DialogHeader>
          <DialogTitle>重命名标签</DialogTitle>
          <DialogDescription>
            重命名到已存在的标签时，两者将合并
          </DialogDescription>
        </DialogHeader>
        <div class="py-4">
          <Label for="tag-rename-name">标签名称</Label>
          <Input
            id="tag-rename-name"
            v-model="renameTagName"
            placeholder="输入新的标签名称"
            class="mt-2"
            @keydown.enter="handleRenameTag"
          />
          <p class="mt-2 text-xs text-muted-foreground">最多 32 个字符</p>
        </div>
        <DialogFooter>
          <Button variant="outline" @click="showRenameTagDialog = false">取消</Button>
          <Button :disabled="!renameTagName.trim()" @click="handleRenameTag">保存</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- 分类右键菜单（空白处新增顶级分类；分类行上新增子分类/重命名/置顶级/删除） -->
    <ContextMenu
      v-model:open="categoryMenu.open"
      :x="categoryMenu.x"
      :y="categoryMenu.y"
      :items="categoryMenuItems"
      @select="handleCategoryMenuSelect"
    />

    <!-- Toast 提示 -->
    <Toast
      v-model:show="toast.show"
      :type="toast.type"
      :message="toast.message"
    />
  </div>
</template>
