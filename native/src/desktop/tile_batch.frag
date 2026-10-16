#version 120

uniform sampler2D atlas;
uniform sampler2D palettes;
uniform float palette_width;
varying vec3 atlas_uv;
varying float line_y;

void main() {
    // Atlas texels contain the original 4-bit index, stored in one byte.
    float index = floor(texture2D(atlas, atlas_uv.xy).r * 255.0 + 0.5);
    if (index < 0.5) discard;
    vec2 palette_uv = vec2(
        (atlas_uv.z * 16.0 + index + 0.5) / palette_width,
        (floor(line_y) + 0.5) / 224.0
    );
    gl_FragColor = vec4(texture2D(palettes, palette_uv).rgb, 1.0);
}
