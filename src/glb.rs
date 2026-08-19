#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::{io, path::Path};

/// One triangle mesh with a texture-less PBR material.
pub struct MeshData {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tex_coords: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
}

const GLB_MAGIC: u32 = 0x4654_6C67;
const GLB_VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn pad4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn min_max3(values: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for value in values.iter() {
        for axis in 0..3 {
            min[axis] = min[axis].min(value[axis]);
            max[axis] = max[axis].max(value[axis]);
        }
    }
    (min, max)
}

fn min_max2(values: &[[f32; 2]]) -> ([f32; 2], [f32; 2]) {
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for value in values.iter() {
        for axis in 0..2 {
            min[axis] = min[axis].min(value[axis]);
            max[axis] = max[axis].max(value[axis]);
        }
    }
    (min, max)
}

/// Encode a GLB with one node per mesh. The first scene owns every node.
pub fn encode_glb(meshes: &[MeshData]) -> io::Result<Vec<u8>> {
    assert!(!meshes.is_empty(), "need at least one mesh");

    let mut bin = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut materials = Vec::new();
    let mut gl_meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut node_indices = Vec::new();

    for (mesh_index, mesh) in meshes.iter().enumerate() {
        assert_eq!(mesh.positions.len(), mesh.normals.len());
        assert_eq!(mesh.positions.len(), mesh.tex_coords.len());
        assert!(!mesh.indices.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);

        let pos_offset = bin.len();
        for pos in mesh.positions.iter() {
            for component in pos.iter() {
                bin.extend_from_slice(&component.to_le_bytes());
            }
        }
        let pos_len = bin.len() - pos_offset;
        bin.resize(pad4(bin.len()), 0);

        let nrm_offset = bin.len();
        for nrm in mesh.normals.iter() {
            for component in nrm.iter() {
                bin.extend_from_slice(&component.to_le_bytes());
            }
        }
        let nrm_len = bin.len() - nrm_offset;
        bin.resize(pad4(bin.len()), 0);

        let uv_offset = bin.len();
        for uv in mesh.tex_coords.iter() {
            for component in uv.iter() {
                bin.extend_from_slice(&component.to_le_bytes());
            }
        }
        let uv_len = bin.len() - uv_offset;
        bin.resize(pad4(bin.len()), 0);

        let idx_offset = bin.len();
        for index in mesh.indices.iter() {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        let idx_len = bin.len() - idx_offset;
        bin.resize(pad4(bin.len()), 0);

        let view_base = buffer_views.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": pos_offset,
            "byteLength": pos_len,
            "target": 34962,
        }));
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": nrm_offset,
            "byteLength": nrm_len,
            "target": 34962,
        }));
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": uv_offset,
            "byteLength": uv_len,
            "target": 34962,
        }));
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": idx_offset,
            "byteLength": idx_len,
            "target": 34963,
        }));

        let (pos_min, pos_max) = min_max3(&mesh.positions);
        let (nrm_min, nrm_max) = min_max3(&mesh.normals);
        let (uv_min, uv_max) = min_max2(&mesh.tex_coords);
        let idx_max = mesh.indices.iter().copied().max().unwrap_or(0);

        let acc_base = accessors.len();
        accessors.push(serde_json::json!({
            "bufferView": view_base,
            "componentType": 5126,
            "count": mesh.positions.len(),
            "type": "VEC3",
            "min": pos_min,
            "max": pos_max,
        }));
        accessors.push(serde_json::json!({
            "bufferView": view_base + 1,
            "componentType": 5126,
            "count": mesh.normals.len(),
            "type": "VEC3",
            "min": nrm_min,
            "max": nrm_max,
        }));
        accessors.push(serde_json::json!({
            "bufferView": view_base + 2,
            "componentType": 5126,
            "count": mesh.tex_coords.len(),
            "type": "VEC2",
            "min": uv_min,
            "max": uv_max,
        }));
        accessors.push(serde_json::json!({
            "bufferView": view_base + 3,
            "componentType": 5125,
            "count": mesh.indices.len(),
            "type": "SCALAR",
            "min": [0],
            "max": [idx_max],
        }));

        materials.push(serde_json::json!({
            "name": mesh.name,
            "pbrMetallicRoughness": {
                "baseColorFactor": mesh.base_color,
                "metallicFactor": mesh.metallic,
                "roughnessFactor": mesh.roughness,
            },
            "emissiveFactor": mesh.emissive,
        }));

        gl_meshes.push(serde_json::json!({
            "name": mesh.name,
            "primitives": [{
                "attributes": {
                    "POSITION": acc_base,
                    "NORMAL": acc_base + 1,
                    "TEXCOORD_0": acc_base + 2,
                },
                "indices": acc_base + 3,
                "material": mesh_index,
            }],
        }));
        nodes.push(serde_json::json!({ "mesh": mesh_index }));
        node_indices.push(mesh_index);
    }

    let root = serde_json::json!({
        "asset": { "version": "2.0", "generator": "redline" },
        "scene": 0,
        "scenes": [{ "nodes": node_indices }],
        "nodes": nodes,
        "meshes": gl_meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{ "byteLength": bin.len() }],
    });

    let mut json = serde_json::to_vec(&root)?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }

    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    write_u32_le(&mut out, GLB_MAGIC);
    write_u32_le(&mut out, GLB_VERSION);
    write_u32_le(&mut out, total as u32);
    write_u32_le(&mut out, json.len() as u32);
    write_u32_le(&mut out, CHUNK_JSON);
    out.extend_from_slice(&json);
    write_u32_le(&mut out, bin.len() as u32);
    write_u32_le(&mut out, CHUNK_BIN);
    out.extend_from_slice(&bin);
    Ok(out)
}

/// Write a GLB with one node per mesh. The first scene owns every node.
pub fn write_glb(path: &Path, meshes: &[MeshData]) -> io::Result<()> {
    let out = encode_glb(meshes)?;
    #[cfg(target_arch = "wasm32")]
    {
        blade_engine::vfs::mount(path, out);
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, out)
    }
}

#[cfg(test)]
mod tests {
    use super::{GLB_MAGIC, MeshData, write_glb};
    use std::fs;

    #[test]
    fn writes_triangle_glb() {
        let dir = std::env::temp_dir().join("redline-glb-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("tri.glb");
        let mesh = MeshData {
            name: "tri".into(),
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tex_coords: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            base_color: [1.0, 0.0, 0.0, 1.0],
            metallic: 0.0,
            roughness: 0.8,
            emissive: [0.0; 3],
        };
        write_glb(&path, &[mesh]).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() > 12);
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert_eq!(magic, GLB_MAGIC);
        let _ = fs::remove_file(path);
    }
}
