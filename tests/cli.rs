//! End to end tests of the idt3tc executable: exit codes, messages and the
//! files it writes. Usage errors must exit with 2, runtime errors with 1,
//! and no input (including special files met during `-r` scans) may make
//! the process hang, so every run is bounded by a timeout.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_idt3tc");

/// Generous enough for unoptimized CI runners; a hanging run exceeds it by
/// far.
const TIMEOUT: Duration = Duration::from_secs(120);

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Output {
    /// Asserts the exit code and that `text` appears in stdout or stderr.
    #[track_caller]
    fn expect(&self, code: i32, text: &str) -> &Self {
        assert_eq!(self.code, code, "unexpected exit code\n{self}");
        assert!(
            self.stdout.contains(text) || self.stderr.contains(text),
            "{text:?} not found in the output\n{self}"
        );
        self
    }

    #[track_caller]
    fn expect_stderr(&self, text: &str) -> &Self {
        assert!(self.stderr.contains(text), "{text:?} not found on stderr\n{self}");
        self
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}\n--- stdout\n{}--- stderr\n{}", self.code, self.stdout, self.stderr)
    }
}

/// A fresh, empty working directory for one test. Tests run in parallel, so
/// each one gets its own.
fn workdir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli").join(test);
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs idt3tc in `dir` and kills it when it exceeds TIMEOUT. The output
/// goes to files next to `dir` rather than to pipes, so a chatty child can
/// never block on a full pipe while it is being waited for, and the logs
/// stay out of the trees the tests scan.
fn run_in(dir: &Path, args: &[&str]) -> Output {
    let stdout_path = dir.with_extension("stdout");
    let stderr_path = dir.with_extension("stderr");
    let mut child = Command::new(EXE)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(File::create(&stdout_path).unwrap())
        .stderr(File::create(&stderr_path).unwrap())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("idt3tc {args:?} did not finish within {TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        // A signal (no exit code) is a crash and must never match.
        code: status.code().unwrap_or(-1),
        stdout: fs::read_to_string(&stdout_path).unwrap(),
        stderr: fs::read_to_string(&stderr_path).unwrap(),
    }
}

/// Runs idt3tc in an empty directory of its own; tests run in parallel and
/// must not share one.
fn run(args: &[&str]) -> Output {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    run_in(&workdir(&format!("run-{}", RUNS.fetch_add(1, Ordering::Relaxed))), args)
}

/// A working directory with src.png (351x291 after the rotation), and the
/// tree set/a.png, set/sub/b.png for the batch tests.
fn with_sources(test: &str) -> PathBuf {
    let dir = workdir(test);
    run_in(
        &dir,
        &[
            "img", "--new", "300x200", "color=#4080c0", "--rotate", "20", "bg=transparent", "--save", "src.png",
            "--flip", "h", "--save", "set/a.png", "--tile", "2", "--save", "set/sub/b.png",
        ],
    )
    .expect(0, "wrote set/sub/b.png");
    dir
}

// --- general ------------------------------------------------------------

