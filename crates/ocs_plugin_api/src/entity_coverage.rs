//! Host-owned coverage generated from the traced entity registry and an
//! explicit policy. `unmapped` fields remain accessible through the typed
//! `CadDocument` snapshot but are not promised by the Python document model.

pub use crate::entity_coverage_types::*;

pub const EMBEDDED_ENTITY_COVERAGE_JSON: &str =
    include_str!(concat!(env!("OUT_DIR"), "/entity_coverage.json"));

pub fn get_embedded_entity_coverage_json() -> &'static str {
    EMBEDDED_ENTITY_COVERAGE_JSON
}

/// Read-only JSON serialization of one typed entity. The outer object
/// has one key: the `EntityType` variant name used by the coverage catalog.
/// JSON does not preserve non-finite floating-point values, so this is an
/// inspection format and must remain separate from the typed entity write path.
#[cfg(feature = "host")]
pub fn entity_snapshot(entity: &crate::host::EntityType) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(entity)
}

#[cfg(feature = "host")]
fn finite_vector(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    if value.x.is_finite() && value.y.is_finite() && value.z.is_finite() { Ok(()) }
    else { Err(format!("{name} must contain finite coordinates")) }
}

#[cfg(feature = "host")]
fn unit_direction(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    finite_vector(name, value)?;
    let length = value.x.hypot(value.y).hypot(value.z);
    if (length - 1.0).abs() <= 1e-6 { Ok(()) }
    else { Err(format!("{name} must be a unit vector")) }
}

#[cfg(feature = "host")]
fn solid_normal(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    finite_vector(name, value)?;
    if value.x.hypot(value.y).hypot(value.z) > 0.0 { Ok(()) }
    else { Err(format!("{name} must be nonzero")) }
}

#[cfg(feature = "host")]
fn insert_scale(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value.abs() >= 1e-12 { Ok(()) }
    else { Err(format!("{name} must be finite and nonzero")) }
}

