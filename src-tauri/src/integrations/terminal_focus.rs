//! 聚焦 Agent 所在的终端（实验性，macOS）。
//!
//! 思路（spec 见 docs/overview.md「Agent 生命周期」节）：由存活的 agent 进程 pid 取其控制终端
//! tty（`ps -o tty=`）+ 终端类型（`agents::detect::terminal_kind`），再用 AppleScript 在对应终端
//! App 里按 `tty` 精确匹配标签页 / 会话，命中则选中并把窗口置前、激活该 App。
//!
//! 已支持：**Terminal.app**（`tab` 的 `tty` 属性）、**iTerm2**（`session` 的 `tty` 属性）。
//! 其余终端（kitty/WezTerm/tmux/编辑器内置终端…）暂不支持。失败（非 macOS / 无 tty / 不支持的
//! 终端 / 未授权自动化 / 找不到）一律返回 `Err`，调用方静默处理。

/// 把 `pid` 所在终端的标签页 / 会话切到前台。成功返回 `Ok(())`。
pub fn focus_agent_terminal(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let dev = tty_device(pid)?;
        match crate::agents::detect::terminal_kind(pid) {
            Some("apple-terminal") => run_focus_script(&apple_terminal_script(&dev)),
            Some("iterm2") => run_focus_script(&iterm2_script(&dev)),
            other => Err(format!(
                "focus terminal: unsupported terminal {}",
                other.unwrap_or("unknown")
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        Err("focus terminal: only supported on macOS for now".to_string())
    }
}

/// 取进程的控制终端设备路径，如 `/dev/ttys003`。无控制终端（MCP / 后台）返回 `Err`。
#[cfg(target_os = "macos")]
fn tty_device(pid: u32) -> Result<String, String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("ps failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("process {pid} not found"));
    }
    let tty = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // 无控制终端时 ps 打印 "??" / "?" / "-"。
    if tty.is_empty() || tty.contains('?') || tty == "-" {
        return Err(format!("process {pid} has no controlling terminal"));
    }
    Ok(if tty.starts_with("/dev/") {
        tty
    } else {
        format!("/dev/{tty}")
    })
}

/// Terminal.app：按 tab 的 `tty` 匹配，选中标签页 + 窗口置前 + 激活。
#[cfg(target_os = "macos")]
fn apple_terminal_script(dev: &str) -> String {
    // dev 仅含 [a-z0-9/]，可安全内联进 AppleScript 字符串字面量。
    format!(
        r#"tell application "Terminal"
    set theTTY to "{dev}"
    set didFocus to false
    repeat with w in windows
        repeat with tb in tabs of w
            if (tty of tb) is theTTY then
                set selected of tb to true
                set frontmost of w to true
                set didFocus to true
            end if
        end repeat
    end repeat
    if didFocus then activate
    return didFocus
end tell"#
    )
}

/// iTerm2：按 session 的 `tty` 匹配，选中会话 / 标签页 / 窗口并激活。
/// 用 bundle id 定位（`com.googlecode.iterm2`），避免 "iTerm" / "iTerm2" 应用名歧义。
#[cfg(target_os = "macos")]
fn iterm2_script(dev: &str) -> String {
    format!(
        r#"tell application id "com.googlecode.iterm2"
    set theTTY to "{dev}"
    set didFocus to false
    repeat with w in windows
        repeat with t in tabs of w
            repeat with s in sessions of t
                if (tty of s) is theTTY then
                    tell w to select
                    tell t to select
                    tell s to select
                    set didFocus to true
                end if
            end repeat
        end repeat
    end repeat
    if didFocus then activate
    return didFocus
end tell"#
    )
}

/// 运行聚焦用 AppleScript：脚本约定返回 `true`(命中) / `false`(未找到)；非零退出多为未授权自动化。
#[cfg(target_os = "macos")]
fn run_focus_script(script: &str) -> Result<(), String> {
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("osascript failed: {e}"))?;
    if !out.status.success() {
        // 典型为未授权自动化（errAEEventNotPermitted / -1743）。
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    if String::from_utf8_lossy(&out.stdout).trim() == "true" {
        Ok(())
    } else {
        Err("no matching terminal tab/session found".to_string())
    }
}