#[test]
fn version() {
    let out = run(&["--version"]);
    out.expect(0, "idt3tc ");
    assert_eq!(out.stdout.trim(), format!("idt3tc {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn usage_without_arguments() {
    run(&[]).expect(2, "usage: idt3tc");
}

#[test]
fn unknown_command() {
    run(&["imgg"]).expect(2, "unknown command 'imgg' (did you mean 'img'?)");
}

#[test]
fn help() {
    run(&["img", "--help"]).expect(0, "usage: idt3tc img");
    run(&["img", "--help", "save"]).expect(0, "codec=auto");
    run(&["img", "--help", "all"]).expect(0, "--normalmap");
    run(&["help", "img"]).expect(0, "usage: idt3tc img");
}

// --- successful runs ------------------------------------------------------

#[test]
fn generate() {
    let dir = with_sources("generate");
    for file in ["src.png", "set/a.png", "set/sub/b.png"] {
        assert!(dir.join(file).is_file(), "{file} missing");
    }
}

#[test]
fn save_tga() {
    let dir = with_sources("save_tga");
    run_in(&dir, &["img", "src.png", "--pot", "--save", "out/{name}.tga", "rle"])
        .expect(0, "wrote out/src.tga (256x256, tga 32-bit rle");
    assert!(dir.join("out/src.tga").is_file());
}

#[test]
fn save_dds_bc1a() {
    let dir = with_sources("save_dds_bc1a");
    run_in(
        &dir,
        &["img", "src.png", "--pot", "max=128", "--save", "out/{name}_{w}.dds", "codec=bc1a", "mip-coverage=0.5"],
    )
    .expect(0, "wrote out/src_128.dds (128x128, dds bc1a/dxt1a, 8 mips");
}

#[test]
fn save_dds_bc7() {
    let dir = with_sources("save_dds_bc7");
    run_in(&dir, &["img", "src.png", "--resize", "64x64", "--save", "out/bc7.dds", "codec=bc7", "quality=0.05", "srgb"])
        .expect(0, "bc7, 7 mips, dx10");
}

#[test]
fn batch() {
    let dir = with_sources("batch");
    run_in(&dir, &["img", "set", "-r", "--resize", "32x32", "--save", "batch/{rel}.tga"])
        .expect(0, "wrote batch/sub/b.tga")
        .expect(0, "wrote batch/a.tga");
}

#[test]
fn info() {
    let dir = with_sources("info");
    run_in(&dir, &["img", "src.png", "--info"])
        .expect(0, "src.png: 351x291 (not power of two), alpha: gradient");
}

#[test]
fn pipeline() {
    let out = run(&[
        "img", "--new", "64x32", "--fill", "ff0000", "--pan", "8,0", "--crop", "32x32+16+0", "--levels", "0,255,1.2",
        "--hue", "120", "--blur", "1", "edge=repeat", "--sharpen", "0.5", "--normalmap", "strength=3", "--info",
    ]);
    assert_eq!(out.code, 0, "{out}");
    assert!(out.stdout.contains("64x32") || out.stdout.contains("32x32"), "{out}");
}

// --- rejected invocations -------------------------------------------------

#[test]
fn unknown_op() {
    run(&["img", "src.png", "--resiz", "10x10"]).expect(2, "did you mean '--resize'");
}

#[test]
fn unknown_op_dashes() {
    run(&["img", "---"]).expect(2, "unknown operation or option '---'");
}

#[test]
fn unknown_key() {
    run(&["img", "--new", "8x8", "--crop", "4x4", "gravty=c"]).expect(2, "did you mean 'gravity'");
}

#[test]
fn foreign_key() {
    run(&["img", "--new", "8x8", "--save", "x.tga", "codec=bc1"]).expect(2, "option 'codec' is not applicable");
}

#[test]
fn no_format() {
    run(&["img", "--new", "8x8", "--save", "x.xyz"]).expect(2, "cannot derive the format");
}

#[test]
fn fixed_output() {
    let dir = with_sources("fixed_output");
    run_in(&dir, &["img", "set", "-r", "--save", "one.tga"]).expect(2, "same for all");
    assert!(!dir.join("one.tga").exists());
}

#[test]
fn missing_input() {
    run(&["img", "nope.png", "--info"]).expect(2, "does not exist");
}

#[test]
fn no_image() {
    run(&["img", "--resize", "10x10"]).expect(1, "no image loaded");
}

#[test]
fn inputs_after_ops() {
    run(&["img", "--new", "8x8", "--info", "extra.png"]).expect(2, "inputs must come before");
}

#[test]
fn bad_geometry() {
    run(&["img", "--new", "8x8", "--resize", "10y10"]).expect(2, "--resize");
}

#[test]
fn oversized_new() {
    run(&["img", "--new", "70000x1", "--info"]).expect(1, "invalid image size 70000x1");
}

#[test]
fn oversized_resize() {
    run(&["img", "--new", "8x8", "--resize", "65536x65536", "--info"]).expect(1, "exceeds the limit");
}

// --- hostile trees (Unix only: FIFOs and symlinks) ------------------------

/// A scan tree with one valid image, one garbage file, and entries that a
/// naive scan would hang or loop on: a FIFO (opening it blocks until a
/// writer appears), a symlink loop between two files, a directory symlink
/// pointing back at the tree and a dangling symlink.
#[cfg(unix)]
fn hostile_tree(test: &str) -> PathBuf {
    use std::os::unix::fs::symlink;

    let dir = workdir(test);
    run_in(&dir, &["img", "--new", "8x8", "--save", "tree/ok.png"]).expect(0, "wrote tree/ok.png");
    fs::write(dir.join("tree/bad.png"), b"garbage, not a PNG").unwrap();

    let fifo = dir.join("tree/fifo.png");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success(), "mkfifo failed");

    symlink("loop-b.png", dir.join("tree/loop-a.png")).unwrap();
    symlink("loop-a.png", dir.join("tree/loop-b.png")).unwrap();
    symlink(".", dir.join("tree/self")).unwrap();
    symlink("..", dir.join("tree/parent")).unwrap();
    symlink("missing.png", dir.join("tree/dangling.png")).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn scan_hostile_tree_stops_on_error() {
    let dir = hostile_tree("scan_hostile_stop");
    run_in(&dir, &["img", "tree", "-r", "--info"])
        .expect(1, "tree/bad.png: 'tree/bad.png': unsupported or corrupt image")
        .expect_stderr("stopping (use -k");
}

#[cfg(unix)]
#[test]
fn scan_hostile_tree_keep_going() {
    let dir = hostile_tree("scan_hostile_keep_going");
    // Only the two regular files are inputs: the FIFO, the links and the
    // loops are skipped instead of counted as failures. "parent" leads to
    // the working directory, whose only entry is the already visited tree.
    let out = run_in(&dir, &["img", "tree", "-r", "-k", "--save", "out/{rel}.tga"]);
    out.expect(1, "wrote out/ok.tga").expect_stderr("1 of 2 inputs failed");
    assert!(dir.join("out/ok.tga").is_file());
    assert!(!dir.join("out/bad.tga").exists());
}

#[cfg(unix)]
#[test]
fn fifo_as_direct_input() {
    let dir = hostile_tree("fifo_direct");
    run_in(&dir, &["img", "tree/fifo.png", "--info"]).expect(1, "'tree/fifo.png': not a regular file");
}

/// A directory symlink that leads out of the scanned tree is followed, but
/// `{rel}` stays relative to the scanned directory, so the output cannot
/// escape the output root.
#[cfg(unix)]
#[test]
fn scan_symlink_out_of_tree_stays_in_output_root() {
    use std::os::unix::fs::symlink;

    let dir = workdir("scan_symlink_escape");
    run_in(&dir, &["img", "--new", "8x8", "--save", "outside/x.png"]).expect(0, "wrote outside/x.png");
    fs::create_dir(dir.join("tree")).unwrap();
    symlink("../outside", dir.join("tree/ext")).unwrap();

    run_in(&dir, &["img", "tree", "-r", "--save", "out/{rel}.tga"]).expect(0, "wrote out/ext/x.tga");
    assert!(dir.join("out/ext/x.tga").is_file());
    assert!(!dir.join("outside/x.tga").exists());
}
