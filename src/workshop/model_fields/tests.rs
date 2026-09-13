use super::*;
use crate::entities::model_rig::ModelRig;
use crate::workshop::document::{fields, patch, Field, Patch};

const SIDECAR: &str = "assets/models/ships/courier.weathered.toml";

fn key(value: &str) -> Segment {
    Segment::Key(value.into())
}

fn field(source: &str, path: &[Segment]) -> Field {
    fields(source, SIDECAR)
        .unwrap()
        .into_iter()
        .find(|field| field.path == path)
        .unwrap()
}

fn edit(source: &str, path: &[Segment], value: &str) -> Result<String, String> {
    patch(
        source,
        &Patch {
            document_path: SIDECAR.into(),
            path: path.into(),
            expected_source: source.into(),
            value_source: value.into(),
        },
    )
}

#[test]
fn model_float_edit_preserves_every_other_byte_and_uses_the_actual_runtime_type() {
    let source = "# 船のリグ\r\n[base] # preserve\noffset = [1, 2, 3] # integer spelling\r\nscale=[2, 2, 2]\n[markers.\"forward emitter\"]\r\nposition = [0, 0, -2]\ndirection=[0, 0, -1] # keep\r\n";
    let path = [key("base"), key("offset"), Segment::Index(0)];
    let current = field(source, &path);
    assert_eq!(current.descriptor.kind, "float");
    assert_eq!(current.source, "1");
    assert!(current.runtime_owned);
    assert_eq!(
        current.descriptor.live_mutability,
        crate::inspector::LiveMutability::RecreateRequired
    );
    let edited = edit(source, &path, "1.5").unwrap();
    assert_eq!(edited, source.replacen("[1, 2, 3]", "[1.5, 2, 3]", 1));
    let actual = ModelRig::from_toml(&edited).unwrap();
    assert_eq!(actual.base.offset, [1.5, 2.0, 3.0]);
    assert_eq!(actual.markers["forward emitter"].position, [0.0, 0.0, -2.0]);
    assert_eq!(edit(&edited, &path, "1").unwrap(), source);
}

#[test]
fn model_fields_repair_malformed_scalars_without_parsing_an_invalid_whole_rig_first() {
    use Segment::Index;
    let mut source = "# retained\r\n[base]\noffset=['bad',2,3]\n[extents]\nmin=['bad',0,0]\nmax=[1,1,1]\nsize=[1,1,1]\n[base_build]\nbudget_mb='bad'\n[markers.fore]\nposition=[0,'bad',0]\ndirection=[0,0,-1]\n[[target_points]]\nposition=[0,0,'bad']\n[[lod]]\nmodel=9\nmax_distance='bad'\ncolour=[1,'bad',0]\nsize=[1,2,'bad']\nrotation=['bad',0,0]\nscale=[1,'bad',1]\n[ lod.generate ]\nratio='bad'\ntexture_size='bad'\n[ lod.capture ]\nresolution='bad'\npitch='bad'\n".to_owned();
    assert!(ModelRig::from_toml(&source).is_err());
    for (path, value) in [
        (vec![key("base"), key("offset"), Index(0)], "1.5"),
        (vec![key("extents"), key("min"), Index(0)], "-1.5"),
        (vec![key("base_build"), key("budget_mb")], "16.5"),
        (
            vec![key("markers"), key("fore"), key("position"), Index(1)],
            "2.5",
        ),
        (
            vec![key("target_points"), Index(0), key("position"), Index(2)],
            "3.5",
        ),
        (
            vec![key("lod"), Index(0), key("model")],
            "'assets/models/courier.glb'",
        ),
        (vec![key("lod"), Index(0), key("max_distance")], "100.5"),
        (vec![key("lod"), Index(0), key("colour"), Index(1)], "0.5"),
        (vec![key("lod"), Index(0), key("size"), Index(2)], "3.5"),
        (vec![key("lod"), Index(0), key("rotation"), Index(0)], "0.5"),
        (vec![key("lod"), Index(0), key("scale"), Index(1)], "1.5"),
        (
            vec![key("lod"), Index(0), key("generate"), key("ratio")],
            "0.25",
        ),
        (
            vec![key("lod"), Index(0), key("generate"), key("texture_size")],
            "256",
        ),
        (
            vec![key("lod"), Index(0), key("capture"), key("resolution")],
            "512",
        ),
        (
            vec![key("lod"), Index(0), key("capture"), key("pitch")],
            "20.5",
        ),
    ] {
        assert!(field(&source, &path).runtime_owned, "{path:?}");
        source = edit(&source, &path, value).unwrap();
    }
    let actual = ModelRig::from_toml(&source).unwrap();
    assert_eq!(actual.base.offset[0], 1.5);
    assert_eq!(actual.extents.unwrap().min[0], -1.5);
    assert_eq!(actual.base_build.unwrap().budget_mb, 16.5);
    assert_eq!(actual.markers["fore"].position[1], 2.5);
    assert_eq!(actual.target_points[0].position[2], 3.5);
    assert_eq!(actual.lod[0].colour, Some(vec![1.0, 0.5, 0.0]));
    assert_eq!(
        actual.lod[0].generate.as_ref().unwrap().texture_size,
        Some(256)
    );
    assert_eq!(
        actual.lod[0].capture.as_ref().unwrap().resolution,
        Some(512)
    );
    assert!(source.starts_with("# retained\r\n"));
    assert!(source.contains("[ lod.generate ]"));
}

