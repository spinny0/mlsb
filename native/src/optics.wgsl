@group(0) @binding(2) var density: texture_3d<f32>;
@group(0) @binding(3) var blocks: texture_3d<f32>;
@group(0) @binding(4) var field_sampler: sampler;
@group(0) @binding(5) var<storage, read> light_map: array<u32>;
@group(0) @binding(6) var<storage, read_write> light_deposit: array<atomic<u32>>;
@group(0) @binding(7) var optical_image: texture_2d<f32>;
struct MeshNode { low:vec4f, high:vec4f }
struct MeshTriangle { a:vec4f, b:vec4f, c:vec4f }
@group(0) @binding(8) var<storage,read> mesh_nodes:array<MeshNode>;
@group(0) @binding(9) var<storage,read> mesh_triangles:array<MeshTriangle>;
@group(0) @binding(10) var panorama:texture_2d<f32>;
@group(0) @binding(11) var panorama_sampler:sampler;

const FIELD_MIN: vec3f = vec3f(-2.16,-0.12,-1.53);
const FIELD_SIZE: vec3f = vec3f(4.32,3.06,3.06);
const BLOCK_SIZE: f32 = 0.18;
const ISO: f32 = 0.30;
const EPS: f32 = 0.004;
const FAR: f32 = 100.0;
const NO_OBJECT: u32 = 255u;
const MAP_SIZE: vec2u = vec2u(128u,96u);
const MAP_LOW: vec2f = vec2f(-2.16,-1.53);
const MAP_SPAN: vec2f = vec2f(4.32,3.06);

fn safe_inverse(ray: vec3f) -> vec3f {
    return select(vec3f(-1.0),vec3f(1.0),ray >= vec3f(0.0)) / max(abs(ray),vec3f(0.00001));
}

fn span(origin: vec3f, inverse: vec3f, low: vec3f, high: vec3f) -> vec2f {
    let a = (low-origin)*inverse;
    let b = (high-origin)*inverse;
    let lo = min(a,b);
    let hi = max(a,b);
    return vec2f(max(lo.x,max(lo.y,lo.z)),min(hi.x,min(hi.y,hi.z)));
}

fn liquid_field(p: vec3f) -> vec2f {
    if (clipped(p)) { return vec2f(0.0); }
    let uv = (p-FIELD_MIN)/FIELD_SIZE;
    if (any(uv < vec3f(0.0)) || any(uv > vec3f(1.0))) { return vec2f(0.0); }
    return textureSampleLevel(density,field_sampler,uv,0.0).xy;
}

fn water_density(p: vec3f) -> f32 { return liquid_field(p).x; }

fn foam_coverage(point: vec3f) -> f32 {
    return textureSampleLevel(density,field_sampler,clamp((point-FIELD_MIN)/FIELD_SIZE,vec3f(0.0),vec3f(1.0)),0.0).z;
}

fn in_medium(p: vec3f, phase: u32) -> bool {
    let field = liquid_field(p);
    return field.x >= ISO && (phase == 0u || (field.y >= 0.5) == (phase == 2u));
}

fn medium_hit(origin: vec3f, ray: vec3f, limit: f32, inside: bool, phase: u32) -> f32 {
    if (frame.bounds_min.w < 0.5) { return FAR; }
    let inverse = safe_inverse(ray);
    let bounds = span(origin,inverse,frame.bounds_min.xyz,frame.bounds_max.xyz);
    var t = max(bounds.x,EPS);
    let end = min(bounds.y + EPS,limit);
    var previous = max(EPS*0.35,t-0.02);
    for (var i = 0u; i < 176u && t < end; i++) {
        let p = origin + ray*t;
        if (!inside) {
            let cell = vec3i(floor((p-FIELD_MIN)/BLOCK_SIZE));
            let safe = clamp(cell,vec3i(0),vec3i(textureDimensions(blocks))-1);
            if (textureLoad(blocks,safe,0).x < 0.1) {
                let edge = FIELD_MIN + (vec3f(cell) + select(vec3f(0.0),vec3f(1.0),ray>=vec3f(0.0)))*BLOCK_SIZE;
                let next = (edge-p)*inverse;
                t += max(0.012,min(next.x,min(next.y,next.z))+0.0008);
                previous = max(previous,t-0.012);
                continue;
            }
        }
        let hit = in_medium(p,phase) != inside;
        if (hit) {
            var a = previous;
            var b = t;
            for (var j = 0u; j < 4u; j++) {
                let m = (a+b)*0.5;
                if (in_medium(origin+ray*m,phase) != inside) { b = m; }
                else { a = m; }
            }
            return (a+b)*0.5;
        }
        previous = t;
        t = min(t+frame.settings.z,end-0.0002);
        if (t <= previous) { break; }
    }
    return FAR;
}

