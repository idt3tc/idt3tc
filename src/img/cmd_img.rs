//! Command line front end of `idt3tc img`: argument parsing, input
//! expansion, pipeline execution and help output.
//!
//! Inputs are processed in parallel: every input is one task on a rayon
//! pool with `-j` threads, and the operations themselves parallelize on the
//! same pool. Messages of a run are buffered in its Context and printed in
//! input order, so the output looks exactly like a sequential run.

use std::collections::BTreeMap;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use super::context::{Context, Stream, has_input_placeholder};
use super::io::{is_image_extension, load_image};
use super::op::{Op, OpInfo, OpRegistry};
use super::options::OptionReader;
use crate::common::fs;
use crate::common::util::{closest_match, parse_long, to_lower, wrap_text};
use crate::common::{Error, Result};

const HELP_WIDTH: usize = 100;

struct Input {
    path: String,
    /// Relative path below a directory input, without extension.
    rel: String,
}

struct PipelineStep {
    info: &'static OpInfo,
    op: Op,
}

struct Settings {
    /// 0 = all hardware threads.
    jobs: usize,
    recursive: bool,
    keep_going: bool,
    verbosity: i32,
    help: bool,
    help_topic: String,
}

fn is_option_token(t: &str) -> bool {
    t.len() > 1 && t.starts_with('-')
}

