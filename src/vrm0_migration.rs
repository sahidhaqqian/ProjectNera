//! In-memory migration for legacy VRM 0.x character imports.
//!
//! The source file is never modified. The migrated GLB is validated by the
//! ordinary VRM 1.0 import path before it is copied into Nera's private store.
//! The mapping follows the VRM 0.x/1.0 specifications and is partially based
//! on ZaberKo/vrm-studio's MIT-licensed migrator (see THIRD_PARTY_NOTICES.md).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

const GLB_JSON: u32 = 0x4E4F_534A;

pub(crate) struct MigratedVrm {
    pub(crate) bytes: Vec<u8>,
    pub(crate) converted: bool,
}

pub(crate) fn migrate_vrm0(bytes: Vec<u8>) -> Result<MigratedVrm, String> {
    let (json_length, mut document) = parse_document(&bytes)?;
    if document.pointer("/extensions/VRMC_vrm").is_some() {
        return Ok(MigratedVrm {
            bytes,
            converted: false,
        });
    }
    let vrm0 = document
        .pointer("/extensions/VRM")
        .cloned()
        .ok_or_else(|| {
            "This file contains neither VRM 0.x nor VRM 1.0 character data.".to_owned()
        })?;

    let mesh_to_node = mesh_to_node_map(&document);
    let mut vrm1 = json!({
        "specVersion": "1.0",
        "meta": migrate_meta(&vrm0, &document),
        "humanoid": { "humanBones": migrate_humanoid(&vrm0)? },
        "expressions": migrate_expressions(&vrm0, &mesh_to_node),
        "firstPerson": { "meshAnnotations": migrate_first_person(&vrm0, &mesh_to_node) },
        "lookAt": migrate_look_at(&vrm0),
    });
    remove_empty_custom_expressions(&mut vrm1);

    let spring = migrate_spring_bones(&vrm0, &mut document)?;
    rotate_scene_roots(&mut document)?;

    let extensions = document
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "Legacy VRM extensions are malformed.".to_owned())?;
    extensions.remove("VRM");
    extensions.insert("VRMC_vrm".into(), vrm1);
    if let Some(spring) = spring {
        extensions.insert("VRMC_springBone".into(), spring);
    }
    replace_extension_name(&mut document, "extensionsUsed", "VRM", "VRMC_vrm");
    replace_extension_name(&mut document, "extensionsRequired", "VRM", "VRMC_vrm");
    if document.pointer("/extensions/VRMC_springBone").is_some() {
        add_extension_name(&mut document, "extensionsUsed", "VRMC_springBone");
    }

    Ok(MigratedVrm {
        bytes: rebuild_glb(&bytes, json_length, &document)?,
        converted: true,
    })
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn parse_document(bytes: &[u8]) -> Result<(usize, Value), String> {
    if bytes.get(..4) != Some(b"glTF")
        || u32_at(bytes, 4) != Some(2)
        || u32_at(bytes, 8).map(|length| length as usize) != Some(bytes.len())
        || u32_at(bytes, 16) != Some(GLB_JSON)
    {
        return Err("This is not a valid binary VRM file.".into());
    }
    let json_length = u32_at(bytes, 12).unwrap_or_default() as usize;
    let end = 20usize
        .checked_add(json_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "Character JSON chunk is incomplete.".to_owned())?;
    let document = serde_json::from_slice::<Value>(&bytes[20..end])
        .map_err(|_| "Character metadata is invalid.".to_owned())?;
    Ok((json_length, document))
}

fn string(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn legacy_allows(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str) == Some("Allow")
}