fn water_hit(origin: vec3f, ray: vec3f, limit: f32, inside: bool) -> f32 {
    return medium_hit(origin,ray,limit,inside,0u);
}

fn medium_at(p: vec3f) -> u32 {
    let field = liquid_field(p);
    return select(0u,select(1u,2u,frame.geometry.y>0.5 && field.y>=0.5),field.x>=ISO);
}

fn interface_normal(p: vec3f) -> vec3f {
    let x = vec3f(0.036,0.0,0.0);
    let y = vec3f(0.0,0.036,0.0);
    let z = vec3f(0.0,0.0,0.036);
    let n = vec3f(liquid_field(p-x).y-liquid_field(p+x).y,
        liquid_field(p-y).y-liquid_field(p+y).y,liquid_field(p-z).y-liquid_field(p+z).y);
    return normalize(n+vec3f(0.00001));
}

fn water_normal(p: vec3f) -> vec3f {
    let d = 0.055;
    let x = vec3f(d,0.0,0.0);
    let y = vec3f(0.0,d,0.0);
    let z = vec3f(0.0,0.0,d);
    return normalize(vec3f(water_density(p-x)-water_density(p+x),
        water_density(p-y)-water_density(p+y),water_density(p-z)-water_density(p+z))+vec3f(0.000001));
}

struct Intersection {
    distance: f32,
    normal: vec3f,
    local: vec3f,
    kind: f32,
    id: u32,
}

fn mesh_hit(origin:vec3f,ray:vec3f,limit:f32,glass:bool) -> Intersection {
    var hit=Intersection(limit,vec3f(0.0,1.0,0.0),vec3f(0.0),-1.0,NO_OBJECT);
    let inv=safe_inverse(ray);
    var i=0u;
    let count=u32(frame.geometry.x);
    for (var guard=0u; i<count && guard<4096u; guard++) {
        let node=mesh_nodes[i];
        let n=u32(node.high.w);
        let interval=span(origin,inv,node.low.xyz,node.high.xyz);
        if (interval.x>interval.y || interval.y<EPS || interval.x>hit.distance) {
            i=select(i+1u,u32(max(0.0,node.low.w)),n==0u);
            continue;
        }
        if (n>0u) {
            let start=u32(-node.low.w-1.0);
            for (var j=0u;j<n;j++) {
                let triangle=mesh_triangles[start+j];
                let transparent=triangle.b.w>0.5;
                if (transparent && !glass) { continue; }
                let e=triangle.b.xyz-triangle.a.xyz;
                let f=triangle.c.xyz-triangle.a.xyz;
                let p=cross(ray,f);
                let d=dot(e,p);
                if (abs(d)<1e-8) { continue; }
                let delta=origin-triangle.a.xyz;
                let u=dot(delta,p)/d;
                let q=cross(delta,e);
                let v=dot(ray,q)/d;
                let t=dot(f,q)/d;
                if (u<0.0 || v<0.0 || u+v>1.0 || t<=EPS || t>=hit.distance || clipped(origin+ray*t)) { continue; }
                let normal=normalize(cross(e,f));
                hit=Intersection(t,select(normal,-normal,dot(normal,ray)>0.0),
                    vec3f(triangle.a.w,0.0,0.0),select(7.0,4.0,transparent),
                    select(NO_OBJECT,u32(triangle.b.w),transparent));
            }
        }
        i+=1u;
    }
    return hit;
}

