// Per-frame uniforms of the viewport, shared by every viewport pass.
// Must match `FrameUniforms` in crates/patine_render/src/viewport.rs.

struct Frame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz: eye position.
    camera_position: vec4<f32>,
    // rgb: linear base color, a: unused.
    base_color: vec4<f32>,
    // x: metallic, y: roughness, z: exposure multiplier, w: tone mapper index.
    material: vec4<f32>,
    // rgb: wireframe color (linear), a: opacity.
    wire_color: vec4<f32>,
    // x, y: size in pixels; z, w: inverse size.
    viewport: vec4<f32>,
    // rgb: background color (linear); a: background mode (0 solid, 1 environment).
    background: vec4<f32>,
    // x: environment rotation (radians); y: background blur (mip); z: shadows on;
    // w: world size of a shadow map texel.
    environment: vec4<f32>,
    // World to shadow map clip space of the dominant light (standard depth, 0 near).
    light_view_proj: mat4x4<f32>,
}
