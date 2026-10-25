<!--
  BackupPanel.vue - 设置页「备份」页签

  列出磁盘上可恢复的历史数据库副本，并提供恢复入口：
  - 旧库隔离：启动时密钥不匹配的旧库被改名保留，随库带凭据快照
  - 恢复前的当前库：上次恢复操作自动另存，用于回退误操作
  - 改密前备份：修改主密码前的整库备份

  恢复会整体替换当前库并立即锁定应用，用户需用该备份当时的主密码解锁。
  后端 `restore_backup` 负责文件操作并 emit `app-locked`，本组件不做跳转。
-->
<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { invoke } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { IconClock, IconInfoCircle, IconLoader, IconTray } from '@/components/icons'

/** 与后端 `db::BackupEntry` 一一对应 */
interface BackupEntry {
  /** 恢复标识（受控文件名） */
  file_name: string
  /** quarantine | current | change_password */
  kind: string
  /** 生成时间（Unix 毫秒） */
  created_at_ms: number
  /** 文件大小（字节） */
  size_bytes: number
  /** 是否带凭据快照 */
  has_meta_snapshot: boolean
}

const emit = defineEmits<{
  (e: 'toast', type: 'success' | 'error', message: string): void
}>()

/** 备份列表 */
const entries = ref<BackupEntry[]>([])
/** 列表加载中 */
const loading = ref(false)
/** 正在恢复的条目标识 */
const restoring = ref('')
/** 已展开二次确认的条目 */
const pendingRestore = ref<BackupEntry | null>(null)
/** 列表读取错误 */
const error = ref('')

/** 拉取可恢复副本列表 */
const load = async () => {
  loading.value = true
  error.value = ''
  try {
    entries.value = await invoke<BackupEntry[]>('get_backups')
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

/** 类别展示文案 */
const kindLabel = (kind: string): string => {
  switch (kind) {
    case 'quarantine':
      return '旧库隔离'
    case 'current':
      return '恢复前的当前库'
    case 'change_password':
      return '改密前备份'
    default:
      return kind
  }
}

/** 毫秒时间戳 → 本地时间字符串 */
const formatTime = (ms: number): string =>
  ms ? new Date(ms).toLocaleString('zh-CN', { hour12: false }) : '时间未知'

/** 字节 → 人类可读大小 */
const formatSize = (bytes: number): string =>
  bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : `${Math.max(1, Math.round(bytes / 1024))} KB`

/** 执行恢复（成功后应用会被后端锁定，组件随即卸载） */
const confirmRestore = async () => {
  const target = pendingRestore.value
  if (!target) return
  restoring.value = target.file_name
  try {
    await invoke('restore_backup', { fileName: target.file_name })
  } catch (e) {
    emit('toast', 'error', '恢复失败: ' + String(e))
    restoring.value = ''
    pendingRestore.value = null
  }
}
</script>

<template>
  <div class="space-y-4">
    <div class="flex items-start justify-between gap-4">
      <div>
        <h3 class="font-medium">历史库备份</h3>
        <p class="text-sm text-muted-foreground">
          恢复会整体替换当前密码库并立即锁定应用，请用该备份当时的主密码解锁。
        </p>
      </div>
      <Button variant="outline" size="sm" :disabled="loading" @click="load">
        <IconLoader v-if="loading" class="h-3.5 w-3.5 animate-spin" />
        <span v-else>刷新</span>
      </Button>
    </div>

    <!-- 读取失败 -->
    <p v-if="error" class="text-sm text-destructive">读取备份列表失败：{{ error }}</p>

    <!-- 空状态 -->
    <div v-else-if="!loading && entries.length === 0" class="p-6 border rounded-lg text-center">
      <IconTray class="h-8 w-8 mx-auto mb-2 text-muted-foreground" />
      <p class="text-sm text-muted-foreground">暂无历史备份</p>
      <p class="text-xs text-muted-foreground mt-1">
        系统在隔离旧库、修改主密码前会自动留档，届时会出现在这里。
      </p>
    </div>

    <!-- 备份列表 -->
    <div v-else class="space-y-3">
      <div v-for="item in entries" :key="item.file_name" class="p-4 border rounded-lg space-y-3">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <div class="flex items-center gap-2 flex-wrap">
              <span class="text-sm font-medium">{{ kindLabel(item.kind) }}</span>
              <span
                v-if="item.has_meta_snapshot"
                class="text-xs px-1.5 py-0.5 rounded bg-primary/10 text-primary-ink"
              >可直锁</span>
              <span
                v-else
                class="text-xs px-1.5 py-0.5 rounded bg-muted text-muted-foreground"
              >缺凭据快照</span>
            </div>
            <p class="text-xs text-muted-foreground mt-1 flex items-center gap-3 flex-wrap">
              <span class="inline-flex items-center gap-1">
                <IconClock class="h-3 w-3" />{{ formatTime(item.created_at_ms) }}
              </span>
              <span>{{ formatSize(item.size_bytes) }}</span>
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            :disabled="restoring === item.file_name"
            @click="pendingRestore = item"
          >恢复</Button>
        </div>

        <!-- 二次确认：把后果讲清楚再动手 -->
        <div v-if="pendingRestore?.file_name === item.file_name" class="p-3 rounded-md bg-muted/50 space-y-2">
          <p class="text-sm flex items-start gap-2">
            <IconInfoCircle class="h-4 w-4 mt-0.5 shrink-0 text-muted-foreground" />
            <span>
              当前库会另存为「恢复前的当前库」，之后仍能在本列表里恢复回来。
              <template v-if="item.has_meta_snapshot">
                该备份带凭据快照，可用当时的主密码直接解锁。
              </template>
              <template v-else>
                该备份<strong>没有</strong>凭据快照；若恢复后解不开，请立即恢复「恢复前的当前库」。
              </template>
            </span>
          </p>
          <div class="flex items-center gap-2">
            <Button size="sm" :disabled="restoring === item.file_name" @click="confirmRestore">
              <IconLoader v-if="restoring === item.file_name" class="h-3.5 w-3.5 animate-spin" />
              <span v-else>确认恢复</span>
            </Button>
            <Button variant="ghost" size="sm" @click="pendingRestore = null">取消</Button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