fn migrate_meta(vrm0: &Value, document: &Value) -> Value {
    let meta = vrm0.get("meta").unwrap_or(&Value::Null);
    let license = meta
        .get("licenseName")
        .and_then(Value::as_str)
        .unwrap_or("Other");
    let mut allow_redistribution = false;
    let mut modification = "prohibited";
    let mut credit = "required";
    match license {
        "CC0" => {
            allow_redistribution = true;
            modification = "allowModificationRedistribution";
            credit = "unnecessary";
        }
        "CC_BY" | "CC_BY_SA" | "CC_BY_NC" | "CC_BY_NC_SA" => {
            allow_redistribution = true;
            modification = "allowModificationRedistribution";
        }
        "CC_BY_ND" | "CC_BY_NC_ND" => allow_redistribution = true,
        _ => {}
    }
    let avatar_permission = match meta.get("allowedUserName").and_then(Value::as_str) {
        Some("Everyone") => "everyone",
        Some("ExplicitlyLicensedPerson") => "onlySeparatelyLicensedPerson",
        _ => "onlyAuthor",
    };
    let commercial = if legacy_allows(meta.get("commercialUssageName"))
        && !matches!(license, "CC_BY_NC" | "CC_BY_NC_SA" | "CC_BY_NC_ND")
    {
        "personalProfit"
    } else {
        "personalNonProfit"
    };
    let mut migrated = json!({
        "name": string(meta.get("title"), "Unknown"),
        "version": string(meta.get("version"), "1.0"),
        "authors": [string(meta.get("author"), "Unknown")],
        "avatarPermission": avatar_permission,
        "commercialUsage": commercial,
        "creditNotation": credit,
        "modification": modification,
        "allowRedistribution": allow_redistribution,
        "allowExcessivelyViolentUsage": legacy_allows(meta.get("violentUssageName")),
        "allowExcessivelySexualUsage": legacy_allows(meta.get("sexualUssageName")),
        "allowPoliticalOrReligiousUsage": false,
        "allowAntisocialOrHateUsage": false,
        "licenseUrl": "https://vrm.dev/licenses/1.0/",
    });
    let license_url = ["otherLicenseUrl", "otherPermissionUrl"]
        .into_iter()
        .filter_map(|key| meta.get(key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .fold(Vec::<&str>::new(), |mut values, value| {
            if !values.contains(&value) {
                values.push(value);
            }
            values
        })
        .join("\n");
    if !license_url.is_empty() {
        migrated["otherLicenseUrl"] = Value::String(license_url);
    }
    if let Some(texture) = meta.get("texture").and_then(Value::as_u64)
        && let Some(source) = document
            .pointer(&format!("/textures/{texture}/source"))
            .and_then(Value::as_u64)
    {
        migrated["thumbnailImage"] = json!(source);
    }
    migrated
}

fn migrate_humanoid(vrm0: &Value) -> Result<Value, String> {
    let bones = vrm0
        .pointer("/humanoid/humanBones")
        .and_then(Value::as_array)
        .ok_or_else(|| "Legacy character has no humanoid mapping.".to_owned())?;
    let mut migrated = Map::new();
    for bone in bones {
        let (Some(name), Some(node)) = (
            bone.get("bone").and_then(Value::as_str),
            bone.get("node").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let mut chars = name.chars();
        let Some(first) = chars.next() else { continue };
        let name = first.to_lowercase().collect::<String>() + chars.as_str();
        migrated.insert(name, json!({ "node": node }));
    }
    Ok(Value::Object(migrated))
}

fn mesh_to_node_map(document: &Value) -> HashMap<u64, u64> {
    document
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(node, value)| {
            value
                .get("mesh")
                .and_then(Value::as_u64)
                .map(|mesh| (mesh, node as u64))
        })
        .collect()
}

fn migrate_expressions(vrm0: &Value, mesh_to_node: &HashMap<u64, u64>) -> Value {
    let presets = HashMap::from([
        ("joy", "happy"),
        ("angry", "angry"),
        ("sorrow", "sad"),
        ("fun", "relaxed"),
        ("surprised", "surprised"),
        ("a", "aa"),
        ("i", "ih"),
        ("u", "ou"),
        ("e", "ee"),
        ("o", "oh"),
        ("neutral", "neutral"),
        ("blink", "blink"),
        ("blink_l", "blinkLeft"),
        ("blink_r", "blinkRight"),
        ("lookup", "lookUp"),
        ("lookdown", "lookDown"),
        ("lookleft", "lookLeft"),
        ("lookright", "lookRight"),
    ]);
    let mut preset = Map::new();
    let mut custom = Map::new();
    for group in vrm0
        .pointer("/blendShapeMaster/blendShapeGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = string(group.get("name"), "expression");
        let preset_name = group
            .get("presetName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let name_as_preset = name.to_ascii_lowercase();
        let key = presets.get(preset_name.as_str()).copied().or_else(|| {
            (preset_name == "unknown")
                .then(|| presets.get(name_as_preset.as_str()).copied())
                .flatten()
        });
        let binds: Vec<_> = group
            .get("binds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|bind| {
                let mesh = bind.get("mesh")?.as_u64()?;
                Some(json!({
                    "node": mesh_to_node.get(&mesh).copied().unwrap_or(mesh),
                    "index": bind.get("index")?.as_u64()?,
                    "weight": bind.get("weight").and_then(Value::as_f64).unwrap_or(100.0) / 100.0,
                }))
            })
            .collect();
        let mut expression = json!({
            "isBinary": group.get("isBinary").and_then(Value::as_bool).unwrap_or(false),
            "overrideBlink": "none",
            "overrideLookAt": "none",
            "overrideMouth": "none",
        });
        if !binds.is_empty() {
            expression["morphTargetBinds"] = Value::Array(binds);
        }
        if let Some(key) = key {
            preset.insert(key.into(), expression);
        } else {
            custom.insert(name, expression);
        }
    }
    json!({ "preset": preset, "custom": custom })
}

fn remove_empty_custom_expressions(vrm1: &mut Value) {
    if vrm1
        .pointer("/expressions/custom")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
        && let Some(expressions) = vrm1.get_mut("expressions").and_then(Value::as_object_mut)
    {
        expressions.remove("custom");
    }
}

fn migrate_first_person(vrm0: &Value, mesh_to_node: &HashMap<u64, u64>) -> Vec<Value> {
    vrm0.pointer("/firstPerson/meshAnnotations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|annotation| {
            let mesh = annotation.get("mesh")?.as_u64()?;
            let flag = match annotation.get("firstPersonFlag").and_then(Value::as_str) {
                Some("FirstPersonOnly") => "firstPersonOnly",
                Some("ThirdPersonOnly") => "thirdPersonOnly",
                Some("Both") => "both",
                _ => "auto",
            };
            Some(json!({
                "node": mesh_to_node.get(&mesh).copied().unwrap_or(mesh),
                "firstPersonFlag": flag,
            }))
        })
        .collect()
}

fn migrate_look_at(vrm0: &Value) -> Value {
    let first = vrm0.get("firstPerson").unwrap_or(&Value::Null);
    let offset = first.get("firstPersonBoneOffset").unwrap_or(&Value::Null);
    let range = |key: &str| {
        let source = first.get(key).unwrap_or(&Value::Null);
        json!({
            "inputMaxValue": source.pointer("/curve/7/x").and_then(Value::as_f64).unwrap_or(90.0),
            "outputScale": source.get("xRange").and_then(Value::as_f64).unwrap_or(10.0),
        })
    };
    json!({
        "offsetFromHeadBone": [
            offset.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            offset.get("y").and_then(Value::as_f64).unwrap_or(0.06),
            offset.get("z").and_then(Value::as_f64).unwrap_or(0.0),
        ],
        "type": if first.get("lookAtTypeName").and_then(Value::as_str) == Some("Bone") { "bone" } else { "expression" },
        "rangeMapHorizontalInner": range("lookAtHorizontalInner"),
        "rangeMapHorizontalOuter": range("lookAtHorizontalOuter"),
        "rangeMapVerticalDown": range("lookAtVerticalDown"),
        "rangeMapVerticalUp": range("lookAtVerticalUp"),
    })
}

fn number(object: &Value, key: &str, fallback: f64) -> f64 {
    object.get(key).and_then(Value::as_f64).unwrap_or(fallback)
}

fn vec3(object: Option<&Value>, fallback: [f64; 3]) -> [f64; 3] {
    let object = object.unwrap_or(&Value::Null);
    [
        number(object, "x", fallback[0]),
        number(object, "y", fallback[1]),
        number(object, "z", fallback[2]),
    ]
}

fn node_paths(
    nodes: &[Value],
    node: usize,
    path: &mut Vec<usize>,
    visiting: &mut HashSet<usize>,
    paths: &mut Vec<Vec<usize>>,
) -> Result<(), String> {
    if node >= nodes.len() {
        return Err("Legacy spring bone points outside the skeleton.".into());
    }
    if !visiting.insert(node) {
        return Err("Legacy spring bone hierarchy contains a cycle.".into());
    }
    path.push(node);
    let children: Vec<_> = nodes[node]
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|child| child as usize)
        .collect();
    if children.is_empty() {
        paths.push(path.clone());
    } else {
        for child in children {
            node_paths(nodes, child, path, visiting, paths)?;
        }
    }
    path.pop();
    visiting.remove(&node);
    Ok(())
}