fn world_hit(origin: vec3f, ray: vec3f, glass: bool) -> Intersection {
    var hit = Intersection(FAR,vec3f(0.0,1.0,0.0),vec3f(0.0),-1.0,NO_OBJECT);
    if (abs(ray.y)>0.0001) {
        let t = (-0.035-origin.y)/ray.y;
        if (t > EPS && t < FAR) { hit = Intersection(t,vec3f(0.0,1.0,0.0),origin+ray*t,0.0,NO_OBJECT); }
    }
    for (var i = 0u; i < u32(frame.settings.x); i++) {
        let object = frame.objects[i];
        let kind = object.center_kind.w;
        if (kind == 4.0 && !glass) { continue; }
        let q = object.rotation*vec4f(-1.0,-1.0,-1.0,1.0);
        let p = rotate_vector(origin-object.center_kind.xyz,q);
        let d = rotate_vector(ray,q);
        let h = object.half_selected.xyz;
        let interval = span(p,safe_inverse(d),-h,h);
        var t = select(interval.y,interval.x,interval.x>EPS);
        if (clipped(origin+ray*t)) { t=interval.y; }
        if (interval.y < interval.x || t <= EPS || t >= hit.distance) { continue; }
        if (clipped(origin+ray*t)) { continue; }
        let local = p+d*t;
        let face = abs(local)/h;
        var n = vec3f(0.0,0.0,sign(local.z));
        if (face.x>=max(face.y,face.z)) { n = vec3f(sign(local.x),0.0,0.0); }
        else if (face.y>=face.z) { n = vec3f(0.0,sign(local.y),0.0); }
        hit = Intersection(t,rotate_vector(n,object.rotation),local,kind,i);
    }
    if (frame.geometry.x>0.5) {
        let edited=mesh_hit(origin,ray,hit.distance,glass);
        if (edited.kind>3.5) { return edited; }
    }
    return hit;
}

fn environment(direction:vec3f,background:bool,blur:f32)->vec3f {
    if (frame.environment.x<0.5 || (background && frame.environment.w<0.5)) {
        return studio(direction);
    }
    let angle=atan2(direction.z,direction.x)+frame.environment.y;
    let uv=vec2f(fract(angle/6.283185+0.5),acos(clamp(direction.y,-1.0,1.0))/3.141593);
    let level=blur*f32(textureNumLevels(panorama)-1u);
    return textureSampleLevel(panorama,panorama_sampler,uv,level).xyz*exp2(frame.environment.z);
}

fn mapped_cell(p: vec3f) -> vec2i {
    return vec2i(floor((p.xz-MAP_LOW)/MAP_SPAN*vec2f(MAP_SIZE)));
}

fn caustic_index(cell: vec2i, base: bool) -> u32 {
    return u32(cell.x)+u32(cell.y)*MAP_SIZE.x + select(0u,MAP_SIZE.x*MAP_SIZE.y,base);
}

fn caustics(p: vec3f, base: bool) -> f32 {
    if (frame.bounds_max.w < 0.5) { return 0.0; }
    let cell = mapped_cell(p);
    if (any(cell < vec2i(1)) || any(cell >= vec2i(MAP_SIZE)-1)) { return 0.0; }
    let i = caustic_index(cell,base);
    let v = f32(light_map[i])*0.40
        + 0.15*f32(light_map[i-1u]+light_map[i+1u]+light_map[i-MAP_SIZE.x]+light_map[i+MAP_SIZE.x]);
    return min(v/1800.0,1.4);
}

fn attenuation(distance: f32) -> vec3f {
    let pigment = clamp(frame.material.xyz, vec3f(0.04), vec3f(1.0));
    let absorption = -log(pigment)*frame.material.w*0.94;
    return exp(-absorption*distance);
}

fn oil_attenuation(distance: f32) -> vec3f {
    return exp(-vec3f(0.12,0.89,2.9)*distance);
}

fn medium_ior(phase: u32) -> f32 {
    return select(1.0,select(frame.settings.w,1.47,phase==2u),phase!=0u);
}

