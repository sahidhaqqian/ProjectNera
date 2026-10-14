//! Build one playable VRMA from trimmed source clips. Sources remain unchanged.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use bevy::math::Quat;
use serde_json::{Value, json};

const SAMPLE_RATE: f32 = 30.0;
const MAX_OUTPUT_SAMPLES: usize = 120_000;
const MAX_OUTPUT_FLOATS: usize = 12_000_000;
const MAX_DURATION: f32 = 3600.0;

#[derive(Clone, Debug)]
pub(super) struct Segment {
    pub key: String,
    pub start: f32,
    pub end: Option<f32>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Target {
    role: String,
    path: String,
}

#[derive(Debug)]
struct Curve {
    times: Vec<f32>,
    values: Vec<Vec<f32>>,
    interpolation: Interpolation,
}

#[derive(Clone, Copy, Debug)]
enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

struct Source {
    json: Value,
    bin: Vec<u8>,
    roles: BTreeMap<String, usize>,
    curves: BTreeMap<Target, Curve>,
    duration: f32,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn input_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Length in seconds of the animation that Nera will play (animations[0]).
pub(super) fn duration(path: &Path) -> io::Result<f32> {
    Ok(read_source(path)?.duration)
}

/// Join trimmed source clips in order and write one new file in Combined.
pub(super) fn combine(root: &Path, segments: &[Segment], output_name: &str) -> io::Result<PathBuf> {
    if segments.len() < 2 {
        return Err(input_error("Choose at least two animation segments"));
    }
    let output_name = super::registry::validated_vrma_name(output_name)?;
    let canonical_root = root.canonicalize()?;
    let mut sources = Vec::with_capacity(segments.len());
    let mut ranges = Vec::with_capacity(segments.len());
    let mut total_duration = 0.0_f32;
    for segment in segments {
        let relative = Path::new(&segment.key);
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
            || relative
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("vrma"))
        {
            return Err(input_error("Invalid animation path"));
        }
        let path = root.join(relative).canonicalize()?;
        if !path.starts_with(&canonical_root) {
            return Err(input_error("Animation must be inside the library"));
        }
        let source = read_source(&path)?;
        let end = segment.end.unwrap_or(source.duration);
        if !segment.start.is_finite()
            || !end.is_finite()
            || segment.start < 0.0
            || end <= segment.start
            || end > source.duration + 0.0001
        {
            return Err(input_error(format!(
                "Invalid trim range for {}",
                segment.key
            )));
        }
        total_duration += end - segment.start;
        if total_duration > MAX_DURATION {
            return Err(input_error("Combined animation cannot exceed one hour"));
        }
        sources.push(source);
        ranges.push((segment.start, end));
    }
    if !total_duration.is_finite() || total_duration <= 0.0 {
        return Err(input_error("Invalid combined duration"));
    }

    let mut combined = build_combined(&sources, &ranges)?;
    combined
        .json
        .get_mut("asset")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid("Missing glTF asset"))?
        .insert("generator".into(), json!("Nera Animation Manager"));
    let bytes = write_glb(&combined.json, &combined.bin)?;

    let directory = root.join("Combined");
    fs::create_dir_all(&directory)?;
    if !directory.canonicalize()?.starts_with(&canonical_root) {
        return Err(input_error(
            "Combined folder must be inside the animation library",
        ));
    }
    if fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&output_name)
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "A combined animation with that name already exists",
        ));
    }
    let destination = directory.join(output_name);
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".nera-combine-{}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    // Publishing a link is atomic and refuses an existing destination, including
    // a file created after our initial name check. The temporary is on this volume.
    let published = fs::hard_link(&temporary, &destination);
    let _ = fs::remove_file(&temporary);
    published?;
    Ok(destination)
}

fn read_source(path: &Path) -> io::Result<Source> {
    let (json, bin) = read_glb(&fs::read(path)?)?;
    let buffers = array(&json, "buffers")?;
    if buffers.len() != 1 || buffers[0].get("uri").is_some() {
        return Err(invalid("VRMA must contain one embedded binary buffer"));
    }
    let used = integer(&buffers[0], "byteLength")?;
    if used > bin.len() {
        return Err(invalid("VRMA binary buffer is truncated"));
    }
    let roles = role_nodes(&json)?;
    let animation = array(&json, "animations")?
        .first()
        .ok_or_else(|| invalid("VRMA has no animation"))?;
    let samplers = array(animation, "samplers")?;
    let channels = array(animation, "channels")?;
    let inverse_roles: BTreeMap<usize, String> = roles
        .iter()
        .map(|(role, node)| (*node, role.clone()))
        .collect();
    let mut curves = BTreeMap::new();
    let mut duration = 0.0_f32;
    for channel in channels {
        let target = channel
            .get("target")
            .ok_or_else(|| invalid("Animation channel has no target"))?;
        let node = integer(target, "node")?;
        // The VRMA loader ignores nodes without a humanoid/expression/gaze role.
        // Some exporters include unmapped finger tracks; ignore those likewise.
        let Some(role) = inverse_roles.get(&node).cloned() else {
            continue;
        };
        let path = string(target, "path")?.to_owned();
        if !matches!(path.as_str(), "translation" | "rotation" | "scale") {
            return Err(invalid(format!("Unsupported animation target: {path}")));
        }
        let sampler_index = integer(channel, "sampler")?;
        let sampler = samplers
            .get(sampler_index)
            .ok_or_else(|| invalid("Animation sampler index is invalid"))?;
        let times = read_accessor(&json, &bin, integer(sampler, "input")?)?;
        let values = read_accessor(&json, &bin, integer(sampler, "output")?)?;
        if times.is_empty() || times.iter().any(|v| v.len() != 1) {
            return Err(invalid("Animation input must contain scalar times"));
        }
        let times: Vec<f32> = times.into_iter().map(|v| v[0]).collect();
        if times.iter().any(|t| !t.is_finite() || *t < 0.0)
            || times.windows(2).any(|pair| pair[1] <= pair[0])
        {
            return Err(invalid("Animation times must increase from zero"));
        }
        let expected_width = if path == "rotation" { 4 } else { 3 };
        if values.iter().any(|value| value.len() != expected_width) {
            return Err(invalid("Animation output has the wrong vector width"));
        }
        let interpolation = match sampler
            .get("interpolation")
            .and_then(Value::as_str)
            .unwrap_or("LINEAR")
        {
            "LINEAR" => Interpolation::Linear,
            "STEP" => Interpolation::Step,
            "CUBICSPLINE" => Interpolation::CubicSpline,
            other => return Err(invalid(format!("Unsupported interpolation: {other}"))),
        };
        let factor = if matches!(interpolation, Interpolation::CubicSpline) {
            3
        } else {
            1
        };
        if values.len() != times.len() * factor {
            return Err(invalid("Animation keyframe count is inconsistent"));
        }
        duration = duration.max(*times.last().unwrap());
        let target = Target { role, path };
        if curves
            .insert(
                target,
                Curve {
                    times,
                    values,
                    interpolation,
                },
            )
            .is_some()
        {
            return Err(invalid("Duplicate animation channel target"));
        }
    }
    if curves.is_empty() || duration <= 0.0 {
        return Err(invalid("VRMA animation has no playable keyframes"));
    }
    Ok(Source {
        json,
        bin,
        roles,
        curves,
        duration,
    })
}