fn append_tail(
    nodes: &mut Vec<Value>,
    source_nodes: &[Value],
    leaf: usize,
    tail_by_leaf: &mut HashMap<usize, usize>,
) -> Result<usize, String> {
    if let Some(tail) = tail_by_leaf.get(&leaf) {
        return Ok(*tail);
    }
    let node = source_nodes
        .get(leaf)
        .ok_or_else(|| "Legacy spring bone leaf is invalid.".to_owned())?;
    let translation = node
        .get("translation")
        .and_then(Value::as_array)
        .filter(|value| value.len() >= 3)
        .map(|value| {
            [
                value[0].as_f64().unwrap_or(0.0),
                value[1].as_f64().unwrap_or(0.0),
                value[2].as_f64().unwrap_or(0.0),
            ]
        })
        .unwrap_or([0.0, -0.07, 0.0]);
    let length = translation
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    let delta = if length > 0.0001 {
        translation.map(|value| value / length * 0.07)
    } else {
        [0.0, -0.07, 0.0]
    };
    let tail = nodes.len();
    nodes.push(json!({
        "name": format!("{}_end", string(node.get("name"), "spring_tail")),
        "translation": delta,
    }));
    let children = nodes
        .get_mut(leaf)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "Legacy spring bone leaf is malformed.".to_owned())?
        .entry("children")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "Legacy spring bone children are malformed.".to_owned())?;
    if !children
        .iter()
        .any(|child| child.as_u64() == Some(tail as u64))
    {
        children.push(json!(tail));
    }
    tail_by_leaf.insert(leaf, tail);
    Ok(tail)
}

