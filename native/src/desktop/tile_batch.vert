#version 120

varying vec3 atlas_uv;
varying float line_y;

void main() {
    gl_Position = gl_Vertex;
    atlas_uv = gl_MultiTexCoord0.xyz;
    line_y = (1.0 - gl_Vertex.y) * 112.0;
}
