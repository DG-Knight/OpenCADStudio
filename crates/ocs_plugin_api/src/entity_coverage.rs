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
pub fn entity_snapshot(
    entity: &crate::host::EntityType,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(entity)
}

#[cfg(feature = "host")]
fn finite_vector(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    if value.x.is_finite() && value.y.is_finite() && value.z.is_finite() {
        Ok(())
    } else {
        Err(format!("{name} must contain finite coordinates"))
    }
}

#[cfg(feature = "host")]
fn unit_direction(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    finite_vector(name, value)?;
    let length = value.x.hypot(value.y).hypot(value.z);
    if (length - 1.0).abs() <= 1e-6 {
        Ok(())
    } else {
        Err(format!("{name} must be a unit vector"))
    }
}

#[cfg(feature = "host")]
fn solid_normal(name: &str, value: &crate::host::acadrust::types::Vector3) -> Result<(), String> {
    finite_vector(name, value)?;
    if value.x.hypot(value.y).hypot(value.z) > 0.0 {
        Ok(())
    } else {
        Err(format!("{name} must be nonzero"))
    }
}

#[cfg(feature = "host")]
fn insert_scale(name: &str, value: f64) -> Result<(), String> {
    if value.is_finite() && value.abs() >= 1e-12 {
        Ok(())
    } else {
        Err(format!("{name} must be finite and nonzero"))
    }
}

#[cfg(feature = "host")]
fn validate_hatch_paths(paths: &[crate::host::acadrust::entities::BoundaryPath]) -> Result<(), String> {
    use crate::host::acadrust::entities::BoundaryEdge;
    use crate::host::acadrust::types::Vector2;
    let close = |a: Vector2, b: Vector2| (a.x - b.x).hypot(a.y - b.y) <= 1e-8;
    let ellipse_point = |center: Vector2, major: Vector2, ratio: f64, angle: f64| {
        Vector2::new(
            center.x + major.x * angle.cos() - major.y * ratio * angle.sin(),
            center.y + major.y * angle.cos() + major.x * ratio * angle.sin(),
        )
    };
    if paths.is_empty() {
        return Err("Hatch requires at least one boundary path".into());
    }
    for (pi, path) in paths.iter().enumerate() {
        if path.edges.is_empty() {
            return Err(format!("Hatch boundary path {pi} has no edges"));
        }
        if path.flags.is_not_closed() {
            return Err(format!("Hatch boundary path {pi} is marked open"));
        }
        if path.flags.bits() & !0x1ff != 0 {
            return Err(format!("Hatch boundary path {pi} has unknown flag bits"));
        }
        let mut endpoints = Vec::with_capacity(path.edges.len());
        for (ei, edge) in path.edges.iter().enumerate() {
            let pair = match edge {
                BoundaryEdge::Line(line) => {
                    if !line.start.x.is_finite() || !line.start.y.is_finite() || !line.end.x.is_finite() || !line.end.y.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} line has non-finite coordinates"));
                    }
                    if (line.start.x - line.end.x).hypot(line.start.y - line.end.y) < 1e-12 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} line has zero length"));
                    }
                    (line.start, line.end)
                }
                BoundaryEdge::CircularArc(arc) => {
                    if !arc.center.x.is_finite() || !arc.center.y.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} arc center has non-finite coordinates"));
                    }
                    if !arc.radius.is_finite() || arc.radius <= 0.0 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} arc radius must be finite and positive"));
                    }
                    if !arc.start_angle.is_finite() || !arc.end_angle.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} arc angles must be finite"));
                    }
                    let point = |angle: f64| Vector2::new(
                        arc.center.x + arc.radius * angle.cos(),
                        arc.center.y + arc.radius * angle.sin(),
                    );
                    let (start, end) = (point(arc.start_angle), point(arc.end_angle));
                    if arc.counter_clockwise { (start, end) } else { (end, start) }
                }
                BoundaryEdge::EllipticArc(arc) => {
                    if !arc.center.x.is_finite() || !arc.center.y.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} ellipse center has non-finite coordinates"));
                    }
                    if !arc.major_axis_endpoint.x.is_finite() || !arc.major_axis_endpoint.y.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} ellipse major axis has non-finite coordinates"));
                    }
                    if arc.major_axis_endpoint.x.hypot(arc.major_axis_endpoint.y) < 1e-12 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} ellipse major axis has zero length"));
                    }
                    if !arc.minor_axis_ratio.is_finite() || !(0.0..=1.0).contains(&arc.minor_axis_ratio) || arc.minor_axis_ratio == 0.0 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} ellipse ratio must be within (0, 1]"));
                    }
                    if !arc.start_angle.is_finite() || !arc.end_angle.is_finite() {
                        return Err(format!("Hatch boundary path {pi} edge {ei} ellipse angles must be finite"));
                    }
                    let start = ellipse_point(arc.center, arc.major_axis_endpoint, arc.minor_axis_ratio, arc.start_angle);
                    let end = ellipse_point(arc.center, arc.major_axis_endpoint, arc.minor_axis_ratio, arc.end_angle);
                    if arc.counter_clockwise { (start, end) } else { (end, start) }
                }
                BoundaryEdge::Polyline(poly) => {
                    if poly.vertices.len() < 3 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} polyline must have at least 3 vertices"));
                    }
                    for (vi, v) in poly.vertices.iter().enumerate() {
                        if !v.x.is_finite() || !v.y.is_finite() || !v.z.is_finite() {
                            return Err(format!("Hatch boundary path {pi} edge {ei} vertex {vi} has non-finite coordinates"));
                        }
                    }
                    if !poly.is_closed {
                        return Err(format!("Hatch boundary path {pi} edge {ei} polyline is open"));
                    }
                    let first = poly.vertices.first().unwrap();
                    (Vector2::new(first.x, first.y), Vector2::new(first.x, first.y))
                }
                BoundaryEdge::Spline(spline) => {
                    if spline.degree < 1 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} spline degree must be at least 1"));
                    }
                    if spline.control_points.len() < spline.degree as usize + 1 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} spline has too few control points for its degree"));
                    }
                    if spline.knots.iter().any(|knot| !knot.is_finite())
                        || spline.knots.windows(2).any(|pair| pair[0] > pair[1])
                    {
                        return Err(format!("Hatch boundary path {pi} edge {ei} spline knots must be finite and nondecreasing"));
                    }
                    for (ci, cp) in spline.control_points.iter().enumerate() {
                        if !cp.x.is_finite() || !cp.y.is_finite() || !cp.z.is_finite() {
                            return Err(format!("Hatch boundary path {pi} edge {ei} control point {ci} has non-finite coordinates"));
                        }
                    }
                    for (fi, fp) in spline.fit_points.iter().enumerate() {
                        if !fp.x.is_finite() || !fp.y.is_finite() {
                            return Err(format!("Hatch boundary path {pi} edge {ei} fit point {fi} has non-finite coordinates"));
                        }
                    }
                    for (name, tangent) in [("start", spline.start_tangent), ("end", spline.end_tangent)] {
                        if !tangent.x.is_finite() || !tangent.y.is_finite() {
                            return Err(format!("Hatch boundary path {pi} edge {ei} spline {name} tangent is non-finite"));
                        }
                    }
                    let points = if spline.fit_points.len() >= 2 {
                        spline.fit_points.clone()
                    } else {
                        spline.control_points.iter().map(|p| Vector2::new(p.x, p.y)).collect()
                    };
                    if points.len() < 2 {
                        return Err(format!("Hatch boundary path {pi} edge {ei} spline has too few points"));
                    }
                    (*points.first().unwrap(), *points.last().unwrap())
                }
            };
            endpoints.push(pair);
        }
        for ei in 0..endpoints.len() {
            let next = (ei + 1) % endpoints.len();
            if !close(endpoints[ei].1, endpoints[next].0) {
                return Err(format!("Hatch boundary path {pi} is not closed between edges {ei} and {next}"));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "host")]
fn validate_hatch_pattern(value: &crate::host::acadrust::entities::Hatch) -> Result<(), String> {
    if value.pattern.name.trim().is_empty() {
        return Err("Hatch.pattern.name is empty".into());
    }
    if value.is_solid {
        if !value.pattern.name.eq_ignore_ascii_case("SOLID") || !value.pattern.lines.is_empty() {
            return Err("solid Hatch requires the SOLID pattern without pattern lines".into());
        }
        return Ok(());
    }
    if !value.pattern_scale.is_finite() || value.pattern_scale <= 0.0 {
        return Err("Hatch.pattern_scale must be finite and greater than zero".into());
    }
    if value.pattern.lines.is_empty() {
        return Err("patterned Hatch requires at least one pattern line".into());
    }
    for (index, line) in value.pattern.lines.iter().enumerate() {
        if !line.angle.is_finite()
            || !line.base_point.x.is_finite() || !line.base_point.y.is_finite()
            || !line.offset.x.is_finite() || !line.offset.y.is_finite()
            || line.dash_lengths.iter().any(|dash| !dash.is_finite())
        {
            return Err(format!("Hatch.pattern.lines[{index}] contains non-finite values"));
        }
        if line.offset.x.hypot(line.offset.y) < 1e-12 {
            return Err(format!("Hatch.pattern.lines[{index}].offset must be nonzero"));
        }
    }
    Ok(())
}

/// Field checks shared by Leader creation and mutation. `old` limits the
/// checks to fields the edit touched so legacy DWG leaders stay editable.
#[cfg(feature = "host")]
fn validate_leader(
    old: Option<&crate::host::acadrust::entities::Leader>,
    new: &crate::host::acadrust::entities::Leader,
) -> Result<(), String> {
    use crate::host::acadrust::entities::LeaderCreationType;
    let vec_changed = |a: &crate::host::acadrust::types::Vector3,
                       b: &crate::host::acadrust::types::Vector3| {
        old.is_none()
            || a.x.to_bits() != b.x.to_bits()
            || a.y.to_bits() != b.y.to_bits()
            || a.z.to_bits() != b.z.to_bits()
    };
    if old.is_none_or(|old| old.vertices != new.vertices) {
        if new.vertices.len() < 2 {
            return Err("Leader.vertices requires at least 2 points".into());
        }
        for (index, vertex) in new.vertices.iter().enumerate() {
            finite_vector(&format!("Leader.vertices[{index}]"), vertex)?;
        }
        if new.vertices.windows(2).all(|pair| pair[0] == pair[1]) {
            return Err("Leader.vertices must not all coincide".into());
        }
    }
    if let Some(old_leader) = old {
        if vec_changed(&old_leader.normal, &new.normal) {
            solid_normal("Leader.normal", &new.normal)?;
        }
    } else {
        solid_normal("Leader.normal", &new.normal)?;
    }
    if vec_changed(
        old.map_or(&new.horizontal_direction, |o| &o.horizontal_direction),
        &new.horizontal_direction,
    ) {
        solid_normal("Leader.horizontal_direction", &new.horizontal_direction)?;
    }
    for (name, before, after) in [
        (
            "block_offset",
            old.map_or(&new.block_offset, |o| &o.block_offset),
            &new.block_offset,
        ),
        (
            "annotation_offset",
            old.map_or(&new.annotation_offset, |o| &o.annotation_offset),
            &new.annotation_offset,
        ),
    ] {
        if vec_changed(before, after) {
            finite_vector(&format!("Leader.{name}"), after)?;
        }
    }
    for (name, before, after) in [
        (
            "text_height",
            old.map_or(new.text_height, |o| o.text_height),
            new.text_height,
        ),
        (
            "text_width",
            old.map_or(new.text_width, |o| o.text_width),
            new.text_width,
        ),
    ] {
        if (old.is_none() || before.to_bits() != after.to_bits())
            && (!after.is_finite() || after < 0.0 || (name == "text_height" && after == 0.0))
        {
            return Err(format!(
                "Leader.{name} must be finite and {}",
                if name == "text_height" { "greater than zero" } else { "non-negative" }
            ));
        }
    }
    if old.is_none_or(|o| o.dimension_style != new.dimension_style)
        && new.dimension_style.trim().is_empty()
    {
        return Err("Leader.dimension_style is empty".into());
    }
    if new.creation_type == LeaderCreationType::NoAnnotation && !new.annotation_handle.is_null() {
        return Err("Leader with no annotation cannot carry an annotation handle".into());
    }
    Ok(())
}

