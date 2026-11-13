//! key=value modifiers given after an operation, e.g.
//! `--resize 512x512 filter=lanczos3 linear`.
//!
//! A bare key (no '=') is shorthand for key=1 and only valid for flags.
//! Every accessor marks its key as used; finish() rejects keys that the
//! operation never asked for, so typos and options that do not apply in the
//! chosen mode (e.g. DDS options on a TGA save) are reported instead of
//! being silently ignored.

use std::collections::{BTreeSet, HashMap};

use super::image::Color;
use super::resample::{Filter, ResampleOptions, filter_names, parse_filter};
use super::sampling::{EdgeMode, Interp};
use super::spec::{Gravity, parse_channels, parse_color, parse_edge_mode, parse_gravity, parse_interp};
use crate::common::util::{closest_match, format_number, parse_double, parse_long};
use crate::common::{Error, Result};

pub struct OptionReader {
    context: String,
    values: HashMap<String, String>,
    /// Sorted, so suggestions do not depend on the order keys were read.
    used: BTreeSet<String>,
    order: Vec<String>,
}

impl OptionReader {
    pub fn new(context: &str, raw: &[(String, String)]) -> Result<Self> {
        let mut values = HashMap::new();
        let mut order = Vec::new();
        for (key, value) in raw {
            if values.contains_key(key) {
                return Err(Error::usage(format!("{context}: option '{key}' given more than once")));
            }
            values.insert(key.clone(), value.clone());
            order.push(key.clone());
        }
        Ok(OptionReader { context: context.to_string(), values, used: BTreeSet::new(), order })
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Marks `key` as used and returns its raw value if given.
    fn lookup(&mut self, key: &str) -> Option<String> {
        self.used.insert(key.to_string());
        self.values.get(key).cloned()
    }

    /// The value of a non-flag option; an empty value (bare key) is an
    /// error. None when the key was not given.
    fn value_of(&mut self, key: &str) -> Result<Option<String>> {
        match self.lookup(key) {
            Some(v) if v.is_empty() => {
                Err(Error::usage(format!("{}: option '{key}' needs a value ({key}=...)", self.context)))
            }
            other => Ok(other),
        }
    }

    fn invalid(&self, key: &str, value: &str, expected: &str) -> Error {
        Error::usage(format!("{}: invalid value '{value}' for option '{key}' (expected {expected})", self.context))
    }

    /// Reads `key` with `parse`, returning `def` when it is absent and a
    /// descriptive usage error when the parser rejects the value.
    fn parsed<T>(&mut self, key: &str, def: T, expected: &str, parse: impl Fn(&str) -> Option<T>) -> Result<T> {
        match self.value_of(key)? {
            None => Ok(def),
            Some(s) => parse(&s).ok_or_else(|| self.invalid(key, &s, expected)),
        }
    }

    pub fn str(&mut self, key: &str, def: &str) -> Result<String> {
        Ok(self.value_of(key)?.unwrap_or_else(|| def.to_string()))
    }

    pub fn number(&mut self, key: &str, def: f64, min: f64, max: f64) -> Result<f64> {
        let expected = format!("a number in [{}, {}]", format_number(min), format_number(max));
        self.parsed(key, def, &expected, |s| parse_double(s).filter(|v| *v >= min && *v <= max))
    }

    pub fn integer(&mut self, key: &str, def: i32, min: i32, max: i32) -> Result<i32> {
        let expected = format!("an integer in [{min}, {max}]");
        self.parsed(key, def, &expected, |s| {
            parse_long(s).filter(|v| *v >= min as i64 && *v <= max as i64).map(|v| v as i32)
        })
    }

    pub fn flag(&mut self, key: &str, def: bool) -> Result<bool> {
        let Some(v) = self.lookup(key) else {
            return Ok(def);
        };
        match v.to_ascii_lowercase().as_str() {
            "" | "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(self.invalid(key, &v, "1/0, true/false, yes/no or on/off")),
        }
    }

    pub fn color(&mut self, key: &str, def: Color) -> Result<Color> {
        self.parsed(key, def, "a color like #rrggbb[aa], r,g,b[,a] or a name", parse_color)
    }

    pub fn filter(&mut self, key: &str, def: Filter) -> Result<Filter> {
        let expected = format!("one of {}", filter_names().join(", "));
        self.parsed(key, def, &expected, parse_filter)
    }

    pub fn edge(&mut self, key: &str, def: EdgeMode) -> Result<EdgeMode> {
        self.parsed(key, def, "clamp, repeat, mirror or fill", parse_edge_mode)
    }

    pub fn interp(&mut self, key: &str, def: Interp) -> Result<Interp> {
        self.parsed(key, def, "nearest, bilinear or bicubic", parse_interp)
    }

    pub fn gravity(&mut self, key: &str, def: Gravity) -> Result<Gravity> {
        self.parsed(key, def, "nw, n, ne, w, center, e, sw, s or se", parse_gravity)
    }

    pub fn channels(&mut self, key: &str, def: u32) -> Result<u32> {
        self.parsed(key, def, "a combination of r, g, b, a (e.g. rgb)", parse_channels)
    }

    /// One of a fixed set of words; returns the index into `choices`.
    pub fn choice(&mut self, key: &str, choices: &[&str], def: usize) -> Result<usize> {
        let Some(v) = self.value_of(key)? else {
            return Ok(def);
        };
        let s = v.to_ascii_lowercase();
        choices
            .iter()
            .position(|c| *c == s)
            .ok_or_else(|| self.invalid(key, &s, &format!("one of {}", choices.join(", "))))
    }

    /// Common resampling knobs: filter, edge, bg, linear, premultiply, blur.
    pub fn resample_options(&mut self, def_filter: Filter, def_edge: EdgeMode, prefix: &str) -> Result<ResampleOptions> {
        Ok(ResampleOptions {
            filter: self.filter(&format!("{prefix}filter"), def_filter)?,
            edge: self.edge(&format!("{prefix}edge"), def_edge)?,
            fill: self.color(&format!("{prefix}bg"), Color::TRANSPARENT)?,
            linear: self.flag(&format!("{prefix}linear"), false)?,
            premultiply: self.flag(&format!("{prefix}premultiply"), true)?,
            blur: self.number(&format!("{prefix}blur"), 1.0, 0.01, 100.0)? as f32,
        })
    }

    /// Fails with a usage error for the first key (in command line order)
    /// that the operation never asked for.
    pub fn finish(&self) -> Result<()> {
        let Some(key) = self.order.iter().find(|k| !self.used.contains(*k)) else {
            return Ok(());
        };
        // Every key the operation asked for is in `used`, so it doubles as
        // the list of valid keys for a typo suggestion.
        let candidates: Vec<&String> = self.used.iter().collect();
        let hint = closest_match(key, &candidates).map(|h| format!(" (did you mean '{h}'?)")).unwrap_or_default();
        Err(Error::usage(format!("{}: option '{key}' is not applicable here{hint}", self.context)))
    }
}