fn light_through_water(world: vec3f, normal: vec3f) -> vec3f {
    let origin = world+normal*0.018;
    let direction = frame.light.xyz;
    var entrance = 0.0;
    if (water_density(origin)<ISO) {
        entrance = water_hit(origin,direction,6.0,false);
    }
    if (entrance>=FAR) { return vec3f(1.0); }
    let boundary = origin+direction*(entrance+0.008);
    let exit = water_hit(boundary,direction,6.0,true);
    let distance = select(0.08,exit,exit<FAR);
    return attenuation(distance)*0.96;
}

fn receiver(hit: Intersection, world: vec3f, ray: vec3f, lit: bool) -> vec3f {
    if (hit.kind < 0.0) {
        if (frame.geometry.z>0.5) {
            return mix(vec3f(0.036),vec3f(0.113),smoothstep(-0.15,0.55,ray.y));
        }
        return environment(ray,true,0.0)*select(0.53,1.0,frame.environment.x>0.5);
    }
    var base = vec3f(0.17,0.22,0.23);
    if (hit.kind == 0.0) {
        let grid = abs(fract(world.xz*4.0+0.5)-0.5);
        let line = 1.0-smoothstep(0.007,0.028,min(grid.x,grid.y));
        let extent = max(abs(world.x)/2.06,abs(world.z)/1.45);
        let platform = 1.0-smoothstep(0.99,1.01,extent);
        base = vec3f(0.025,0.041,0.054) + platform*(vec3f(0.035,0.049,0.055) + line*vec3f(0.030,0.053,0.065));
        if (frame.geometry.z>0.5) {
            let fade=1.0-smoothstep(4.0,24.0,hit.distance);
            let coarse=abs(fract(world.xz+0.5)-0.5);
            let major=1.0-smoothstep(0.004,0.012,min(coarse.x,coarse.y));
            base=vec3f(0.083)+(line*0.028+major*0.035)*fade;
            base=mix(base,vec3f(0.36,0.074,0.084),(1.0-smoothstep(0.004,0.014,abs(world.z)))*fade);
            base=mix(base,vec3f(0.15,0.26,0.053),(1.0-smoothstep(0.004,0.014,abs(world.x)))*fade);
        }
    }
    if (hit.kind == 2.0) { base = vec3f(0.10,0.37,0.43); }
    if (hit.kind == 3.0) { base = vec3f(0.23,0.29,0.32); }
    if (hit.kind == 6.0) {
        let check = (i32(floor(hit.local.x*5.0)) + i32(floor(hit.local.y*5.0))) & 1;
        base = select(vec3f(0.058,0.079,0.084),vec3f(0.73,0.73,0.64),check==0);
    }
    let n = hit.normal;
    var visible = 1.0;
    if (lit && dot(n,frame.light.xyz) > 0.0) {
        let blocker = world_hit(world+n*0.018,frame.light.xyz,false);
        visible = select(1.0,0.27,blocker.kind >= 0.0 && blocker.distance < 8.0);
    }
    let diffuse = max(0.0,dot(n,frame.light.xyz))*frame.light.w*visible;
    var transmission = vec3f(1.0);
    if (lit && diffuse>0.01 && frame.bounds_min.w>0.5) {
        transmission = light_through_water(world,n);
    }
    var light = vec3f(0.32) + 0.58*diffuse*transmission;
    if (lit && (hit.kind == 0.0 || hit.kind == 3.0) && n.y > 0.35) {
        let c = caustics(world,hit.kind == 3.0);
        light += c*frame.light.w*transmission;
    }
    let edge = pow(1.0-clamp(dot(-ray,n),0.0,1.0),4.0);
    var selection = 0.0;
    if (hit.id < 128u) { selection = frame.objects[hit.id].half_selected.w; }
    if (hit.kind>6.5) { selection = hit.local.x; }
    let specular = pow(max(0.0,dot(reflect(-frame.light.xyz,n),-ray)),19.0) * visible
        * frame.light.w * select(0.065,0.014,hit.kind == 0.0);
    let fill = environment(n,false,0.86);
    let shaded = base*(light + fill*select(0.0,0.30,frame.environment.x>0.5))
        + edge*0.045*vec3f(0.37,0.74,0.83) + selection*0.048*vec3f(1.0,0.43,0.09)
        + specular*vec3f(0.81,0.91,1.0);
    return mix(shaded,environment(ray,true,0.0)*0.72,smoothstep(8.0,23.0,hit.distance));
}

