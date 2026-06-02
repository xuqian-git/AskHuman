pub mod args;
pub mod help;
pub mod image_writer;
pub mod output;

use std::process::exit;

/// 入口分发：在创建任何窗口前按 argv 分流。
pub fn dispatch() {
    let argv: Vec<String> = std::env::args().collect();

    if argv.len() < 2 {
        eprintln!("错误: 缺少提问内容\n");
        println!("{}", help::help_text());
        exit(1);
    }

    match argv[1].as_str() {
        "--help" | "-h" => {
            println!("{}", help::help_text());
            exit(0);
        }
        "--version" | "-v" => {
            println!("{}", help::version_text());
            exit(0);
        }
        "--settings" => {
            crate::app::run_window("settings");
            exit(0);
        }
        first if first.starts_with('-') => {
            eprintln!("错误: 未知选项 {}\n", first);
            println!("{}", help::help_text());
            exit(1);
        }
        _ => match args::parse_ask(&argv[1..]) {
            Ok(_parsed) => {
                // Step 1：先打通窗口，结果流程在后续步骤实现。
                crate::app::run_window("popup");
                exit(0);
            }
            Err(e) => {
                eprintln!("错误: {}\n", e);
                println!("{}", help::help_text());
                exit(1);
            }
        },
    }
}
