//! 后端国际化：单一配置 `general.language`（auto|en|zh）解析为 `Lang`，
//! 用于 CLI / 窗口标题 / macOS 菜单·Dock / 通知 / 远程渠道等用户可见文案。
//!
//! 源语言为英文；缺失/未知一律回退英文。词条在各里程碑逐步扩充（M4/M5）。

use crate::config::AppConfig;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// 由配置语言字符串解析：显式 en/zh 直用；其余（auto/未知）跟随系统。
    pub fn resolve(cfg_language: &str) -> Lang {
        match cfg_language {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            _ => Lang::from_system(),
        }
    }

    /// 跟随系统：首选语言以 "zh" 开头→中文，否则英文。
    pub fn from_system() -> Lang {
        match sys_locale::get_locale() {
            Some(l) if l.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
            _ => Lang::En,
        }
    }

    /// 读取已保存配置解析当前界面语言。
    pub fn current() -> Lang {
        Lang::resolve(&AppConfig::load().general.language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_overrides_system() {
        assert_eq!(Lang::resolve("en"), Lang::En);
        assert_eq!(Lang::resolve("zh"), Lang::Zh);
    }

    #[test]
    fn auto_or_unknown_follows_system() {
        // 仅验证不 panic 且落在二者之一。
        let a = Lang::resolve("auto");
        let b = Lang::resolve("nonsense");
        assert!(matches!(a, Lang::En | Lang::Zh));
        assert!(matches!(b, Lang::En | Lang::Zh));
    }
}