/// Field checks shared by MLine creation and mutation. Vertex `direction`,
/// `miter` and segment cut parameters are derived by the host, so only their
/// finiteness is checked here.
#[cfg(feature = "host")]
fn validate_mline(
    old: Option<&crate::host::acadrust::entities::MLine>,
    new: &crate::host::acadrust::entities::MLine,
) -> Result<(), String> {
    if old.is_none_or(|old| old.flags != new.flags) && new.flags.bits() & !0x0f != 0 {
        return Err("MLine.flags has unknown bits".into());
    }
    if old.is_none_or(|old| old.scale_factor.to_bits() != new.scale_factor.to_bits())
        && (!new.scale_factor.is_finite() || new.scale_factor == 0.0)
    {
        return Err("MLine.scale_factor must be finite and nonzero".into());
    }
    if old.is_none_or(|old| old.normal != new.normal) {
        solid_normal("MLine.normal", &new.normal)?;
    }
    if old.is_none_or(|old| old.style_name != new.style_name) && new.style_name.trim().is_empty() {
        return Err("MLine.style_name is empty".into());
    }
    if old.is_none_or(|old| old.vertices != new.vertices) {
        if new.vertices.len() < 2 {
            return Err("MLine.vertices requires at least 2 points".into());
        }
        for (vi, vertex) in new.vertices.iter().enumerate() {
            finite_vector(&format!("MLine.vertices[{vi}].position"), &vertex.position)?;
            finite_vector(&format!("MLine.vertices[{vi}].direction"), &vertex.direction)?;
            finite_vector(&format!("MLine.vertices[{vi}].miter"), &vertex.miter)?;
            for (si, segment) in vertex.segments.iter().enumerate() {
                if segment
                    .parameters
                    .iter()
                    .chain(&segment.area_fill_parameters)
                    .any(|value| !value.is_finite())
                {
                    return Err(format!(
                        "MLine.vertices[{vi}].segments[{si}] contains non-finite parameters"
                    ));
                }
            }
        }
        if new
            .vertices
            .windows(2)
            .all(|pair| pair[0].position == pair[1].position)
        {
            return Err("MLine.vertices must not all coincide".into());
        }
    }
    if new.flags.contains(crate::host::acadrust::entities::MLineFlags::CLOSED)
        && new.vertices.len() < 3
    {
        return Err("closed MLine requires at least 3 vertices".into());
    }
    Ok(())
}

/// Field checks for every Dimension subtype. Derived fields (base definition
/// point, `actual_measurement`) are recomputed by the host, so they are not
/// validated as inputs; the measurement the geometry implies must be finite.
#[cfg(feature = "host")]
fn validate_dimension(
    old: Option<&crate::host::acadrust::entities::Dimension>,
    new: &crate::host::acadrust::entities::Dimension,
) -> Result<(), String> {
    use crate::host::acadrust::entities::Dimension as D;
    if old == Some(new) {
        return Ok(());
    }
    let base = new.base();
    solid_normal("Dimension.normal", &base.normal)?;
    finite_vector("Dimension.text_middle_point", &base.text_middle_point)?;
    if !base.text_rotation.is_finite() || !base.horizontal_direction.is_finite() {
        return Err("Dimension.text_rotation and horizontal_direction must be finite".into());
    }
    if base.style_name.trim().is_empty() {
        return Err("Dimension.style_name is empty".into());
    }
    let (points, scalars, distinct): (Vec<(&str, &crate::host::acadrust::types::Vector3)>, Vec<(&str, f64)>, Vec<(&str, &str)>) = match new {
        D::Aligned(d) => (
            vec![("definition_point", &d.definition_point), ("first_point", &d.first_point), ("second_point", &d.second_point)],
            vec![("ext_line_rotation", d.ext_line_rotation)],
            vec![("first_point", "second_point")],
        ),
        D::Linear(d) => (
            vec![("definition_point", &d.definition_point), ("first_point", &d.first_point), ("second_point", &d.second_point)],
            vec![("rotation", d.rotation), ("ext_line_rotation", d.ext_line_rotation)],
            vec![("first_point", "second_point")],
        ),
        D::Radius(d) => (
            vec![("definition_point", &d.definition_point), ("angle_vertex", &d.angle_vertex)],
            vec![("leader_length", d.leader_length)],
            vec![("definition_point", "angle_vertex")],
        ),
        D::Diameter(d) => (
            vec![("definition_point", &d.definition_point), ("angle_vertex", &d.angle_vertex)],
            vec![("leader_length", d.leader_length)],
            vec![("definition_point", "angle_vertex")],
        ),
        D::Angular2Ln(d) => (
            vec![("definition_point", &d.definition_point), ("dimension_arc", &d.dimension_arc), ("first_point", &d.first_point), ("second_point", &d.second_point), ("angle_vertex", &d.angle_vertex)],
            vec![],
            vec![],
        ),
        D::Angular3Pt(d) => (
            vec![("definition_point", &d.definition_point), ("first_point", &d.first_point), ("second_point", &d.second_point), ("angle_vertex", &d.angle_vertex)],
            vec![],
            vec![("first_point", "angle_vertex"), ("second_point", "angle_vertex")],
        ),
        D::Ordinate(d) => (
            vec![("definition_point", &d.definition_point), ("feature_location", &d.feature_location), ("leader_endpoint", &d.leader_endpoint)],
            vec![],
            vec![],
        ),
        D::Arc(d) => (
            vec![("definition_point", &d.definition_point), ("first_extension_point", &d.first_extension_point), ("second_extension_point", &d.second_extension_point), ("center_point", &d.center_point), ("first_leader_point", &d.first_leader_point), ("second_leader_point", &d.second_leader_point)],
            vec![("arc_start_parameter", d.arc_start_parameter), ("arc_end_parameter", d.arc_end_parameter)],
            vec![("first_extension_point", "second_extension_point")],
        ),
        D::LargeRadial(d) => (
            vec![("definition_point", &d.definition_point), ("chord_point", &d.chord_point), ("override_center", &d.override_center), ("jog_point", &d.jog_point)],
            vec![("jog_angle", d.jog_angle)],
            vec![("definition_point", "chord_point")],
        ),
    };
    for (name, point) in &points {
        finite_vector(&format!("Dimension.{name}"), point)?;
    }
    for (name, value) in &scalars {
        if !value.is_finite() {
            return Err(format!("Dimension.{name} must be finite"));
        }
    }
    let find = |name: &str| points.iter().find(|(n, _)| *n == name).map(|(_, p)| **p);
    for (a, b) in distinct {
        if find(a) == find(b) {
            return Err(format!("Dimension.{a} and {b} must not coincide"));
        }
    }
    if !new.measurement().is_finite() {
        return Err("Dimension geometry does not define a finite measurement".into());
    }
    Ok(())
}

/// Field checks shared by MultiLeader creation and mutation.
#[cfg(feature = "host")]
fn validate_multileader(
    old: Option<&crate::host::acadrust::entities::MultiLeader>,
    new: &crate::host::acadrust::entities::MultiLeader,
) -> Result<(), String> {
    use crate::host::acadrust::entities::LeaderContentType;
    if old == Some(new) {
        return Ok(());
    }
    for (name, value, allow_zero) in [
        ("dogleg_length", new.dogleg_length, true),
        ("arrowhead_size", new.arrowhead_size, true),
        ("text_height", new.text_height, false),
        ("scale_factor", new.scale_factor, false),
    ] {
        if !value.is_finite() || value < 0.0 || (!allow_zero && value == 0.0) {
            return Err(format!(
                "MultiLeader.{name} must be finite and {}",
                if allow_zero { "non-negative" } else { "greater than zero" }
            ));
        }
    }
    if !new.block_rotation.is_finite() {
        return Err("MultiLeader.block_rotation must be finite".into());
    }
    finite_vector("MultiLeader.block_scale", &new.block_scale)?;
    let context = &new.context;
    if !context.scale_factor.is_finite() || context.scale_factor <= 0.0 {
        return Err("MultiLeader.context.scale_factor must be finite and greater than zero".into());
    }
    for (name, value) in [
        ("text_height", context.text_height),
        ("text_width", context.text_width),
        ("text_rotation", context.text_rotation),
        ("landing_gap", context.landing_gap),
        ("arrowhead_size", context.arrowhead_size),
    ] {
        if !value.is_finite() {
            return Err(format!("MultiLeader.context.{name} must be finite"));
        }
    }
    for (name, point) in [
        ("content_base_point", &context.content_base_point),
        ("text_location", &context.text_location),
        ("block_content_location", &context.block_content_location),
        ("base_point", &context.base_point),
    ] {
        finite_vector(&format!("MultiLeader.context.{name}"), point)?;
    }
    if context.transform_matrix.iter().any(|value| !value.is_finite()) {
        return Err("MultiLeader.context.transform_matrix must be finite".into());
    }
    for (ri, root) in context.leader_roots.iter().enumerate() {
        finite_vector(&format!("MultiLeader.context.leader_roots[{ri}].connection_point"), &root.connection_point)?;
        finite_vector(&format!("MultiLeader.context.leader_roots[{ri}].direction"), &root.direction)?;
        if !root.landing_distance.is_finite() {
            return Err(format!("MultiLeader.context.leader_roots[{ri}].landing_distance must be finite"));
        }
        for (li, line) in root.lines.iter().enumerate() {
            if line.points.is_empty() {
                return Err(format!("MultiLeader.context.leader_roots[{ri}].lines[{li}] has no points"));
            }
            for (pi, point) in line.points.iter().enumerate() {
                finite_vector(&format!("MultiLeader.context.leader_roots[{ri}].lines[{li}].points[{pi}]"), point)?;
            }
        }
    }
    match new.content_type {
        LeaderContentType::MText if !context.has_text_contents => {
            return Err("MText MultiLeader requires context.has_text_contents".into());
        }
        LeaderContentType::Block if !context.has_block_contents => {
            return Err("Block MultiLeader requires context.has_block_contents".into());
        }
        LeaderContentType::None if context.has_text_contents || context.has_block_contents => {
            return Err("MultiLeader without content cannot carry text or block contents".into());
        }
        _ => {}
    }
    Ok(())
}

