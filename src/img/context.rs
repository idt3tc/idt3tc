//! State threaded through one pipeline run (one run per input file).

use super::image::Image;
use crate::common::fs;
use crate::common::{Error, Result};

/// Where a buffered message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Default)]
pub struct Context {
    pub image: Image,
    pub has_image: bool,
    /// The input this run was started for; empty when the pipeline builds
    /// its image itself (--new / --load).
    pub input_path: String,
    /// Input path relative to the directory it was found in when a
    /// directory was given as input (without extension), else the input
    /// stem.
    pub input_rel: String,
    /// 0-based input index.
    pub index: usize,
    /// Number of inputs.
    pub count: usize,
    /// 0 = quiet, 1 = normal, 2 = verbose.
    pub verbosity: i32,
    /// Messages of this run. Runs for different inputs execute in parallel;
    /// their output is collected here and printed in input order.
    pub output: Vec<(Stream, String)>,
}

impl Context {
    pub fn new(verbosity: i32) -> Self {
        Context { count: 1, verbosity, ..Default::default() }
    }

    /// Fails with a usage error if no image has been loaded yet.
    pub fn require_image(&mut self, op: &str) -> Result<&mut Image> {
        if !self.has_image {
            return Err(Error::usage(format!(
                "{op}: no image loaded (give an input file, --load FILE or --new WxH first)"
            )));
        }
        Ok(&mut self.image)
    }

    /// Replaces the current image (after a successful operation).
    pub fn set_image(&mut self, img: Image) {
        self.image = img;
        self.has_image = true;
    }

    /// Expands {name}, {stem}, {ext}, {dir}, {rel}, {index}, {w}, {h} and
    /// {{ / }} in an output path template.
    pub fn expand_path(&self, tmpl: &str) -> Result<String> {
        let mut out = String::new();
        let mut rest = tmpl;
        while let Some(c) = rest.chars().next() {
            if rest.starts_with("{{") {
                out.push('{');
                rest = &rest[2..];
                continue;
            }
            if rest.starts_with("}}") {
                out.push('}');
                rest = &rest[2..];
                continue;
            }
            if c != '{' {
                out.push(c);
                rest = &rest[c.len_utf8()..];
                continue;
            }
            let Some(close) = rest.find('}') else {
                return Err(Error::usage(format!("unterminated '{{' in output path '{tmpl}'")));
            };
            let key = &rest[1..close];
            match key {
                "name" | "stem" => {
                    out += &if self.input_path.is_empty() { "image".to_string() } else { fs::stem(&self.input_path) }
                }
                "ext" => out += &fs::extension(&self.input_path),
                "dir" => {
                    let d = fs::dirname(&self.input_path);
                    out += if d.is_empty() { "." } else { &d };
                }
                "rel" => out += if self.input_rel.is_empty() { "image" } else { &self.input_rel },
                "index" => out += &self.index.to_string(),
                "w" => out += &self.image.width.to_string(),
                "h" => out += &self.image.height.to_string(),
                _ => {
                    return Err(Error::usage(format!(
                        "unknown placeholder '{{{key}}}' in output path (known: name, ext, dir, rel, index, w, h)"
                    )));
                }
            }
            rest = &rest[close + 1..];
        }
        Ok(out)
    }

    /// Message shown unless quiet (stderr).
    pub fn info(&mut self, msg: impl Into<String>) {
        if self.verbosity >= 1 {
            self.output.push((Stream::Stderr, msg.into()));
        }
    }

    /// Message shown with -v (stderr).
    pub fn verbose(&mut self, msg: impl Into<String>) {
        if self.verbosity >= 2 {
            self.output.push((Stream::Stderr, msg.into()));
        }
    }

    /// Regular command output such as --info (stdout, always shown).
    pub fn print(&mut self, msg: impl Into<String>) {
        self.output.push((Stream::Stdout, msg.into()));
    }

    /// Warning (stderr, always shown).
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.output.push((Stream::Stderr, msg.into()));
    }
}

/// True if `tmpl` contains any placeholder that differs between inputs.
pub fn has_input_placeholder(tmpl: &str) -> bool {
    ["{name}", "{stem}", "{ext}", "{dir}", "{rel}", "{index}"].iter().any(|k| tmpl.contains(k))
}
