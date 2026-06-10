# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## Daemon 优雅排空换新（graceful drain）

代码已实现、单测全过、已 install 实测通过（spec/plan 见 `docs/specs/daemon-graceful-drain.md` / `docs/plans/daemon-graceful-drain.md`）。实测覆盖：A1 占位提问在途 → 装新指纹二进制 → A2 撞排空打印等待提示并阻塞、`daemon status` 标注 `(draining)` → 答 A1 后旧 daemon 排空退出、新 daemon 自动拉起、A2 弹窗出现并完成；`daemon stop --force` 立即终止。同轮顺带验证 `-o!`（弹窗推荐 Badge、提交回原文）。剩余：**未提交**，待用户确认后提交。

> 测试注意（经验）：手动安装临时二进制验证 drain 时，**必须用稳定身份 `com.naituw.humaninloop` 重新签名**（同 install.sh），否则 ad-hoc 签名的 daemon 启动读密钥会触发大量钥匙串弹框；且覆盖正在运行的 daemon 可执行文件要用**原子 mv（换新 inode）**而非原地 cp，否则该路径新 exec 会被 SIGKILL。