/// Field checks shared by Table creation and mutation.
#[cfg(feature = "host")]
fn validate_table(
    old: Option<&crate::host::acadrust::entities::Table>,
    new: &crate::host::acadrust::entities::Table,
) -> Result<(), String> {
    if old == Some(new) {
        return Ok(());
    }
    finite_vector("Table.insertion_point", &new.insertion_point)?;
    solid_normal("Table.normal", &new.normal)?;
    solid_normal("Table.horizontal_direction", &new.horizontal_direction)?;
    if !new.break_spacing.is_finite() || new.break_spacing < 0.0 {
        return Err("Table.break_spacing must be finite and non-negative".into());
    }
    if new.rows.is_empty() || new.columns.is_empty() {
        return Err("Table requires at least one row and one column".into());
    }
    for (ci, column) in new.columns.iter().enumerate() {
        if !column.width.is_finite() || column.width <= 0.0 {
            return Err(format!("Table.columns[{ci}].width must be finite and greater than zero"));
        }
    }
    for (ri, row) in new.rows.iter().enumerate() {
        if !row.height.is_finite() || row.height <= 0.0 {
            return Err(format!("Table.rows[{ri}].height must be finite and greater than zero"));
        }
        if row.cells.len() != new.columns.len() {
            return Err(format!(
                "Table.rows[{ri}] has {} cells but the table has {} columns",
                row.cells.len(),
                new.columns.len()
            ));
        }
        for (ci, cell) in row.cells.iter().enumerate() {
            if !cell.rotation.is_finite() || !cell.geometry_scale.is_finite() || !cell.block_scale.is_finite() {
                return Err(format!("Table.rows[{ri}].cells[{ci}] has non-finite rotation or scale"));
            }
            for (ki, content) in cell.contents.iter().enumerate() {
                if !content.rotation.is_finite() || !content.scale.is_finite() || !content.text_height.is_finite() || content.text_height < 0.0 {
                    return Err(format!(
                        "Table.rows[{ri}].cells[{ci}].contents[{ki}] has invalid rotation, scale or text height"
                    ));
                }
                if !content.value.numeric_value.is_finite() {
                    return Err(format!(
                        "Table.rows[{ri}].cells[{ci}].contents[{ki}].value.numeric_value must be finite"
                    ));
                }
            }
        }
    }
    let (rows, columns) = (new.rows.len(), new.columns.len());
    for (mi, range) in new.merged_ranges.iter().enumerate() {
        if range.top_row > range.bottom_row
            || range.left_col > range.right_col
            || range.bottom_row >= rows
            || range.right_col >= columns
        {
            return Err(format!("Table.merged_ranges[{mi}] lies outside the {rows}x{columns} grid"));
        }
        for (mj, other) in new.merged_ranges.iter().enumerate().skip(mi + 1) {
            let separate = range.bottom_row < other.top_row
                || other.bottom_row < range.top_row
                || range.right_col < other.left_col
                || other.right_col < range.left_col;
            if !separate {
                return Err(format!("Table.merged_ranges[{mi}] overlaps merged_ranges[{mj}]"));
            }
        }
    }
    Ok(())
}