fn object_selection(hit: Intersection) -> f32 {
    if (hit.kind>6.5 || (hit.id>=128u && hit.id<144u)) { return hit.local.x; }
    if (hit.id<128u) { return frame.objects[hit.id].half_selected.w; }
    return 0.0;
}

fn fresnel(cosine: f32, n1: f32, n2: f32) -> f32 {
    let r = (n1-n2)/(n1+n2);
    let f0 = r*r;
    let angle = 1.0-clamp(cosine,0.0,1.0);
    return f0 + (1.0-f0)*angle*angle*angle*angle*angle;
}

fn trace(eye: vec3f, initial: vec3f) -> vec3f {
    let mode = frame.settings.y;
    let detailed = mode == 2.0;
    let solid = mode == 3.0;
    var origin = eye;
    var ray = initial;
    var phase = medium_at(origin);
    var glass_id = NO_OBJECT;
    var weight = vec3f(1.0);
    var color = vec3f(0.0);
    let bounces = select(7u,9u,detailed);
    for (var hop=0u; hop<bounces; hop++) {
        let object = world_hit(origin,ray,!solid);
        var t = FAR;
        if (glass_id == NO_OBJECT) {
            t = medium_hit(origin,ray,object.distance,phase!=0u,
                select(0u,phase,frame.geometry.y>0.5));
        }
        let water = t < object.distance;
        let point = origin + ray*min(t,object.distance);
        if (phase!=0u) {
            let segment = min(4.0,min(t,object.distance));
            weight *= select(attenuation(segment),oil_attenuation(segment),phase==2u);
        }
        if (!water && object.kind != 4.0) {
            return color + weight*receiver(object,point,ray,detailed);
        }
        var normal = object.normal;
        var next_phase = medium_at(point+ray*0.018);
        if (water) {
            if (next_phase==phase) {
                next_phase = select(0u,select(1u,2u,liquid_field(point).y>=0.5 && frame.geometry.y>0.5),phase==0u);
            }
            normal = water_normal(point);
            if (phase!=0u && next_phase!=0u) { normal = interface_normal(point); }
        }
        if (dot(normal,ray)>0.0) { normal = -normal; }
        if (solid) {
            let shade = 0.28+0.72*max(0.0,dot(normal,frame.light.xyz));
            return mix(vec3f(0.035,0.20,0.24),select(vec3f(0.21,0.76,0.74),vec3f(0.96,0.62,0.16),max(phase,next_phase)==2u),shade);
        }
        let n1 = select(medium_ior(phase),1.52,glass_id!=NO_OBJECT);
        var next_glass = glass_id;
        if (!water) {
            let exit = glass_id == object.id;
            next_glass = select(object.id,NO_OBJECT,exit);
            if (!exit) { next_phase = 0u; }
        }
        let n2 = select(medium_ior(next_phase),1.52,next_glass!=NO_OBJECT);
        let transmitted = refract(ray,normal,n1/n2);
        let reflected = reflect(ray,normal);
        if (dot(transmitted,transmitted)<0.000001) {
            ray = reflected;
            origin = point + normal*0.008 + ray*0.002;
            continue;
        }
        let f = fresnel(dot(-ray,normal),n1,n2);
        var reflection = environment(reflected,false,0.014);
        if (detailed && hop < 2u && f > 0.012) {
            let ro = point+reflected*0.012;
            let rh = world_hit(ro,reflected,false);
            if (rh.kind >= 0.0) { reflection = receiver(rh,ro+reflected*rh.distance,reflected,false); }
        }
        if (water && hop < 2u) {
            let glint = pow(max(0.0,dot(reflected,frame.light.xyz)),256.0);
            reflection += vec3f(1.0,0.96,0.88)*glint*frame.light.w*0.75;
        }
        if (water && phase==0u && frame.size_radius.w>0.5) {
            let coverage=foam_coverage(point);
            if (coverage>0.006) {
                let grain=0.72+0.28*sin(point.x*453.0+sin(point.z*201.0))*sin(point.z*461.0+point.y*157.0);
                let foam=0.48*smoothstep(0.017,0.53,coverage*grain)*smoothstep(-0.24,0.76,normal.y);
                let skylight=environment(normal,false,0.78);
                let diffuse=0.18+0.32*max(0.0,dot(normal,frame.light.xyz))*frame.light.w;
                color+=weight*foam*(vec3f(0.77,0.83,0.85)*diffuse+min(skylight,vec3f(1.0))*0.24);
                weight*=1.0-foam;
            }
        }
        color += weight*reflection*f;
        weight *= 1.0-f;
        if (!water) { weight *= vec3f(0.996,0.999,0.998); }
        ray = normalize(transmitted);
        origin = point-normal*0.005+ray*0.003;
        phase = next_phase;
        glass_id = next_glass;
    }
    return color + weight*environment(ray,false,0.1)*0.32;
}