/// Global flags; returns true if `args[*i]` (and possibly the next
/// argument) was consumed.
fn parse_global(args: &[String], i: &mut usize, s: &mut Settings) -> Result<bool> {
    let t = args[*i].as_str();
    let (name, value) = match t.split_once('=') {
        Some((n, v)) if t.starts_with("--") => (n, Some(v.to_string())),
        _ => (t, None),
    };

    if name == "-h" || name == "--help" {
        s.help = true;
        if let Some(v) = value {
            s.help_topic = v;
        } else if *i + 1 < args.len() && !is_option_token(&args[*i + 1]) {
            *i += 1;
            s.help_topic = args[*i].clone();
        }
        return Ok(true);
    }
    let short_j = t.starts_with("-j") && t.len() > 2 && !t.starts_with("--");
    if name == "-j" || name == "--jobs" || short_j {
        let value = match value {
            Some(v) => v,
            None if short_j => t[2..].to_string(),
            None if *i + 1 < args.len() => {
                *i += 1;
                args[*i].clone()
            }
            None => return Err(Error::usage(format!("{name}: missing number of jobs"))),
        };
        let n = parse_long(&value).filter(|n| (0..=1024).contains(n)).ok_or_else(|| {
            Error::usage(format!("{name}: expected a number of jobs 0..1024 (0 = all cores), got '{value}'"))
        })?;
        s.jobs = n as usize;
        return Ok(true);
    }
    if value.is_some() {
        return Ok(false);
    }
    match t {
        "-r" | "--recursive" => s.recursive = true,
        "-k" | "--keep-going" => s.keep_going = true,
        "-q" | "--quiet" => s.verbosity = 0,
        "-v" | "--verbose" => s.verbosity = 2,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Looks up an operation token ("--resize", "--resize=64x64", "-o").
/// Returns the operation and the inline argument after '=', if any.
fn find_op(tok: &str) -> Option<(&'static OpInfo, Option<String>)> {
    let name = if let Some(n) = tok.strip_prefix("--") {
        n
    } else if tok.len() == 2 && tok.starts_with('-') {
        &tok[1..]
    } else {
        return None;
    };
    let (name, inline) = match name.split_once('=') {
        Some((n, a)) => (n, Some(a.to_string())),
        None => (name, None),
    };
    OpRegistry::instance().find(name).map(|info| (info, inline))
}

fn unknown_op(tok: &str) -> Error {
    let name = tok.trim_start_matches('-');
    let name = name.split('=').next().unwrap_or("");
    let hint = closest_match(name, &OpRegistry::instance().names())
        .map(|h| format!(" (did you mean '--{h}'?)"))
        .unwrap_or_default();
    Error::usage(format!("unknown operation or option '{tok}'{hint}; see 'idt3tc img --help'"))
}

fn is_known_option_key(info: &OpInfo, key: &str) -> bool {
    info.options.iter().any(|o| o.key == key)
}

/// Expands directory inputs to the image files they contain.
fn expand_inputs(raw: &[String], recursive: bool) -> Result<Vec<Input>> {
    let mut out = Vec::new();
    for p in raw {
        if fs::is_directory(p) {
            let before = out.len();
            for f in fs::list_files(p, recursive)? {
                if !is_image_extension(&fs::extension(&f)) {
                    continue;
                }
                let rel = fs::without_extension(&fs::relative_to(&f, p));
                out.push(Input { path: f, rel });
            }
            if out.len() == before {
                return Err(Error::usage(format!(
                    "no image files found in directory '{p}'{}",
                    if recursive { "" } else { " (use -r to include subdirectories)" }
                )));
            }
        } else if fs::exists(p) {
            out.push(Input { path: p.clone(), rel: fs::stem(p) });
        } else {
            return Err(Error::usage(format!("input '{p}' does not exist")));
        }
    }
    Ok(out)
}

fn op_details(info: &OpInfo) -> String {
    let mut s = format!("--{}{}\n", info.name, if info.arg.is_empty() { String::new() } else { format!(" {}", info.arg) });
    if !info.aliases.is_empty() {
        let a: Vec<String> =
            info.aliases.iter().map(|a| format!("{}{a}", if a.len() == 1 { "-" } else { "--" })).collect();
        s += &format!("  aliases: {}\n", a.join(", "));
    }
    s += &format!("\n  {}\n", wrap_text(&info.summary, HELP_WIDTH, 2));
    if !info.details.is_empty() {
        s += &format!("\n  {}\n", wrap_text(&info.details, HELP_WIDTH, 2));
    }
    if !info.options.is_empty() {
        s += "\n  options (key=value, flags also as bare key):\n";
        let w = info.options.iter().map(|o| o.key.len() + o.def.len() + 3).max().unwrap_or(0).min(30);
        for o in &info.options {
            let left = if o.def.is_empty() { o.key.clone() } else { format!("{}={}", o.key, o.def) };
            let pad = if left.len() < w { w - left.len() } else { 1 };
            let mut line = format!("    {left}{}  ", " ".repeat(pad));
            let mut text = wrap_text(&o.help, HELP_WIDTH, line.len());
            if left.len() >= w + 2 {
                // Long key/default: put the description on its own line.
                line = format!("    {left}\n{}", " ".repeat(w + 6));
                text = wrap_text(&o.help, HELP_WIDTH, w + 6);
            }
            s += &format!("{line}{text}\n");
        }
    }
    s
}

/// Prints the overview, the details of one operation, or with "all" the
/// overview followed by the details of every operation.
pub fn print_img_help(topic: &str) -> Result<()> {
    let reg = OpRegistry::instance();
    let mut s = String::new();
    if !topic.is_empty() && topic != "all" {
        let name = topic.trim_start_matches('-');
        let Some(info) = reg.find(name) else {
            let hint = closest_match(name, &reg.names()).map(|h| format!(" (did you mean '{h}'?)")).unwrap_or_default();
            return Err(Error::usage(format!("unknown operation '{topic}'{hint}")));
        };
        print!("{}", op_details(info));
        return Ok(());
    }

    s += "usage: idt3tc img [OPTIONS] [INPUT...] --OP [ARG] [key=value...] [--OP ...]\n\
          \n\
          Runs a pipeline of image operations, in the given order, once for every input. Inputs may be\n\
          image files or directories (all images inside are processed). Without inputs, the pipeline has to\n\
          create its image with --new or --load.\n\
          \n\
          options:\n  \
          -j, --jobs N       worker threads (0 = all cores, default)\n  \
          -r, --recursive    descend into subdirectories of directory inputs\n  \
          -k, --keep-going   continue with the next input after an error\n  \
          -q, --quiet        only print errors\n  \
          -v, --verbose      print every step\n  \
          -h, --help [OP]    this help, details of one operation, or 'all'\n";

    let mut categories: Vec<&str> = Vec::new();
    for op in reg.all() {
        if !categories.contains(&op.category) {
            categories.push(op.category);
        }
    }
    const COL: usize = 30;
    for cat in categories {
        s += &format!("\n{}:\n", to_lower(cat));
        for op in reg.all().iter().filter(|o| o.category == cat) {
            let left = format!("  --{}{}", op.name, if op.arg.is_empty() { String::new() } else { format!(" {}", op.arg) });
            let summary = wrap_text(&op.summary, HELP_WIDTH, COL);
            if left.len() + 2 > COL {
                s += &format!("{left}\n{}{summary}\n", " ".repeat(COL));
            } else {
                s += &format!("{left}{}{summary}\n", " ".repeat(COL - left.len()));
            }
        }
    }

    s += "\n\
          Every operation takes key=value options; see 'idt3tc img --help OP'.\n\
          \n\
          examples:\n  \
          idt3tc img photo.jpg --pot --save textures/mymap/wall.tga\n  \
          idt3tc img src/ -r --resize 50% filter=lanczos3 --save out/{rel}.tga rle\n  \
          idt3tc img grate.png --pot max=512 --save grate.dds codec=bc1a mip-coverage=0.5\n  \
          idt3tc img height.png --normalmap strength=4 --save wall_n.dds codec=bc5\n  \
          idt3tc img logo.png --rotate 15 --canvas 256x256 --pan 16,0 --save logo.tga\n";

    if topic == "all" {
        for info in reg.all() {
            s += "\n";
            s += &op_details(info);
        }
    }
    print!("{s}");
    Ok(())
}

/// Result of parsing the command line.
enum Parsed {
    /// Exit right away with this code (help was shown).
    Exit(i32),
    Run(Settings, Vec<String>, Vec<PipelineStep>),
}

fn parse_args(args: &[String]) -> Result<Parsed> {
    let mut settings =
        Settings { jobs: 0, recursive: false, keep_going: false, verbosity: 1, help: false, help_topic: String::new() };
    let mut raw_inputs = Vec::new();
    let mut steps: Vec<PipelineStep> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let t = args[i].as_str();
        if t == "--" {
            // Tokens after "--" are inputs even if they start with '-', up
            // to the next operation.
            if !steps.is_empty() {
                return Err(Error::usage("'--' must come before the first operation"));
            }
            i += 1;
            while i < args.len() && find_op(&args[i]).is_none() {
                raw_inputs.push(args[i].clone());
                i += 1;
            }
            continue;
        }
        if is_option_token(t) && parse_global(args, &mut i, &mut settings)? {
            i += 1;
            continue;
        }

        let found = if is_option_token(t) { find_op(t) } else { None };
        let Some((info, inline_arg)) = found else {
            if is_option_token(t) {
                return Err(unknown_op(t));
            }
            if let Some(last) = steps.last() {
                return Err(Error::usage(format!(
                    "unexpected argument '{t}' after --{} (inputs must come before the first operation)",
                    last.info.name
                )));
            }
            raw_inputs.push(t.to_string());
            i += 1;
            continue;
        };

        let ctx_name = format!("--{}", info.name);
        let mut arg = String::new();
        if !info.arg.is_empty() {
            if let Some(a) = inline_arg {
                arg = a;
            } else if i + 1 < args.len() {
                // May start with '-' (e.g. --rotate -90).
                i += 1;
                arg = args[i].clone();
            } else {
                return Err(Error::usage(format!("{ctx_name}: missing argument {}", info.arg)));
            }
        } else if inline_arg.is_some() {
            return Err(Error::usage(format!("{ctx_name} takes no argument")));
        }

        // Options: key=value tokens or bare flag keys known to this op.
        let mut kv = Vec::new();
        while i + 1 < args.len() {
            let n = args[i + 1].as_str();
            if is_option_token(n) {
                break;
            }
            match n.find('=') {
                Some(eq) if eq > 0 => kv.push((n[..eq].to_string(), n[eq + 1..].to_string())),
                _ if is_known_option_key(info, n) => kv.push((n.to_string(), String::new())),
                _ => {
                    let keys: Vec<&str> = info.options.iter().map(|o| o.key.as_str()).collect();
                    let hint = closest_match(n, &keys).map(|h| format!(" (did you mean '{h}'?)")).unwrap_or_default();
                    return Err(Error::usage(format!(
                        "unexpected argument '{n}' after {ctx_name}{hint} (inputs must come before the first \
                         operation)"
                    )));
                }
            }
            i += 1;
        }

        let mut reader = OptionReader::new(&ctx_name, &kv)?;
        let op = (info.create)(&arg, &mut reader)?;
        reader.finish()?;
        steps.push(PipelineStep { info, op });
        i += 1;
    }

    if settings.help {
        print_img_help(&settings.help_topic)?;
        return Ok(Parsed::Exit(0));
    }
    if steps.is_empty() && raw_inputs.is_empty() {
        print_img_help("")?;
        return Ok(Parsed::Exit(2));
    }
    if steps.is_empty() {
        return Err(Error::usage("no operations given; e.g. add '--save {name}.tga' (see 'idt3tc img --help')"));
    }
    Ok(Parsed::Run(settings, raw_inputs, steps))
}

/// Outcome of one pipeline run, sent from the worker to the printer.
struct RunResult {
    index: usize,
    output: Vec<(Stream, String)>,
    /// None if the run was skipped because an earlier input failed.
    error: Option<Option<Error>>,
}

/// Runs the pipeline for input `r` (or once without input).
fn run_one(r: usize, inputs: &[Input], steps: &[PipelineStep], settings: &Settings, runs: usize) -> RunResult {
    let mut ctx = Context::new(settings.verbosity);
    ctx.index = r;
    ctx.count = runs;
    let res = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        if let Some(input) = inputs.get(r) {
            ctx.input_path = input.path.clone();
            ctx.input_rel = input.rel.clone();
            let (img, li) = load_image(&input.path)?;
            ctx.set_image(img);
            ctx.verbose(format!(
                "loaded {} ({}x{}, {}, {} channel{}, {} bit)",
                ctx.input_path,
                ctx.image.width,
                ctx.image.height,
                li.format,
                li.channels,
                if li.channels == 1 { "" } else { "s" },
                li.bits_per_channel
            ));
        }
        for s in steps {
            ctx.verbose(format!("  --{}", s.info.name));
            s.op.apply(&mut ctx)?;
        }
        Ok(())
    }));
    // A panic is a bug, but it must not take down the other runs; report
    // it like any other failure of this input.
    let error = match res {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(p) => {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Some(Error::runtime(format!("internal error: {what}")))
        }
    };
    RunResult { index: r, output: ctx.output, error: Some(error) }
}