/// Validate newly created geometry for the kinds whose mapped fields have
/// host checks. Called by the Python add path before an entity enters the document.
#[cfg(feature = "host")]
pub fn validate_new_canvas_entity(entity: &crate::host::EntityType) -> Result<(), String> {
    use crate::host::EntityType;
    if entity.common().layer.trim().is_empty() {
        return Err("layer name is empty".into());
    }
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
                ("first_corner", &value.first_corner),
                ("second_corner", &value.second_corner),
                ("third_corner", &value.third_corner),
                ("fourth_corner", &value.fourth_corner),
            ] {
                finite_vector(&format!("Solid.{name}"), corner)?;
            }
            solid_normal("Solid.normal", &value.normal)?;
            if !value.thickness.is_finite() {
                return Err("Solid.thickness must be finite".into());
            }
        }
        EntityType::Face3D(value) => {
            for (name, corner) in [
                ("first_corner", &value.first_corner),
                ("second_corner", &value.second_corner),
                ("third_corner", &value.third_corner),
                ("fourth_corner", &value.fourth_corner),
            ] {
                finite_vector(&format!("Face3D.{name}"), corner)?;
            }
            if value.invisible_edges.bits() & !0x0f != 0 {
                return Err("Face3D.invisible_edges has unknown bits".into());
            }
        }
        EntityType::Tolerance(value) => {
            finite_vector("Tolerance.insertion_point", &value.insertion_point)?;
            unit_direction("Tolerance.direction", &value.direction)?;
            solid_normal("Tolerance.normal", &value.normal)?;
            if value.text.trim().is_empty() {
                return Err("Tolerance.text is empty".into());
            }
            if value.dimension_style_name.trim().is_empty() {
                return Err("Tolerance.dimension_style_name is empty".into());
            }
            if !value.text_height.is_finite() || value.text_height <= 0.0 {
                return Err("Tolerance.text_height must be finite and greater than zero".into());
            }
            if !value.dimension_gap.is_finite() {
                return Err("Tolerance.dimension_gap must be finite".into());
            }
        }
        EntityType::Shape(value) => {
            finite_vector("Shape.insertion_point", &value.insertion_point)?;
            solid_normal("Shape.normal", &value.normal)?;
            if !value.size.is_finite() || value.size <= 0.0 {
                return Err("Shape.size must be finite and greater than zero".into());
            }
            if value.shape_name.trim().is_empty() {
                return Err("Shape.shape_name is empty".into());
            }
            if !(1..=i16::MAX as i32).contains(&value.shape_number) {
                return Err("Shape.shape_number must be between 1 and 32767".into());
            }
            if !value.rotation.is_finite() {
                return Err("Shape.rotation must be finite".into());
            }
            if !value.relative_x_scale.is_finite() || value.relative_x_scale.abs() < 1e-12 {
                return Err("Shape.relative_x_scale must be finite and nonzero".into());
            }
            if !value.oblique_angle.is_finite()
                || value.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err("Shape.oblique_angle must be finite and between -PI/2 and PI/2".into());
            }
            if !value.thickness.is_finite() {
                return Err("Shape.thickness must be finite".into());
            }
            if value.style_name.trim().is_empty() {
                return Err("Shape.style_name is empty".into());
            }
        }
        EntityType::Leader(value) => validate_leader(None, value)?,
        EntityType::MLine(value) => validate_mline(None, value)?,
        EntityType::Dimension(value) => validate_dimension(None, value)?,
        EntityType::MultiLeader(value) => validate_multileader(None, value)?,
        EntityType::Table(value) => validate_table(None, value)?,
        EntityType::AttributeDefinition(value) => {
            finite_vector(
                "AttributeDefinition.insertion_point",
                &value.insertion_point,
            )?;
            finite_vector(
                "AttributeDefinition.alignment_point",
                &value.alignment_point,
            )?;
            solid_normal("AttributeDefinition.normal", &value.normal)?;
            if value.tag.trim().is_empty() || value.tag.chars().any(char::is_whitespace) {
                return Err(
                    "AttributeDefinition.tag must be nonempty and contain no whitespace".into(),
                );
            }
            if !value.height.is_finite() || value.height <= 0.0 {
                return Err(
                    "AttributeDefinition.height must be finite and greater than zero".into(),
                );
            }
            if !value.rotation.is_finite() {
                return Err("AttributeDefinition.rotation must be finite".into());
            }
            if !value.width_factor.is_finite() || value.width_factor.abs() < 1e-12 {
                return Err("AttributeDefinition.width_factor must be finite and nonzero".into());
            }
            if !value.oblique_angle.is_finite()
                || value.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err(
                    "AttributeDefinition.oblique_angle must be finite and between -PI/2 and PI/2"
                        .into(),
                );
            }
            if value.text_style.trim().is_empty() {
                return Err("AttributeDefinition.text_style is empty".into());
            }
            if value.field_length < 0 {
                return Err("AttributeDefinition.field_length must be nonnegative".into());
            }
            if value.line_count < 1 {
                return Err("AttributeDefinition.line_count must be greater than zero".into());
            }
        }
        EntityType::AttributeEntity(value) => {
            finite_vector("AttributeEntity.insertion_point", &value.insertion_point)?;
            finite_vector("AttributeEntity.alignment_point", &value.alignment_point)?;
            solid_normal("AttributeEntity.normal", &value.normal)?;
            if value.tag.trim().is_empty() || value.tag.chars().any(char::is_whitespace) {
                return Err(
                    "AttributeEntity.tag must be nonempty and contain no whitespace".into(),
                );
            }
            if !value.height.is_finite() || value.height <= 0.0 {
                return Err("AttributeEntity.height must be finite and greater than zero".into());
            }
            if !value.rotation.is_finite() {
                return Err("AttributeEntity.rotation must be finite".into());
            }
            if !value.width_factor.is_finite() || value.width_factor.abs() < 1e-12 {
                return Err("AttributeEntity.width_factor must be finite and nonzero".into());
            }
            if !value.oblique_angle.is_finite()
                || value.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err(
                    "AttributeEntity.oblique_angle must be finite and between -PI/2 and PI/2"
                        .into(),
                );
            }
            if value.text_style.trim().is_empty() {
                return Err("AttributeEntity.text_style is empty".into());
            }
            if value.field_length < 0 {
                return Err("AttributeEntity.field_length must be nonnegative".into());
            }
            if value.line_count < 1 {
                return Err("AttributeEntity.line_count must be greater than zero".into());
            }
        }
        EntityType::Hatch(value) => {
            solid_normal("Hatch.normal", &value.normal)?;
            if !value.elevation.is_finite() {
                return Err("Hatch.elevation must be finite".into());
            }
            validate_hatch_pattern(value)?;
            if !value.pattern_angle.is_finite() {
                return Err("Hatch.pattern_angle must be finite".into());
            }
            if !value.pixel_size.is_finite() || value.pixel_size < 0.0 {
                return Err("Hatch.pixel_size must be finite and non-negative".into());
            }
            for (si, sp) in value.seed_points.iter().enumerate() {
                if !sp.x.is_finite() || !sp.y.is_finite() {
                    return Err(format!("Hatch.seed_points[{si}] has non-finite coordinates"));
                }
            }
            validate_hatch_paths(&value.paths)?;
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
        if after.common().layer.trim().is_empty() {
            return Err("layer name is empty".into());
        }
        if matches!(
            after,
            EntityType::Block(_)
                | EntityType::BlockEnd(_)
                | EntityType::Seqend(_)
                | EntityType::Extended(_)
                | EntityType::Unknown(_)
        ) {
            return Err("entity kind does not support canvas layer edits".into());
        }
    }
    let finite3 = |name: &str, x: f64, y: f64, z: f64| {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            Ok(())
        } else {
            Err(format!("{name} must contain finite coordinates"))
        }
    };
    let same3 = |a: &crate::host::acadrust::types::Vector3,
                 b: &crate::host::acadrust::types::Vector3| {
        a.x.to_bits() == b.x.to_bits()
            && a.y.to_bits() == b.y.to_bits()
            && a.z.to_bits() == b.z.to_bits()
    };
    let radius = |name: &str, value: f64| {
        if value.is_finite() && value > 0.0 {
            Ok(())
        } else {
            Err(format!("{name} must be finite and greater than zero"))
        }
    };
    let changed3 = |name: &str,
                    old: &crate::host::acadrust::types::Vector3,
                    new: &crate::host::acadrust::types::Vector3| {
        if !same3(old, new) {
            finite_vector(name, new)
        } else {
            Ok(())
        }
    };
    match (before, after) {
        (EntityType::Point(old), EntityType::Point(new)) => {
            if !same3(&old.location, &new.location) {
                finite3(
                    "Point.location",
                    new.location.x,
                    new.location.y,
                    new.location.z,
                )?;
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
            if old.radius.to_bits() != new.radius.to_bits() {
                radius("Circle.radius", new.radius)?;
            }
        }
        (EntityType::Arc(old), EntityType::Arc(new)) => {
            if !same3(&old.center, &new.center) {
                finite3("Arc.center", new.center.x, new.center.y, new.center.z)?;
            }
            if old.radius.to_bits() != new.radius.to_bits() {
                radius("Arc.radius", new.radius)?;
            }
        }
        (EntityType::Ray(old), EntityType::Ray(new)) => {
            changed3("Ray.base_point", &old.base_point, &new.base_point)?;
            if !same3(&old.direction, &new.direction) {
                unit_direction("Ray.direction", &new.direction)?;
            }
        }
        (EntityType::XLine(old), EntityType::XLine(new)) => {
            changed3("XLine.base_point", &old.base_point, &new.base_point)?;
            if !same3(&old.direction, &new.direction) {
                unit_direction("XLine.direction", &new.direction)?;
            }
        }
        (EntityType::Solid(old), EntityType::Solid(new)) => {
            for (name, old_corner, new_corner) in [
                ("first_corner", &old.first_corner, &new.first_corner),
                ("second_corner", &old.second_corner, &new.second_corner),
                ("third_corner", &old.third_corner, &new.third_corner),
                ("fourth_corner", &old.fourth_corner, &new.fourth_corner),
            ] {
                changed3(&format!("Solid.{name}"), old_corner, new_corner)?;
            }
            if !same3(&old.normal, &new.normal) {
                solid_normal("Solid.normal", &new.normal)?;
            }
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
            ] {
                changed3(&format!("Face3D.{name}"), old_corner, new_corner)?;
            }
            if old.invisible_edges != new.invisible_edges && new.invisible_edges.bits() & !0x0f != 0
            {
                return Err("Face3D.invisible_edges has unknown bits".into());
            }
        }
        (EntityType::Insert(old), EntityType::Insert(new)) => {
            if old.block_name != new.block_name
                || old.attributes != new.attributes
                || old.view_rep_handle != new.view_rep_handle
                || old.seqend_handle != new.seqend_handle
            {
                return Err("Insert block identity and attached records cannot change in a geometry transaction".into());
            }
            changed3("Insert.insert_point", &old.insert_point, &new.insert_point)?;
            if !same3(&old.normal, &new.normal) {
                solid_normal("Insert.normal", &new.normal)?;
            }
            for (name, before, after) in [
                ("x_scale", old.x_scale(), new.x_scale()),
                ("y_scale", old.y_scale(), new.y_scale()),
                ("z_scale", old.z_scale(), new.z_scale()),
            ] {
                if before.to_bits() != after.to_bits() {
                    insert_scale(&format!("Insert.{name}"), after)?;
                }
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
            if (old.column_count != new.column_count && new.column_count == 0)
                || (old.row_count != new.row_count && new.row_count == 0)
            {
                return Err("Insert array counts must be greater than zero".into());
            }
        }
        (EntityType::Tolerance(old), EntityType::Tolerance(new)) => {
            changed3(
                "Tolerance.insertion_point",
                &old.insertion_point,
                &new.insertion_point,
            )?;
            if !same3(&old.direction, &new.direction) {
                unit_direction("Tolerance.direction", &new.direction)?;
            }
            if !same3(&old.normal, &new.normal) {
                solid_normal("Tolerance.normal", &new.normal)?;
            }
            if old.text != new.text && new.text.trim().is_empty() {
                return Err("Tolerance.text is empty".into());
            }
            if old.dimension_style_name != new.dimension_style_name
                && new.dimension_style_name.trim().is_empty()
            {
                return Err("Tolerance.dimension_style_name is empty".into());
            }
            if old.text_height.to_bits() != new.text_height.to_bits()
                && (!new.text_height.is_finite() || new.text_height <= 0.0)
            {
                return Err("Tolerance.text_height must be finite and greater than zero".into());
            }
            if old.dimension_gap.to_bits() != new.dimension_gap.to_bits()
                && !new.dimension_gap.is_finite()
            {
                return Err("Tolerance.dimension_gap must be finite".into());
            }
            if old.dimension_style_handle != new.dimension_style_handle
                && old.dimension_style_name == new.dimension_style_name
            {
                return Err("Tolerance.dimension_style_handle is read-only".into());
            }
        }
        (EntityType::Shape(old), EntityType::Shape(new)) => {
            changed3(
                "Shape.insertion_point",
                &old.insertion_point,
                &new.insertion_point,
            )?;
            if !same3(&old.normal, &new.normal) {
                solid_normal("Shape.normal", &new.normal)?;
            }
            if old.size.to_bits() != new.size.to_bits()
                && (!new.size.is_finite() || new.size <= 0.0)
            {
                return Err("Shape.size must be finite and greater than zero".into());
            }
            if old.shape_name != new.shape_name && new.shape_name.trim().is_empty() {
                return Err("Shape.shape_name is empty".into());
            }
            if old.shape_number != new.shape_number
                && !(1..=i16::MAX as i32).contains(&new.shape_number)
            {
                return Err("Shape.shape_number must be between 1 and 32767".into());
            }
            if old.rotation.to_bits() != new.rotation.to_bits() && !new.rotation.is_finite() {
                return Err("Shape.rotation must be finite".into());
            }
            if old.relative_x_scale.to_bits() != new.relative_x_scale.to_bits()
                && (!new.relative_x_scale.is_finite() || new.relative_x_scale.abs() < 1e-12)
            {
                return Err("Shape.relative_x_scale must be finite and nonzero".into());
            }
            if old.oblique_angle.to_bits() != new.oblique_angle.to_bits()
                && (!new.oblique_angle.is_finite()
                    || new.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2)
            {
                return Err("Shape.oblique_angle must be finite and between -PI/2 and PI/2".into());
            }
            if old.thickness.to_bits() != new.thickness.to_bits() && !new.thickness.is_finite() {
                return Err("Shape.thickness must be finite".into());
            }
            if old.style_name != new.style_name && new.style_name.trim().is_empty() {
                return Err("Shape.style_name is empty".into());
            }
            if old.style_handle != new.style_handle
                && !(old.style_name != new.style_name && new.style_handle.is_some())
            {
                return Err("Shape.style_handle is read-only".into());
            }
        }
        (EntityType::Dimension(old), EntityType::Dimension(new)) => validate_dimension(Some(old), new)?,
        (EntityType::MultiLeader(old), EntityType::MultiLeader(new)) => validate_multileader(Some(old), new)?,
        (EntityType::Table(old), EntityType::Table(new)) => validate_table(Some(old), new)?,
        (EntityType::MLine(old), EntityType::MLine(new)) => validate_mline(Some(old), new)?,
        (EntityType::Leader(old), EntityType::Leader(new)) => validate_leader(Some(old), new)?,
        (EntityType::AttributeDefinition(old), EntityType::AttributeDefinition(new)) => {
            changed3(
                "AttributeDefinition.insertion_point",
                &old.insertion_point,
                &new.insertion_point,
            )?;
            changed3(
                "AttributeDefinition.alignment_point",
                &old.alignment_point,
                &new.alignment_point,
            )?;
            if !same3(&old.normal, &new.normal) {
                solid_normal("AttributeDefinition.normal", &new.normal)?;
            }
            if old.tag != new.tag
                && (new.tag.trim().is_empty() || new.tag.chars().any(char::is_whitespace))
            {
                return Err(
                    "AttributeDefinition.tag must be nonempty and contain no whitespace".into(),
                );
            }
            if old.height.to_bits() != new.height.to_bits()
                && (!new.height.is_finite() || new.height <= 0.0)
            {
                return Err(
                    "AttributeDefinition.height must be finite and greater than zero".into(),
                );
            }
            for (name, before, after) in [
                ("rotation", old.rotation, new.rotation),
                ("oblique_angle", old.oblique_angle, new.oblique_angle),
            ] {
                if before.to_bits() != after.to_bits() && !after.is_finite() {
                    return Err(format!("AttributeDefinition.{name} must be finite"));
                }
            }
            if old.width_factor.to_bits() != new.width_factor.to_bits()
                && (!new.width_factor.is_finite() || new.width_factor.abs() < 1e-12)
            {
                return Err("AttributeDefinition.width_factor must be finite and nonzero".into());
            }
            if old.oblique_angle.to_bits() != new.oblique_angle.to_bits()
                && new.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err(
                    "AttributeDefinition.oblique_angle must be between -PI/2 and PI/2".into(),
                );
            }
            if old.text_style != new.text_style && new.text_style.trim().is_empty() {
                return Err("AttributeDefinition.text_style is empty".into());
            }
            if old.field_length != new.field_length && new.field_length < 0 {
                return Err("AttributeDefinition.field_length must be nonnegative".into());
            }
            if old.line_count != new.line_count && new.line_count < 1 {
                return Err("AttributeDefinition.line_count must be greater than zero".into());
            }
            if old.embedded_mtext != new.embedded_mtext {
                return Err("AttributeDefinition.embedded_mtext is unmapped and read-only".into());
            }
        }
        (EntityType::AttributeEntity(old), EntityType::AttributeEntity(new)) => {
            changed3(
                "AttributeEntity.insertion_point",
                &old.insertion_point,
                &new.insertion_point,
            )?;
            changed3(
                "AttributeEntity.alignment_point",
                &old.alignment_point,
                &new.alignment_point,
            )?;
            if !same3(&old.normal, &new.normal) {
                solid_normal("AttributeEntity.normal", &new.normal)?;
            }
            if old.tag != new.tag
                && (new.tag.trim().is_empty() || new.tag.chars().any(char::is_whitespace))
            {
                return Err(
                    "AttributeEntity.tag must be nonempty and contain no whitespace".into(),
                );
            }
            if old.height.to_bits() != new.height.to_bits()
                && (!new.height.is_finite() || new.height <= 0.0)
            {
                return Err("AttributeEntity.height must be finite and greater than zero".into());
            }
            for (name, before, after) in [
                ("rotation", old.rotation, new.rotation),
                ("oblique_angle", old.oblique_angle, new.oblique_angle),
            ] {
                if before.to_bits() != after.to_bits() && !after.is_finite() {
                    return Err(format!("AttributeEntity.{name} must be finite"));
                }
            }
            if old.width_factor.to_bits() != new.width_factor.to_bits()
                && (!new.width_factor.is_finite() || new.width_factor.abs() < 1e-12)
            {
                return Err("AttributeEntity.width_factor must be finite and nonzero".into());
            }
            if old.oblique_angle.to_bits() != new.oblique_angle.to_bits()
                && new.oblique_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err("AttributeEntity.oblique_angle must be between -PI/2 and PI/2".into());
            }
            if old.text_style != new.text_style && new.text_style.trim().is_empty() {
                return Err("AttributeEntity.text_style is empty".into());
            }
            if old.field_length != new.field_length && new.field_length < 0 {
                return Err("AttributeEntity.field_length must be nonnegative".into());
            }
            if old.line_count != new.line_count && new.line_count < 1 {
                return Err("AttributeEntity.line_count must be greater than zero".into());
            }
            if old.embedded_mtext != new.embedded_mtext {
                return Err("AttributeEntity.embedded_mtext is unmapped and read-only".into());
            }
            if old.attdef_handle != new.attdef_handle
                && !(old.tag != new.tag && !new.attdef_handle.is_null())
            {
                return Err("AttributeEntity.attdef_handle is read-only".into());
            }
        }
        (EntityType::Hatch(old), EntityType::Hatch(new)) => {
            if !same3(&old.normal, &new.normal) {
                solid_normal("Hatch.normal", &new.normal)?;
            }
            if old.elevation.to_bits() != new.elevation.to_bits() && !new.elevation.is_finite() {
                return Err("Hatch.elevation must be finite".into());
            }
            if old.pattern != new.pattern
                || old.is_solid != new.is_solid
                || old.pattern_scale.to_bits() != new.pattern_scale.to_bits()
            {
                validate_hatch_pattern(new)?;
            }
            if old.pattern_angle.to_bits() != new.pattern_angle.to_bits()
                && !new.pattern_angle.is_finite()
            {
                return Err("Hatch.pattern_angle must be finite".into());
            }
            if old.pixel_size.to_bits() != new.pixel_size.to_bits()
                && (!new.pixel_size.is_finite() || new.pixel_size < 0.0)
            {
                return Err("Hatch.pixel_size must be finite and non-negative".into());
            }
            if old.paths != new.paths {
                validate_hatch_paths(&new.paths)?;
            }
            if old.seed_points != new.seed_points {
                for (si, sp) in new.seed_points.iter().enumerate() {
                    if !sp.x.is_finite() || !sp.y.is_finite() {
                        return Err(format!("Hatch.seed_points[{si}] has non-finite coordinates"));
                    }
                }
            }
            if old.gradient_color != new.gradient_color {
                return Err("Hatch.gradient_color is unmapped and read-only".into());
            }
            if old.is_mpolygon != new.is_mpolygon
                || old.mpolygon_hatch_color != new.mpolygon_hatch_color
                || old.mpolygon_x_direction != new.mpolygon_x_direction
                || old.mpolygon_boundary_handle_count != new.mpolygon_boundary_handle_count
            {
                return Err("Hatch MPOLYGON fields are unmapped and read-only".into());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate references which require the surrounding document. Keeping this
/// in the host API gives every scripting adapter the same reference rules.
#[cfg(feature = "host")]
pub fn validate_canvas_entity_references(
    document: &crate::host::CadDocument,
    entity: &crate::host::EntityType,
) -> Result<(), String> {
    use crate::host::EntityType;
    let owner = entity.common().owner_handle;
    if !owner.is_null() {
        let block_owner = document
            .block_records
            .iter()
            .any(|record| record.handle == owner);
        let insert_owner = matches!(entity, EntityType::AttributeEntity(_))
            && matches!(document.get_entity(owner), Some(EntityType::Insert(_)));
        if !block_owner && !insert_owner {
            return Err(format!("entity owner {owner:?} does not exist"));
        }
    }
    if let EntityType::Hatch(value) = entity {
        let handles = value.paths.iter().flat_map(|path| &path.boundary_handles);
        let mut count = 0usize;
        for handle in handles {
            count += 1;
            if handle.is_null() || document.get_entity(*handle).is_none() {
                return Err(format!("Hatch boundary handle {handle:?} does not exist"));
            }
        }
        if value.is_associative && count == 0 {
            return Err("associative Hatch requires boundary handles".into());
        }
        if !value.is_associative && count != 0 {
            return Err("non-associative Hatch cannot carry boundary handles".into());
        }
    }
    if let EntityType::Tolerance(value) = entity {
        let found = value
            .dimension_style_handle
            .filter(|handle| !handle.is_null())
            .map_or_else(
                || {
                    document.dim_styles.iter().any(|style| {
                        style
                            .name
                            .eq_ignore_ascii_case(value.dimension_style_name.trim())
                    })
                },
                |handle| {
                    document
                        .dim_styles
                        .iter()
                        .any(|style| style.handle == handle)
                },
        );
        if !found {
            return Err(format!(
                "Tolerance dimension style {:?} does not exist",
                value.dimension_style_name
            ));
        }
    }
    if let EntityType::Shape(value) = entity {
        let style = value
            .style_handle
            .filter(|handle| !handle.is_null())
            .and_then(|handle| {
                document
                    .text_styles
                    .iter()
                    .find(|style| style.handle == handle)
            })
            .or_else(|| {
                document
                    .text_styles
                    .iter()
                    .find(|style| style.name.eq_ignore_ascii_case(value.style_name.trim()))
            })
            .ok_or_else(|| format!("Shape text style {:?} does not exist", value.style_name))?;
        if !style.is_shape_file {
            return Err(format!(
                "Shape text style {:?} is not a shape-file style",
                style.name
            ));
        }
        if style.font_file.trim().is_empty() {
            return Err(format!("Shape text style {:?} has no SHX file", style.name));
        }
    }
    if let EntityType::Table(value) = entity {
        if let Some(handle) = value.table_style_handle.filter(|handle| !handle.is_null()) {
            if !matches!(
                document.objects.get(&handle),
                Some(crate::host::acadrust::objects::ObjectType::TableStyle(_))
            ) {
                return Err(format!("Table style handle {handle:?} does not exist"));
            }
        }
        for (ri, row) in value.rows.iter().enumerate() {
            for (ci, cell) in row.cells.iter().enumerate() {
                for content in &cell.contents {
                    if let Some(handle) = content.text_style_handle.filter(|handle| !handle.is_null()) {
                        if !document.text_styles.iter().any(|style| style.handle == handle) {
                            return Err(format!("Table.rows[{ri}].cells[{ci}] text style handle {handle:?} does not exist"));
                        }
                    }
                    if let Some(handle) = content.block_handle.filter(|handle| !handle.is_null()) {
                        if !document.block_records.iter().any(|record| record.handle == handle) {
                            return Err(format!("Table.rows[{ri}].cells[{ci}] block handle {handle:?} does not exist"));
                        }
                    }
                }
            }
        }
    }
    if let EntityType::MultiLeader(value) = entity {
        use crate::host::acadrust::objects::ObjectType;
        let handle_exists = |handle: Option<crate::host::acadrust::Handle>, what: &str, ok: &dyn Fn(crate::host::acadrust::Handle) -> bool| {
            match handle.filter(|handle| !handle.is_null()) {
                Some(handle) if !ok(handle) => Err(format!("MultiLeader {what} handle {handle:?} does not exist")),
                _ => Ok(()),
            }
        };
        let is_block = |h| document.block_records.iter().any(|record| record.handle == h);
        let is_text_style = |h| document.text_styles.iter().any(|style| style.handle == h);
        let is_linetype = |h| document.line_types.iter().any(|ltype| ltype.handle == h);
        let is_mleader_style = |h| matches!(document.objects.get(&h), Some(ObjectType::MultiLeaderStyle(_)));
        handle_exists(value.style_handle, "style", &is_mleader_style)?;
        handle_exists(value.text_style_handle, "text style", &is_text_style)?;
        handle_exists(value.context.text_style_handle, "context text style", &is_text_style)?;
        handle_exists(value.line_type_handle, "line type", &is_linetype)?;
        handle_exists(value.arrowhead_handle, "arrowhead block", &is_block)?;
        handle_exists(value.block_content_handle, "block content", &is_block)?;
        handle_exists(value.context.block_content_handle, "context block content", &is_block)?;
    }
    if let EntityType::Dimension(value) = entity {
        let name = value.base().style_name.trim();
        if !document.dim_styles.iter().any(|style| style.name.eq_ignore_ascii_case(name)) {
            return Err(format!("Dimension style {:?} does not exist", value.base().style_name));
        }
    }
    if let EntityType::MLine(value) = entity {
        let style = resolve_mline_style(document, value).ok_or_else(|| {
            format!("MLine style {:?} does not exist", value.style_name)
        })?;
        if style.elements.is_empty() {
            return Err(format!("MLine style {:?} has no elements", style.name));
        }
        for (index, vertex) in value.vertices.iter().enumerate() {
            if !vertex.segments.is_empty() && vertex.segments.len() != style.elements.len() {
                return Err(format!(
                    "MLine.vertices[{index}] has {} segments but style {:?} has {} elements",
                    vertex.segments.len(),
                    style.name,
                    style.elements.len()
                ));
            }
        }
    }
    if let EntityType::Leader(value) = entity {
        use crate::host::acadrust::entities::LeaderCreationType;
        if !document
            .dim_styles
            .iter()
            .any(|style| style.name.eq_ignore_ascii_case(value.dimension_style.trim()))
        {
            return Err(format!(
                "Leader dimension style {:?} does not exist",
                value.dimension_style
            ));
        }
        if !value.annotation_handle.is_null() {
            if value.creation_type == LeaderCreationType::NoAnnotation {
                return Err("Leader with no annotation cannot carry an annotation handle".into());
            }
            let annotation = document
                .get_entity(value.annotation_handle)
                .filter(|_| value.annotation_handle != value.common.handle)
                .ok_or_else(|| {
                    format!(
                        "Leader annotation handle {:?} does not exist",
                        value.annotation_handle
                    )
                })?;
            let expected = match value.creation_type {
                LeaderCreationType::WithText => matches!(
                    annotation,
                    EntityType::Text(_) | EntityType::MText(_)
                ),
                LeaderCreationType::WithTolerance => matches!(annotation, EntityType::Tolerance(_)),
                LeaderCreationType::WithBlock => matches!(annotation, EntityType::Insert(_)),
                LeaderCreationType::NoAnnotation => unreachable!(),
            };
            if !expected {
                return Err(format!(
                    "Leader annotation {:?} does not match creation_type {:?}",
                    value.annotation_handle, value.creation_type
                ));
            }
        }
    }
    if let EntityType::AttributeDefinition(value) = entity {
        let owner = entity.common().owner_handle;
        let block = document
            .block_records
            .iter()
            .find(|record| record.handle == owner)
            .ok_or_else(|| {
                "AttributeDefinition requires an existing block-record owner".to_owned()
            })?;
        if block.is_model_space() || block.is_paper_space() {
            return Err("AttributeDefinition owner must be a block definition".into());
        }
        let style = document
            .text_styles
            .get(value.text_style.trim())
            .ok_or_else(|| {
                format!(
                    "AttributeDefinition text style {:?} does not exist",
                    value.text_style
                )
            })?;
        if style.is_shape_file {
            return Err(format!(
                "AttributeDefinition text style {:?} is a shape-file style",
                style.name
            ));
        }
    }
    if let EntityType::AttributeEntity(value) = entity {
        let owner = entity.common().owner_handle;
        let insert = match document.get_entity(owner) {
            Some(EntityType::Insert(insert)) => insert,
            _ => return Err("AttributeEntity requires an existing Insert owner".into()),
        };
        let block = document
            .block_records
            .get(insert.block_name.trim())
            .ok_or_else(|| {
                format!(
                    "AttributeEntity owner references missing block {:?}",
                    insert.block_name
                )
            })?;
        let definition = block
            .entity_handles
            .iter()
            .find_map(|handle| match document.get_entity(*handle) {
                Some(EntityType::AttributeDefinition(definition))
                    if definition.common.handle == value.attdef_handle =>
                {
                    Some(definition)
                }
                _ => None,
            })
            .ok_or_else(|| {
                "AttributeEntity attdef_handle does not belong to the owner's block".to_owned()
            })?;
        if !definition.tag.eq_ignore_ascii_case(value.tag.trim()) {
            return Err("AttributeEntity tag does not match its attribute definition".into());
        }
        let style = document
            .text_styles
            .get(value.text_style.trim())
            .ok_or_else(|| {
                format!(
                    "AttributeEntity text style {:?} does not exist",
                    value.text_style
                )
            })?;
        if style.is_shape_file {
            return Err(format!(
                "AttributeEntity text style {:?} is a shape-file style",
                style.name
            ));
        }
    }
    Ok(())
}

/// Stable handle wins; the name is the fallback for legacy entities.
#[cfg(feature = "host")]
fn resolve_mline_style<'a>(
    document: &'a crate::host::CadDocument,
    mline: &crate::host::acadrust::entities::MLine,
) -> Option<&'a crate::host::acadrust::objects::MLineStyle> {
    use crate::host::acadrust::objects::ObjectType;
    mline
        .style_handle
        .filter(|handle| !handle.is_null())
        .and_then(|handle| match document.objects.get(&handle) {
            Some(ObjectType::MLineStyle(style)) => Some(style),
            _ => None,
        })
        .or_else(|| {
            document.objects.values().find_map(|object| match object {
                ObjectType::MLineStyle(style)
                    if style.name.eq_ignore_ascii_case(mline.style_name.trim()) =>
                {
                    Some(style)
                }
                _ => None,
            })
        })
}

