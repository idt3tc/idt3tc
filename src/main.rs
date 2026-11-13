//! idt3tc: tools for provisioning idTech 3 maps, mods and texture packs.

use std::process::ExitCode;

use idt3tc::common::Result;
use idt3tc::common::util::closest_match;
use idt3tc::img::cmd_img::{print_img_help, run_img_command};

struct Command {
    name: &'static str,
    summary: &'static str,
    run: fn(&[String]) -> Result<i32>,
    help: fn(&str) -> Result<()>,
}

const COMMANDS: &[Command] = &[Command {
    name: "img",
    summary: "convert and process images (TGA, DDS/BCn, PNG, JPG) for idTech 3",
    run: run_img_command,
    help: print_img_help,
}];

fn print_usage() {
    let mut s = String::from("usage: idt3tc [--version] [--help] COMMAND [ARGS...]\n\ncommands:\n");
    for c in COMMANDS {
        s += &format!("  {:<8} {}\n", c.name, c.summary);
    }
    s += "\nRun 'idt3tc COMMAND --help' or 'idt3tc help COMMAND' for details.\n";
    print!("{s}");
}

fn find_command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

fn unknown_command(name: &str) -> i32 {
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let hint = closest_match(name, &names).map(|h| format!(" (did you mean '{h}'?)")).unwrap_or_default();
    eprintln!("idt3tc: unknown command '{name}'{hint}");
    2
}

fn run(args: &[String]) -> Result<i32> {
    let Some(first) = args.first() else {
        print_usage();
        return Ok(2);
    };
    match first.as_str() {
        "-h" | "--help" => {
            print_usage();
            Ok(0)
        }
        "--version" | "-V" => {
            println!("idt3tc {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        "help" => {
            let Some(name) = args.get(1) else {
                print_usage();
                return Ok(0);
            };
            let Some(c) = find_command(name) else {
                return Ok(unknown_command(name));
            };
            (c.help)(args.get(2).map_or("", String::as_str))?;
            Ok(0)
        }
        name => match find_command(name) {
            Some(c) => (c.run)(&args[1..]),
            None => Ok(unknown_command(name)),
        },
    }
}

fn main() -> ExitCode {
    // Arguments that are not valid Unicode are passed on lossily; they can
    // only name files, which then fail to open with a clear message.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let code = match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("idt3tc: {}", e.message());
            if e.is_usage() { 2 } else { 1 }
        }
    };
    ExitCode::from(code as u8)
}
