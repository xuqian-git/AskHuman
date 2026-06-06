"use strict";

// 解析当前平台的 AskHuman 二进制路径，供下游程序集成调用。
// 解析顺序：环境变量 ASKHUMAN_BINARY（兼容旧 HUMANINLOOP_BINARY）→ 平台子包 → 系统 PATH。

const fs = require("fs");
const path = require("path");

// platformKey -> 平台子包名
const PLATFORM_PACKAGES = {
  "darwin-arm64": "@humaninloop/darwin-arm64",
  "darwin-x64": "@humaninloop/darwin-x64",
  "win32-x64": "@humaninloop/win32-x64",
  "linux-x64": "@humaninloop/linux-x64",
};

function binName() {
  return process.platform === "win32" ? "AskHuman.exe" : "AskHuman";
}

function platformKey() {
  return `${process.platform}-${process.arch}`;
}

function isExecutableFile(p) {
  try {
    const st = fs.statSync(p);
    if (!st.isFile()) return false;
  } catch {
    return false;
  }
  if (process.platform === "win32") return true; // X_OK 在 Windows 无意义
  try {
    fs.accessSync(p, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function fromEnv() {
  const p = process.env.ASKHUMAN_BINARY || process.env.HUMANINLOOP_BINARY;
  return p && fs.existsSync(p) ? p : null;
}

function fromPlatformPackage() {
  const pkg = PLATFORM_PACKAGES[platformKey()];
  if (!pkg) return null;
  try {
    return require.resolve(`${pkg}/bin/${binName()}`);
  } catch {
    return null;
  }
}

function fromPath() {
  const exe = binName();
  const dirs = (process.env.PATH || "").split(path.delimiter);
  for (const dir of dirs) {
    if (!dir) continue;
    const candidate = path.join(dir, exe);
    if (fs.existsSync(candidate)) return candidate;
  }
  return null;
}

// 返回二进制绝对路径；找不到返回 null。
function getBinaryPath() {
  return fromEnv() || fromPlatformPackage() || fromPath() || null;
}

// 二进制是否已就位且可执行。
// 注意：仅代表二进制可被调用，不代表 GUI 弹窗可用；
// 具体哪个 channel 可用由二进制运行期决定（见退出码契约）。
function isAvailable() {
  const p = getBinaryPath();
  return p != null && isExecutableFile(p);
}

module.exports = { getBinaryPath, isAvailable };