/// Validate newly created geometry for the kinds whose mapped fields have
/// host checks. Called by the Python add path before an entity enters the document.
#[cfg(feature = "host")]
pub fn validate_new_canvas_entity(entity: &crate::host::EntityType) -> Result<(), String> {
    use crate::host::EntityType;
    if entity.common().layer.trim().is_empty() { return Err("layer name is empty".into()); }
    match entity {
        EntityType::Ray(value) => {
            finite_vector("Ray.base_point", &value.base_point)?;
            unit_direction("Ray.direction", &value.direction)?;
        }
        EntityType::XLine(value) => {
            finite_vector("XLine.base_point", &value.base_point)?;
            unit_direction("XLine.direction", &value.direction)?;
        }
        EntityType::Solid(value) => {
            for (name, corner) in [
                ("first_corner", &value.first_corner), ("second_corner", &value.second_corner),
                ("third_corner", &value.third_corner), ("fourth_corner", &value.fourth_corner),
            ] { finite_vector(&format!("Solid.{name}"), corner)?; }
            solid_normal("Solid.normal", &value.normal)?;
            if !value.thickness.is_finite() { return Err("Solid.thickness must be finite".into()); }
        }
        EntityType::Face3D(value) => {
            for (name, corner) in [
                ("first_corner", &value.first_corner), ("second_corner", &value.second_corner),
                ("third_corner", &value.third_corner), ("fourth_corner", &value.fourth_corner),
            ] { finite_vector(&format!("Face3D.{name}"), corner)?; }
            if value.invisible_edges.bits() & !0x0f != 0 {
                return Err("Face3D.invisible_edges has unknown bits".into());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate geometry changed by a typed replacement before an undoable
/// transaction commits. Legacy invalid values in untouched fields remain
/// untouched, so editing a layer does not unexpectedly reject an old drawing.
#[cfg(feature = "host")]
pub fn validate_entity_mutation(
    before: &crate::host::EntityType,
    after: &crate::host::EntityType,
) -> Result<(), String> {
    use crate::host::EntityType;
    if before.common().layer != after.common().layer {
        if after.common().layer.trim().is_empty() { return Err("layer name is empty".into()); }
        if matches!(after, EntityType::Block(_) | EntityType::BlockEnd(_) | EntityType::Seqend(_)
            | EntityType::Extended(_) | EntityType::Unknown(_)) {
            return Err("entity kind does not support canvas layer edits".into());
        }
    }
    let finite3 = |name: &str, x: f64, y: f64, z: f64| {
        if x.is_finite() && y.is_finite() && z.is_finite() { Ok(()) }
        else { Err(format!("{name} must contain finite coordinates")) }
    };
    let same3 = |a: &crate::host::acadrust::types::Vector3,
                 b: &crate::host::acadrust::types::Vector3| {
        a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits() && a.z.to_bits() == b.z.to_bits()
    };
    let radius = |name: &str, value: f64| {
        if value.is_finite() && value > 0.0 { Ok(()) }
        else { Err(format!("{name} must be finite and greater than zero")) }
    };
    let changed3 = |name: &str, old: &crate::host::acadrust::types::Vector3,
                    new: &crate::host::acadrust::types::Vector3| {
        if !same3(old, new) { finite_vector(name, new) } else { Ok(()) }
    };
    match (before, after) {
        (EntityType::Point(old), EntityType::Point(new)) => {
            if !same3(&old.location, &new.location) {
                finite3("Point.location", new.location.x, new.location.y, new.location.z)?;
            }
        }
        (EntityType::Line(old), EntityType::Line(new)) => {
            if !same3(&old.start, &new.start) {
                finite3("Line.start", new.start.x, new.start.y, new.start.z)?;
            }
            if !same3(&old.end, &new.end) {
                finite3("Line.end", new.end.x, new.end.y, new.end.z)?;
            }
        }
        (EntityType::Circle(old), EntityType::Circle(new)) => {
            if !same3(&old.center, &new.center) {
                finite3("Circle.center", new.center.x, new.center.y, new.center.z)?;
            }
            if old.radius.to_bits() != new.radius.to_bits() { radius("Circle.radius", new.radius)?; }
        }
        (EntityType::Arc(old), EntityType::Arc(new)) => {
            if !same3(&old.center, &new.center) {
                finite3("Arc.center", new.center.x, new.center.y, new.center.z)?;
            }
            if old.radius.to_bits() != new.radius.to_bits() { radius("Arc.radius", new.radius)?; }
        }
        (EntityType::Ray(old), EntityType::Ray(new)) => {
            changed3("Ray.base_point", &old.base_point, &new.base_point)?;
            if !same3(&old.direction, &new.direction) { unit_direction("Ray.direction", &new.direction)?; }
        }
        (EntityType::XLine(old), EntityType::XLine(new)) => {
            changed3("XLine.base_point", &old.base_point, &new.base_point)?;
            if !same3(&old.direction, &new.direction) { unit_direction("XLine.direction", &new.direction)?; }
        }
        (EntityType::Solid(old), EntityType::Solid(new)) => {
            for (name, old_corner, new_corner) in [
                ("first_corner", &old.first_corner, &new.first_corner),
                ("second_corner", &old.second_corner, &new.second_corner),
                ("third_corner", &old.third_corner, &new.third_corner),
                ("fourth_corner", &old.fourth_corner, &new.fourth_corner),
            ] { changed3(&format!("Solid.{name}"), old_corner, new_corner)?; }
            if !same3(&old.normal, &new.normal) { solid_normal("Solid.normal", &new.normal)?; }
            if old.thickness.to_bits() != new.thickness.to_bits() && !new.thickness.is_finite() {
                return Err("Solid.thickness must be finite".into());
            }
        }
        (EntityType::Face3D(old), EntityType::Face3D(new)) => {
            for (name, old_corner, new_corner) in [
                ("first_corner", &old.first_corner, &new.first_corner),
                ("second_corner", &old.second_corner, &new.second_corner),
                ("third_corner", &old.third_corner, &new.third_corner),
                ("fourth_corner", &old.fourth_corner, &new.fourth_corner),
            ] { changed3(&format!("Face3D.{name}"), old_corner, new_corner)?; }
            if old.invisible_edges != new.invisible_edges && new.invisible_edges.bits() & !0x0f != 0 {
                return Err("Face3D.invisible_edges has unknown bits".into());
            }
        }
        (EntityType::Insert(old), EntityType::Insert(new)) => {
            if old.block_name != new.block_name || old.attributes != new.attributes ||
                old.view_rep_handle != new.view_rep_handle || old.seqend_handle != new.seqend_handle {
                return Err("Insert block identity and attached records cannot change in a geometry transaction".into());
            }
            changed3("Insert.insert_point", &old.insert_point, &new.insert_point)?;
            if !same3(&old.normal, &new.normal) { solid_normal("Insert.normal", &new.normal)?; }
            for (name, before, after) in [
                ("x_scale", old.x_scale(), new.x_scale()),
                ("y_scale", old.y_scale(), new.y_scale()),
                ("z_scale", old.z_scale(), new.z_scale()),
            ] {
                if before.to_bits() != after.to_bits() { insert_scale(&format!("Insert.{name}"), after)?; }
            }
            for (name, before, after) in [
                ("rotation", old.rotation, new.rotation),
                ("column_spacing", old.column_spacing, new.column_spacing),
                ("row_spacing", old.row_spacing, new.row_spacing),
            ] {
                if before.to_bits() != after.to_bits() && !after.is_finite() {
                    return Err(format!("Insert.{name} must be finite"));
                }
            }
            if (old.column_count != new.column_count && new.column_count == 0) ||
                (old.row_count != new.row_count && new.row_count == 0) {
                return Err("Insert array counts must be greater than zero".into());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Clone a canvas entity with a new layer, preserving its other snapshot
/// fields. Internal records and opaque fallbacks are excluded.
#[cfg(feature = "host")]
pub fn patch_canvas_layer(entity: &crate::host::EntityType, layer: &str) -> Result<crate::host::EntityType, String> {
    use crate::host::EntityType;
    if layer.trim().is_empty() { return Err("layer name is empty".into()); }
    if matches!(entity, EntityType::Block(_) | EntityType::BlockEnd(_) | EntityType::Seqend(_)
        | EntityType::Extended(_) | EntityType::Unknown(_)) {
        return Err("entity kind does not support canvas layer edits".into());
    }
    let mut changed = entity.clone();
    changed.common_mut().layer = layer.to_owned();
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{get_embedded_type_registry_json, TypeId, TypeRegistry};

    #[test]
    fn every_entity_variant_has_exactly_one_classification() {
        let catalog: EntityCoverageCatalog = serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let registry: TypeRegistry = serde_json::from_str(get_embedded_type_registry_json()).unwrap();
        let variants = &registry.types[&TypeId::new("EntityType")].variants;
        assert_eq!(catalog.entity_kinds.len(), variants.len());
        let mut names = std::collections::HashSet::new();
        for entry in &catalog.entity_kinds {
            assert!(names.insert(&entry.kind), "duplicate kind {}", entry.kind);
            assert!(variants.iter().any(|v| v.name == entry.kind));
            assert!(entry.properties.iter().any(|p| p.name == "handle" && p.model_access == ModelAccess::ReadOnly));
            assert!(entry.properties.iter().any(|p| p.name == "kind" && p.model_access == ModelAccess::ReadOnly));
            let mut source_paths = std::collections::HashSet::new();
            for property in &entry.properties {
                assert!(source_paths.insert(&property.source_path), "duplicate source path in {}: {}", entry.kind, property.source_path);
                assert!(property.snapshot_readable, "unreadable traced property in {}: {}", entry.kind, property.name);
                if property.model_access == ModelAccess::ReadWrite {
                    assert!(matches!(property.validation.as_str(), "type_conversion_only" | "transaction_geometry" | "transaction_nonempty"));
                }
            }
        }
    }

    #[test]
    fn unsupported_kinds_have_no_editable_properties() {
        let catalog: EntityCoverageCatalog = serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        for entry in &catalog.entity_kinds {
            if entry.scope != EntityScope::Canvas || !matches!(entry.kind.as_str(),
                "Point" | "Line" | "Circle" | "Arc" | "Ellipse" | "Polyline" |
                "Polyline2D" | "Polyline3D" | "LwPolyline" | "Spline" | "Text" | "MText" |
                "Ray" | "XLine" | "Solid" | "Face3D" | "Insert") {
                assert!(entry.properties.iter().all(|p| p.model_access != ModelAccess::ReadWrite
                    || (entry.scope == EntityScope::Canvas && p.name == "layer")), "{}", entry.kind);
            }
        }
    }

    #[test]
    fn catalog_names_the_fields_with_transaction_geometry_checks() {
        let catalog: EntityCoverageCatalog = serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let checked: std::collections::HashSet<_> = catalog.entity_kinds.iter()
            .flat_map(|entry| entry.properties.iter()
                .filter(|property| property.validation == "transaction_geometry")
                .map(|property| format!("{}.{}", entry.kind, property.name)))
            .collect();
        assert_eq!(checked, std::collections::HashSet::from([
            "Point.location".to_owned(), "Line.start".to_owned(), "Line.end".to_owned(),
            "Circle.center".to_owned(), "Circle.radius".to_owned(),
            "Arc.center".to_owned(), "Arc.radius".to_owned(),
            "Ray.base_point".to_owned(), "Ray.direction".to_owned(),
            "XLine.base_point".to_owned(), "XLine.direction".to_owned(),
            "Solid.first_corner".to_owned(), "Solid.second_corner".to_owned(),
            "Solid.third_corner".to_owned(), "Solid.fourth_corner".to_owned(),
            "Solid.normal".to_owned(), "Solid.thickness".to_owned(),
            "Face3D.first_corner".to_owned(), "Face3D.second_corner".to_owned(),
            "Face3D.third_corner".to_owned(), "Face3D.fourth_corner".to_owned(),
            "Face3D.invisible_edges".to_owned(),
            "Insert.insert_point".to_owned(), "Insert.x_scale".to_owned(),
            "Insert.y_scale".to_owned(), "Insert.z_scale".to_owned(),
            "Insert.rotation".to_owned(), "Insert.normal".to_owned(),
            "Insert.column_count".to_owned(), "Insert.row_count".to_owned(),
            "Insert.column_spacing".to_owned(), "Insert.row_spacing".to_owned(),
        ]));
    }

    #[test]
    fn layer_write_coverage_matches_canvas_scope() {
        let catalog: EntityCoverageCatalog = serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        assert_eq!(catalog.entity_kinds.iter().filter(|entry| entry.scope == EntityScope::Canvas).count(), 43);
        for entry in &catalog.entity_kinds {
            let layer = entry.properties.iter().find(|property| property.name == "layer").unwrap();
            assert_eq!(layer.model_access == ModelAccess::ReadWrite, entry.scope == EntityScope::Canvas);
            if entry.scope == EntityScope::Canvas {
                assert_eq!(layer.validation, "transaction_nonempty");
            }
        }
    }

    #[test]
    fn insert_catalog_separates_transform_writes_from_block_identity() {
        let catalog: EntityCoverageCatalog = serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let insert = catalog.entity_kinds.iter().find(|entry| entry.kind == "Insert").unwrap();
        let block_name = insert.properties.iter().find(|property| property.name == "block_name").unwrap();
        assert_eq!(block_name.model_access, ModelAccess::ReadOnly);
        assert_eq!(block_name.validation, "none");
        assert!(insert.properties.iter().find(|property| property.name == "attributes").unwrap().model_access == ModelAccess::Unmapped);
        assert!(insert.properties.iter().find(|property| property.name == "insert_point").unwrap().model_access == ModelAccess::ReadWrite);
    }

    #[cfg(feature = "host")]
    #[test]
    fn unsupported_canvas_kind_has_typed_snapshot_fields() {
        use crate::host::acadrust;
        let hatch = acadrust::EntityType::Hatch(acadrust::entities::Hatch::default());
        let snapshot = entity_snapshot(&hatch).unwrap();
        let fields = snapshot.get("Hatch").and_then(|value| value.as_object()).unwrap();
        assert!(fields.contains_key("common"));
        assert!(fields.len() > 1);
    }

    #[cfg(feature = "host")]
    #[test]
    fn changed_geometry_is_validated_without_rejecting_untouched_legacy_values() {
        use crate::host::acadrust;
        let mut before = acadrust::entities::Circle::default();
        before.radius = f64::NAN;
        let mut layer_only = before.clone();
        layer_only.common.layer = "OTHER".into();
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Circle(before.clone()),
            &acadrust::EntityType::Circle(layer_only),
        ).is_ok());
        let mut bad_radius = before.clone();
        bad_radius.radius = -1.0;
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Circle(before),
            &acadrust::EntityType::Circle(bad_radius),
        ).unwrap_err().contains("Circle.radius"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn new_phase_seven_shapes_reject_invalid_geometry_and_keep_legacy_fields() {
        use crate::host::acadrust::{self, types::Vector3};
        let mut ray = acadrust::entities::Ray::default();
        ray.direction = Vector3::new(2.0, 0.0, 0.0);
        assert!(validate_new_canvas_entity(&acadrust::EntityType::Ray(ray.clone())).unwrap_err().contains("Ray.direction"));
        let before = acadrust::EntityType::Ray(ray.clone());
        ray.common.layer = "CONSTRUCTION".into();
        assert!(validate_entity_mutation(&before, &acadrust::EntityType::Ray(ray.clone())).is_ok());
        ray.direction = Vector3::new(0.0, 1.0, 0.0);
        assert!(validate_entity_mutation(&before, &acadrust::EntityType::Ray(ray)).is_ok());

        let mut xline = acadrust::entities::XLine::default();
        xline.base_point.x = f64::NAN;
        assert!(validate_new_canvas_entity(&acadrust::EntityType::XLine(xline)).unwrap_err().contains("XLine.base_point"));

        let solid = acadrust::entities::Solid::new(Vector3::ZERO, Vector3::UNIT_X, Vector3::UNIT_Y, Vector3::ZERO);
        let mut changed = solid.clone();
        changed.thickness = f64::INFINITY;
        assert!(validate_entity_mutation(&acadrust::EntityType::Solid(solid), &acadrust::EntityType::Solid(changed))
            .unwrap_err().contains("Solid.thickness"));

        let face = acadrust::entities::Face3D::new(Vector3::ZERO, Vector3::UNIT_X, Vector3::UNIT_Y, Vector3::ZERO);
        let mut changed = face.clone();
        changed.invisible_edges = acadrust::entities::InvisibleEdgeFlags::from_bits(0x10);
        assert!(validate_entity_mutation(&acadrust::EntityType::Face3D(face), &acadrust::EntityType::Face3D(changed))
            .unwrap_err().contains("Face3D.invisible_edges"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn insert_transaction_checks_transforms_and_protects_references() {
        use crate::host::acadrust::{self, types::Vector3};
        let before = acadrust::entities::Insert::new("DOOR", Vector3::new(1.0, 2.0, 0.0));
        let mut moved = before.clone();
        moved.insert_point = Vector3::new(5.0, 6.0, 0.0);
        moved.set_x_scale(2.0);
        assert!(validate_entity_mutation(&acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(moved)).is_ok());
        let mut bad = before.clone();
        bad.set_y_scale(f64::NAN);
        assert!(validate_entity_mutation(&acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(bad)).unwrap_err().contains("Insert.y_scale"));
        let mut bad = before.clone();
        bad.column_count = 0;
        assert!(validate_entity_mutation(&acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(bad)).unwrap_err().contains("array counts"));
        let mut bad = before.clone();
        bad.block_name = "OTHER".into();
        assert!(validate_entity_mutation(&acadrust::EntityType::Insert(before),
            &acadrust::EntityType::Insert(bad)).unwrap_err().contains("block identity"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn canvas_layer_patch_preserves_unmapped_hatch_fields() {
        use crate::host::acadrust;
        let hatch = acadrust::EntityType::Hatch(acadrust::entities::Hatch::default());
        let patched = patch_canvas_layer(&hatch, "HATCHES").unwrap();
        assert_eq!(patched.common().layer, "HATCHES");
        assert_eq!(hatch.common().layer, "0");
        let mut expected = hatch.clone();
        expected.common_mut().layer = "HATCHES".into();
        assert_eq!(patched, expected);
        assert!(patch_canvas_layer(&hatch, " ").is_err());
    }
}
