use acadrust::{EntityType, Handle};
use glam::DVec3;

use crate::command::{CadCommand, CmdOption, CmdResult, CoincidentPick};
use crate::scene::parametric_constraints::{
    measured_expression, resolve_point, ConstraintKind, ParametricRef,
};

/// Which distance a dimensional constraint measures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DimConstraintAxis {
    /// Horizontal or vertical, decided by where the dimension line goes.
    Linear,
    Horizontal,
    Vertical,
    Aligned,
}

#[derive(Clone, Copy)]
enum Step {
    /// `Specify first constraint point or [Object] <Object>:`
    First,
    /// `Select object:`
    Object,
    /// `Specify second constraint point:`
    Second {
        first: ParametricRef,
        first_point: DVec3,
    },
    /// `Specify dimension line location:`
    Location {
        first: ParametricRef,
        second: ParametricRef,
        first_point: DVec3,
        second_point: DVec3,
    },
    /// `Enter value or name and value <d1=100>:`
    Value {
        first: ParametricRef,
        second: ParametricRef,
        first_point: DVec3,
        second_point: DVec3,
        location: DVec3,
        axis: DVec3,
        kind: ConstraintKind,
        measured: f64,
    },
}

/// The reference's Linear/Horizontal/Vertical/Aligned dimensional constraint:
/// two constraint points (or one object's ends), a dimension line location,
/// then the parameter name and expression the dynamic dimension carries.
pub struct DimConstraintCommand {
    axis: DimConstraintAxis,
    step: Step,
    picked_entity: Option<EntityType>,
    /// The next free `dN` the host reserved for this constraint.
    default_name: String,
}

impl DimConstraintCommand {
    pub const NO_OBJECT: &'static str = "No object found.";
    pub const NO_POINT: &'static str = "No valid constraint point found.";
    pub const SAME_POINT: &'static str =
        "The object or point is already selected.  Select a different object or constraint point.";

    pub fn new(axis: DimConstraintAxis, default_name: String) -> Self {
        Self {
            axis,
            step: Step::First,
            picked_entity: None,
            default_name,
        }
    }

    fn command_name(&self) -> &'static str {
        match self.axis {
            DimConstraintAxis::Linear => "DCLINEAR",
            DimConstraintAxis::Horizontal => "DCHORIZONTAL",
            DimConstraintAxis::Vertical => "DCVERTICAL",
            DimConstraintAxis::Aligned => "DCALIGNED",
        }
    }

    fn noun(&self) -> &'static str {
        match self.axis {
            DimConstraintAxis::Linear => "Linear",
            DimConstraintAxis::Horizontal => "Horizontal",
            DimConstraintAxis::Vertical => "Vertical",
            DimConstraintAxis::Aligned => "Aligned",
        }
    }

    fn report(message: &str) -> CmdResult {
        CmdResult::ReportError(message.to_string())
    }

    /// The two end points an object pick constrains: a line's or an arc's
    /// ends, the picked polyline segment's vertices.
    fn object_ends(
        entity: &EntityType,
        handle: Handle,
        point: DVec3,
    ) -> Option<(ParametricRef, ParametricRef)> {
        match entity {
            EntityType::Line(_) | EntityType::Arc(_) => {
                Some((ParametricRef::point(handle, 0), ParametricRef::point(handle, 1)))
            }
            EntityType::LwPolyline(_) | EntityType::Polyline2D(_) => {
                let (source, _, _) =
                    crate::scene::centerline::picked_source(entity, handle, point)?;
                let index = source.segment_index;
                // A closed polyline's last segment ends at the first vertex.
                let end = if resolve_point(entity, index + 1).is_some() {
                    index + 1
                } else {
                    0
                };
                Some((ParametricRef::point(handle, index), ParametricRef::point(handle, end)))
            }
            _ => None,
        }
    }

    fn world(entity: &EntityType, reference: ParametricRef) -> Option<DVec3> {
        let point = resolve_point(entity, reference.marker?)?;
        Some(DVec3::new(point.x, point.y, point.z))
    }

    /// The measured kind and axis: Linear reads the dimension line location
    /// the way a linear dimension does, the others are fixed.
    fn decide(&self, first: DVec3, second: DVec3, location: DVec3) -> (ConstraintKind, DVec3) {
        match self.axis {
            DimConstraintAxis::Linear => {
                let axis =
                    crate::modules::annotate::linear_dim::measure_axis(first, second, location);
                if axis.x.abs() > 0.5 {
                    (ConstraintKind::DistanceX, DVec3::X)
                } else {
                    (ConstraintKind::DistanceY, DVec3::Y)
                }
            }
            DimConstraintAxis::Horizontal => (ConstraintKind::DistanceX, DVec3::X),
            DimConstraintAxis::Vertical => (ConstraintKind::DistanceY, DVec3::Y),
            DimConstraintAxis::Aligned => (
                ConstraintKind::Distance,
                (second - first).normalize_or(DVec3::X),
            ),
        }
    }

    fn measured(kind: ConstraintKind, first: DVec3, second: DVec3) -> f64 {
        match kind {
            ConstraintKind::DistanceX => (second.x - first.x).abs(),
            ConstraintKind::DistanceY => (second.y - first.y).abs(),
            _ => (second - first).length(),
        }
    }

    fn build(&self, name: String, expression: String) -> Option<CmdResult> {
        let Step::Value {
            first,
            second,
            first_point,
            second_point,
            location,
            axis,
            kind,
            ..
        } = self.step
        else {
            return None;
        };
        if expression.trim().is_empty() {
            return None;
        }
        Some(CmdResult::AddDimensionalConstraint {
            kind,
            first,
            second,
            first_point,
            second_point,
            location,
            axis,
            name,
            expression,
            label: match self.axis {
                DimConstraintAxis::Linear => "Linear constraint",
                DimConstraintAxis::Horizontal => "Horizontal distance constraint",
                DimConstraintAxis::Vertical => "Vertical distance constraint",
                DimConstraintAxis::Aligned => "Aligned constraint",
            },
        })
    }
}

