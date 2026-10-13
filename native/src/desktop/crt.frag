#version 120

uniform sampler2D screen;
uniform vec2 source_size;
uniform vec2 beam_size;
uniform vec2 viewport_size;
uniform float curvature;
uniform float scanlines;
varying vec2 screen_uv;

vec3 light(vec2 uv) {
    vec2 half_texel = 0.5 / source_size;
    return pow(texture2D(screen, clamp(uv, half_texel, 1.0 - half_texel)).rgb, vec3(2.2));
}

void main() {
    // Inverse barrel mapping bends the picture and the glass edges together.
    vec2 p = screen_uv * 2.0 - 1.0;
    p *= 1.0 + curvature * p.yx * p.yx;
    vec2 uv = p * 0.5 + 0.5;
    vec2 corner = abs(p) - vec2(0.965);
    float glass = length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - 0.035;
    float edge = 1.0 - smoothstep(-2.0 / min(viewport_size.x, viewport_size.y), 0.0, glass);
    if (edge <= 0.0) {
        gl_FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    // A slightly soft analogue beam, blended in linear light.
    vec2 dx = vec2(0.55 / beam_size.x, 0.0);
    vec3 color = light(uv) * 0.64 + (light(uv - dx) + light(uv + dx)) * 0.18;
    vec3 halo = light(uv - dx * 2.5) + light(uv + dx * 2.5)
              + light(uv - vec2(0.0, 1.0 / beam_size.y))
              + light(uv + vec2(0.0, 1.0 / beam_size.y));
    color += max(halo * 0.25 - vec3(0.25), 0.0) * 0.10;

    // Brighter phosphors produce wider beams. Fade unresolved lines away in
    // small windows instead of letting them alias into large dark bands.
    float luminance = dot(color, vec3(0.2126, 0.7152, 0.0722));
    float width = 0.50 + 0.08 * sqrt(luminance) - 0.14 * scanlines;
    float d = fract(uv.y * beam_size.y) - 0.5;
    float beam = exp(-d * d / (width * width))
               + exp(-(d - 1.0) * (d - 1.0) / (width * width))
               + exp(-(d + 1.0) * (d + 1.0) / (width * width));
    beam /= 1.0 + 2.0 * exp(-1.0 / (width * width));
    color *= mix(1.0, beam, smoothstep(1.0, 2.0, viewport_size.y / beam_size.y));

    // RGB aperture grille in physical output pixels, stable across resizing.
    float stripe = mod(floor(gl_FragCoord.x), 3.0);
    vec3 mask = vec3(0.82);
    if (stripe < 1.0) mask.r = 1.0;
    else if (stripe < 2.0) mask.g = 1.0;
    else mask.b = 1.0;
    float mask_visibility = smoothstep(1.5, 3.0, viewport_size.x / beam_size.x);
    color *= mix(vec3(1.0), mask * 1.10, mask_visibility);
    color *= 1.0 - 0.10 * dot(p * p, vec2(0.5));
    gl_FragColor = vec4(pow(max(color, 0.0), vec3(1.0 / 2.2)) * edge, 1.0);
}