fn role_nodes(json: &Value) -> io::Result<BTreeMap<String, usize>> {
    let extension = json
        .pointer("/extensions/VRMC_vrm_animation")
        .ok_or_else(|| invalid("Missing VRMC_vrm_animation extension"))?;
    if extension
        .get("specVersion")
        .and_then(Value::as_str)
        .is_some_and(|version| version != "1.0")
    {
        return Err(invalid("Unsupported VRMA spec version"));
    }
    let mut roles = BTreeMap::new();
    if let Some(bones) = extension
        .pointer("/humanoid/humanBones")
        .and_then(Value::as_object)
    {
        for (name, value) in bones {
            roles.insert(format!("bone:{name}"), integer(value, "node")?);
        }
    }
    if let Some(expressions) = extension.get("expressions") {
        for kind in ["preset", "custom"] {
            if let Some(map) = expressions.get(kind).and_then(Value::as_object) {
                for (name, value) in map {
                    roles.insert(format!("expression:{kind}:{name}"), integer(value, "node")?);
                }
            }
        }
    }
    if let Some(look_at) = extension.pointer("/lookAt/node").and_then(Value::as_u64) {
        roles.insert("lookAt".into(), look_at as usize);
    }
    let nodes = array(json, "nodes")?;
    if roles.values().any(|index| *index >= nodes.len()) {
        return Err(invalid("VRMA role points to an invalid node"));
    }
    if roles.is_empty() {
        return Err(invalid("VRMA has no humanoid, expression, or gaze mapping"));
    }
    Ok(roles)
}

fn array<'a>(value: &'a Value, property: &str) -> io::Result<&'a Vec<Value>> {
    value
        .get(property)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("Missing or invalid {property}")))
}

fn integer(value: &Value, property: &str) -> io::Result<usize> {
    value
        .get(property)
        .and_then(Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| invalid(format!("Missing or invalid {property}")))
}

fn string<'a>(value: &'a Value, property: &str) -> io::Result<&'a str> {
    value
        .get(property)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("Missing or invalid {property}")))
}

