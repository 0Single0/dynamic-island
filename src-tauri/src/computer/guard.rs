use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{
            OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, IsWindowVisible},
    },
};

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn allowed(hwnd: isize, pid: u32, title: &str) -> Result<String, String> {
    unsafe {
        let hwnd = HWND(hwnd as *mut _);
        let mut actual_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut actual_pid));
        if !IsWindow(Some(hwnd)).as_bool()
            || !IsWindowVisible(hwnd).as_bool()
            || actual_pid != pid
            || pid == std::process::id()
        {
            return Err("目标窗口不存在、身份改变或属于 Agent 自身".into());
        }
        let process = OwnedHandle(
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                .map_err(|_| "无法验证目标进程权限")?,
        );
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token).map_err(|_| "无法验证目标权限")?;
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut length = 0;
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
        .map_err(|_| "无法验证权限级别")?;
        if elevation.TokenIsElevated != 0 {
            return Err("不允许操作管理员窗口".into());
        }
        let mut path = [0u16; 2048];
        let mut size = path.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut size,
        )
        .map_err(|_| "无法确认进程路径")?;
        let path = String::from_utf16_lossy(&path[..size as usize]);
        let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
        if blocked(&name, title) {
            return Err("此窗口属于受保护目标，不能通过电脑插件读写".into());
        }
        Ok(name)
    }
}

fn blocked(process: &str, title: &str) -> bool {
    let title = title.to_lowercase();
    // Terminal and browser automation need their own scoped backends, not generic UIA input.
    let processes = [
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "windowsterminal.exe",
        "conhost.exe",
        "regedit.exe",
        "mmc.exe",
        "taskmgr.exe",
        "credentialuibroker.exe",
        "consent.exe",
        "logonui.exe",
        "chrome.exe",
        "msedge.exe",
        "firefox.exe",
        "brave.exe",
        "code.exe",
        "codex.exe",
    ];
    processes.contains(&process)
        || [
            "devtools",
            "developer tools",
            "开发者工具",
            "password",
            "密码",
            "captcha",
            "验证码",
            "payment",
            "支付",
            "credential",
            "凭据",
            "administrator",
            "管理员",
        ]
        .iter()
        .any(|s| title.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_targets_are_denied() {
        assert!(blocked("powershell.exe", "console"));
        assert!(blocked("chrome.exe", "normal page"));
        assert!(blocked("notepad.exe", "支付确认"));
        assert!(!blocked("notepad.exe", "Untitled - Notepad"));
    }
}
