struct Object {
    center_kind: vec4f,
    half_selected: vec4f,
    rotation: vec4f,
}
struct Frame {
    matrix: mat4x4f,
    inverse: mat4x4f,
    eye: vec4f,
    right: vec4f,
    up: vec4f,
    forward: vec4f,
    size_radius: vec4f,
    material: vec4f,
    settings: vec4f,
    light: vec4f,
    bounds_min: vec4f,
    bounds_max: vec4f,
    environment: vec4f,
    geometry: vec4f,
    clip_plane: vec4f,
    objects: array<Object,128>,
}
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var particle_image: texture_2d<f32>;



struct Screen {
    @builtin(position) clip: vec4f,
}

@vertex fn full_screen(@builtin(vertex_index) index: u32) -> Screen {
    return Screen(vec4f(select(-1.0,3.0,index==1u),select(-1.0,3.0,index==2u),0.0,1.0));
}

struct Splat {
    @builtin(position) clip: vec4f,
    @location(0) local: vec2f,
    @location(1) @interpolate(flat) particle: vec4f,
}

fn splat_radius(particle: vec4f) -> f32 {
    if (frame.size_radius.w < 0.5) { return frame.size_radius.z; }
    let kind = floor(particle.w);
    let variation = fract(sin(dot(particle.xyz,vec3f(8.0,13.0,5.0)))*23.5);
    return mix(0.0045,0.0095,variation) * select(1.0,0.84,kind==3.0);
}

fn clipped(point: vec3f) -> bool {
    return dot(frame.clip_plane.xyz,point)>frame.clip_plane.w;
}

@vertex fn splat(@builtin(vertex_index) index: u32, @location(0) particle: vec4f) -> Splat {
    let corners = array<vec2f,4>(vec2f(-1.0,-1.0),vec2f(1.0,-1.0),vec2f(-1.0,1.0),vec2f(1.0,1.0));
    let local = corners[index];
    let world = particle.xyz + (frame.right.xyz*local.x+frame.up.xyz*local.y)*splat_radius(particle);
    return Splat(frame.matrix * vec4f(world,1.0), local, particle);
}

struct SplatDepth {
    @location(0) color: vec4f,
    @builtin(frag_depth) depth: f32,
}

@fragment fn closest(input: Splat) -> SplatDepth {
    if (frame.size_radius.w>0.5 && floor(input.particle.w)==2.0) { discard; }
    let r2 = dot(input.local,input.local);
    if (r2 > 1.0) { discard; }
    let cap = sqrt(max(0.0,1.0-r2));
    let offset = frame.right.xyz*input.local.x + frame.up.xyz*input.local.y - frame.forward.xyz*cap;
    let world = input.particle.xyz + offset*splat_radius(input.particle);
    if (clipped(world)) { discard; }
    let depth = dot(world-frame.eye.xyz,frame.forward.xyz);
    let clip = frame.matrix*vec4f(world,1.0);
    return SplatDepth(vec4f(depth,input.particle.w,cap,1.0),clamp(clip.z/clip.w,0.0,1.0));
}

fn ray_at(p: vec2f) -> vec3f {
    if (frame.eye.w > 0.5) { return frame.forward.xyz; }
    let ndc = (p/frame.size_radius.xy)*vec2f(2.0,-2.0)+vec2f(-1.0,1.0);
    let far = frame.inverse*vec4f(ndc,1.0,1.0);
    return normalize(far.xyz/far.w-frame.eye.xyz);
}

fn ray_origin(p: vec2f) -> vec3f {
    if (frame.eye.w < 0.5) { return frame.eye.xyz; }
    let ndc = (p/frame.size_radius.xy)*vec2f(2.0,-2.0)+vec2f(-1.0,1.0);
    let near = frame.inverse*vec4f(ndc,0.0,1.0);
    let offset = near.xyz/near.w-frame.eye.xyz;
    return frame.eye.xyz+offset-frame.forward.xyz*dot(offset,frame.forward.xyz);
}

fn rotate_vector(p: vec3f, q: vec4f) -> vec3f {
    let t = 2.0*cross(q.xyz,p);
    return p + q.w*t + cross(q.xyz,t);
}

fn studio(direction: vec3f) -> vec3f {
    let sky = mix(vec3f(0.012,0.024,0.042),vec3f(0.16,0.28,0.41),smoothstep(-0.12,0.86,direction.y));
    let panel = smoothstep(0.81,0.975,dot(direction,normalize(vec3f(-0.55,0.74,0.35))));
    let rim = smoothstep(0.83,0.97,dot(direction,normalize(vec3f(0.78,0.40,-0.34))));
    let strip = smoothstep(0.65,0.87,direction.y)*(1.0-smoothstep(0.10,0.31,abs(direction.x-0.18)));
    return sky + panel*vec3f(2.1,2.35,2.62) + rim*vec3f(0.34,0.89,1.05)
        + strip*vec3f(0.39,0.64,0.79);
}