fn read_accessor(json: &Value, bin: &[u8], index: usize) -> io::Result<Vec<Vec<f32>>> {
    let accessor = array(json, "accessors")?
        .get(index)
        .ok_or_else(|| invalid("Invalid accessor index"))?;
    if integer(accessor, "componentType")? != 5126 || accessor.get("sparse").is_some() {
        return Err(invalid(
            "Combine supports float animation accessors without sparse storage",
        ));
    }
    let width = match string(accessor, "type")? {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        _ => return Err(invalid("Unsupported animation accessor type")),
    };
    let count = integer(accessor, "count")?;
    let view = array(json, "bufferViews")?
        .get(integer(accessor, "bufferView")?)
        .ok_or_else(|| invalid("Invalid buffer view"))?;
    if integer(view, "buffer")? != 0 {
        return Err(invalid("External buffers are unsupported"));
    }
    let view_start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let view_length = integer(view, "byteLength")?;
    let start = accessor
        .get("byteOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let stride = view
        .get("byteStride")
        .and_then(Value::as_u64)
        .unwrap_or((width * 4) as u64) as usize;
    if count == 0 || stride < width * 4 || stride % 4 != 0 {
        return Err(invalid("Invalid animation accessor stride or count"));
    }
    let end = count
        .checked_sub(1)
        .and_then(|n| n.checked_mul(stride))
        .and_then(|n| n.checked_add(start))
        .and_then(|n| n.checked_add(width * 4))
        .ok_or_else(|| invalid("Animation accessor overflow"))?;
    if end > view_length
        || view_start
            .checked_add(view_length)
            .is_none_or(|end| end > bin.len())
    {
        return Err(invalid("Animation accessor is outside its binary buffer"));
    }
    let bytes = &bin[view_start..view_start + view_length];
    let mut result = Vec::with_capacity(count);
    for frame in 0..count {
        let offset = start + frame * stride;
        let mut value = Vec::with_capacity(width);
        for component in 0..width {
            let index = offset + component * 4;
            let component = f32::from_le_bytes(bytes[index..index + 4].try_into().unwrap());
            if !component.is_finite() {
                return Err(invalid("Non-finite animation sample"));
            }
            value.push(component);
        }
        result.push(value);
    }
    Ok(result)
}

fn append_accessor(
    json: &mut Value,
    bin: &mut Vec<u8>,
    values: &[f32],
    width: usize,
    times: bool,
) -> io::Result<usize> {
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let offset = bin.len();
    for value in values {
        if !value.is_finite() {
            return Err(invalid("Non-finite combined sample"));
        }
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let views = json["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| invalid("Invalid buffer views"))?;
    let view = views.len();
    views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": values.len() * 4}));
    let mut accessor = json!({
        "bufferView": view,
        "componentType": 5126,
        "count": values.len() / width,
        "type": match width { 1 => "SCALAR", 3 => "VEC3", 4 => "VEC4", _ => return Err(invalid("Unsupported output width")) }
    });
    if times {
        accessor["min"] = json!([values[0]]);
        accessor["max"] = json!([values[values.len() - 1]]);
    }
    let accessors = json["accessors"]
        .as_array_mut()
        .ok_or_else(|| invalid("Invalid accessors"))?;
    let index = accessors.len();
    accessors.push(accessor);
    Ok(index)
}

fn read_glb(bytes: &[u8]) -> io::Result<(Value, Vec<u8>)> {
    let word = |offset: usize| -> io::Result<u32> {
        let bytes = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("Truncated GLB header"))?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    };
    if word(0)? != 0x4654_6c67 || word(4)? != 2 || word(8)? as usize != bytes.len() {
        return Err(invalid("VRMA must be a complete glTF 2 binary file"));
    }
    let mut offset = 12usize;
    let mut json = None;
    let mut bin = None;
    while offset < bytes.len() {
        let length = word(offset)? as usize;
        let kind = word(offset + 4)?;
        let end = offset
            .checked_add(8)
            .and_then(|v| v.checked_add(length))
            .ok_or_else(|| invalid("GLB chunk length overflow"))?;
        let chunk = bytes
            .get(offset + 8..end)
            .ok_or_else(|| invalid("Truncated GLB chunk"))?;
        match kind {
            0x4e4f_534a if json.is_none() => {
                json = Some(
                    serde_json::from_slice(chunk).map_err(|error| invalid(error.to_string()))?,
                );
            }
            0x004e_4942 if bin.is_none() => bin = Some(chunk.to_vec()),
            _ => {}
        }
        offset = end;
    }
    Ok((
        json.ok_or_else(|| invalid("Missing GLB JSON chunk"))?,
        bin.ok_or_else(|| invalid("Missing GLB binary chunk"))?,
    ))
}

fn write_glb(json: &Value, bin: &[u8]) -> io::Result<Vec<u8>> {
    let mut json = serde_json::to_vec(json).map_err(|error| invalid(error.to_string()))?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let binary_length = bin
        .len()
        .checked_add(3)
        .map(|n| n & !3)
        .ok_or_else(|| invalid("Combined binary is too large"))?;
    let total_length = 28usize
        .checked_add(json.len())
        .and_then(|n| n.checked_add(binary_length))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| invalid("Combined VRMA is too large"))?;
    let mut output = Vec::with_capacity(total_length as usize);
    for word in [0x4654_6c67, 2, total_length, json.len() as u32, 0x4e4f_534a] {
        output.extend_from_slice(&word.to_le_bytes());
    }
    output.extend_from_slice(&json);
    output.extend_from_slice(&(binary_length as u32).to_le_bytes());
    output.extend_from_slice(&0x004e_4942u32.to_le_bytes());
    output.extend_from_slice(bin);
    output.resize(total_length as usize, 0);
    Ok(output)
}

#[derive(Clone, Copy)]
struct Rest {
    translation: bevy::math::Vec3,
    rotation: Quat,
    scale: bevy::math::Vec3,
    global: bevy::math::Mat4,
}

fn rest_transforms(json: &Value) -> io::Result<Vec<Rest>> {
    use bevy::math::{Mat4, Vec3};
    let nodes = array(json, "nodes")?;
    let mut parents = vec![None; nodes.len()];
    for (parent, node) in nodes.iter().enumerate() {
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                let child = child
                    .as_u64()
                    .ok_or_else(|| invalid("Invalid child index"))?
                    as usize;
                if child >= nodes.len() || parents[child].replace(parent).is_some() {
                    return Err(invalid("Invalid VRMA node hierarchy"));
                }
            }
        }
    }
    let mut rests = Vec::with_capacity(nodes.len());
    for node in nodes {
        let (scale, rotation, translation) = if let Some(matrix) = node.get("matrix") {
            let values = number_array(matrix, 16)?;
            Mat4::from_cols_array(values.as_slice().try_into().unwrap())
                .to_scale_rotation_translation()
        } else {
            let translation = Vec3::from_array(
                node_vector(node, "translation", &[0., 0., 0.])?
                    .try_into()
                    .unwrap(),
            );
            let rotation = quaternion(&node_vector(node, "rotation", &[0., 0., 0., 1.])?)?;
            let scale = Vec3::from_array(
                node_vector(node, "scale", &[1., 1., 1.])?
                    .try_into()
                    .unwrap(),
            );
            (scale, rotation, translation)
        };
        if !scale.is_finite()
            || !rotation.is_finite()
            || !translation.is_finite()
            || scale.abs().min_element() <= f32::EPSILON
        {
            return Err(invalid("Invalid VRMA rest transform"));
        }
        rests.push(Rest {
            translation,
            rotation: rotation.normalize(),
            scale,
            global: Mat4::IDENTITY,
        });
    }
    fn visit(
        index: usize,
        parents: &[Option<usize>],
        marks: &mut [u8],
        rests: &mut [Rest],
    ) -> io::Result<()> {
        if marks[index] == 2 {
            return Ok(());
        }
        if marks[index] == 1 {
            return Err(invalid("Cyclic VRMA node hierarchy"));
        }
        marks[index] = 1;
        let parent_global = if let Some(parent) = parents[index] {
            visit(parent, parents, marks, rests)?;
            rests[parent].global
        } else {
            Mat4::IDENTITY
        };
        let rest = &mut rests[index];
        rest.global = parent_global
            * Mat4::from_scale_rotation_translation(rest.scale, rest.rotation, rest.translation);
        marks[index] = 2;
        Ok(())
    }
    let mut marks = vec![0; nodes.len()];
    for index in 0..nodes.len() {
        visit(index, &parents, &mut marks, &mut rests)?;
    }
    Ok(rests)
}