fn migrate_spring_bones(vrm0: &Value, document: &mut Value) -> Result<Option<Value>, String> {
    let Some(secondary) = vrm0.get("secondaryAnimation") else {
        return Ok(None);
    };
    let legacy_groups = secondary
        .get("colliderGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut colliders = Vec::new();
    let mut collider_groups = Vec::new();
    for (group_index, group) in legacy_groups.iter().enumerate() {
        let node = group.get("node").and_then(Value::as_u64).unwrap_or(0);
        let mut indices = Vec::new();
        for collider in group
            .get("colliders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            indices.push(colliders.len());
            let offset = vec3(collider.get("offset"), [0.0, 0.0, 0.0]);
            colliders.push(json!({
                "node": node,
                "shape": { "sphere": {
                    "offset": offset,
                    "radius": number(collider, "radius", 0.1),
                }},
            }));
        }
        collider_groups.push(json!({
            "name": format!("ColliderGroup_{group_index}"),
            "colliders": indices,
        }));
    }

    let source_nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "Legacy character has no skeleton nodes.".to_owned())?;
    let mut springs = Vec::new();
    let mut tail_by_leaf = HashMap::new();
    for (group_index, group) in secondary
        .get("boneGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let roots: Vec<_> = group
            .get("bones")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|node| node as usize)
            .collect();
        let valid_collider_groups: Vec<_> = group
            .get("colliderGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|index| index as usize)
            .filter(|index| *index < collider_groups.len())
            .collect();
        let center = group
            .get("center")
            .and_then(Value::as_i64)
            .filter(|center| *center >= 0 && (*center as usize) < source_nodes.len())
            .map(|center| center as usize);
        let name = string(group.get("comment"), &format!("Spring_{group_index}"));
        let mut paths = Vec::new();
        for root in roots {
            node_paths(
                &source_nodes,
                root,
                &mut Vec::new(),
                &mut HashSet::new(),
                &mut paths,
            )?;
        }
        for (path_index, path) in paths.iter().enumerate() {
            let leaf = *path
                .last()
                .expect("node_paths always emits a non-empty path");
            let tail = {
                let nodes = document
                    .get_mut("nodes")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "Legacy character has no skeleton nodes.".to_owned())?;
                append_tail(nodes, &source_nodes, leaf, &mut tail_by_leaf)?
            };
            let mut joints: Vec<_> = path
                .iter()
                .map(|node| {
                    json!({
                        "node": node,
                        "stiffness": number(group, "stiffiness", 1.0),
                        "gravityPower": number(group, "gravityPower", 0.0),
                        "gravityDir": vec3(group.get("gravityDir"), [0.0, -1.0, 0.0]),
                        "dragForce": number(group, "dragForce", 0.4),
                        "hitRadius": number(group, "hitRadius", 0.02),
                    })
                })
                .collect();
            joints.push(json!({ "node": tail }));
            springs.push(json!({
                "name": if paths.len() > 1 { format!("{name}_{path_index}") } else { name.clone() },
                "joints": joints,
                "colliderGroups": valid_collider_groups,
                "center": center,
            }));
        }
    }
    Ok(Some(json!({
        "specVersion": "1.0",
        "colliders": colliders,
        "colliderGroups": collider_groups,
        "springs": springs,
    })))
}

