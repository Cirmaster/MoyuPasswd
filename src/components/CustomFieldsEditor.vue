<!--
  CustomFieldsEditor.vue - 自定义字段编辑器

  编辑一条密码携带的附加字段（如数据库连接地址/端口/连接命令）。
  每行：字段名 + 字段值 + 敏感开关 + 删除按钮。
  敏感字段的值加密存储，快捷查询中需展开详情才能查看。

  @example
  ```vue
  <CustomFieldsEditor v-model="form.extra_fields" />
  ```
-->

<script setup lang="ts">
import type { Directive } from 'vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import type { CustomField } from '@/stores/password'

/**
 * 组件 Props
 */
interface Props {
  /** 字段列表（v-model 双向绑定） */
  modelValue: CustomField[]
}

const props = defineProps<Props>()

/**
 * 组件事件
 * - update:modelValue: 更新字段列表（v-model 双向绑定）
 */
const emit = defineEmits<{
  'update:modelValue': [value: CustomField[]]
}>()

/**
 * 按内容自动增高 textarea（长值如连接命令不再被裁成一行）
 */
const resizeTextarea = (el: HTMLElement) => {
  const ta = el as HTMLTextAreaElement
  ta.style.height = 'auto'
  ta.style.height = `${ta.scrollHeight}px`
}

/** v-auto-grow 指令：挂载/更新时都重算高度（覆盖详情载入等场景） */
const vAutoGrow: Directive = {
  mounted: resizeTextarea,
  updated: resizeTextarea,
}

/**
 * 更新指定行的字段
 * @param index - 行索引
 * @param patch - 要合并的字段变更
 */
const updateField = (index: number, patch: Partial<CustomField>) => {
  const next = props.modelValue.map((f, i) => (i === index ? { ...f, ...patch } : f))
  emit('update:modelValue', next)
}

/**
 * 新增一行空字段
 */
const addField = () => {
  emit('update:modelValue', [
    ...props.modelValue,
    { label: '', value: '', sensitive: false },
  ])
}

/**
 * 删除指定行
 * @param index - 行索引
 */
const removeField = (index: number) => {
  emit('update:modelValue', props.modelValue.filter((_, i) => i !== index))
}

/**
 * 插入数据库连接模板
 * 一键填入连接地址/端口/数据库名/连接命令四行（连接命令默认敏感）
 */
const insertDbTemplate = () => {
  emit('update:modelValue', [
    ...props.modelValue,
    { label: '连接地址', value: '', sensitive: false },
    { label: '端口', value: '', sensitive: false },
    { label: '数据库名', value: '', sensitive: false },
    { label: '连接命令', value: '', sensitive: true },
  ])
}
</script>

<template>
  <div class="grid gap-2">
    <!-- 标题行 -->
    <div class="flex items-center justify-between">
      <Label>自定义字段</Label>
      <div class="flex gap-1">
        <Button variant="ghost" size="sm" class="h-7 px-2 text-xs" @click="insertDbTemplate">
          数据库模板
        </Button>
        <Button variant="outline" size="sm" class="h-7 px-2 text-xs" @click="addField">
          + 添加字段
        </Button>
      </div>
    </div>

    <!-- 空状态提示 -->
    <p v-if="modelValue.length === 0" class="text-xs text-muted-foreground">
      可添加连接地址、端口、连接命令等附加信息，查询时可单独复制
    </p>

    <!-- 字段行：字段名定宽 + 值自适应增高 + 敏感开关 + 删除 -->
    <div
      v-for="(field, index) in modelValue"
      :key="index"
      class="flex items-start gap-2"
    >
      <Input
        :model-value="field.label"
        placeholder="字段名"
        class="w-24 h-9 shrink-0"
        @update:model-value="updateField(index, { label: $event as string })"
      />
      <!-- 值：auto-grow textarea；敏感字段以圆点掩码显示（WebView2 支持 -webkit-text-security） -->
      <textarea
        v-auto-grow
        :value="field.value"
        :placeholder="field.sensitive ? '敏感字段值（加密存储）' : '字段值'"
        rows="1"
        class="flex-1 min-w-0 resize-none overflow-hidden rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
        :class="{ 'sensitive-mask': field.sensitive }"
        @input="updateField(index, { value: ($event.target as HTMLTextAreaElement).value })"
      ></textarea>
      <!-- 敏感开关：开启后字段值加密存储 -->
      <label
        class="flex items-center gap-1 shrink-0 h-9 text-xs text-muted-foreground cursor-pointer"
        title="敏感字段的值加密存储，查询时需展开详情查看"
      >
        <input
          :checked="field.sensitive"
          type="checkbox"
          class="h-3.5 w-3.5"
          @change="updateField(index, { sensitive: ($event.target as HTMLInputElement).checked })"
        />
        敏感
      </label>
      <!-- 删除按钮 -->
      <Button
        variant="ghost"
        size="sm"
        class="h-9 w-7 p-0 shrink-0 text-muted-foreground"
        title="删除字段"
        @click="removeField(index)"
      >
        ✕
      </Button>
    </div>
  </div>
</template>

<style scoped>
/* 敏感字段值掩码（Chromium/WebView2 专有属性，不支持时降级为明文显示） */
.sensitive-mask {
  -webkit-text-security: disc;
}
</style>