fn node_vector(node: &Value, path: &str, default: &[f32]) -> io::Result<Vec<f32>> {
    node.get(path).map_or_else(
        || Ok(default.to_vec()),
        |value| number_array(value, default.len()),
    )
}

fn number_array(value: &Value, width: usize) -> io::Result<Vec<f32>> {
    let array = value
        .as_array()
        .ok_or_else(|| invalid("Expected numeric vector"))?;
    if array.len() != width {
        return Err(invalid("Wrong numeric vector width"));
    }
    array
        .iter()
        .map(|number| {
            let value = number
                .as_f64()
                .ok_or_else(|| invalid("Invalid numeric vector"))? as f32;
            if !value.is_finite() {
                return Err(invalid("Non-finite animation value"));
            }
            Ok(value)
        })
        .collect()
}

fn quaternion(values: &[f32]) -> io::Result<Quat> {
    let quat = Quat::from_array(
        values
            .try_into()
            .map_err(|_| invalid("Invalid quaternion width"))?,
    );
    if !quat.is_finite() || quat.length_squared() < 1.0e-12 {
        return Err(invalid("Invalid animation quaternion"));
    }
    Ok(quat.normalize())
}

fn sample(curve: &Curve, time: f32, rotation: bool) -> io::Result<Vec<f32>> {
    let cubic = matches!(curve.interpolation, Interpolation::CubicSpline);
    let value_index = |index| if cubic { index * 3 + 1 } else { index };
    let upper = curve.times.partition_point(|key| *key <= time);
    let value = if upper == 0 {
        curve.values[value_index(0)].clone()
    } else if upper >= curve.times.len() {
        curve.values[value_index(curve.times.len() - 1)].clone()
    } else {
        let lower = upper - 1;
        let a = &curve.values[value_index(lower)];
        let b = &curve.values[value_index(upper)];
        let dt = curve.times[upper] - curve.times[lower];
        let t = (time - curve.times[lower]) / dt;
        match curve.interpolation {
            Interpolation::Step => a.clone(),
            Interpolation::Linear if rotation => {
                return Ok(quaternion(a)?
                    .slerp(quaternion(b)?, t)
                    .normalize()
                    .to_array()
                    .to_vec());
            }
            Interpolation::Linear => a.iter().zip(b).map(|(a, b)| a + (b - a) * t).collect(),
            Interpolation::CubicSpline => {
                let outgoing = &curve.values[lower * 3 + 2];
                let incoming = &curve.values[upper * 3];
                let t2 = t * t;
                let t3 = t2 * t;
                (0..a.len())
                    .map(|i| {
                        (2. * t3 - 3. * t2 + 1.) * a[i]
                            + (t3 - 2. * t2 + t) * dt * outgoing[i]
                            + (-2. * t3 + 3. * t2) * b[i]
                            + (t3 - t2) * dt * incoming[i]
                    })
                    .collect()
            }
        }
    };
    if rotation {
        Ok(quaternion(&value)?.to_array().to_vec())
    } else {
        Ok(value)
    }
}

fn retarget(
    value: &[f32],
    target: &Target,
    source: Rest,
    destination: Rest,
) -> io::Result<Vec<f32>> {
    use bevy::math::Vec3;
    if !target.role.starts_with("bone:") {
        return Ok(value.to_vec());
    }
    match target.path.as_str() {
        "rotation" => {
            // Match bevy_vrm1's source -> normalized -> destination conversion.
            let (_, src_global, _) = source.global.to_scale_rotation_translation();
            let (_, dst_global, _) = destination.global.to_scale_rotation_translation();
            let normalized =
                src_global * source.rotation.inverse() * quaternion(value)? * src_global.inverse();
            Ok(
                (destination.rotation * dst_global.inverse() * normalized * dst_global)
                    .normalize()
                    .to_array()
                    .to_vec(),
            )
        }
        "translation" if target.role == "bone:hips" => {
            let src_height = source.global.w_axis.y;
            let dst_height = destination.global.w_axis.y;
            let scaling = if src_height.abs() < f32::EPSILON {
                1.
            } else {
                dst_height / src_height
            };
            let pose = Vec3::from_array(value.try_into().unwrap());
            Ok(
                (destination.translation + (pose - source.translation) * scaling)
                    .to_array()
                    .to_vec(),
            )
        }
        "translation" => Err(invalid(
            "Only hips translation is supported for humanoid combine",
        )),
        "scale" => Err(invalid(
            "Humanoid scale animation cannot be safely retargeted",
        )),
        _ => Err(invalid("Unsupported animation target")),
    }
}

fn resting_value(rest: Rest, path: &str) -> Vec<f32> {
    match path {
        "rotation" => rest.rotation.to_array().to_vec(),
        "translation" => rest.translation.to_array().to_vec(),
        _ => rest.scale.to_array().to_vec(),
    }
}