@fragment fn optical_view(input: Screen) -> @location(0) vec4f {
    let origin = ray_origin(input.clip.xy);
    let ray = ray_at(input.clip.xy);
    var color: vec3f;
    if (frame.settings.y == 1.0) {
        let hit = world_hit(origin,ray,false);
        let raw = textureLoad(particle_image,vec2i(input.clip.xy),0);
        let distance = hit.distance * dot(ray,frame.forward.xyz);
        color = receiver(hit,origin+ray*hit.distance,ray,false);
        if (raw.x > 0.0 && raw.x < distance) {
            color = mix(select(vec3f(0.08,0.65,0.74),vec3f(0.95,0.60,0.12),raw.y<0.0),
                vec3f(1.0,0.64,0.18),smoothstep(0.0,4.2,abs(raw.y)))*(0.32+0.66*raw.z);
        }
    } else {
        color = trace(origin,ray);
        if (frame.size_radius.w>0.5) {
            let raw=textureLoad(particle_image,vec2i(input.clip.xy),0);
            if (raw.x>0.0) {
                let t=raw.x/max(0.001,dot(ray,frame.forward.xyz));
                let hit=world_hit(origin,ray,false);
                if (t<hit.distance+0.01) {
                    let phase=floor(raw.y);
                    let fade=clamp(fract(raw.y)/0.75,0.0,1.0);
                    let point=origin+ray*t;
                    let bubble=phase==3.0;
                    let depth=water_density(point);
                    let rim=pow(1.0-raw.z,2.0);
                    let light=environment(normalize(-ray+vec3f(0.0,0.65,0.0)),false,0.11);
                    if (bubble && depth>ISO) {
                        color=mix(color*mix(1.0,0.83,rim*fade),light*0.65,0.17*rim*fade);
                    } else if (!bubble && depth<ISO) {
                        let glint=0.12+0.42*rim;
                        color=mix(color,light,glint*fade);
                    }
                }
            }
        }
    }
    let mapped = max(color,vec3f(0.0))/(1.0+max(color,vec3f(0.0)));
    var selected=0.0;
    if (frame.geometry.w>0.5) { selected=object_selection(world_hit(origin,ray,frame.settings.y!=3.0)); }
    return vec4f(pow(mapped,vec3f(1.0/2.2)),selected);
}

fn luma(rgb: vec3f) -> f32 {
    return dot(rgb,vec3f(0.299,0.587,0.114));
}

fn screen_sample(uv: vec2f) -> vec3f {
    return textureSampleLevel(optical_image,field_sampler,uv,0.0).xyz;
}

