//! 结果区块格式化（与当前 Swift 版逐字对齐）。

pub const CANCEL_STATUS_TEXT: &str =
    "用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复";

/// 取消路径输出。
pub fn cancel_output() -> String {
    format!("[状态]\n{}", CANCEL_STATUS_TEXT)
}

/// 成功路径输出（图片已落盘，传入路径列表）。
pub fn send_output(
    selected_options: &[String],
    user_input: Option<&str>,
    image_paths: &[String],
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !selected_options.is_empty() {
        sections.push(format!("[选择的选项]\n{}", selected_options.join(", ")));
    }

    if let Some(input) = user_input {
        let trimmed = input.trim();
        if !trimmed.is_empty() {
            sections.push(format!("[用户输入]\n{}", trimmed));
        }
    }

    if !image_paths.is_empty() {
        sections.push(format!("[图片]\n{}", image_paths.join("\n")));
    }

    if sections.is_empty() {
        sections.push("[用户输入]\n用户确认继续".to_string());
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn options_only() {
        let out = send_output(&s(&["A", "B"]), None, &[]);
        assert_eq!(out, "[选择的选项]\nA, B");
    }

    #[test]
    fn input_trimmed() {
        let out = send_output(&[], Some("  你好  \n"), &[]);
        assert_eq!(out, "[用户输入]\n你好");
    }

    #[test]
    fn empty_input_omitted() {
        let out = send_output(&[], Some("   "), &[]);
        assert_eq!(out, "[用户输入]\n用户确认继续");
    }

    #[test]
    fn all_sections_blank_line_separated() {
        let out = send_output(&s(&["A"]), Some("hi"), &s(&["/tmp/a.png"]));
        assert_eq!(out, "[选择的选项]\nA\n\n[用户输入]\nhi\n\n[图片]\n/tmp/a.png");
    }

    #[test]
    fn empty_all_confirms_continue() {
        let out = send_output(&[], None, &[]);
        assert_eq!(out, "[用户输入]\n用户确认继续");
    }

    #[test]
    fn cancel_text() {
        assert_eq!(
            cancel_output(),
            "[状态]\n用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复"
        );
    }
}
