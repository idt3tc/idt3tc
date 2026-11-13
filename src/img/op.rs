//! Operation descriptions (parsing and help) and the registry of all
//! built-in operations.

use std::sync::OnceLock;

use super::context::Context;
use super::options::OptionReader;
use super::resample::filter_names;
use crate::common::Result;

/// A parsed, validated pipeline step.
pub struct Op {
    apply: Box<dyn Fn(&mut Context) -> Result<()> + Send + Sync>,
    /// Output path template for ops that write files (used to check that
    /// batch runs do not write every input to the same file).
    pub output_template: String,
}

impl Op {
    pub fn new(f: impl Fn(&mut Context) -> Result<()> + Send + Sync + 'static) -> Self {
        Op { apply: Box::new(f), output_template: String::new() }
    }

    pub fn with_output(f: impl Fn(&mut Context) -> Result<()> + Send + Sync + 'static, tmpl: &str) -> Self {
        Op { apply: Box::new(f), output_template: tmpl.to_string() }
    }

    pub fn apply(&self, ctx: &mut Context) -> Result<()> {
        (self.apply)(ctx)
    }
}

#[derive(Debug, Clone)]
pub struct OptionHelp {
    pub key: String,
    /// Default shown in help ("" = none).
    pub def: String,
    pub help: String,
}

pub fn opt(key: &str, def: &str, help: &str) -> OptionHelp {
    OptionHelp { key: key.to_string(), def: def.to_string(), help: help.to_string() }
}

/// Parses the argument and options of one operation occurrence.
pub type OpFactory = fn(&str, &mut OptionReader) -> Result<Op>;

/// Static description of an operation: drives parsing and help output.
pub struct OpInfo {
    /// Without leading dashes.
    pub name: &'static str,
    /// E.g. "o" for "-o".
    pub aliases: &'static [&'static str],
    /// Placeholder of the required argument, "" if none.
    pub arg: &'static str,
    pub category: &'static str,
    pub summary: String,
    pub details: String,
    pub options: Vec<OptionHelp>,
    pub create: OpFactory,
}

pub struct OpRegistry {
    ops: Vec<OpInfo>,
}

impl OpRegistry {
    fn add(&mut self, info: OpInfo) {
        assert!(self.find(info.name).is_none(), "duplicate operation '{}'", info.name);
        self.ops.push(info);
    }

    pub fn find(&self, name: &str) -> Option<&OpInfo> {
        self.ops.iter().find(|o| o.name == name || o.aliases.contains(&name))
    }

    pub fn all(&self) -> &[OpInfo] {
        &self.ops
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.ops.iter().map(|o| o.name).collect()
    }

    /// The registry with every built-in operation, built on first use.
    pub fn instance() -> &'static OpRegistry {
        static REGISTRY: OnceLock<OpRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let mut r = OpRegistry { ops: Vec::new() };
            for list in [super::io_ops::ops(), super::geometry_ops::ops(), super::color_ops::ops()] {
                for info in list {
                    r.add(info);
                }
            }
            r
        })
    }
}

/// Help snippets for the resampling options shared by several operations.
pub fn resample_option_help(def_filter: &str, def_edge: &str, prefix: &str) -> Vec<OptionHelp> {
    let p = |k: &str| format!("{prefix}{k}");
    vec![
        opt(&p("filter"), def_filter, &format!("resampling filter: {}", filter_names().join(", "))),
        opt(
            &p("edge"),
            def_edge,
            "how the filter samples beyond the border: clamp, repeat (tiling textures), mirror, fill",
        ),
        opt(&p("bg"), "transparent", "color used with edge=fill"),
        opt(&p("linear"), "0", "filter in linear light instead of sRGB (gamma-correct)"),
        opt(&p("premultiply"), "1", "weight colors by alpha while filtering (prevents dark fringes around cutouts)"),
        opt(&p("blur"), "1", "filter width multiplier; >1 softer, <1 sharper"),
    ]
}

/// Rounds half up like the C idiom floor(v + 0.5), saturating at the i64
/// range instead of being undefined for huge values.
pub fn round_to_int(v: f64) -> i64 {
    libm::floor(v + 0.5) as i64
}