#[test]
fn only_real_runtime_defaults_are_present_and_entity_fallbacks_remain_unspecified() {
    let source = "[base]\noffset=[9,9,9]\nrotation=[9,9,9]\nscale=[9,9,9]\n[extents]\nmin=[0,0,0]\nmax=[1,1,1]\nsize=[1,1,1]\n[base_build]\nbudget_mb=16\n[markers.fore]\nposition=[0,0,0]\ndirection=[0,0,-1]\n[[target_points]]\nposition=[0,0,0]\n[[lod]]\nmax_distance=50\nshape='sphere'\ntier_rig='identity'\ncolour=[1,1,1]\nradius=1\nsize=[1,1,1]\nminor_radius=1\nemissive=1\nrotation=[0,0,0]\nscale=[1,1,1]\n[ lod.generate ]\nsource='assets/models/courier.glb'\nratio=1\nerror=1\ntexture_size=512\nremesh_voxel_size=1\n[ lod.capture ]\nsource='assets/models/courier.glb'\nyaw_views=8\nresolution=256\npitch=20\n";
    let baseline = ModelRig::from_toml("").unwrap();
    for field in fields(source, SIDECAR).unwrap() {
        assert!(field.runtime_owned, "{:?}", field.path);
        let expected = match field.path.as_slice() {
            [Segment::Key(table), Segment::Key(name), Segment::Index(axis)] if table == "base" => {
                let value = match name.as_str() {
                    "offset" => baseline.base.offset[*axis],
                    "rotation" => baseline.base.rotation[*axis],
                    "scale" => baseline.base.scale[*axis],
                    _ => panic!("unexpected base field"),
                };
                Some(value.to_string())
            }
            _ => None,
        };
        assert_eq!(
            field.descriptor.default_source, expected,
            "{:?}",
            field.path
        );
    }
    assert!(toml::from_str::<Marker>("").is_err());
    assert!(toml::from_str::<TargetPoint>("").is_err());
    assert!(toml::from_str::<Extents>("").is_err());
    assert!(toml::from_str::<BaseBuild>("").is_err());
}

#[test]
fn model_enums_and_unsigned_integers_use_runtime_deserialization_for_refusals() {
    let source = "[[lod]]\nshape='sphere'\ntier_rig='identity'\n[lod.generate]\ntexture_size=256\n[lod.capture]\nyaw_views=8\n";
    let shape = [key("lod"), Segment::Index(0), key("shape")];
    let rig = [key("lod"), Segment::Index(0), key("tier_rig")];
    for value in ["'Sphere'", "'cube'", "3", "true"] {
        assert!(edit(source, &shape, value).is_err(), "{value}");
    }
    for value in ["'Identity'", "'default'", "3"] {
        assert!(edit(source, &rig, value).is_err(), "{value}");
    }
    let torus = toml::Value::try_from(MeshShape::Torus).unwrap().to_string();
    let baked = toml::Value::try_from(TierRig::Baked).unwrap().to_string();
    let edited = edit(source, &shape, &torus).unwrap();
    let edited = edit(&edited, &rig, &baked).unwrap();
    let actual = ModelRig::from_toml(&edited).unwrap();
    assert_eq!(actual.lod[0].shape, Some(MeshShape::Torus));
    assert_eq!(actual.lod[0].tier_rig, Some(TierRig::Baked));
    for path in [
        vec![
            key("lod"),
            Segment::Index(0),
            key("generate"),
            key("texture_size"),
        ],
        vec![
            key("lod"),
            Segment::Index(0),
            key("capture"),
            key("yaw_views"),
        ],
    ] {
        assert_eq!(field(source, &path).descriptor.kind, "integer");
        for value in ["1.5", "-1", "4294967296", "'512'"] {
            assert!(edit(source, &path, value).is_err(), "{path:?} {value}");
        }
        assert!(ModelRig::from_toml(&edit(source, &path, "512").unwrap()).is_ok());
    }
}

#[test]
fn model_metadata_is_scoped_to_portable_sidecar_paths_and_known_scalar_fields() {
    let source = "[base]\noffset=[1,2,3]\nextension=4\n";
    let path = [key("base"), key("offset"), Segment::Index(0)];
    for document_path in [
        "scenarios.toml",
        "assets/worlds/test.model.toml",
        "assets/entities/test.model.toml",
        "assets/models/test.toml",
        "assets/models/.model.toml",
        "assets/models/test..toml",
        "assets/models/../test.model.toml",
        "assets/models//test.model.toml",
        "assets/models/nested\\test.model.toml",
        "assets/models/drive:test.model.toml",
        "assets/models/nested /test.model.toml",
        "assets/models/test\n.model.toml",
    ] {
        let field = fields(source, document_path).unwrap().remove(0);
        assert!(!field.runtime_owned, "{document_path}");
        assert_eq!(field.descriptor.kind, "integer");
        assert!(patch(
            source,
            &Patch {
                document_path: document_path.into(),
                path: path.to_vec(),
                expected_source: source.into(),
                value_source: "1.5".into()
            }
        )
        .is_err());
    }
    let unknown = field(source, &[key("base"), key("extension")]);
    assert!(!unknown.runtime_owned);
    assert_eq!(unknown.descriptor.kind, "integer");
    assert_eq!(unknown.descriptor.default_source, None);
    assert_eq!(
        edit(source, &unknown.path, "5").unwrap(),
        source.replace("extension=4", "extension=5")
    );
    assert!(field(source, &path).runtime_owned);
}