fn print_output(output: &[(Stream, String)]) {
    for (stream, msg) in output {
        match stream {
            Stream::Stdout => {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{msg}");
                let _ = out.flush();
            }
            Stream::Stderr => eprintln!("{msg}"),
        }
    }
}

/// Runs `idt3tc img ARGS...` and returns the exit code. Errors that are
/// not usage errors before the first run is started are returned to the
/// caller.
pub fn run_img_command(args: &[String]) -> Result<i32> {
    let usage_exit = |e: Error| -> Result<i32> {
        if e.is_usage() {
            eprintln!("idt3tc img: {}", e.message());
            Ok(2)
        } else {
            Err(e)
        }
    };
    let (settings, raw_inputs, steps) = match parse_args(args) {
        Ok(Parsed::Exit(code)) => return Ok(code),
        Ok(Parsed::Run(s, i, st)) => (s, i, st),
        Err(e) => return usage_exit(e),
    };

    let inputs = match expand_inputs(&raw_inputs, settings.recursive) {
        Ok(i) => i,
        Err(e) => return usage_exit(e),
    };
    // Writing several inputs to one fixed path silently overwrites all but
    // the last result.
    if inputs.len() > 1 {
        for s in &steps {
            let tmpl = &s.op.output_template;
            if !tmpl.is_empty() && !has_input_placeholder(tmpl) {
                return usage_exit(Error::usage(format!(
                    "--{} '{tmpl}' is the same for all {} inputs; use a placeholder such as {{name}} or {{rel}}",
                    s.info.name,
                    inputs.len()
                )));
            }
        }
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(settings.jobs)
        .build()
        .map_err(|e| Error::runtime(format!("cannot start worker threads ({e})")))?;
    let runs = inputs.len().max(1);
    // Index of the first failed input when stopping at errors; later
    // inputs that have not started yet are skipped.
    let first_failure = AtomicUsize::new(usize::MAX);
    let (tx, rx) = mpsc::channel::<RunResult>();

    let mut failed = 0usize;
    let mut code = 0;
    std::thread::scope(|scope| {
        let (inputs, steps, settings, first_failure) = (&inputs, &steps, &settings, &first_failure);
        let pool = &pool;
        scope.spawn(move || {
            // FIFO spawning starts the inputs in order, which keeps the
            // number of buffered results small.
            pool.scope_fifo(|s| {
                for r in 0..runs {
                    let tx = tx.clone();
                    s.spawn_fifo(move |_| {
                        let result = if r > first_failure.load(Ordering::SeqCst) {
                            RunResult { index: r, output: Vec::new(), error: None }
                        } else {
                            let res = run_one(r, inputs, steps, settings, runs);
                            if matches!(res.error, Some(Some(_))) && !settings.keep_going {
                                first_failure.fetch_min(r, Ordering::SeqCst);
                            }
                            res
                        };
                        // The receiver only goes away after a stop.
                        let _ = tx.send(result);
                    });
                }
            });
        });

        // Print the results in input order as they become available.
        let mut pending = BTreeMap::new();
        let mut next = 0usize;
        for res in rx.iter() {
            pending.insert(res.index, res);
            while let Some(res) = pending.remove(&next) {
                next += 1;
                print_output(&res.output);
                let Some(Some(e)) = res.error else {
                    continue;
                };
                failed += 1;
                let label = inputs.get(res.index).map_or("pipeline", |i| i.path.as_str());
                eprintln!("idt3tc img: {label}: {}", e.message());
                if !settings.keep_going {
                    if res.index + 1 < runs {
                        eprintln!("idt3tc img: stopping (use -k to continue with the remaining inputs)");
                    }
                    code = 1;
                    return;
                }
            }
        }
    });
    if code != 0 {
        return Ok(code);
    }
    if failed > 0 {
        eprintln!("idt3tc img: {failed} of {runs} input{} failed", if runs == 1 { "" } else { "s" });
        return Ok(1);
    }
    Ok(0)
}
