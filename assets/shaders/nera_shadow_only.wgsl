// Project Nera - shadow-only receiver.
//
// The receiver is an ordinary StandardMaterial extension, so Bevy performs its
// normal directional-light shadow sampling. We then throw away the lit surface
// and keep only darkness caused by the shadow map as black alpha.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, alpha_discard},
    forward_io::{VertexOutput, FragmentOutput},
}

struct NeraShadowOnlyExtension {
    opacity: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> nera_shadow: NeraShadowOnlyExtension;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Force a predictable white, fully opaque PBR probe surface. It is never
    // shown directly; its lighting result is used only to recover shadowing.
    pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, 1.0);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material,
        pbr_input.material.base_color,
    );

    let lit = apply_pbr_lighting(pbr_input);
    let luminance = dot(lit.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));

    // Lit pixels from Nera's ~10k lux key light are comfortably above this
    // range and become fully transparent. Shadowed / penumbra pixels survive.
    // smoothstep also preserves Bevy's filtered shadow-map edge rather than
    // creating a binary cutout.
    let darkness = 1.0 - clamp(luminance, 0.0, 1.0);
    let shadow_alpha = smoothstep(0.20, 0.92, darkness)
        * clamp(nera_shadow.opacity, 0.0, 1.0);

    var out: FragmentOutput;
    out.color = vec4<f32>(0.0, 0.0, 0.0, shadow_alpha);
    return out;
}