@fragment fn resolve_view(input: Screen) -> @location(0) vec4f {
    let pixel = 1.0/frame.size_radius.xy;
    let uv = input.clip.xy*pixel;
    let center = screen_sample(uv);
    if (frame.geometry.w>0.5) {
        let pos=vec2i(input.clip.xy);
        let size=vec2i(textureDimensions(optical_image));
        let selected=textureLoad(optical_image,pos,0).a;
        let left=textureLoad(optical_image,clamp(pos+vec2i(-1,0),vec2i(0),size-1),0).a;
        let right=textureLoad(optical_image,clamp(pos+vec2i(1,0),vec2i(0),size-1),0).a;
        let above=textureLoad(optical_image,clamp(pos+vec2i(0,-1),vec2i(0),size-1),0).a;
        let below=textureLoad(optical_image,clamp(pos+vec2i(0,1),vec2i(0),size-1),0).a;
        if (selected<0.5 && max(max(left,right),max(above,below))>0.5) {
            return vec4f(1.0,0.53,0.12,1.0);
        }
    }
    let nw = luma(screen_sample(uv + pixel*vec2f(-1.0,-1.0)));
    let ne = luma(screen_sample(uv + pixel*vec2f(1.0,-1.0)));
    let sw = luma(screen_sample(uv + pixel*vec2f(-1.0,1.0)));
    let se = luma(screen_sample(uv + pixel*vec2f(1.0,1.0)));
    let min_luma = min(luma(center),min(min(nw,ne),min(sw,se)));
    let max_luma = max(luma(center),max(max(nw,ne),max(sw,se)));
    if (max_luma-min_luma < max(0.025,max_luma*0.090)) { return vec4f(center,1.0); }
    let direction = vec2f(sw+se-nw-ne,nw+sw-ne-se);
    let reduce = max((nw+ne+sw+se)*0.020,0.0078);
    let step = clamp(direction/(min(abs(direction.x),abs(direction.y))+reduce),vec2f(-5.0),vec2f(5.0))*pixel;
    let a = (screen_sample(uv+step/6.0)+screen_sample(uv-step/6.0))*0.5;
    let b = a*0.5+(screen_sample(uv+step*0.5)+screen_sample(uv-step*0.5))*0.25;
    return vec4f(select(b,a,luma(b)<min_luma || luma(b)>max_luma),1.0);
}

// Forward light rays deposit their energy on the floor or a tank bottom.
// A refracting ripple gathers more rays in some cells and fewer in others.
@compute @workgroup_size(8,8)
fn focus_light(@builtin(global_invocation_id) id: vec3u) {
    let light = frame.light.xyz;
    let jitter = vec2f(fract(sin(dot(vec2f(id.xy),vec2f(47.2,91.7)))*23781.6),
        fract(sin(dot(vec2f(id.xy),vec2f(67.1,29.2)))*51872.7));
    let uv = (vec2f(id.xy)+jitter)/vec2f(MAP_SIZE*2u);
    let plane = MAP_LOW+uv*MAP_SPAN;
    let ray = -light;
    let origin = vec3f(plane.x,0.0,plane.y) + light*(3.08/light.y);
    let first = world_hit(origin,ray,false);
    let t = water_hit(origin,ray,first.distance,false);
    if (t >= first.distance) { return; }
    let point = origin+ray*t;
    let n = water_normal(point);
    let bent = refract(ray,n,1.0/frame.settings.w);
    if (bent.y>=-0.005) { return; }
    let start = point+bent*0.009;
    var hit = world_hit(start,bent,false);
    let exit = water_hit(start,bent,hit.distance,true);
    var destination = start+bent*hit.distance;
    if (exit < hit.distance) {
        let e = start+bent*exit;
        let out_ray = refract(bent,-water_normal(e),frame.settings.w);
        if (out_ray.y>=-0.005) { return; }
        let next = e+out_ray*0.009;
        hit = world_hit(next,out_ray,false);
        destination = next+out_ray*hit.distance;
    }
    if ((hit.kind != 0.0 && hit.kind != 3.0) || hit.normal.y<0.40) { return; }
    let cell = mapped_cell(destination);
    if (any(cell<vec2i(0)) || any(cell>=vec2i(MAP_SIZE))) { return; }
    let energy = (1.0-fresnel(dot(-ray,n),1.0,frame.settings.w))*256.0;
    atomicAdd(&light_deposit[caustic_index(cell,hit.kind==3.0)],u32(energy));
}
