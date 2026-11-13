// Resampling, geometric transforms and registered operations.

#include "helpers.h"
#include "test.h"

#include "common/util.h"
#include "img/mipmap.h"
#include "img/op.h"
#include "img/resample.h"
#include "img/transform.h"

using namespace idt3;
using namespace idt3::img;
using idt3test::makeTestImage;
using idt3test::maxByteDiff;

namespace {

// Runs a single registered operation like the CLI would.
void runOp(Context& ctx, const std::string& name, const std::string& arg,
           const std::vector<std::pair<std::string, std::string> >& kv =
               std::vector<std::pair<std::string, std::string> >())
{
    const OpInfo* info = OpRegistry::instance().find(name);
    if (!info)
        throw Error("no op " + name);
    OptionReader r("--" + name, kv);
    std::unique_ptr<Op> op = info->create(arg, r);
    r.finish();
    op->apply(ctx);
}

Context contextWith(const Image& img)
{
    Context ctx;
    ctx.image = img;
    ctx.hasImage = true;
    ctx.jobs = 2;
    return ctx;
}

} // namespace

TEST(resample_identity)
{
    Image src = makeTestImage(37, 21, true);
    ResampleOptions o;
    o.filter = Filter::Lanczos3;
    Image out = resample(src, 37, 21, o, 1);
    CHECK_EQ(maxByteDiff(src, out), 0);
}

TEST(resample_box_halving_averages)
{
    Image src(2, 2);
    const float v[4] = {0.0f, 1.0f, 0.5f, 0.5f};
    for (int i = 0; i < 4; ++i) {
        float* p = &src.data[i * 4];
        p[0] = p[1] = p[2] = v[i];
        p[3] = 1.0f;
    }
    ResampleOptions o;
    o.filter = Filter::Box;
    Image out = resample(src, 1, 1, o, 1);
    CHECK_NEAR(out.data[0], 0.5, 1e-5);
}

TEST(resample_threads_match)
{
    Image src = makeTestImage(129, 77, true);
    ResampleOptions o;
    o.filter = Filter::Mitchell;
    Image a = resample(src, 50, 200, o, 1);
    Image b = resample(src, 50, 200, o, 8);
    CHECK(a.data == b.data);
}

TEST(rotate_quarter_roundtrip)
{
    Image src = makeTestImage(5, 3, true);
    Image r = rotateQuarter(src, 1);
    CHECK_EQ(r.width, 3);
    CHECK_EQ(r.height, 5);
    // Clockwise: the top-left pixel ends up at the top-right.
    CHECK(std::equal(src.pixel(0, 0), src.pixel(0, 0) + 4, r.pixel(2, 0)));
    Image back = rotateQuarter(rotateQuarter(rotateQuarter(r, 1), 1), 1);
    CHECK(back.data == src.data);
}

TEST(rotate_arbitrary_90_matches_quarter)
{
    Image src = makeTestImage(16, 16, false);
    AffineOptions o;
    o.interp = Interp::Bilinear;
    Image a = rotateArbitrary(src, 90.0, true, o, 2);
    Image b = rotateQuarter(src, 1);
    CHECK_EQ(a.width, b.width);
    CHECK_EQ(a.height, b.height);
    CHECK(maxByteDiff(a, b) <= 1);
}

TEST(translate_wraps)
{
    Image src = makeTestImage(8, 4, true);
    AffineOptions o;
    o.edge = EdgeMode::Repeat;
    Image out = translate(src, 3, -1, o, 1);
    for (int y = 0; y < 4; ++y) {
        for (int x = 0; x < 8; ++x) {
            const float* d = out.pixel(x, y);
            const float* s = src.pixel((x - 3 + 8) % 8, (y + 1) % 4);
            CHECK(std::equal(s, s + 4, d));
        }
    }
}

TEST(op_order_matters)
{
    // crop then resize != resize then crop
    Image src = makeTestImage(64, 64, false);
    Context a = contextWith(src), b = contextWith(src);
    runOp(a, "crop", "32x32");
    runOp(a, "resize", "16x16");
    runOp(b, "resize", "16x16");
    runOp(b, "crop", "8x8");
    CHECK_EQ(a.image.width, 16);
    CHECK_EQ(b.image.width, 8);
}

TEST(op_pot)
{
    Context ctx = contextWith(makeTestImage(300, 200, false));
    runOp(ctx, "pot", "");
    CHECK_EQ(ctx.image.width, 256);
    CHECK_EQ(ctx.image.height, 256);

    Context up = contextWith(makeTestImage(300, 70, false));
    std::vector<std::pair<std::string, std::string> > kv;
    kv.push_back(std::make_pair("mode", "up"));
    kv.push_back(std::make_pair("max", "256"));
    runOp(up, "pot", "", kv);
    CHECK_EQ(up.image.width, 256);
    CHECK_EQ(up.image.height, 128);
}