/// Fill name-based table references with stable handles before a new entity or
/// replacement is committed. Existing non-null handles remain authoritative
/// for legacy DWG entities whose fallback name can be stale.
#[cfg(feature = "host")]
pub fn bind_canvas_entity_references(
    document: &crate::host::CadDocument,
    entity: &mut crate::host::EntityType,
) -> Result<(), String> {
    use crate::host::EntityType;
    match entity {
        EntityType::Tolerance(value)
            if value
                .dimension_style_handle
                .filter(|h| !h.is_null())
                .is_none() =>
        {
            value.dimension_style_handle = document
                .dim_styles
                .iter()
                .find(|style| {
                    style
                        .name
                        .eq_ignore_ascii_case(value.dimension_style_name.trim())
                })
                .map(|style| style.handle);
        }
        EntityType::MLine(value) if value.style_handle.filter(|h| !h.is_null()).is_none() => {
            value.style_handle = resolve_mline_style(document, value).map(|style| style.handle);
        }
        EntityType::Shape(value) if value.style_handle.filter(|h| !h.is_null()).is_none() => {
            value.style_handle = document
                .text_styles
                .iter()
                .find(|style| style.name.eq_ignore_ascii_case(value.style_name.trim()))
                .map(|style| style.handle);
        }
        EntityType::AttributeEntity(value) => {
            let insert = match document.get_entity(value.common.owner_handle) {
                Some(EntityType::Insert(insert)) => insert,
                _ => return Err("AttributeEntity requires an existing Insert owner".into()),
            };
            let block = document
                .block_records
                .get(insert.block_name.trim())
                .ok_or_else(|| {
                    format!(
                        "AttributeEntity owner references missing block {:?}",
                        insert.block_name
                    )
                })?;
            let current_matches = block.entity_handles.iter().any(|handle| {
                *handle == value.attdef_handle
                    && matches!(document.get_entity(*handle),
                    Some(EntityType::AttributeDefinition(definition))
                        if definition.tag.eq_ignore_ascii_case(value.tag.trim()))
            });
            if !current_matches {
                value.attdef_handle = block
                    .entity_handles
                    .iter()
                    .find_map(|handle| match document.get_entity(*handle) {
                        Some(EntityType::AttributeDefinition(definition))
                            if definition.tag.eq_ignore_ascii_case(value.tag.trim()) =>
                        {
                            Some(*handle)
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        format!(
                            "AttributeEntity tag {:?} has no definition in block {:?}",
                            value.tag, insert.block_name
                        )
                    })?;
            }
        }
        _ => {}
    }
    validate_canvas_entity_references(document, entity)
}

/// Clone a canvas entity with a new layer, preserving its other snapshot
/// fields. Internal records and opaque fallbacks are excluded.
#[cfg(feature = "host")]
pub fn patch_canvas_layer(
    entity: &crate::host::EntityType,
    layer: &str,
) -> Result<crate::host::EntityType, String> {
    use crate::host::EntityType;
    if layer.trim().is_empty() {
        return Err("layer name is empty".into());
    }
    if matches!(
        entity,
        EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::Extended(_)
            | EntityType::Unknown(_)
    ) {
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
        let catalog: EntityCoverageCatalog =
            serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let registry: TypeRegistry =
            serde_json::from_str(get_embedded_type_registry_json()).unwrap();
        let variants = &registry.types[&TypeId::new("EntityType")].variants;
        assert_eq!(catalog.entity_kinds.len(), variants.len());
        let mut names = std::collections::HashSet::new();
        for entry in &catalog.entity_kinds {
            assert!(names.insert(&entry.kind), "duplicate kind {}", entry.kind);
            assert!(variants.iter().any(|v| v.name == entry.kind));
            assert!(entry
                .properties
                .iter()
                .any(|p| p.name == "handle" && p.model_access == ModelAccess::ReadOnly));
            assert!(entry
                .properties
                .iter()
                .any(|p| p.name == "kind" && p.model_access == ModelAccess::ReadOnly));
            assert!(entry
                .properties
                .iter()
                .any(|p| p.name == "owner_handle" && p.model_access == ModelAccess::ReadOnly));
            let mut source_paths = std::collections::HashSet::new();
            for property in &entry.properties {
                assert!(
                    source_paths.insert(&property.source_path),
                    "duplicate source path in {}: {}",
                    entry.kind,
                    property.source_path
                );
                assert!(
                    property.snapshot_readable,
                    "unreadable traced property in {}: {}",
                    entry.kind, property.name
                );
                if property.model_access == ModelAccess::ReadWrite {
                    assert!(matches!(
                        property.validation.as_str(),
                        "type_conversion_only" | "transaction_geometry" | "transaction_nonempty"
                    ));
                }
            }
        }
    }

    #[test]
    fn unsupported_kinds_have_no_editable_properties() {
        let catalog: EntityCoverageCatalog =
            serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        for entry in &catalog.entity_kinds {
            if entry.scope != EntityScope::Canvas
                || !matches!(
                    entry.kind.as_str(),
                    "Point"
                        | "Line"
                        | "Circle"
                        | "Arc"
                        | "Ellipse"
                        | "Polyline"
                        | "Polyline2D"
                        | "Polyline3D"
                        | "LwPolyline"
                        | "Spline"
                        | "Text"
                        | "MText"
                        | "Ray"
                        | "XLine"
                        | "Solid"
                        | "Face3D"
                        | "Insert"
                        | "Tolerance"
                        | "Shape"
                        | "AttributeDefinition"
                        | "AttributeEntity"
                        | "Hatch"
                        | "Leader"
                        | "MLine"
                        | "Dimension"
                        | "MultiLeader"
                        | "Table"
                )
            {
                assert!(
                    entry
                        .properties
                        .iter()
                        .all(|p| p.model_access != ModelAccess::ReadWrite
                            || (entry.scope == EntityScope::Canvas && p.name == "layer")),
                    "{}",
                    entry.kind
                );
            }
        }
    }

    #[test]
    fn catalog_names_the_fields_with_transaction_geometry_checks() {
        let catalog: EntityCoverageCatalog =
            serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let checked: std::collections::HashSet<_> = catalog
            .entity_kinds
            .iter()
            .flat_map(|entry| {
                entry
                    .properties
                    .iter()
                .filter(|property| property.validation == "transaction_geometry")
                    .map(|property| format!("{}.{}", entry.kind, property.name))
            })
            .collect();
        assert_eq!(
            checked,
            std::collections::HashSet::from([
                "Point.location".to_owned(),
                "Line.start".to_owned(),
                "Line.end".to_owned(),
                "Circle.center".to_owned(),
                "Circle.radius".to_owned(),
                "Arc.center".to_owned(),
                "Arc.radius".to_owned(),
                "Ray.base_point".to_owned(),
                "Ray.direction".to_owned(),
                "XLine.base_point".to_owned(),
                "XLine.direction".to_owned(),
                "Solid.first_corner".to_owned(),
                "Solid.second_corner".to_owned(),
                "Solid.third_corner".to_owned(),
                "Solid.fourth_corner".to_owned(),
                "Solid.normal".to_owned(),
                "Solid.thickness".to_owned(),
                "Face3D.first_corner".to_owned(),
                "Face3D.second_corner".to_owned(),
                "Face3D.third_corner".to_owned(),
                "Face3D.fourth_corner".to_owned(),
            "Face3D.invisible_edges".to_owned(),
                "Insert.insert_point".to_owned(),
                "Insert.x_scale".to_owned(),
                "Insert.y_scale".to_owned(),
                "Insert.z_scale".to_owned(),
                "Insert.rotation".to_owned(),
                "Insert.normal".to_owned(),
                "Insert.column_count".to_owned(),
                "Insert.row_count".to_owned(),
                "Insert.column_spacing".to_owned(),
                "Insert.row_spacing".to_owned(),
                "Tolerance.insertion_point".to_owned(),
                "Tolerance.direction".to_owned(),
                "Tolerance.normal".to_owned(),
                "Tolerance.text".to_owned(),
                "Tolerance.dimension_style_name".to_owned(),
                "Tolerance.text_height".to_owned(),
            "Tolerance.dimension_gap".to_owned(),
                "Shape.insertion_point".to_owned(),
                "Shape.size".to_owned(),
                "Shape.shape_name".to_owned(),
                "Shape.shape_number".to_owned(),
                "Shape.rotation".to_owned(),
                "Shape.relative_x_scale".to_owned(),
                "Shape.oblique_angle".to_owned(),
                "Shape.normal".to_owned(),
                "Shape.thickness".to_owned(),
                "Shape.style_name".to_owned(),
                "AttributeDefinition.tag".to_owned(),
                "AttributeDefinition.prompt".to_owned(),
            "AttributeDefinition.default_value".to_owned(),
            "AttributeDefinition.insertion_point".to_owned(),
            "AttributeDefinition.alignment_point".to_owned(),
                "AttributeDefinition.height".to_owned(),
                "AttributeDefinition.rotation".to_owned(),
            "AttributeDefinition.width_factor".to_owned(),
            "AttributeDefinition.oblique_angle".to_owned(),
            "AttributeDefinition.text_style".to_owned(),
            "AttributeDefinition.text_generation_flags".to_owned(),
            "AttributeDefinition.horizontal_alignment".to_owned(),
            "AttributeDefinition.vertical_alignment".to_owned(),
                "AttributeDefinition.flags".to_owned(),
                "AttributeDefinition.field_length".to_owned(),
                "AttributeDefinition.normal".to_owned(),
                "AttributeDefinition.mtext_flag".to_owned(),
                "AttributeDefinition.is_multiline".to_owned(),
                "AttributeDefinition.line_count".to_owned(),
            "AttributeDefinition.lock_position".to_owned(),
                "AttributeEntity.tag".to_owned(),
                "AttributeEntity.value".to_owned(),
                "AttributeEntity.insertion_point".to_owned(),
                "AttributeEntity.alignment_point".to_owned(),
                "AttributeEntity.height".to_owned(),
                "AttributeEntity.rotation".to_owned(),
                "AttributeEntity.width_factor".to_owned(),
                "AttributeEntity.oblique_angle".to_owned(),
                "AttributeEntity.text_style".to_owned(),
                "AttributeEntity.text_generation_flags".to_owned(),
                "AttributeEntity.horizontal_alignment".to_owned(),
                "AttributeEntity.vertical_alignment".to_owned(),
                "AttributeEntity.flags".to_owned(),
                "AttributeEntity.field_length".to_owned(),
                "AttributeEntity.normal".to_owned(),
                "AttributeEntity.mtext_flag".to_owned(),
                "AttributeEntity.is_multiline".to_owned(),
                "AttributeEntity.line_count".to_owned(),
                "AttributeEntity.lock_position".to_owned(),
                "Hatch.elevation".to_owned(),
                "Hatch.normal".to_owned(),
                "Hatch.is_solid".to_owned(),
                "Hatch.pattern".to_owned(),
                "Hatch.pattern_angle".to_owned(),
                "Hatch.pattern_scale".to_owned(),
                "Hatch.pattern_type".to_owned(),
                "Hatch.is_double".to_owned(),
                "Hatch.style".to_owned(),
                "Hatch.is_associative".to_owned(),
                "Hatch.pixel_size".to_owned(),
                "Hatch.paths".to_owned(),
                "Hatch.seed_points".to_owned(),
                "Leader.dimension_style".to_owned(),
                "Leader.arrow_enabled".to_owned(),
                "Leader.path_type".to_owned(),
                "Leader.creation_type".to_owned(),
                "Leader.hookline_direction".to_owned(),
                "Leader.hookline_enabled".to_owned(),
                "Leader.text_height".to_owned(),
                "Leader.text_width".to_owned(),
                "Leader.vertices".to_owned(),
                "Leader.annotation_handle".to_owned(),
                "Leader.override_color".to_owned(),
                "Leader.normal".to_owned(),
                "Leader.horizontal_direction".to_owned(),
                "Leader.block_offset".to_owned(),
                "Leader.annotation_offset".to_owned(),
                "MLine.flags".to_owned(),
                "MLine.justification".to_owned(),
                "MLine.normal".to_owned(),
                "MLine.scale_factor".to_owned(),
                "MLine.style_name".to_owned(),
                "MLine.vertices".to_owned(),
                "Dimension.text".to_owned(),
                "Dimension.style_name".to_owned(),
                "Dimension.normal".to_owned(),
                "Dimension.text_middle_point".to_owned(),
                "Dimension.attachment_point".to_owned(),
                "Dimension.text_rotation".to_owned(),
                "Dimension.horizontal_direction".to_owned(),
                "Dimension.flip_arrow1".to_owned(),
                "Dimension.flip_arrow2".to_owned(),
                "Dimension.text_user_positioned".to_owned(),
                "Dimension.definition_point".to_owned(),
                "Dimension.first_point".to_owned(),
                "Dimension.second_point".to_owned(),
                "Dimension.angle_vertex".to_owned(),
                "Dimension.dimension_arc".to_owned(),
                "Dimension.feature_location".to_owned(),
                "Dimension.leader_endpoint".to_owned(),
                "Dimension.first_extension_point".to_owned(),
                "Dimension.second_extension_point".to_owned(),
                "Dimension.center_point".to_owned(),
                "Dimension.first_leader_point".to_owned(),
                "Dimension.second_leader_point".to_owned(),
                "Dimension.chord_point".to_owned(),
                "Dimension.override_center".to_owned(),
                "Dimension.jog_point".to_owned(),
                "Dimension.rotation".to_owned(),
                "Dimension.ext_line_rotation".to_owned(),
                "Dimension.leader_length".to_owned(),
                "Dimension.arc_start_parameter".to_owned(),
                "Dimension.arc_end_parameter".to_owned(),
                "Dimension.jog_angle".to_owned(),
                "Dimension.is_ordinate_type_x".to_owned(),
                "Dimension.is_partial".to_owned(),
                "Dimension.has_leader".to_owned(),
                "MultiLeader.content_type".to_owned(),
                "MultiLeader.path_type".to_owned(),
                "MultiLeader.line_color".to_owned(),
                "MultiLeader.line_weight".to_owned(),
                "MultiLeader.enable_landing".to_owned(),
                "MultiLeader.enable_dogleg".to_owned(),
                "MultiLeader.dogleg_length".to_owned(),
                "MultiLeader.arrowhead_size".to_owned(),
                "MultiLeader.text_color".to_owned(),
                "MultiLeader.text_frame".to_owned(),
                "MultiLeader.text_left_attachment".to_owned(),
                "MultiLeader.text_right_attachment".to_owned(),
                "MultiLeader.text_top_attachment".to_owned(),
                "MultiLeader.text_bottom_attachment".to_owned(),
                "MultiLeader.text_attachment_direction".to_owned(),
                "MultiLeader.text_attachment_point".to_owned(),
                "MultiLeader.text_alignment".to_owned(),
                "MultiLeader.text_angle_type".to_owned(),
                "MultiLeader.text_direction_negative".to_owned(),
                "MultiLeader.scale_factor".to_owned(),
                "MultiLeader.enable_annotation_scale".to_owned(),
                "MultiLeader.extend_leader_to_text".to_owned(),
                "MultiLeader.block_content_color".to_owned(),
                "MultiLeader.block_connection_type".to_owned(),
                "MultiLeader.block_rotation".to_owned(),
                "MultiLeader.block_scale".to_owned(),
                "MultiLeader.context".to_owned(),
                "MultiLeader.style_handle".to_owned(),
                "MultiLeader.text_style_handle".to_owned(),
                "MultiLeader.arrowhead_handle".to_owned(),
                "MultiLeader.line_type_handle".to_owned(),
                "MultiLeader.block_content_handle".to_owned(),
                "Table.insertion_point".to_owned(),
                "Table.horizontal_direction".to_owned(),
                "Table.normal".to_owned(),
                "Table.table_style_handle".to_owned(),
                "Table.rows".to_owned(),
                "Table.columns".to_owned(),
                "Table.merged_ranges".to_owned(),
                "Table.break_spacing".to_owned(),
                "Table.break_flow_direction".to_owned(),
                "Table.break_options".to_owned(),
            ])
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn tolerance_geometry_and_style_references_are_validated() {
        use crate::host::acadrust::{self, types::Vector3};
        let document = acadrust::CadDocument::new();
        let mut tolerance = acadrust::entities::Tolerance::with_text(
            Vector3::new(1.0, 2.0, 0.0),
            "{\\Fgdt;p}%%v0.1",
        );
        let entity = acadrust::EntityType::Tolerance(tolerance.clone());
        validate_new_canvas_entity(&entity).unwrap();
        validate_canvas_entity_references(&document, &entity).unwrap();
        tolerance.direction = Vector3::new(2.0, 0.0, 0.0);
        assert!(
            validate_new_canvas_entity(&acadrust::EntityType::Tolerance(tolerance.clone()))
                .unwrap_err()
                .contains("unit vector")
        );
        tolerance.direction = Vector3::UNIT_X;
        tolerance.dimension_style_name = "Missing".into();
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Tolerance(tolerance)
        )
        .unwrap_err()
        .contains("does not exist"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn leader_geometry_and_annotation_references_are_validated() {
        use crate::host::acadrust::{self, entities::LeaderCreationType, types::Vector3};
        let mut document = acadrust::CadDocument::new();
        let mut text = acadrust::entities::Text::new();
        text.common.handle = document.allocate_handle();
        let text_handle = text.common.handle;
        document.add_entity(acadrust::EntityType::Text(text)).unwrap();
        let mut leader = acadrust::entities::Leader::two_point(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 5.0, 0.0),
        );
        leader.annotation_handle = text_handle;
        let entity = acadrust::EntityType::Leader(leader.clone());
        validate_new_canvas_entity(&entity).unwrap();
        validate_canvas_entity_references(&document, &entity).unwrap();

        let mut wrong_kind = leader.clone();
        wrong_kind.creation_type = LeaderCreationType::WithTolerance;
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Leader(wrong_kind)
        )
        .unwrap_err()
        .contains("does not match"));
        let mut missing = leader.clone();
        missing.annotation_handle = acadrust::Handle::new(0xdead);
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Leader(missing)
        )
        .unwrap_err()
        .contains("does not exist"));
        let mut none_with_handle = leader.clone();
        none_with_handle.creation_type = LeaderCreationType::NoAnnotation;
        assert!(validate_new_canvas_entity(&acadrust::EntityType::Leader(none_with_handle))
            .unwrap_err()
            .contains("no annotation"));
        let mut one_point = leader.clone();
        one_point.vertices.truncate(1);
        assert!(validate_new_canvas_entity(&acadrust::EntityType::Leader(one_point))
            .unwrap_err()
            .contains("at least 2"));
        let mut bad_style = leader.clone();
        bad_style.dimension_style = "Missing".into();
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Leader(bad_style)
        )
        .unwrap_err()
        .contains("dimension style"));

        let mut edited = leader.clone();
        edited.vertices[1] = Vector3::new(f64::NAN, 0.0, 0.0);
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Leader(leader.clone()),
            &acadrust::EntityType::Leader(edited)
        )
        .unwrap_err()
        .contains("finite"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn mline_geometry_and_style_references_are_validated_and_bound() {
        use crate::host::acadrust::{self, entities::MLineVertex, types::Vector3};
        let document = acadrust::CadDocument::new();
        let mut mline = acadrust::entities::MLine::new();
        for x in [0.0, 5.0] {
            mline.vertices.push(MLineVertex::new(Vector3::new(x, 0.0, 0.0)));
        }
        let mut entity = acadrust::EntityType::MLine(mline.clone());
        validate_new_canvas_entity(&entity).unwrap();
        bind_canvas_entity_references(&document, &mut entity).unwrap();
        assert!(matches!(&entity, acadrust::EntityType::MLine(value) if value.style_handle.is_some()));

        let mut missing = mline.clone();
        missing.style_name = "Missing".into();
        assert!(bind_canvas_entity_references(&document, &mut acadrust::EntityType::MLine(missing))
            .unwrap_err().contains("does not exist"));
        let mut wrong_segments = mline.clone();
        wrong_segments.vertices[0].init_segments(3);
        assert!(bind_canvas_entity_references(&document, &mut acadrust::EntityType::MLine(wrong_segments))
            .unwrap_err().contains("segments but style"));
        let mut one = mline.clone();
        one.vertices.truncate(1);
        assert!(validate_new_canvas_entity(&acadrust::EntityType::MLine(one)).unwrap_err().contains("at least 2"));
        let mut closed = mline.clone();
        closed.flags |= acadrust::entities::MLineFlags::CLOSED;
        assert!(validate_new_canvas_entity(&acadrust::EntityType::MLine(closed)).unwrap_err().contains("at least 3"));
        let mut scale = mline.clone();
        scale.scale_factor = 0.0;
        assert!(validate_new_canvas_entity(&acadrust::EntityType::MLine(scale)).unwrap_err().contains("nonzero"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn shape_geometry_and_shape_file_style_are_validated_and_bound() {
        use crate::host::acadrust::{self, tables::TableEntry, types::Vector3};
        let mut document = acadrust::CadDocument::new();
        let mut style = acadrust::tables::TextStyle::new("Symbols");
        style.set_handle(document.allocate_handle());
        style.is_shape_file = true;
        style.font_file = "symbols.shx".into();
        let style_handle = style.handle;
        document.text_styles.add(style).unwrap();
        let mut shape = acadrust::entities::Shape::with_style(
            Vector3::new(1.0, 2.0, 0.0),
            "ARROW",
            "Symbols",
            2.0,
            0.5,
        );
        shape.shape_number = 1;
        let mut entity = acadrust::EntityType::Shape(shape);
        validate_new_canvas_entity(&entity).unwrap();
        bind_canvas_entity_references(&document, &mut entity).unwrap();
        assert!(matches!(entity, acadrust::EntityType::Shape(ref value)
            if value.style_handle == Some(style_handle)));
        let acadrust::EntityType::Shape(mut bad) = entity else {
            unreachable!()
        };
        bad.size = 0.0;
        assert!(
            validate_new_canvas_entity(&acadrust::EntityType::Shape(bad))
                .unwrap_err()
                .contains("greater than zero")
        );
    }

    #[test]
    fn layer_write_coverage_matches_canvas_scope() {
        let catalog: EntityCoverageCatalog =
            serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        assert_eq!(
            catalog
                .entity_kinds
                .iter()
                .filter(|entry| entry.scope == EntityScope::Canvas)
                .count(),
            43
        );
        for entry in &catalog.entity_kinds {
            let layer = entry
                .properties
                .iter()
                .find(|property| property.name == "layer")
                .unwrap();
            assert_eq!(
                layer.model_access == ModelAccess::ReadWrite,
                entry.scope == EntityScope::Canvas
            );
            if entry.scope == EntityScope::Canvas {
                assert_eq!(layer.validation, "transaction_nonempty");
            }
        }
    }

    #[test]
    fn insert_catalog_separates_transform_writes_from_block_identity() {
        let catalog: EntityCoverageCatalog =
            serde_json::from_str(get_embedded_entity_coverage_json()).unwrap();
        let insert = catalog
            .entity_kinds
            .iter()
            .find(|entry| entry.kind == "Insert")
            .unwrap();
        let block_name = insert
            .properties
            .iter()
            .find(|property| property.name == "block_name")
            .unwrap();
        assert_eq!(block_name.model_access, ModelAccess::ReadOnly);
        assert_eq!(block_name.validation, "none");
        assert!(
            insert
                .properties
                .iter()
                .find(|property| property.name == "attributes")
                .unwrap()
                .model_access
                == ModelAccess::Unmapped
        );
        assert!(
            insert
                .properties
                .iter()
                .find(|property| property.name == "insert_point")
                .unwrap()
                .model_access
                == ModelAccess::ReadWrite
        );
    }

    #[cfg(feature = "host")]
    #[test]
    fn unsupported_canvas_kind_has_typed_snapshot_fields() {
        use crate::host::acadrust;
        let hatch = acadrust::EntityType::Hatch(acadrust::entities::Hatch::default());
        let snapshot = entity_snapshot(&hatch).unwrap();
        let fields = snapshot
            .get("Hatch")
            .and_then(|value| value.as_object())
            .unwrap();
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
        )
        .is_ok());
        let mut bad_radius = before.clone();
        bad_radius.radius = -1.0;
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Circle(before),
            &acadrust::EntityType::Circle(bad_radius),
        )
        .unwrap_err()
        .contains("Circle.radius"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn new_phase_seven_shapes_reject_invalid_geometry_and_keep_legacy_fields() {
        use crate::host::acadrust::{self, types::Vector3};
        let mut ray = acadrust::entities::Ray::default();
        ray.direction = Vector3::new(2.0, 0.0, 0.0);
        assert!(
            validate_new_canvas_entity(&acadrust::EntityType::Ray(ray.clone()))
                .unwrap_err()
                .contains("Ray.direction")
        );
        let before = acadrust::EntityType::Ray(ray.clone());
        ray.common.layer = "CONSTRUCTION".into();
        assert!(validate_entity_mutation(&before, &acadrust::EntityType::Ray(ray.clone())).is_ok());
        ray.direction = Vector3::new(0.0, 1.0, 0.0);
        assert!(validate_entity_mutation(&before, &acadrust::EntityType::Ray(ray)).is_ok());

        let mut xline = acadrust::entities::XLine::default();
        xline.base_point.x = f64::NAN;
        assert!(
            validate_new_canvas_entity(&acadrust::EntityType::XLine(xline))
                .unwrap_err()
                .contains("XLine.base_point")
        );

        let solid = acadrust::entities::Solid::new(
            Vector3::ZERO,
            Vector3::UNIT_X,
            Vector3::UNIT_Y,
            Vector3::ZERO,
        );
        let mut changed = solid.clone();
        changed.thickness = f64::INFINITY;
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Solid(solid),
            &acadrust::EntityType::Solid(changed)
        )
        .unwrap_err()
        .contains("Solid.thickness"));

        let face = acadrust::entities::Face3D::new(
            Vector3::ZERO,
            Vector3::UNIT_X,
            Vector3::UNIT_Y,
            Vector3::ZERO,
        );
        let mut changed = face.clone();
        changed.invisible_edges = acadrust::entities::InvisibleEdgeFlags::from_bits(0x10);
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Face3D(face),
            &acadrust::EntityType::Face3D(changed)
        )
        .unwrap_err()
        .contains("Face3D.invisible_edges"));
    }

    #[cfg(feature = "host")]
    #[test]
    fn insert_transaction_checks_transforms_and_protects_references() {
        use crate::host::acadrust::{self, types::Vector3};
        let before = acadrust::entities::Insert::new("DOOR", Vector3::new(1.0, 2.0, 0.0));
        let mut moved = before.clone();
        moved.insert_point = Vector3::new(5.0, 6.0, 0.0);
        moved.set_x_scale(2.0);
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(moved)
        )
        .is_ok());
        let mut bad = before.clone();
        bad.set_y_scale(f64::NAN);
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(bad)
        )
        .unwrap_err()
        .contains("Insert.y_scale"));
        let mut bad = before.clone();
        bad.column_count = 0;
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Insert(before.clone()),
            &acadrust::EntityType::Insert(bad)
        )
        .unwrap_err()
        .contains("array counts"));
        let mut bad = before.clone();
        bad.block_name = "OTHER".into();
        assert!(validate_entity_mutation(
            &acadrust::EntityType::Insert(before),
            &acadrust::EntityType::Insert(bad)
        )
        .unwrap_err()
        .contains("block identity"));
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

    #[cfg(feature = "host")]
    #[test]
    fn attribute_definition_requires_a_block_owner_and_text_style() {
        use crate::host::acadrust::{
            self,
            entities::{Block, BlockEnd},
            types::{Handle, Vector3},
        };
        let mut document = acadrust::CadDocument::new();
        let next = document.next_handle();
        let record_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let end_handle = Handle::new(next + 2);
        let mut record = acadrust::tables::BlockRecord::new("TAGBLOCK");
        record.handle = record_handle;
        record.block_entity_handle = block_handle;
        record.block_end_handle = end_handle;
        document.block_records.add(record).unwrap();
        let mut block = Block::new("TAGBLOCK", Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = record_handle;
        document
            .add_entity(acadrust::EntityType::Block(block))
            .unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = record_handle;
        document
            .add_entity(acadrust::EntityType::BlockEnd(end))
            .unwrap();

        let mut definition = acadrust::entities::AttributeDefinition::new(
            "PART_NO".into(),
            "Part number".into(),
            "PN-001".into(),
        );
        definition.common.owner_handle = record_handle;
        definition.insertion_point = Vector3::new(1.0, 2.0, 0.0);
        let entity = acadrust::EntityType::AttributeDefinition(definition.clone());
        validate_new_canvas_entity(&entity).unwrap();
        validate_canvas_entity_references(&document, &entity).unwrap();

        definition.common.owner_handle = Handle::NULL;
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::AttributeDefinition(definition.clone())
        )
        .unwrap_err()
        .contains("block-record owner"));
        definition.common.owner_handle = record_handle;
        definition.text_style = "Missing".into();
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::AttributeDefinition(definition.clone())
        )
        .unwrap_err()
        .contains("does not exist"));
        definition.text_style = "Standard".into();
        definition.tag = "BAD TAG".into();
        assert!(
            validate_new_canvas_entity(&acadrust::EntityType::AttributeDefinition(definition))
                .unwrap_err()
                .contains("no whitespace")
        );
    }
    #[test]
    fn hatch_geometry_and_boundaries_are_validated() {
        use acadrust::entities::hatch::{BoundaryEdge, BoundaryPath, LineEdge};
        let mut hatch = acadrust::entities::Hatch::new();
        let mut path = BoundaryPath::new();
        for (start, end) in [
            ((0.0, 0.0), (10.0, 0.0)),
            ((10.0, 0.0), (10.0, 10.0)),
            ((10.0, 10.0), (0.0, 10.0)),
            ((0.0, 10.0), (0.0, 0.0)),
        ] {
            path.edges.push(BoundaryEdge::Line(LineEdge {
                start: acadrust::types::Vector2::new(start.0, start.1),
                end: acadrust::types::Vector2::new(end.0, end.1),
            }));
        }
        hatch.paths.push(path);
        let entity = acadrust::EntityType::Hatch(hatch.clone());
        validate_new_canvas_entity(&entity).unwrap();

        let mut invalid = hatch.clone();
        invalid.paths.clear();
        assert!(validate_new_canvas_entity(&acadrust::EntityType::Hatch(invalid))
            .unwrap_err()
            .contains("at least one boundary path"));

        let mut invalid = hatch.clone();
        invalid.is_solid = false;
        invalid.pattern_scale = 0.0;
        assert!(validate_new_canvas_entity(&acadrust::EntityType::Hatch(invalid))
            .unwrap_err()
            .contains("greater than zero"));

        let mut document = acadrust::CadDocument::new();
        let boundary = document.add_entity(acadrust::EntityType::Line(
            acadrust::entities::Line::from_points(
                acadrust::types::Vector3::ZERO,
                acadrust::types::Vector3::UNIT_X,
            ),
        )).unwrap();
        let mut associative = hatch.clone();
        associative.is_associative = true;
        associative.paths[0].boundary_handles.push(boundary);
        validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Hatch(associative.clone()),
        ).unwrap();
        associative.paths[0].boundary_handles[0] = acadrust::Handle::new(u64::MAX);
        assert!(validate_canvas_entity_references(
            &document,
            &acadrust::EntityType::Hatch(associative),
        ).unwrap_err().contains("does not exist"));
    }
}
