// zgx entry points: fs=fs vs=vs
// language: metal2.4
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Out {
    metal::float4 position;
    metal::int2 shift;
    char _pad2[8];
};
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct vsInput {
    metal::float4 rect [[attribute(0)]];
    metal::float4 shift [[attribute(1)]];
};
struct vsOutput {
    metal::float4 position [[position]];
    metal::int2 shift [[user(loc0), flat]];
};
vertex vsOutput vs(
  vsInput varyings [[stage_in]]
, uint i [[vertex_id]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
) {
    const auto rect = varyings.rect;
    const auto shift = varyings.shift;
    Out o = {};
    metal::float2 corner = metal::float2(static_cast<float>(i & 1u), static_cast<float>(i >> 1u));
    metal::float2 pixel = rect.xy + (corner * rect.zw);
    metal::float2 size = static_cast<metal::float2>(metal::uint2(source.get_width(), source.get_height()));
    o.position = metal::float4(((pixel.x / size.x) * 2.0) - 1.0, 1.0 - ((pixel.y / size.y) * 2.0), 0.0, 1.0);
    o.shift = naga_f2i32(shift.xy);
    Out _e39 = o;
    const auto _tmp = _e39;
    return vsOutput { _tmp.position, _tmp.shift };
}


struct fsInput {
    metal::int2 shift [[user(loc0), flat]];
};
struct fsOutput {
    metal::float4 member_1 [[color(0)]];
};
fragment fsOutput fs(
  fsInput varyings_1 [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
) {
    const Out v = { position, varyings_1.shift };
    metal::float4 _e8 = source.read(metal::uint2(as_type<metal::int2>(as_type<metal::uint2>(naga_f2i32(v.position.xy)) + as_type<metal::uint2>(v.shift))), 0);
    return fsOutput { _e8 };
}