TEST(op_trim_alpha)
{
    Image src(20, 10, Color(0, 0, 0, 0));
    for (int y = 3; y < 6; ++y) {
        for (int x = 5; x < 12; ++x)
            src.pixel(x, y)[3] = 1.0f;
    }
    Context ctx = contextWith(src);
    runOp(ctx, "trim", "");
    CHECK_EQ(ctx.image.width, 7);
    CHECK_EQ(ctx.image.height, 3);
}

TEST(op_canvas_and_pad)
{
    Context ctx = contextWith(makeTestImage(10, 10, false));
    runOp(ctx, "canvas", "16x12");
    CHECK_EQ(ctx.image.width, 16);
    CHECK_EQ(ctx.image.height, 12);
    runOp(ctx, "pad", "1,2");
    CHECK_EQ(ctx.image.width, 18);
    CHECK_EQ(ctx.image.height, 16);
}

TEST(op_color_basics)
{
    Context ctx = contextWith(Image(2, 2, Color(0.2f, 0.4f, 0.6f, 1.0f)));
    runOp(ctx, "invert", "");
    CHECK_NEAR(ctx.image.data[0], 0.8, 1e-6);
    CHECK_NEAR(ctx.image.data[3], 1.0, 1e-6); // alpha untouched by default
    runOp(ctx, "swizzle", "bgr1");
    CHECK_NEAR(ctx.image.data[0], 0.4, 1e-6);
    runOp(ctx, "grayscale", "");
    CHECK_NEAR(ctx.image.data[0], ctx.image.data[1], 1e-6);
    std::vector<std::pair<std::string, std::string> > kv;
    kv.push_back(std::make_pair("value", "0.25"));
    runOp(ctx, "alpha", "set", kv);
    CHECK_NEAR(ctx.image.data[3], 0.25, 1e-6);
}

TEST(op_colorkey)
{
    Image src(2, 1, Color(1, 0, 1, 1));
    src.pixel(1, 0)[1] = 1.0f; // white
    Context ctx = contextWith(src);
    runOp(ctx, "colorkey", "#ff00ff");
    CHECK_NEAR(ctx.image.pixel(0, 0)[3], 0.0, 1e-6);
    CHECK_NEAR(ctx.image.pixel(1, 0)[3], 1.0, 1e-6);
}

TEST(op_normalmap_flat)
{
    Context ctx = contextWith(Image(8, 8, Color(0.5f, 0.5f, 0.5f, 1)));
    runOp(ctx, "normalmap", "");
    const float* p = ctx.image.pixel(3, 3);
    CHECK_NEAR(p[0], 0.5, 1e-5);
    CHECK_NEAR(p[1], 0.5, 1e-5);
    CHECK_NEAR(p[2], 1.0, 1e-5);
}

TEST(op_unknown_option_rejected)
{
    Context ctx = contextWith(makeTestImage(4, 4, false));
    std::vector<std::pair<std::string, std::string> > kv;
    kv.push_back(std::make_pair("gravty", "c"));
    CHECK_THROWS(runOp(ctx, "crop", "2x2", kv), UsageError);
}

TEST(mip_chain_sizes)
{
    MipOptions o;
    std::vector<Image> chain = buildMipChain(makeTestImage(256, 64, true), o, 2);
    CHECK_EQ(chain.size(), size_t(9));
    CHECK_EQ(fullMipCount(256, 64), 9);
    CHECK_EQ(chain.back().width, 1);
    CHECK_EQ(chain.back().height, 1);
    CHECK_EQ(chain[3].width, 32);
    CHECK_EQ(chain[3].height, 8);
}

TEST(alpha_coverage_preserved)
{
    // Fine alpha-tested detail (foliage, grates) loses coverage when
    // downsampling averages alpha towards the middle.
    Image src(64, 64, Color(1, 1, 1, 0));
    for (int y = 0; y < 64; ++y) {
        for (int x = 0; x < 64; ++x)
            src.pixel(x, y)[3] = 0.5f + 0.5f * std::sin(x * 1.3f) * std::sin(y * 0.9f) - 0.15f;
    }
    clampImage(src);
    const float target = alphaCoverage(src, 0.5f);
    MipOptions plain, kept;
    kept.coverageRef = 0.5f;
    std::vector<Image> a = buildMipChain(src, plain, 2);
    std::vector<Image> b = buildMipChain(src, kept, 2);
    for (size_t i = 1; i < 4; ++i) {
        float drift = std::fabs(alphaCoverage(a[i], 0.5f) - target);
        float kept_ = std::fabs(alphaCoverage(b[i], 0.5f) - target);
        CHECK(kept_ <= drift);
        CHECK_NEAR(alphaCoverage(b[i], 0.5f), target, 0.05);
    }
}
