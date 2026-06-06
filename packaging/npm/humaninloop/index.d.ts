/**
 * 返回当前平台的 AskHuman 二进制绝对路径。
 * 解析顺序：环境变量 `ASKHUMAN_BINARY`（兼容旧 `HUMANINLOOP_BINARY`）→ 平台子包 → 系统 `PATH`。
 * 找不到时返回 `null`。
 */
export declare function getBinaryPath(): string | null;

/**
 * 二进制是否已就位且可执行。
 * 注意：仅代表二进制可被调用，不代表 GUI 弹窗可用；
 * 具体哪个 channel 可用由二进制运行期决定（见退出码契约：3 = 无可用 channel）。
 */
export declare function isAvailable(): boolean;