impl CadCommand for DimConstraintCommand {
    fn name(&self) -> &'static str {
        self.command_name()
    }

    fn prompt(&self) -> String {
        let name = self.command_name();
        match self.step {
            Step::First => {
                format!("{name}  Specify first constraint point or [Object] <Object>:")
            }
            Step::Object => format!("{name}  Select object:"),
            Step::Second { .. } => format!("{name}  Specify second constraint point:"),
            Step::Location { .. } => format!("{name}  Specify dimension line location:"),
            Step::Value { measured, .. } => format!(
                "{name}  Enter value or name and value <{}={}>:",
                self.default_name,
                measured_expression(measured)
            ),
        }
    }

    fn options(&self) -> Vec<CmdOption> {
        matches!(self.step, Step::First)
            .then(|| vec![CmdOption::new("Object", "O")])
            .unwrap_or_default()
    }

    fn wants_text_input(&self) -> bool {
        true
    }

    fn point_step_accepts_keywords(&self) -> bool {
        matches!(self.step, Step::First | Step::Value { .. })
    }

    fn on_text_input(&mut self, text: &str) -> Option<CmdResult> {
        match self.step {
            Step::First => {
                let keyword = text.trim().trim_start_matches('_').to_ascii_uppercase();
                if matches!(keyword.as_str(), "O" | "OBJECT") {
                    self.step = Step::Object;
                    return Some(CmdResult::NeedPoint);
                }
                None
            }
            Step::Value { .. } => {
                let text = text.trim();
                let (name, expression) = match text.split_once('=') {
                    Some((name, expression)) => (name.trim().to_string(), expression.trim().to_string()),
                    None => (self.default_name.clone(), text.to_string()),
                };
                self.build(name, expression)
            }
            _ => None,
        }
    }

    fn needs_entity_pick(&self) -> bool {
        matches!(self.step, Step::First | Step::Object | Step::Second { .. })
    }

    fn entity_pick_accepts_points(&self) -> bool {
        true
    }

    fn typed_point_picks_entity(&self) -> bool {
        true
    }

    fn entity_pick_highlights_hover(&self) -> bool {
        true
    }

    fn inject_before_entity_pick(&self) -> bool {
        true
    }

    fn inject_picked_entity(&mut self, entity: EntityType) {
        self.picked_entity = Some(entity);
    }

    fn on_entity_pick(&mut self, handle: Handle, point: DVec3) -> CmdResult {
        match self.step {
            // The host resolves the constraint point and hands it back
            // through `accept_constraint_point`.
            Step::First | Step::Second { .. } => CmdResult::CheckConstraintPoint(CoincidentPick {
                handle: (!handle.is_null()).then_some(handle),
                point,
                whole_curve: false,
            }),
            Step::Object => {
                if handle.is_null() {
                    return Self::report(Self::NO_OBJECT);
                }
                let Some(entity) = self.picked_entity.take() else {
                    return CmdResult::NeedPoint;
                };
                let Some((first, second)) = Self::object_ends(&entity, handle, point) else {
                    return Self::report(&format!(
                        "Invalid selection for {}. Select a line, polyline segment or arc.",
                        self.noun()
                    ));
                };
                let (Some(first_point), Some(second_point)) =
                    (Self::world(&entity, first), Self::world(&entity, second))
                else {
                    return Self::report(Self::NO_POINT);
                };
                self.step = Step::Location {
                    first,
                    second,
                    first_point,
                    second_point,
                };
                CmdResult::NeedPoint
            }
            Step::Location { .. } | Step::Value { .. } => self.on_point(point),
        }
    }

    fn accept_constraint_point(&mut self, reference: ParametricRef, point: DVec3) -> CmdResult {
        match self.step {
            Step::First => {
                self.step = Step::Second {
                    first: reference,
                    first_point: point,
                };
                CmdResult::NeedPoint
            }
            Step::Second { first, first_point } => {
                if first == reference {
                    return Self::report(Self::SAME_POINT);
                }
                self.step = Step::Location {
                    first,
                    second: reference,
                    first_point,
                    second_point: point,
                };
                CmdResult::NeedPoint
            }
            _ => CmdResult::NeedPoint,
        }
    }

    fn on_point(&mut self, point: DVec3) -> CmdResult {
        match self.step {
            Step::First | Step::Second { .. } => CmdResult::CheckConstraintPoint(CoincidentPick {
                handle: None,
                point,
                whole_curve: false,
            }),
            Step::Object => Self::report(Self::NO_OBJECT),
            Step::Location {
                first,
                second,
                first_point,
                second_point,
            } => {
                let (kind, axis) = self.decide(first_point, second_point, point);
                let measured = Self::measured(kind, first_point, second_point);
                self.step = Step::Value {
                    first,
                    second,
                    first_point,
                    second_point,
                    location: point,
                    axis,
                    kind,
                    measured,
                };
                CmdResult::ReportMeasurement(format!("Dimension text = {measured:.4}"))
            }
            Step::Value { .. } => CmdResult::NeedPoint,
        }
    }

    fn on_enter(&mut self) -> CmdResult {
        match self.step {
            Step::First => {
                self.step = Step::Object;
                CmdResult::NeedPoint
            }
            Step::Value { measured, .. } => self
                .build(self.default_name.clone(), measured_expression(measured))
                .unwrap_or(CmdResult::Cancel),
            _ => CmdResult::Cancel,
        }
    }

    fn on_escape(&mut self) -> CmdResult {
        CmdResult::Cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::Line;
    use acadrust::types::Vector3;

    #[test]
    fn object_pick_takes_the_line_ends_then_asks_for_the_location() {
        let mut command = DimConstraintCommand::new(DimConstraintAxis::Linear, "d1".into());
        assert!(matches!(command.on_enter(), CmdResult::NeedPoint));
        command.inject_picked_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(100.0, 50.0, 0.0),
        )));
        assert!(matches!(
            command.on_entity_pick(Handle::new(7), DVec3::new(50.0, 25.0, 0.0)),
            CmdResult::NeedPoint
        ));
        // Below the pair: horizontal, so the measured value is the X span.
        let CmdResult::ReportMeasurement(text) = command.on_point(DVec3::new(50.0, -30.0, 0.0))
        else {
            panic!("the location must report the dimension text");
        };
        assert_eq!(text, "Dimension text = 100.0000");
        assert_eq!(
            command.prompt(),
            "DCLINEAR  Enter value or name and value <d1=100>:"
        );
        let CmdResult::AddDimensionalConstraint {
            kind, name, expression, ..
        } = command.on_text_input("w=d1*2").expect("a value builds the constraint")
        else {
            panic!("the value must build the constraint");
        };
        assert_eq!(kind, ConstraintKind::DistanceX);
        assert_eq!(name, "w");
        assert_eq!(expression, "d1*2");
    }

    #[test]
    fn a_location_beside_the_pair_measures_vertically() {
        let mut command = DimConstraintCommand::new(DimConstraintAxis::Linear, "d1".into());
        let first = ParametricRef::point(Handle::new(7), 0);
        let second = ParametricRef::point(Handle::new(7), 1);
        command.accept_constraint_point(first, DVec3::ZERO);
        command.accept_constraint_point(second, DVec3::new(100.0, 50.0, 0.0));
        let CmdResult::ReportMeasurement(text) = command.on_point(DVec3::new(140.0, 25.0, 0.0))
        else {
            panic!("the location must report the dimension text");
        };
        assert_eq!(text, "Dimension text = 50.0000");
    }
}
