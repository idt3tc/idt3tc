// Command line value syntaxes and option handling.

#include "test.h"

#include "common/util.h"
#include "img/context.h"
#include "img/options.h"
#include "img/spec.h"

using namespace idt3;
using namespace idt3::img;

TEST(geometry_full)
{
    Geometry g = parseGeometry("256x128+4-2", "t");
    CHECK(g.hasW && g.hasH && g.hasOffset);
    CHECK_EQ(g.w.value, 256.0);
    CHECK_EQ(g.h.value, 128.0);
    CHECK_EQ(g.x.value, 4.0);
    CHECK_EQ(g.y.value, -2.0);
}

TEST(geometry_partial_and_percent)
{
    Geometry a = parseGeometry("x64", "t");
    CHECK(!a.hasW && a.hasH);
    Geometry b = parseGeometry("50%", "t");
    CHECK(b.hasW && b.hasH && b.w.percent && b.h.percent);
    CHECK_EQ(b.w.resolve(300), 150.0);
    Geometry c = parseGeometry("512", "t");
    CHECK(c.hasW && !c.hasH);
}

TEST(geometry_invalid)
{
    CHECK_THROWS(parseGeometry("abc", "t"), UsageError);
    CHECK_THROWS(parseGeometry("10x10x10", "t"), UsageError);
    CHECK_THROWS(parseGeometry("", "t"), UsageError);
}

TEST(colors)
{
    Color c;
    CHECK(parseColor("#ff000080", c));
    CHECK_NEAR(c.r, 1.0, 1e-6);
    CHECK_NEAR(c.a, 128 / 255.0, 1e-6);
    CHECK(parseColor("#0f0", c));
    CHECK_NEAR(c.g, 1.0, 1e-6);
    CHECK_NEAR(c.a, 1.0, 1e-6);
    CHECK(parseColor("0,0,255", c));
    CHECK_NEAR(c.b, 1.0, 1e-6);
    CHECK(parseColor("transparent", c));
    CHECK_NEAR(c.a, 0.0, 1e-6);
    CHECK(parseColor("80ff00", c));
    CHECK_NEAR(c.r, 128 / 255.0, 1e-6);
    CHECK(!parseColor("#12345", c));
    CHECK(!parseColor("notacolor", c));
}

TEST(option_reader_rejects_unused_keys)
{
    std::vector<std::pair<std::string, std::string> > kv;
    kv.push_back(std::make_pair("filter", "lanczos3"));
    kv.push_back(std::make_pair("bogus", "1"));
    OptionReader r("--test", kv);
    CHECK(r.filter("filter", Filter::Box) == Filter::Lanczos3);
    CHECK_THROWS(r.finish(), UsageError);
}

TEST(option_reader_flags_and_ranges)
{
    std::vector<std::pair<std::string, std::string> > kv;
    kv.push_back(std::make_pair("linear", ""));
    kv.push_back(std::make_pair("n", "20"));
    OptionReader r("--test", kv);
    CHECK(r.flag("linear", false));
    CHECK_THROWS(r.integer("n", 0, 0, 10), UsageError);
}

TEST(path_placeholders)
{
    Context ctx;
    ctx.inputPath = "textures/base/wall.png";
    ctx.inputRel = "base/wall";
    ctx.index = 3;
    ctx.image.allocate(64, 32);
    CHECK_EQ(ctx.expandPath("out/{rel}_{w}x{h}.tga"), std::string("out/base/wall_64x32.tga"));
    CHECK_EQ(ctx.expandPath("{dir}/{name}.{ext}.dds"), std::string("textures/base/wall.png.dds"));
    CHECK_EQ(ctx.expandPath("{{literal}}_{index}"), std::string("{literal}_3"));
    CHECK_THROWS(ctx.expandPath("{nope}"), UsageError);
    CHECK(hasInputPlaceholder("x/{name}.tga"));
    CHECK(!hasInputPlaceholder("x/{w}.tga"));
}
