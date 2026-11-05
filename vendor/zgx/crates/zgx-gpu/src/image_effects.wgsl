// Image effects (`ImageData::characters`, `dithered`), appended to draw.wgsl
// as a pipeline of their own (`image_effect_source`), compiled when an image
// first draws with one: the quad shader every other draw runs carries none
// of this, neither its registers nor its compile.
//
// An image quad says its effect's size in its texture lane (`options.y`, 2
// plus it, premultiplied), characters theirs in `options.w` (-1, where
// positive is per-corner radii) and their reveal in the colour, white for
// any image: front, band, raggedness in `color.rgb`, the centre's offset in
// `border.zw`.

// `ImageData::characters`' font: glyph `i` of ` .-+xXS80#@` (darkest first),
// 5 by 7 pixels, rows top first, five bits a row (the leftmost highest): rows
// 0 to 5 in `x`, row 6 in `y`. A switch, not an array a runtime index reads
// (which Metal spills to memory).
fn glyph(i:u32)->vec2<u32> {
 switch i {
  case 1u: {return vec2(402653184u,12u);} case 2u: {return vec2(1015808u,0u);}
  case 3u: {return vec2(139432064u,0u);} case 4u: {return vec2(340083712u,17u);}
  case 5u: {return vec2(581052977u,17u);} case 6u: {return vec2(35078671u,30u);}
  case 7u: {return vec2(588727854u,14u);} case 8u: {return vec2(597347886u,14u);}
  case 9u: {return vec2(368409930u,10u);} case 10u: {return vec2(561700398u,15u);}
  default: {return vec2(0u);}
 }
}
// A cell's own random number, 0 to 1 (a 32-bit integer hash).
fn cell_hash(c:vec2<u32>)->f32 {
 var h=c.x*1664525u+c.y*1013904223u+0x9e3779b9u;
 h^=h>>16u; h*=2246822519u; h^=h>>13u; h*=3266489917u; h^=h>>16u;
 return f32(h&16777215u)/16777216.;
}
// Characters: cells 6 by 8 font pixels on the screen's grid, each a glyph
// picked by the brightness at its centre and lit in that colour at full
// value, over the image dimmed. Premultiplied.
fn characters(v:Out)->vec4<f32> {
 let size=(v.options.y-2.)*viewport.z;
 let reveal=v.color.rgb;
 let cell_size=vec2(6.,8.)*size;
 let cell=floor(v.position.xy/cell_size);
 let to_centre=(cell+.5)*cell_size-v.position.xy;
 let at=textureSampleLevel(atlas,tex_sampler,v.uv+dpdxFine(v.uv)*to_centre.x+dpdyFine(v.uv)*to_centre.y,0.);
 let own=textureSample(atlas,tex_sampler,v.uv);
 let c=vec2<u32>(vec2<i32>(cell));
 // The reveal, from the point it opens at out to a rounded box at 1: the
 // picture fades in smoothly, pixel by pixel, behind the band (no cells
 // popping); characters scatter ahead of it, sparse and faint at the
 // band's outer edge, denser and brighter inward, scrambling until the
 // picture reaches them.
 var shown=1.; var glyph_alpha=1.; var scrambling=false; var scramble=0u;
 if reveal.y>0. {
   let q=abs(v.position.xy/viewport.z/viewport.xy-.5-v.border.zw)*2.; let q4=q*q*q*q;
   shown=smoothstep(reveal.y*.35,reveal.y*1.35,reveal.x-sqrt(sqrt(q4.x+q4.y)));
   let qc=abs((cell+.5)*cell_size/viewport.z/viewport.xy-.5-v.border.zw)*2.; let qc4=qc*qc*qc*qc;
   let front=reveal.x-sqrt(sqrt(qc4.x+qc4.y))+(cell_hash(c)-.5)*reveal.z;
   let into=front/reveal.y;
   glyph_alpha=select(0.,smoothstep(0.,.6,into),cell_hash(c+vec2(77u,13u))<pow(clamp(into,0.,1.),.7));
   scrambling=into<1.; scramble=u32(max(into,0.)*12.);
 }
 let straight=select(vec3(0.),at.rgb/at.a,at.a>0.);
 // Glyphs for highlights, light ones for midtones, none for shadows.
 var index=u32(pow(clamp((dot(straight,vec3(.299,.587,.114))*at.a-.3)/.65,0.,1.),1.6)*10.999);
 if scrambling {index=1u+u32(cell_hash(c+vec2(scramble*7u+1u,scramble))*9.999);}
 let g=glyph(index);
 let fp=vec2<u32>(floor((v.position.xy-cell*cell_size)/size));
 var lit=0.;
 if fp.x<5u && fp.y<7u {lit=f32((select(g.y,g.x>>(fp.y*5u),fp.y<6u)>>(4u-fp.x))&1u);}
 let ink=straight/max(max(straight.r,max(straight.g,straight.b)),.25);
 // The glyph over the picture (dimmed), its strength easing from whole,
 // alone ahead of the picture, to 0.65 over it.
 let k=lit*glyph_alpha*mix(1.,.65,shown);
 return vec4(ink*at.a*k+own.rgb*.88*shown*(1.-k),at.a*k+own.a*shown*(1.-k));
}
// Dithered: one sample per dot, at its centre on the screen's grid (the uv
// there through the quad's own slopes, so moved, scaled and covering images
// dither alike), then Zeron's two levels: the colour at full value where its
// brightest channel passes the dot's 4x4 Bayer threshold, else 8% of it.
// Premultiplied.
fn dithered(v:Out)->vec4<f32> {
 let dot=(v.options.y-2.)*viewport.z;
 if dot<=0.5 {return textureSample(atlas,tex_sampler,v.uv);}
 let cell=floor(v.position.xy/dot);
 let centre=(cell+.5)*dot-v.position.xy;
 let tex=textureSampleLevel(atlas,tex_sampler,v.uv+dpdxFine(v.uv)*centre.x+dpdyFine(v.uv)*centre.y,0.);
 if tex.a<=0. {return tex;}
 let c=vec2<u32>(vec2<i32>(cell))&vec2(3u);
 let a=c.x^c.y; let b=c.y;
 let bayer=f32(((a&1u)<<3u)|((b&1u)<<2u)|(a&2u)|((b&2u)>>1u));
 let straight=tex.rgb/tex.a; let peak=max(straight.r,max(straight.g,straight.b));
 return vec4(select(straight*.08,straight/max(peak,1e-5),peak>(bayer+.5)/16.)*tex.a,tex.a);
}
// An image effect's draw: its effect quads' texel worked out here, then
// shaded as any quad (corners, fades, clips, colour filters); its other
// quads as `shade` draws them.
@fragment fn zgx_image_effect(input:Out)->@location(0) vec4<f32> {
 if input.options.y<=2. {return shade(input);}
 var v=input;
 var tex:vec4<f32>;
 if v.options.w<-.5 {tex=characters(v); v.color=vec4(1.,1.,1.,v.color.a);}
 else {tex=dithered(v);}
 return shade_texel(v,tex,true);
}
