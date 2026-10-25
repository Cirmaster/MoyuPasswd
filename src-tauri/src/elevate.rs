//! 管理员身份模块
//!
//! 两件事：
//! 1. `is_elevated`：当前进程是否以管理员身份运行（设置页展示用）。
//! 2. `set_run_as_admin`：开关「启动时以管理员身份运行」——
//!    写/删 HKCU 下 AppCompatFlags\Layers 兼容层标记（`~ RUNASADMIN`），
//!    由 Windows 在下次拉起本程序时弹 UAC 以管理员身份启动。
//!
//! 为什么走系统兼容层而不是「启动时 ShellExecute runas 自提权重启」：
//! - 本应用带单实例守卫（tauri-plugin-single-instance），提权重启出的新进程
//!   会被判定为重复实例直接退出，只剩旧进程退出后的空窗；
//! - 兼容层由 Windows 在进程创建前完成提权，天然只有一个实例，启动流程零改动。
//!
//! 注意：该标记以 exe 完整路径为值名。换路径安装/更新后重新保存一次设置即可
//! （保存时按当前 exe 路径重写），否则 Windows 找不到标记、启动仍是普通权限。

/// Windows 实现（注册表兼容层 + 令牌提权检测）
#[cfg(windows)]
mod win_impl {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::{w, PCWSTR};

    /// 兼容层标记所在注册表键（用户级，普通权限即可写，无需管理员）
    const LAYERS_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";

    /// 「以管理员身份运行」兼容层标记值
    /// （与资源管理器「兼容性 → 以管理员身份运行」勾选后写入的值一致）
    const RUN_AS_ADMIN_VALUE: &str = "~ RUNASADMIN";

    /// 把 &str 转成注册表用的 NUL 结尾 UTF-16 宽字符串
    fn to_wide(s: &str) -> Vec<u16> {
        let mut wide: Vec<u16> = s.encode_utf16().collect();
        wide.push(0);
        wide
    }

    /// 当前进程是否以管理员身份运行
    ///
    /// 查访问令牌的 TokenElevation（Vista+）；任何一步失败按「非管理员」处理，
    /// 宁可少报不误报。
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token = Default::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            let _ = CloseHandle(token);
            ok.is_ok() && elevation.TokenIsElevated != 0
        }
    }

    /// 设置「启动时以管理员身份运行」开关
    ///
    /// - `enable = true`：往兼容层键写 `~ RUNASADMIN`（值名 = 当前 exe 完整路径）
    /// - `enable = false`：删除该值（不存在视为成功，保证幂等）
    ///
    /// 整值由本开关管理（覆盖写 / 整值删），不与手动设置的其他兼容层合并。
    ///
    /// 开发模式（cargo run）只「关」不「开」：标记会让 cargo 拉起 app.exe 时报
    /// os error 740（请求的操作需要提升），dev 起不来；正式安装包不受影响。
    /// 「关」照常执行，顺便清理历史标记，dev 里也能把误写的标记删掉。
    pub fn set_run_as_admin(enable: bool) -> Result<(), String> {
        if enable && cfg!(debug_assertions) {
            log::warn!(
                "开发运行跳过「以管理员身份运行」标记写入（cargo 无法拉起需提权的进程），该开关在安装包下生效"
            );
            return Ok(());
        }

        // 值名 = exe 完整路径：Windows 兼容层按「哪个程序」匹配，路径即身份。
        // 用 encode_wide 从 OsStr 直接转宽字符，不做有损字符串转换
        let exe = std::env::current_exe()
            .map_err(|e| format!("获取当前 exe 路径失败: {e}"))?;
        let mut value_name: Vec<u16> = exe.as_os_str().encode_wide().collect();
        value_name.push(0);
        let key_path = to_wide(LAYERS_KEY);

        unsafe {
            let mut hkey = HKEY::default();
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                0,
                w!(""),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            );
            if status != ERROR_SUCCESS {
                return Err(format!("打开兼容层注册表键失败: {:?}", status));
            }

            let result = if enable {
                // REG_SZ 数据同样要 NUL 结尾的 UTF-16 字节序列
                let data = to_wide(RUN_AS_ADMIN_VALUE);
                let bytes: Vec<u8> = data.iter().flat_map(|u| u.to_le_bytes()).collect();
                let status = RegSetValueExW(
                    hkey,
                    PCWSTR(value_name.as_ptr()),
                    0,
                    REG_SZ,
                    Some(&bytes),
                );
                if status == ERROR_SUCCESS {
                    Ok(())
                } else {
                    Err(format!("写入管理员启动标记失败: {:?}", status))
                }
            } else {
                let status = RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()));
                // 值本来就不存在 = 已处于目标状态，幂等成功
                if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("删除管理员启动标记失败: {:?}", status))
                }
            };

            let _ = RegCloseKey(hkey);
            result
        }
    }
}

/// 非 Windows 平台：不支持提权语义，读作「非管理员」，设置写入空操作
#[cfg(not(windows))]
mod win_impl {
    pub fn is_elevated() -> bool {
        false
    }

    pub fn set_run_as_admin(_enable: bool) -> Result<(), String> {
        Ok(())
    }
}

pub use win_impl::{is_elevated, set_run_as_admin};
