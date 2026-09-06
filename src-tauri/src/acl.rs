//! 文件 ACL 加固模块
//!
//! 在 Windows 上把数据库文件的安全描述符替换为仅当前用户可访问，
//! 并阻止从父目录继承权限，防止同机其他账户读取密码库文件。
//! 非 Windows 平台为无操作。

/// 收紧文件 ACL：仅当前用户可访问（Windows），失败不致命，由调用方记录日志。
pub fn harden_file_acl(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        win_impl::harden_file_acl(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
mod win_impl {
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE};
    use windows::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows::Win32::Security::{
        AddAccessAllowedAce, GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor,
        SetSecurityDescriptorDacl, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, PSECURITY_DESCRIPTOR, SECURITY_DESCRIPTOR,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub fn harden_file_acl(path: &Path) -> Result<(), String> {
        unsafe {
            // 1. 获取当前进程令牌中的用户 SID
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(|e| format!("OpenProcessToken 失败: {e}"))?;

            let mut size = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            if size == 0 {
                let _ = CloseHandle(token);
                return Err("GetTokenInformation 获取大小失败".to_string());
            }

            let mut sid_buf = vec![0u8; size as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(sid_buf.as_mut_ptr() as *mut core::ffi::c_void),
                size,
                &mut size,
            )
            .map_err(|e| format!("GetTokenInformation 失败: {e}"))?;
            let _ = CloseHandle(token);

            let token_user = &*(sid_buf.as_ptr() as *const TOKEN_USER);
            let user_sid: PSID = token_user.User.Sid;

            // 2. 构建 DACL：仅当前用户 FULL 访问
            let mut acl_buf = vec![0u8; 256];
            let acl = acl_buf.as_mut_ptr() as *mut ACL;
            InitializeAcl(acl, 256, ACL_REVISION)
                .map_err(|e| format!("InitializeAcl 失败: {e}"))?;
            AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_ALL.0, user_sid)
                .map_err(|e| format!("AddAccessAllowedAce 失败: {e}"))?;

            // 3. 构建安全描述符并挂载 DACL
            let mut sd = SECURITY_DESCRIPTOR::default();
            let sd_ptr =
                PSECURITY_DESCRIPTOR(&mut sd as *mut _ as *mut core::ffi::c_void);
            InitializeSecurityDescriptor(sd_ptr, 1)
                .map_err(|e| format!("InitializeSecurityDescriptor 失败: {e}"))?;
            SetSecurityDescriptorDacl(sd_ptr, true, Some(acl), false)
                .map_err(|e| format!("SetSecurityDescriptorDacl 失败: {e}"))?;

            // 4. 应用：仅 DACL + 阻止继承（PROTECTED_DACL）
            let wide: Vec<u16> = path
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let result = SetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            );
            if result.0 != 0 {
                return Err(format!("SetNamedSecurityInfoW 失败: {:?}", result));
            }
            Ok(())
        }
    }
}
