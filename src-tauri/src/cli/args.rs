//! 提问参数解析（纯逻辑，可单测）。
//!
//! 模型：第一个位置参数是共享 Message；`-q` 声明实际问题。
//! 完全没有 `-q` 时，第一个参数等价于 `-q`（`AskHuman "X"` ≡ `AskHuman -q "X"`），
//! 被提升为唯一问题，其前置 `-o` 归该问题；`-f` 始终归 Message（位置不限）。

/// 单个问题的原始参数（路径未解析/校验）。
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionArgs {
    pub message: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AskArgs {
    /// 共享 Message 文本（无 `-q` 时为空——其内容已提升进 `questions`）。
    pub message_text: String,
    /// `-f`/`--file` 给出的原始路径（按出现顺序，归 Message，未做解析/校验）。
    pub message_files: Vec<String>,
    /// 问题列表（解析+归一化后恒 ≥1）。
    pub questions: Vec<QuestionArgs>,
    /// 是否按 Markdown 渲染（全局，对所有问题生效）。
    pub is_markdown: bool,
}

/// 解析 `AskHuman <Message> [-f <path>] [-q <text> [-o <opt>] ...] [--no-markdown]`。
///
/// 规则：
/// - 第一个位置参数 = Message 文本；无任何 `-q` 时它被提升为唯一问题。
/// - `-o` 归「最近声明的问题」；存在 `-q` 时不能出现在第一个 `-q` 之前。
/// - `-f` 始终归 Message（位置不限）。
/// - `--no-markdown` 全局。
///
/// 失败时返回中文错误描述。
pub fn parse_ask(args: &[String]) -> Result<AskArgs, String> {
    let mut message_text = String::new();
    let mut message_files: Vec<String> = Vec::new();
    let mut questions: Vec<QuestionArgs> = Vec::new();
    // 无 `-q` 时，位于第一个参数之后的 `-o` 暂存于此，归一化时挂到被提升的问题。
    let mut lead_options: Vec<String> = Vec::new();
    let mut is_markdown = true;

    let mut seen_positional = false;
    let mut seen_question_flag = false;
    // 预扫描：是否存在任一 `-q`。决定 `-o` 在题前是报错还是归 Message（被提升的问题）。
    let has_q = args.iter().any(|a| a == "-q" || a == "--question");

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-q" | "--question" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} 选项缺少参数值", arg));
                }
                questions.push(QuestionArgs {
                    message: args[i + 1].clone(),
                    options: Vec::new(),
                });
                seen_question_flag = true;
                i += 2;
            }
            "-o" | "--option" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} 选项缺少参数值", arg));
                }
                let value = args[i + 1].clone();
                match questions.last_mut() {
                    Some(q) => q.options.push(value),
                    None => {
                        if has_q {
                            // 存在 -q 时，-o 不能出现在第一个 -q 之前。
                            return Err(format!("{} 不能出现在第一个问题(-q)之前", arg));
                        }
                        // 无 `-q`：暂存，归一化时归属被提升的问题。
                        lead_options.push(value);
                    }
                }
                i += 2;
            }
            "-f" | "--file" => {
                if i + 1 >= args.len() {
                    return Err(format!("{} 选项缺少参数值", arg));
                }
                message_files.push(args[i + 1].clone());
                i += 2;
            }
            "--no-markdown" => {
                is_markdown = false;
                i += 1;
            }
            a if a.starts_with('-') => {
                return Err(format!("未知选项: {}", a));
            }
            _ => {
                // 位置参数：仅允许作为第一个 token（Message），且需在任何 -q 之前。
                if seen_positional || seen_question_flag {
                    return Err("位置参数只能作为 Message，且需在最前".to_string());
                }
                message_text = arg.clone();
                seen_positional = true;
                i += 1;
            }
        }
    }

    // 有效性校验（归一化前）：至少有 Message 文本 / 一个 -q / 一个 -f；仅 -o 不算有效。
    if message_text.trim().is_empty() && questions.is_empty() && message_files.is_empty() {
        return Err("缺少提问内容".to_string());
    }

    // 归一化：无 `-q` 时，第一个参数（含其前置 -o）提升为唯一问题，Message 文本清空。
    if !seen_question_flag {
        questions.push(QuestionArgs {
            message: std::mem::take(&mut message_text),
            options: lead_options,
        });
    }

    Ok(AskArgs {
        message_text,
        message_files,
        questions,
        is_markdown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positional_promoted_equals_q() {
        let a = parse_ask(&v(&["X"])).unwrap();
        let b = parse_ask(&v(&["-q", "X"])).unwrap();
        assert_eq!(a.message_text, "");
        assert_eq!(a.questions, vec![QuestionArgs { message: "X".into(), options: vec![] }]);
        assert_eq!(a.questions, b.questions);
        assert_eq!(b.message_text, "");
        assert!(a.is_markdown);
    }

    #[test]
    fn single_with_options_promoted() {
        let p = parse_ask(&v(&["X", "-o", "A", "--option", "B"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.questions, vec![QuestionArgs { message: "X".into(), options: v(&["A", "B"]) }]);
    }

    #[test]
    fn single_with_files_promoted() {
        let p = parse_ask(&v(&["X", "-f", "f.png"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.message_files, v(&["f.png"]));
        assert_eq!(p.questions, vec![QuestionArgs { message: "X".into(), options: vec![] }]);
    }

    #[test]
    fn multi_message_and_questions() {
        let p = parse_ask(&v(&[
            "M", "-q", "Q1", "-o", "A", "-q", "Q2", "-o", "B",
        ]))
        .unwrap();
        assert_eq!(p.message_text, "M");
        assert_eq!(p.questions.len(), 2);
        assert_eq!(p.questions[0], QuestionArgs { message: "Q1".into(), options: v(&["A"]) });
        assert_eq!(p.questions[1], QuestionArgs { message: "Q2".into(), options: v(&["B"]) });
    }

    #[test]
    fn file_after_q_belongs_to_message() {
        let p = parse_ask(&v(&["M", "-q", "Q1", "-f", "x.png"])).unwrap();
        assert_eq!(p.message_text, "M");
        assert_eq!(p.message_files, v(&["x.png"]));
        assert_eq!(p.questions[0].options.len(), 0);
    }

    #[test]
    fn optional_message_questions_only() {
        let p = parse_ask(&v(&["-q", "Q1", "-q", "Q2"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.questions.len(), 2);
    }

    #[test]
    fn optional_message_file_only() {
        let p = parse_ask(&v(&["-f", "x.png", "-q", "Q1"])).unwrap();
        assert_eq!(p.message_text, "");
        assert_eq!(p.message_files, v(&["x.png"]));
        assert_eq!(p.questions.len(), 1);
    }

    #[test]
    fn no_markdown_is_global() {
        let p = parse_ask(&v(&["M", "-q", "Q1", "--no-markdown"])).unwrap();
        assert!(!p.is_markdown);
        assert_eq!(p.questions.len(), 1);
    }

    #[test]
    fn option_before_any_question_errors() {
        assert!(parse_ask(&v(&["M", "-o", "A", "-q", "Q1"])).is_err());
    }

    #[test]
    fn option_only_is_invalid() {
        assert!(parse_ask(&v(&["-o", "A"])).is_err());
    }

    #[test]
    fn second_positional_errors() {
        assert!(parse_ask(&v(&["M", "extra"])).is_err());
        assert!(parse_ask(&v(&["-q", "Q1", "extra"])).is_err());
    }

    #[test]
    fn question_requires_value() {
        assert!(parse_ask(&v(&["-q"])).is_err());
        assert!(parse_ask(&v(&["M", "-q"])).is_err());
    }

    #[test]
    fn requires_some_content() {
        assert!(parse_ask(&v(&["--no-markdown"])).is_err());
        assert!(parse_ask(&v(&[])).is_err());
    }

    #[test]
    fn requires_option_value() {
        assert!(parse_ask(&v(&["msg", "-o"])).is_err());
    }

    #[test]
    fn requires_file_value() {
        assert!(parse_ask(&v(&["msg", "-f"])).is_err());
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(parse_ask(&v(&["msg", "--foo"])).is_err());
    }
}
