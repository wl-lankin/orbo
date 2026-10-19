use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

const CUBE: [[f32; 3]; 8] = [
    [0.0, 0.0, 0.0],
    [10.0, 0.0, 0.0],
    [10.0, 20.0, 0.0],
    [0.0, 20.0, 0.0],
    [0.0, 0.0, 30.0],
    [10.0, 0.0, 30.0],
    [10.0, 20.0, 30.0],
    [0.0, 20.0, 30.0],
];
const FACES: [[usize; 3]; 12] = [
    [0, 2, 1],
    [0, 3, 2],
    [4, 5, 6],
    [4, 6, 7],
    [0, 1, 5],
    [0, 5, 4],
    [1, 2, 6],
    [1, 6, 5],
    [2, 3, 7],
    [2, 7, 6],
    [3, 0, 4],
    [3, 4, 7],
];

fn cube_ascii_stl() -> String {
    let mut s = String::from("solid cube\n");
    for f in FACES {
        s.push_str("facet normal 0 0 0\n outer loop\n");
        for i in f {
            let p = CUBE[i];
            s.push_str(&format!("  vertex {} {} {}\n", p[0], p[1], p[2]));
        }
        s.push_str(" endloop\nendfacet\n");
    }
    s.push_str("endsolid cube\n");
    s
}

fn cube_obj() -> String {
    let mut s = String::from("# cube\no cube\n");
    for p in CUBE {
        s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for f in FACES {
        s.push_str(&format!("f {} {} {}\n", f[0] + 1, f[1] + 1, f[2] + 1));
    }
    s
}

fn binary_stl_count(path: &Path) -> u32 {
    let b = fs::read(path).unwrap();
    assert_eq!(b.len(), 84 + 50 * u32::from_le_bytes([b[80], b[81], b[82], b[83]]) as usize, "binary STL size matches its count");
    u32::from_le_bytes([b[80], b[81], b[82], b[83]])
}

#[test]
fn stl_obj_glb_stl_keeps_triangle_count() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("cube.stl");
    fs::write(&src, cube_ascii_stl()).unwrap();

    let obj = run(&src, "model_obj").unwrap();
    assert_eq!(obj.extension().unwrap(), "obj");
    let text = fs::read_to_string(&obj).unwrap();
    assert_eq!(text.lines().filter(|l| l.starts_with("v ")).count(), 8, "STL corners are welded");
    assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 12);

    let glb = run(&obj, "model_glb").unwrap();
    assert_eq!(&fs::read(&glb).unwrap()[..4], b"glTF");

    let stl = run(&glb, "model_stl").unwrap();
    assert_eq!(binary_stl_count(&stl), 12);
}

#[test]
fn quads_are_triangulated_and_negative_indices_work() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("quad.obj");
    fs::write(&src, "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4/1/1 -3/2/1 -2/3/1 -1/4/1\n").unwrap();
    let stl = run(&src, "model_stl").unwrap();
    assert_eq!(binary_stl_count(&stl), 2);
}

#[test]
fn ply_roundtrip_and_ascii_ply_with_quads() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("cube.obj");
    fs::write(&obj, cube_obj()).unwrap();
    let ply = run(&obj, "model_ply").unwrap();
    assert!(fs::read(&ply).unwrap().starts_with(b"ply\nformat binary_little_endian"));
    let back = run(&ply, "model_stl").unwrap();
    assert_eq!(binary_stl_count(&back), 12);

    let ascii = dir.path().join("quad.ply");
    fs::write(
        &ascii,
        "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255\n1 0 0 255\n1 1 0 255\n0 1 0 255\n4 0 1 2 3\n",
    )
    .unwrap();
    assert_eq!(binary_stl_count(&run(&ascii, "model_stl").unwrap()), 2);
}

#[test]
fn threemf_roundtrip_keeps_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let obj = dir.path().join("cube.obj");
    fs::write(&obj, cube_obj()).unwrap();
    let mf = run(&obj, "model_3mf").unwrap();
    assert_eq!(mf.extension().unwrap(), "3mf");
    let stl = run(&mf, "model_stl").unwrap();
    assert_eq!(binary_stl_count(&stl), 12);
    let back = run(&stl, "model_obj").unwrap();
    let text = fs::read_to_string(back).unwrap();
    assert!(text.contains("v 10 20 30"), "coordinates stay unchanged");
}

#[test]
fn threemf_build_item_transform_is_applied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("moved.3mf");
    {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        zip.start_file("3D/3dmodel.model", o).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?><model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 5 6 7"/></build></model>"#,
        )
        .unwrap();
        zip.finish().unwrap();
    }
    let obj = run(&path, "model_obj").unwrap();
    assert!(fs::read_to_string(obj).unwrap().contains("v 5 6 7"));
}

#[test]
fn gltf_with_data_uri_is_read() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut bin = Vec::new();
    for c in [0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&c.to_le_bytes());
    }
    let mut b64 = String::new();
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for ch in bin.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= ch.len() {
                b64.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"translation":[10,0,0]}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"buffers":[{{"byteLength":36,"uri":"data:application/octet-stream;base64,{b64}"}}],"bufferViews":[{{"buffer":0,"byteLength":36}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}]}}"#
    );
    let path = dir.path().join("tri.gltf");
    fs::File::create(&path).unwrap().write_all(json.as_bytes()).unwrap();
    let obj = run(&path, "model_obj").unwrap();
    let text = fs::read_to_string(obj).unwrap();
    assert!(text.contains("v 10 0 0") && text.contains("f 1 2 3"), "{text}");
}

#[test]
fn collada_polylist_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quad.dae");
    fs::write(
        &path,
        r##"<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema"><library_geometries><geometry id="g"><mesh>
<source id="pos"><float_array id="pos-array" count="12">0 0 0 1 0 0 1 1 0 0 1 0</float_array><technique_common><accessor source="#pos-array" count="4" stride="3"/></technique_common></source>
<vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
<polylist count="1"><input semantic="VERTEX" source="#v" offset="0"/><vcount>4</vcount><p>0 1 2 3</p></polylist>
</mesh></geometry></library_geometries></COLLADA>"##,
    )
    .unwrap();
    assert_eq!(binary_stl_count(&run(&path, "model_stl").unwrap()), 2);
}

#[test]
fn preview_png_shows_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("cube.obj");
    fs::write(&src, cube_obj()).unwrap();
    let png = run(&src, "model_preview").unwrap();
    let img = image::open(&png).unwrap().to_rgb8();
    assert_eq!(img.dimensions(), (1024, 1024));
    assert_eq!(img.get_pixel(2, 2).0, [255, 255, 255], "background stays white");
    assert!(img.pixels().filter(|p| p.0 != [255, 255, 255]).count() > 20_000, "the model is visible");
}

#[test]
fn empty_model_reports_a_german_error() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("leer.obj");
    fs::write(&src, "# nichts\n").unwrap();
    let err = run(&src, "model_stl").unwrap_err().to_string();
    assert!(err.contains("keine Dreiecke"), "{err}");
}

#[test]
fn dae_with_xml_declaration_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("teil.dae");
    std::fs::write(&p, "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\"></COLLADA>\n").unwrap();
    assert_eq!(orbo_core::format::detect(&p).unwrap().0, orbo_core::format::Format::Dae);
}
