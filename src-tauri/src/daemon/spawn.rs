//! 后台拉起 Daemon：detach（新会话）+ stdio 重定向到 daemon.log，使其脱离 CLI 终端独立存活。

#[cfg(unix)]
pub fn spawn_detached() -> std::io::Result<()> {
    use super::lifecycle;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe()?;
    if let Some(dir) = lifecycle::log_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(lifecycle::log_path())?;
    let log_err = log.try_clone()?;

    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // 新建会话，彻底脱离调用方的控制终端 / 进程组（终端关闭不会带走 daemon）。
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}