fn build_combined(sources: &[Source], ranges: &[(f32, f32)]) -> io::Result<Source> {
    // The richest skeleton avoids dropping optional bones present in only one
    // clip. A different node order is harmless: all channels use VRMA roles.
    let template = sources
        .iter()
        .max_by_key(|source| {
            source
                .roles
                .keys()
                .filter(|role| role.starts_with("bone:"))
                .count()
        })
        .ok_or_else(|| input_error("No source clips"))?;
    let mut targets: BTreeSet<Target> = sources
        .iter()
        .flat_map(|source| source.curves.keys().cloned())
        .collect();
    let width_total: usize = targets
        .iter()
        .map(|target| if target.path == "rotation" { 4 } else { 3 })
        .sum();
    let estimated_samples: usize = ranges
        .iter()
        .map(|(start, end)| ((end - start) * SAMPLE_RATE).ceil() as usize + 2)
        .sum();
    if estimated_samples
        .checked_mul(width_total)
        .is_none_or(|count| count > MAX_OUTPUT_FLOATS)
    {
        return Err(input_error(
            "Combined animation is too large; use shorter segments",
        ));
    }
    let mut json = template.json.clone();
    let mut roles = template.roles.clone();
    for target in &targets {
        if roles.contains_key(&target.role) {
            continue;
        }
        if target.role.starts_with("bone:") {
            return Err(invalid(format!(
                "These clips have incompatible skeletons: {} is absent from the output rig",
                target.role.trim_start_matches("bone:")
            )));
        }
        let source = sources
            .iter()
            .find(|source| source.roles.contains_key(&target.role))
            .unwrap();
        let node = &source.json["nodes"][source.roles[&target.role]];
        let mut copied = json!({"name": target.role});
        for property in ["rotation", "translation", "scale"] {
            if let Some(value) = node.get(property) {
                copied[property] = value.clone();
            }
        }
        let nodes = json["nodes"]
            .as_array_mut()
            .ok_or_else(|| invalid("Invalid nodes"))?;
        let index = nodes.len();
        nodes.push(copied);
        let scene_index = json.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
        json["scenes"][scene_index]["nodes"]
            .as_array_mut()
            .ok_or_else(|| invalid("Invalid VRMA scene"))?
            .push(json!(index));
        let extension = &mut json["extensions"]["VRMC_vrm_animation"];
        if target.role == "lookAt" {
            extension["lookAt"] = source.json["extensions"]["VRMC_vrm_animation"]["lookAt"].clone();
            extension["lookAt"]["node"] = json!(index);
        } else {
            let parts: Vec<_> = target.role.splitn(3, ':').collect();
            extension["expressions"][parts[1]][parts[2]] = json!({"node": index});
        }
        roles.insert(target.role.clone(), index);
    }
    let extension = json["extensions"]["VRMC_vrm_animation"]
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid VRMA extension"))?;
    extension.insert("specVersion".into(), json!("1.0"));
    extension.remove("meta"); // Source duration/frame count no longer describes this clip.
    let destination_rest = rest_transforms(&json)?;
    let source_rests: Vec<_> = sources
        .iter()
        .map(|source| rest_transforms(&source.json))
        .collect::<io::Result<_>>()?;
    // Exporters often write constant rest translation/scale channels. They
    // carry no motion and need no retargeting; dynamic ones remain rejected.
    targets.retain(|target| {
        let rest_only = target.role.starts_with("bone:")
            && (target.path == "scale"
                || (target.path == "translation" && target.role != "bone:hips"))
            && sources.iter().enumerate().all(|(index, source)| {
                let Some(curve) = source.curves.get(target) else {
                    return true;
                };
                let Some(&node) = source.roles.get(&target.role) else {
                    return false;
                };
                let rest = resting_value(source_rests[index][node], &target.path);
                curve.values.iter().enumerate().all(|(index, value)| {
                    let tangent =
                        matches!(curve.interpolation, Interpolation::CubicSpline) && index % 3 != 1;
                    value.iter().zip(&rest).all(|(value, rest)| {
                        (*value - if tangent { 0.0 } else { *rest }).abs() < 1e-6
                    })
                })
            });
        !rest_only
    });
    let mut times = Vec::new();
    let mut samples = Vec::new();
    let mut offset = 0.0_f32;
    for (source_index, (source, &(start, end))) in sources.iter().zip(ranges).enumerate() {
        let length = end - start;
        let frames = (length * SAMPLE_RATE).ceil() as usize;
        if frames > MAX_OUTPUT_SAMPLES
            || samples.len().saturating_add(frames + 2) > MAX_OUTPUT_SAMPLES
        {
            return Err(input_error(
                "Combined animation is too long (maximum about one hour)",
            ));
        }
        let mut local_times: BTreeSet<u32> = (0..frames)
            .map(|frame| (start + frame as f32 / SAMPLE_RATE).to_bits())
            .collect();
        // Preserve existing keyframe locations and STEP discontinuities.
        for curve in source.curves.values() {
            for &time in &curve.times {
                if time > start && time < end {
                    local_times.insert(time.to_bits());
                    if matches!(curve.interpolation, Interpolation::Step) {
                        local_times.insert(f32::from_bits(time.to_bits() - 1).max(start).to_bits());
                    }
                    if local_times.len() > MAX_OUTPUT_SAMPLES {
                        return Err(input_error("Combined animation has too many keyframes"));
                    }
                }
            }
        }
        local_times.insert(end.to_bits());
        for source_bits in local_times {
            let source_time = f32::from_bits(source_bits);
            let mut time = offset + (source_time - start);
            if source_time == end && source_index + 1 < sources.len() {
                // A cut retains both boundary poses without duplicate timestamps.
                time = f32::from_bits((offset + length).to_bits() - 1);
            }
            if times.last().is_some_and(|previous| *previous >= time) {
                continue;
            }
            times.push(time);
            samples.push((source_index, source_time));
            if samples.len() > MAX_OUTPUT_SAMPLES {
                return Err(input_error("Combined animation has too many keyframes"));
            }
        }
        offset += length;
    }
    if times.len() < 2 {
        return Err(input_error("Combined animation is too short"));
    }
    if samples
        .len()
        .checked_mul(width_total)
        .is_none_or(|count| count > MAX_OUTPUT_FLOATS)
    {
        return Err(input_error(
            "Combined animation is too large; use shorter segments",
        ));
    }

    // Plain animation rigs reference binary data only from their channels.
    // Rebuild that data rather than retaining every previous combine payload.
    // Mesh/skin/image or foreign-extension buffers keep their original indices.
    let animation_only = ["meshes", "skins", "images"].iter().all(|name| {
        json.get(*name)
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    }) && json
        .get("extensions")
        .and_then(Value::as_object)
        .is_some_and(|extensions| extensions.keys().all(|name| name == "VRMC_vrm_animation"))
        && ["extensionsUsed", "extensionsRequired"].iter().all(|name| {
            json.get(*name)
                .and_then(Value::as_array)
                .is_none_or(|extensions| {
                    extensions
                        .iter()
                        .all(|name| name.as_str() == Some("VRMC_vrm_animation"))
                })
        });
    let mut bin = if animation_only {
        json["bufferViews"] = json!([]);
        json["accessors"] = json!([]);
        Vec::new()
    } else {
        template.bin.clone()
    };
    let input = append_accessor(&mut json, &mut bin, &times, 1, true)?;
    let mut channels = Vec::new();
    let mut samplers = Vec::new();
    for target in targets {
        let destination_node = roles[&target.role];
        let destination = destination_rest[destination_node];
        let width = if target.path == "rotation" { 4 } else { 3 };
        let mut values = Vec::with_capacity(samples.len() * width);
        for &(source_index, time) in &samples {
            let source = &sources[source_index];
            let value = if let Some(&node) = source.roles.get(&target.role) {
                let rest = source_rests[source_index][node];
                let pose = if let Some(curve) = source.curves.get(&target) {
                    sample(curve, time, target.path == "rotation")?
                } else {
                    resting_value(rest, &target.path)
                };
                retarget(&pose, &target, rest, destination)?
            } else {
                resting_value(destination, &target.path)
            };
            values.extend(value);
        }
        let output = append_accessor(&mut json, &mut bin, &values, width, false)?;
        let sampler_index = samplers.len();
        samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
        channels.push(json!({"sampler": sampler_index, "target": {"node": destination_node, "path": target.path}}));
    }
    json["animations"] =
        json!([{"name": "Combined Animation", "channels": channels, "samplers": samplers}]);
    json["buffers"][0]["byteLength"] = json!(bin.len());
    Ok(Source {
        json,
        bin,
        roles,
        curves: BTreeMap::new(),
        duration: offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec3;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "nera-combine-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(path: &Path, base: f32) {
        let mut json = json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["VRMC_vrm_animation"],
            "extensions": {"VRMC_vrm_animation": {
                "specVersion": "1.0",
                "humanoid": {"humanBones": {"hips": {"node": 0}}}
            }},
            "nodes": [{"name": "hips", "translation": [0., 1., 0.]}],
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "buffers": [{"byteLength": 0}],
            "bufferViews": [],
            "accessors": []
        });
        let mut bin = Vec::new();
        let input = append_accessor(&mut json, &mut bin, &[0., 1., 2.], 1, true).unwrap();
        let output = append_accessor(
            &mut json,
            &mut bin,
            &[base, 1., 0., base + 1., 1., 0., base + 2., 1., 0.],
            3,
            false,
        )
        .unwrap();
        json["animations"] = json!([{
            "samplers": [{"input": input, "output": output, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]
        }]);
        json["buffers"][0]["byteLength"] = json!(bin.len());
        fs::write(path, write_glb(&json, &bin).unwrap()).unwrap();
    }

    #[test]
    fn import_preflight_rejects_missing_hips_missing_extension_and_invalid_keyframe_times() {
        let root = TestDirectory::new();
        let path = root.0.join("valid.vrma");
        fixture(&path, 0.0);
        let bytes = fs::read(&path).unwrap();
        assert!(crate::animation::validate_vrma_import(&bytes).is_ok());
        let (json, bin) = read_glb(&bytes).unwrap();
        let mut missing = json.clone();
        missing["extensions"]["VRMC_vrm_animation"]["humanoid"]["humanBones"] =
            json!({"spine": {"node": 0}});
        assert!(
            crate::animation::validate_vrma_import(&write_glb(&missing, &bin).unwrap()).is_err()
        );
        missing = json.clone();
        missing["extensions"] = json!({});
        missing["extensionsUsed"] = json!([]);
        assert!(
            crate::animation::validate_vrma_import(&write_glb(&missing, &bin).unwrap()).is_err()
        );
        for invalid_time in [f32::NAN, f32::INFINITY, f32::MAX, 0.0] {
            let mut broken = bin.clone();
            // This fixture's first buffer view holds [0,1,2] seconds.
            broken[8..12].copy_from_slice(&invalid_time.to_le_bytes());
            assert!(
                crate::animation::validate_vrma_import(&write_glb(&json, &broken).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn repeated_combines_do_not_retain_old_animation_buffers() {
        let root = TestDirectory::new();
        let path = root.0.join("source.vrma");
        fixture(&path, 0.0);
        for _ in 0..8 {
            let sources = [read_source(&path).unwrap(), read_source(&path).unwrap()];
            let output = build_combined(&sources, &[(0.0, 1.0), (0.0, 1.0)]).unwrap();
            assert_eq!(output.json["bufferViews"].as_array().unwrap().len(), 2);
            assert_eq!(output.json["accessors"].as_array().unwrap().len(), 2);
            assert!(output.bin.len() < 3_000);
            fs::write(&path, write_glb(&output.json, &output.bin).unwrap()).unwrap();
        }
    }

    #[test]
    fn constant_rest_channels_are_ignored_but_dynamic_non_hips_translation_still_fails() {
        let root = TestDirectory::new();
        let path = root.0.join("source.vrma");
        fixture(&path, 0.0);
        let mut source = read_source(&path).unwrap();
        source.json["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "spine", "translation": [0.0, 2.0, 0.0]}));
        source.json["nodes"][0]["children"] = json!([1]);
        source.json["extensions"]["VRMC_vrm_animation"]["humanoid"]["humanBones"]["spine"] =
            json!({"node": 1});
        source.roles.insert("bone:spine".into(), 1);
        let translation = Target {
            role: "bone:spine".into(),
            path: "translation".into(),
        };
        source.curves.insert(
            translation.clone(),
            Curve {
                times: vec![0.0, 2.0],
                values: vec![vec![0.0, 2.0, 0.0]; 2],
                interpolation: Interpolation::Linear,
            },
        );
        source.curves.insert(
            Target {
                role: "bone:hips".into(),
                path: "scale".into(),
            },
            Curve {
                times: vec![0.0, 2.0],
                values: vec![vec![1.0; 3]; 2],
                interpolation: Interpolation::Linear,
            },
        );
        let output = build_combined(std::slice::from_ref(&source), &[(0.0, 1.0)]).unwrap();
        assert_eq!(
            output.json["animations"][0]["channels"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        source.curves.get_mut(&translation).unwrap().values[1][1] = 2.5;
        assert!(build_combined(&[source], &[(0.0, 1.0)]).is_err());
    }

    #[test]
    fn trims_join_in_order_and_sources_are_unchanged() {
        let root = TestDirectory::new();
        fixture(&root.0.join("first.vrma"), 0.);
        fixture(&root.0.join("second.vrma"), 10.);
        let first_before = fs::read(root.0.join("first.vrma")).unwrap();
        let second_before = fs::read(root.0.join("second.vrma")).unwrap();
        let segments = [
            Segment {
                key: "first.vrma".into(),
                start: 0.5,
                end: Some(1.5),
            },
            Segment {
                key: "second.vrma".into(),
                start: 1.,
                end: Some(2.),
            },
            Segment {
                key: "first.vrma".into(),
                start: 1.5,
                end: None,
            },
        ];
        let path = combine(&root.0, &segments, "My Sequence").unwrap();
        assert_eq!(path, root.0.join("Combined/My Sequence.vrma"));
        let combined = read_source(&path).unwrap();
        assert!((combined.duration - 2.5).abs() < 0.0001);
        let curve = &combined.curves[&Target {
            role: "bone:hips".into(),
            path: "translation".into(),
        }];
        for (time, expected) in [
            (0., 0.5),
            (0.5, 1.),
            (1., 11.),
            (1.5, 11.5),
            (2., 1.5),
            (2.5, 2.),
        ] {
            assert!(
                (sample(curve, time, false).unwrap()[0] - expected).abs() < 0.0001,
                "at {time}"
            );
        }
        assert_eq!(fs::read(root.0.join("first.vrma")).unwrap(), first_before);
        assert_eq!(fs::read(root.0.join("second.vrma")).unwrap(), second_before);
        let result_before = fs::read(&path).unwrap();
        assert_eq!(
            combine(&root.0, &segments, "my sequence")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(path).unwrap(), result_before);
        assert!(
            fs::read_dir(root.0.join("Combined"))
                .unwrap()
                .all(|entry| entry.unwrap().path().extension().unwrap() == "vrma")
        );
    }

    #[test]
    fn invalid_trim_and_paths_do_not_create_output() {
        let root = TestDirectory::new();
        fixture(&root.0.join("first.vrma"), 0.);
        let valid = Segment {
            key: "first.vrma".into(),
            start: 0.,
            end: None,
        };
        for invalid_segment in [
            Segment {
                start: 2.,
                ..valid.clone()
            },
            Segment {
                start: -1.,
                ..valid.clone()
            },
            Segment {
                end: Some(3.),
                ..valid.clone()
            },
            Segment {
                start: f32::NAN,
                ..valid.clone()
            },
            Segment {
                key: "../first.vrma".into(),
                ..valid.clone()
            },
        ] {
            assert!(combine(&root.0, &[valid.clone(), invalid_segment], "result").is_err());
        }
        assert!(combine(&root.0, &[valid.clone(), valid.clone()], "../unsafe").is_err());
        assert!(combine(&root.0, &[valid], "only one").is_err());
        assert!(!root.0.join("Combined").exists());
    }

    #[test]
    fn retarget_preserves_normalized_motion_between_different_rest_rigs() {
        use bevy::math::Mat4;
        let source = Rest {
            translation: Vec3::new(0., 2., 0.),
            rotation: Quat::from_rotation_z(0.7),
            scale: Vec3::ONE,
            global: Mat4::from_rotation_translation(
                Quat::from_rotation_y(1.1),
                Vec3::new(0., 2., 0.),
            ),
        };
        let destination = Rest {
            translation: Vec3::new(0., 1., 0.),
            rotation: Quat::from_rotation_x(0.3),
            scale: Vec3::ONE,
            global: Mat4::from_rotation_translation(
                Quat::from_rotation_z(-0.4),
                Vec3::new(0., 1., 0.),
            ),
        };
        let normalized_motion = Quat::from_rotation_x(0.6) * Quat::from_rotation_z(-0.2);
        let (_, source_global, _) = source.global.to_scale_rotation_translation();
        let source_pose =
            source.rotation * source_global.inverse() * normalized_motion * source_global;
        let result = retarget(
            &source_pose.to_array(),
            &Target {
                role: "bone:head".into(),
                path: "rotation".into(),
            },
            source,
            destination,
        )
        .unwrap();
        let (_, destination_global, _) = destination.global.to_scale_rotation_translation();
        let normalized_output = destination_global
            * destination.rotation.inverse()
            * quaternion(&result).unwrap()
            * destination_global.inverse();
        assert!(normalized_output.angle_between(normalized_motion).abs() < 0.001);
        let hips = retarget(
            &[2., 3., 4.],
            &Target {
                role: "bone:hips".into(),
                path: "translation".into(),
            },
            source,
            destination,
        )
        .unwrap();
        assert_eq!(hips, vec![1., 1.5, 2.]);
    }

    #[test]
    fn parser_rejects_truncated_and_invalid_animation_buffers() {
        let root = TestDirectory::new();
        let path = root.0.join("test.vrma");
        fixture(&path, 0.);
        let bytes = fs::read(&path).unwrap();
        assert!(read_glb(&bytes[..bytes.len() - 1]).is_err());
        let (mut json, bin) = read_glb(&bytes).unwrap();
        json["accessors"][0]["count"] = json!(u64::MAX);
        fs::write(&path, write_glb(&json, &bin).unwrap()).unwrap();
        assert!(duration(&path).is_err());
    }

    #[test]
    fn step_and_cubic_interpolation_are_evaluated_before_export() {
        let step = Curve {
            times: vec![0., 1.],
            values: vec![vec![0., 0., 0.], vec![2., 0., 0.]],
            interpolation: Interpolation::Step,
        };
        assert_eq!(sample(&step, 0.99, false).unwrap()[0], 0.);
        assert_eq!(sample(&step, 1., false).unwrap()[0], 2.);
        let cubic = Curve {
            times: vec![0., 1.],
            values: vec![
                vec![0., 0., 0.],
                vec![0., 0., 0.],
                vec![4., 0., 0.],
                vec![0., 0., 0.],
                vec![2., 0., 0.],
                vec![0., 0., 0.],
            ],
            interpolation: Interpolation::CubicSpline,
        };
        assert!((sample(&cubic, 0.5, false).unwrap()[0] - 1.5).abs() < 0.0001);
    }

    #[test]
    fn publication_cannot_replace_an_existing_result() {
        let root = TestDirectory::new();
        let temporary = root.0.join("export.tmp");
        let destination = root.0.join("existing.vrma");
        fs::write(&temporary, b"new").unwrap();
        fs::write(&destination, b"original").unwrap();
        assert!(fs::hard_link(&temporary, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
    }

    #[test]
    #[ignore = "requires local VRMA files; validates real source/output poses without rendering"]
    fn real_mixed_rigs_preserve_normalized_poses_after_export() {
        let root = PathBuf::from(crate::nera_asset_path()).join("animations");
        let fixtures = [
            "animasi_basic/idle.vrma",
            "animasi_basic/LookAround.vrma",
            "Clawatar_animations/161_Waving.vrma",
            "Mixamo-VRMA/Agreeing.vrma",
        ];
        for order in [[0, 1, 2, 3], [1, 3, 0, 2]] {
            let sources: Vec<_> = order
                .iter()
                .map(|index| read_source(&root.join(fixtures[*index])).unwrap())
                .collect();
            let built = build_combined(&sources, &[(0.1, 0.6); 4]).unwrap();
            let temporary = TestDirectory::new();
            let output_path = temporary.0.join("combined.vrma");
            fs::write(&output_path, write_glb(&built.json, &built.bin).unwrap()).unwrap();
            let output = read_source(&output_path).unwrap();
            let output_rests = rest_transforms(&output.json).unwrap();
            for (index, source) in sources.iter().enumerate() {
                let source_rests = rest_transforms(&source.json).unwrap();
                let output_time = index as f32 * 0.5 + 0.2;
                for (target, curve) in &source.curves {
                    let src = source_rests[source.roles[&target.role]];
                    let dst = output_rests[output.roles[&target.role]];
                    let original = sample(curve, 0.3, target.path == "rotation").unwrap();
                    let result = sample(
                        &output.curves[target],
                        output_time,
                        target.path == "rotation",
                    )
                    .unwrap();
                    if target.path == "rotation" {
                        let (_, src_global, _) = src.global.to_scale_rotation_translation();
                        let (_, dst_global, _) = dst.global.to_scale_rotation_translation();
                        let expected = src_global
                            * src.rotation.inverse()
                            * quaternion(&original).unwrap()
                            * src_global.inverse();
                        let actual = dst_global
                            * dst.rotation.inverse()
                            * quaternion(&result).unwrap()
                            * dst_global.inverse();
                        assert!(
                            expected.angle_between(actual).abs() < 0.003,
                            "{} {}",
                            fixtures[order[index]],
                            target.role
                        );
                    } else if target.role == "bone:hips" && target.path == "translation" {
                        let expected =
                            (Vec3::from_slice(&original) - src.translation) / src.global.w_axis.y;
                        let actual =
                            (Vec3::from_slice(&result) - dst.translation) / dst.global.w_axis.y;
                        assert!(expected.distance(actual) < 0.0001, "hips delta changed");
                    }
                }
            }
        }
    }
}