fn rotate_scene_roots(document: &mut Value) -> Result<(), String> {
    let scene = document.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let roots: Vec<_> = document
        .get("scenes")
        .and_then(Value::as_array)
        .and_then(|scenes| scenes.get(scene))
        .and_then(|scene| scene.get("nodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|node| node as usize)
        .collect();
    let nodes = document
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "Legacy character has no skeleton nodes.".to_owned())?;
    for root in roots {
        let node = nodes
            .get_mut(root)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "Legacy scene points outside the skeleton.".to_owned())?;
        let current = node
            .get("rotation")
            .and_then(Value::as_array)
            .filter(|rotation| rotation.len() == 4)
            .map(|rotation| {
                [
                    rotation[0].as_f64().unwrap_or(0.0),
                    rotation[1].as_f64().unwrap_or(0.0),
                    rotation[2].as_f64().unwrap_or(0.0),
                    rotation[3].as_f64().unwrap_or(1.0),
                ]
            })
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let [x, y, z, w] = current;
        node.insert("rotation".into(), json!([-z, w, x, -y]));
    }
    Ok(())
}

fn replace_extension_name(document: &mut Value, field: &str, old: &str, new: &str) {
    let Some(values) = document.get_mut(field).and_then(Value::as_array_mut) else {
        return;
    };
    values.retain(|value| value.as_str() != Some(old));
    if !values.iter().any(|value| value.as_str() == Some(new)) {
        values.push(Value::String(new.to_owned()));
    }
}

fn add_extension_name(document: &mut Value, field: &str, name: &str) {
    let values = document
        .as_object_mut()
        .expect("a parsed glTF document is an object")
        .entry(field)
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(values) = values.as_array_mut()
        && !values.iter().any(|value| value.as_str() == Some(name))
    {
        values.push(Value::String(name.to_owned()));
    }
}

fn rebuild_glb(bytes: &[u8], old_json_length: usize, document: &Value) -> Result<Vec<u8>, String> {
    let old_json_end = 20usize
        .checked_add(old_json_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "Character JSON chunk is incomplete.".to_owned())?;
    let mut json = serde_json::to_vec(document)
        .map_err(|error| format!("Cannot write migrated character metadata: {error}"))?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total_length = 20usize
        .checked_add(json.len())
        .and_then(|length| length.checked_add(bytes.len() - old_json_end))
        .filter(|length| *length <= u32::MAX as usize)
        .ok_or_else(|| "Migrated character is too large.".to_owned())?;
    let mut output = Vec::with_capacity(total_length);
    output.extend_from_slice(&bytes[..20]);
    output[8..12].copy_from_slice(&(total_length as u32).to_le_bytes());
    output[12..16].copy_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(&json);
    output.extend_from_slice(&bytes[old_json_end..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_vrm1::prelude::VrmExtensions;

    #[test]
    fn bundled_legacy_character_migrates_without_touching_binary_payload() {
        let original = include_bytes!("../assets/vrm/nera0.0.vrm").to_vec();
        let old_json_length = u32_at(&original, 12).unwrap() as usize;
        let migrated = migrate_vrm0(original.clone()).unwrap();
        assert!(migrated.converted);
        let (new_json_length, document) = parse_document(&migrated.bytes).unwrap();
        assert!(document.pointer("/extensions/VRM").is_none());
        assert_eq!(
            document.pointer("/extensions/VRMC_vrm/specVersion"),
            Some(&json!("1.0"))
        );
        serde_json::from_value::<VrmExtensions>(document["extensions"].clone()).unwrap();
        assert_eq!(
            &original[20 + old_json_length..],
            &migrated.bytes[20 + new_json_length..]
        );

        let second = migrate_vrm0(migrated.bytes.clone()).unwrap();
        assert!(!second.converted);
        assert_eq!(second.bytes, migrated.bytes);
    }
}
