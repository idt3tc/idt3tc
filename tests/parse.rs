//! Command line value syntaxes and option handling.

mod common;

use common::kv;
use idt3tc::img::context::{Context, has_input_placeholder};
use idt3tc::img::image::Image;
use idt3tc::img::options::OptionReader;
use idt3tc::img::resample::Filter;
use idt3tc::img::spec::{parse_color, parse_geometry};

fn near(a: f32, b: f64, eps: f64) -> bool {
    (a as f64 - b).abs() <= eps
}

#[test]
fn geometry_full() {
    let g = parse_geometry("256x128+4-2", "t").unwrap();
    assert!(g.has_w && g.has_h && g.has_offset);
    assert_eq!(g.w.value, 256.0);
    assert_eq!(g.h.value, 128.0);
    assert_eq!(g.x.value, 4.0);
    assert_eq!(g.y.value, -2.0);
}

#[test]
fn geometry_partial_and_percent() {
    let a = parse_geometry("x64", "t").unwrap();
    assert!(!a.has_w && a.has_h);
    let b = parse_geometry("50%", "t").unwrap();
    assert!(b.has_w && b.has_h && b.w.percent && b.h.percent);
    assert_eq!(b.w.resolve(300.0), 150.0);
    let c = parse_geometry("512", "t").unwrap();
    assert!(c.has_w && !c.has_h);
}

#[test]
fn geometry_invalid() {
    for s in ["abc", "10x10x10", "", "nanxnan", "1e400x5", "+inf+0", "x", "%"] {
        let e = parse_geometry(s, "t").expect_err(s);
        assert!(e.is_usage(), "{s}: {e}");
    }
}

#[test]
fn colors() {
    let c = parse_color("#ff000080").unwrap();
    assert!(near(c.r, 1.0, 1e-6) && near(c.a, 128.0 / 255.0, 1e-6));
    let c = parse_color("#0f0").unwrap();
    assert!(near(c.g, 1.0, 1e-6) && near(c.a, 1.0, 1e-6));
    assert!(near(parse_color("0,0,255").unwrap().b, 1.0, 1e-6));
    assert!(near(parse_color("transparent").unwrap().a, 0.0, 1e-6));
    assert!(near(parse_color("80ff00").unwrap().r, 128.0 / 255.0, 1e-6));
    assert!(parse_color("#12345").is_none());
    assert!(parse_color("notacolor").is_none());
    // Multi-byte characters must not be split in the middle.
    assert!(parse_color("#ääää").is_none());
    assert!(parse_color("1e999,0,0").is_none());
}

#[test]
fn option_reader_rejects_unused_keys() {
    let mut r = OptionReader::new("--test", &kv(&[("filter", "lanczos3"), ("bogus", "1")])).unwrap();
    assert_eq!(r.filter("filter", Filter::Box).unwrap(), Filter::Lanczos3);
    assert!(r.finish().unwrap_err().is_usage());
}

#[test]
fn option_reader_flags_and_ranges() {
    let mut r = OptionReader::new("--test", &kv(&[("linear", ""), ("n", "20"), ("x", "nan")])).unwrap();
    assert!(r.flag("linear", false).unwrap());
    assert!(r.integer("n", 0, 0, 10).unwrap_err().is_usage());
    assert!(r.number("x", 0.0, 0.0, 1.0).unwrap_err().is_usage());
}

#[test]
fn option_reader_duplicate_keys() {
    assert!(OptionReader::new("--test", &kv(&[("a", "1"), ("a", "2")])).is_err());
}

#[test]
fn path_placeholders() {
    let mut ctx = Context::new(1);
    ctx.input_path = "textures/base/wall.png".into();
    ctx.input_rel = "base/wall".into();
    ctx.index = 3;
    ctx.image = Image::new(64, 32);
    assert_eq!(ctx.expand_path("out/{rel}_{w}x{h}.tga").unwrap(), "out/base/wall_64x32.tga");
    assert_eq!(ctx.expand_path("{dir}/{name}.{ext}.dds").unwrap(), "textures/base/wall.png.dds");
    assert_eq!(ctx.expand_path("{{literal}}_{index}").unwrap(), "{literal}_3");
    assert!(ctx.expand_path("{nope}").unwrap_err().is_usage());
    assert!(ctx.expand_path("{name").unwrap_err().is_usage());
    assert!(has_input_placeholder("x/{name}.tga"));
    assert!(!has_input_placeholder("x/{w}.tga"));
}
