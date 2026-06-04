//! 让弹窗使用 macOS 原生窗口出现动画（缩放 + 淡入）。
//!
//! 关键：`NSWindowAnimationBehavior` 的取值为
//! Default=0、None=2、DocumentWindow=3、UtilityWindow=4、AlertPanel=5。
//! （注意 2 是 None，会禁用动画。）具体取值由用户在设置页选择。
//!
//! 用法：窗口需「隐藏构建」，设好 animationBehavior 后再 `show()`，
//! 这样 `orderFront` 才会播放系统出现动画。

use objc2::msg_send;
use objc2::runtime::AnyObject;
use std::ffi::c_void;

/// 给 `NSWindow` 设置原生出现动画行为；`behavior` 为 `NSWindowAnimationBehavior` 原始取值。
/// `ns_window` 来自 `WebviewWindow::ns_window()`；为空则忽略。
pub fn set_appear_animation(ns_window: *mut c_void, behavior: isize) {
    if ns_window.is_null() {
        return;
    }
    let win = ns_window as *mut AnyObject;
    unsafe {
        let _: () = msg_send![win, setAnimationBehavior: behavior];
    }
}
