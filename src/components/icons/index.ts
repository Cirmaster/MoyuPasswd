/**
 * index.ts - 全局 SVG 图标库（统一导入入口）
 *
 * 图标源文件是真正的 .svg 文件，存放在 src/assets/svg/（IDE 可直接预览），
 * 由 vite-svg-loader 在构建时转为 Vue 组件使用。
 * 所有界面图标统一从这里导入复用，不要在组件里内联 SVG。
 *
 * @example
 * ```vue
 * import { IconSearch, IconLock } from '@/components/icons'
 * <IconSearch class="h-4 w-4" />
 * ```
 */

import IconArrowLeft from '@/assets/svg/arrow-left.svg'
import IconArrowUp from '@/assets/svg/arrow-up.svg'
import IconArrowDown from '@/assets/svg/arrow-down.svg'
import IconCheckCircle from '@/assets/svg/check-circle.svg'
import IconChevronRight from '@/assets/svg/chevron-right.svg'
import IconChevronDown from '@/assets/svg/chevron-down.svg'
import IconClock from '@/assets/svg/clock.svg'
import IconClose from '@/assets/svg/close.svg'
import IconCopy from '@/assets/svg/copy.svg'
import IconEmpty from '@/assets/svg/empty.svg'
import IconGrid from '@/assets/svg/grid.svg'
import IconInfoCircle from '@/assets/svg/info-circle.svg'
import IconKey from '@/assets/svg/key.svg'
import IconLoader from '@/assets/svg/loader.svg'
import IconLock from '@/assets/svg/lock.svg'
import IconLogoLock from '@/assets/svg/logo-lock.svg'
import IconMinus from '@/assets/svg/minus.svg'
import IconMoon from '@/assets/svg/moon.svg'
import IconPencil from '@/assets/svg/pencil.svg'
import IconPlus from '@/assets/svg/plus.svg'
import IconSearch from '@/assets/svg/search.svg'
import IconSettings from '@/assets/svg/settings.svg'
import IconSpinner from '@/assets/svg/spinner.svg'
import IconSquarePen from '@/assets/svg/square-pen.svg'
import IconStar from '@/assets/svg/star.svg'
import IconSun from '@/assets/svg/sun.svg'
import IconTag from '@/assets/svg/tag.svg'
import IconTrash from '@/assets/svg/trash.svg'
import IconTray from '@/assets/svg/tray.svg'
import IconXCircle from '@/assets/svg/x-circle.svg'

export {
  IconArrowLeft,
  IconArrowUp,
  IconArrowDown,
  IconCheckCircle,
  IconChevronRight,
  IconChevronDown,
  IconClock,
  IconClose,
  IconCopy,
  IconEmpty,
  IconGrid,
  IconInfoCircle,
  IconKey,
  IconLoader,
  IconLock,
  IconLogoLock,
  IconMinus,
  IconMoon,
  IconPencil,
  IconPlus,
  IconSearch,
  IconSettings,
  IconSpinner,
  IconSquarePen,
  IconStar,
  IconSun,
  IconTag,
  IconTrash,
  IconTray,
  IconXCircle,
}
