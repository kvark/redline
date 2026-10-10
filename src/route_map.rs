//! Display-only, heading-up route guidance. No steering or race state is changed.

use crate::{planet, race, vehicle::Isometry};

/// Azimuthal equidistant map of the planet surface, in meters. X points right;
/// negative Y points forward, matching screen coordinates. Radial height does
/// not affect the map, so jumping does not move the ribbon underneath it.
struct HeadingMap {
    up: glam::Vec3,
    forward: glam::Vec3,
    right: glam::Vec3,
    radius: f32,
}

impl HeadingMap {
    fn new(
        position: glam::Vec3,
        forward: glam::Vec3,
        fallback_forward: glam::Vec3,
        radius: f32,
    ) -> Self {
        let up = position.try_normalize().unwrap_or(glam::Vec3::Y);
        let forward = forward
            .reject_from(up)
            .try_normalize()
            .or_else(|| fallback_forward.reject_from(up).try_normalize())
            .unwrap_or_else(|| up.any_orthonormal_vector());
        Self {
            up,
            forward,
            // Match the game's right-handed chase view (look_at_rh), whose
            // screen-right axis is forward × up rather than the craft's +X.
            right: forward.cross(up),
            radius,
        }
    }

    fn project(&self, position: glam::Vec3) -> Option<glam::Vec2> {
        let direction = position.try_normalize()?;
        let cosine = direction.dot(self.up).clamp(-1.0, 1.0);
        let tangent = direction - self.up * cosine;
        let sine = tangent.length();
        if sine < 1e-6 {
            // Antipodal points have no unique bearing. Never draw them at the
            // player, as an ordinary tangent-plane projection would do.
            return (cosine > 0.0).then_some(glam::Vec2::ZERO);
        }
        let distance = sine.atan2(cosine) * self.radius;
        let bearing = tangent / sine;
        Some(glam::Vec2::new(bearing.dot(self.right), -bearing.dot(self.forward)) * distance)
    }
}

/// Keep distant targets at the map border while preserving their bearing.
fn border_marker(point: glam::Vec2, half_size: glam::Vec2) -> (glam::Vec2, bool) {
    let extent = (point.abs() / half_size).max_element().max(1.0);
    (point / extent, extent > 1.0)
}

fn bearing_label(point: glam::Vec2) -> &'static str {
    if point.length_squared() < 1.0 {
        return "here";
    }
    let angle = point.x.atan2(-point.y);
    let octant = (angle / std::f32::consts::FRAC_PI_4).round() as i32;
    match octant {
        0 => "ahead",
        1 => "ahead-right",
        2 => "right",
        3 => "behind-right",
        -1 => "ahead-left",
        -2 => "left",
        -3 => "behind-left",
        _ => "behind",
    }
}

pub fn show(
    ui: &mut egui::Ui,
    pose: &Isometry,
    planet: &planet::GeneratedPlanet,
    race: &race::Race,
    query: &planet::TrackQuery,
) {
    let map = HeadingMap::new(
        pose.position,
        pose.orientation * glam::Vec3::Z,
        query.tangent,
        planet.radius,
    );
    let ribbon = egui::Color32::from_rgb(75, 215, 240);
    let gate_color = egui::Color32::from_rgb(255, 225, 80);
    let return_color = egui::Color32::from_rgb(255, 155, 85);
    ui.small("ROUTE · heading up");
    let size = egui::vec2(ui.available_width(), 142.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(10, 21, 30));
    let center = rect.center();
    let range = planet.radius * 0.6;
    let scale = rect.height() * 0.5 / range;
    let screen = |point: glam::Vec2| center + egui::vec2(point.x, point.y) * scale;
    let half_size = glam::Vec2::new(rect.width(), rect.height()) * 0.5;
    let marker = |point: glam::Vec2| border_marker(point * scale, half_size - 10.0);
    painter.line_segment(
        [rect.center_top(), rect.center_bottom()],
        (1.0, egui::Color32::from_gray(45)),
    );
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        (1.0, egui::Color32::from_gray(45)),
    );

    // Only draw the local hemisphere: the back of the planet must not fold
    // through the map or create a line across its antipodal discontinuity.
    let local_point = |sample: &planet::TrackSample| {
        (sample.normal.dot(map.up) > 0.0)
            .then(|| map.project(sample.position))
            .flatten()
    };
    let mut segments = Vec::new();
    for index in 0..planet.track.len() {
        let current = &planet.track[index];
        let next = &planet.track[(index + 1) % planet.track.len()];
        if let (Some(a), Some(b)) = (local_point(current), local_point(next)) {
            let segment = [screen(a), screen(b)];
            painter.line_segment(
                segment,
                (
                    planet.track_width * scale,
                    egui::Color32::from_rgb(25, 84, 102),
                ),
            );
            segments.push((index, segment));
        }
    }
    // Draw the centerline/chevrons after all road fill so later neighboring
    // segments cannot cover the direction cues.
    for (index, segment) in segments {
        painter.line_segment(segment, (2.0, ribbon));
        if index % 12 == 0 {
            let direction = (segment[1] - segment[0]).normalized();
            painter.arrow(
                segment[0] - direction * 5.0,
                direction * 10.0,
                (1.5, ribbon),
            );
        }
    }

    let off_course = planet::off_track_distance(query, planet.track_width) > 0.0;
    let return_point = off_course
        .then(|| map.project(query.sample.position))
        .flatten();
    if let Some(point) = return_point {
        let (offset, _) = marker(point);
        let target = center + egui::vec2(offset.x, offset.y);
        painter.line_segment([center, target], (1.0, return_color));
        painter.circle_stroke(target, 5.0, (2.0, return_color));
    }

    let gate = (!race.finished)
        .then(|| race.checkpoints.get(race.next_checkpoint))
        .flatten()
        .and_then(|&position| map.project(position));
    if let Some(point) = gate {
        let (offset, off_map) = marker(point);
        let target = center + egui::vec2(offset.x, offset.y);
        if off_map {
            let direction = egui::vec2(offset.x, offset.y).normalized();
            painter.arrow(
                target - direction * 15.0,
                direction * 15.0,
                (2.5, gate_color),
            );
        } else {
            painter.circle(target, 5.0, gate_color, (1.5, egui::Color32::BLACK));
        }
    }
    // Always points up: the map rotates with the craft's surface heading.
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(0.0, -8.0),
            center + egui::vec2(5.0, 6.0),
            center + egui::vec2(-5.0, 6.0),
        ],
        egui::Color32::WHITE,
        egui::Stroke::new(1.5, egui::Color32::BLACK),
    ));
    painter.text(
        rect.left_bottom() + egui::vec2(6.0, -4.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{range:.0} m to top"),
        egui::FontId::proportional(10.0),
        egui::Color32::LIGHT_GRAY,
    );
    if let Some(point) = gate {
        ui.colored_label(
            gate_color,
            format!(
                "Gate {}: {:.0} m · {}",
                race.sector(),
                point.length(),
                bearing_label(point)
            ),
        );
    } else if !race.finished {
        ui.colored_label(gate_color, format!("Gate {}: opposite side", race.sector()));
    }
    if let Some(point) = return_point {
        ui.colored_label(return_color, format!("Ribbon: {}", bearing_label(point)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};

    fn map(forward: Vec3) -> HeadingMap {
        HeadingMap::new(Vec3::Y * 80.0, forward, Vec3::Z, 80.0)
    }

    fn close(actual: Vec2, expected: Vec2) {
        assert!(
            (actual - expected).length() < 1e-3,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn ahead_and_right_match_screen_axes_and_surface_distance() {
        let map = map(Vec3::Z);
        let angle = 0.4_f32;
        close(
            map.project(Vec3::new(0.0, angle.cos(), angle.sin()))
                .unwrap(),
            Vec2::new(0.0, -32.0),
        );
        close(
            map.project(Vec3::new(-angle.sin(), angle.cos(), 0.0))
                .unwrap(),
            Vec2::new(32.0, 0.0),
        );
    }

    #[test]
    fn turning_rotates_the_route_and_missed_gate_stays_behind() {
        let gate = Vec3::new(0.0, 1.0, 0.4).normalize();
        assert_eq!(bearing_label(map(Vec3::Z).project(gate).unwrap()), "ahead");
        assert_eq!(bearing_label(map(Vec3::X).project(gate).unwrap()), "right");
        assert_eq!(
            bearing_label(map(-Vec3::Z).project(gate).unwrap()),
            "behind"
        );
    }

    #[test]
    fn horizontal_bearings_match_the_chase_camera_view_transform() {
        for rotation in [glam::Quat::IDENTITY, glam::Quat::from_rotation_z(1.2)] {
            let up = rotation * Vec3::Y;
            let forward = rotation * Vec3::Z;
            let position = up * 80.0;
            let map = HeadingMap::new(position, forward, forward, 80.0);
            let view = glam::Mat4::look_at_rh(
                position - forward * 10.0 + up * 4.0,
                position + up * 0.6,
                up,
            );
            for side in [Vec3::X, -Vec3::X] {
                let target = (up + rotation * side * 0.2 + forward * 0.2).normalize() * 80.0;
                assert_eq!(
                    map.project(target).unwrap().x.signum(),
                    view.transform_point3(target).x.signum(),
                    "HUD left/right must agree with the chase view"
                );
            }
        }
    }

    #[test]
    fn projection_ignores_vehicle_and_target_altitude() {
        let target = Vec3::new(0.2, 1.0, 0.3).normalize();
        let airborne = HeadingMap::new(Vec3::Y * 110.0, Vec3::Z, Vec3::Z, 80.0);
        close(
            map(Vec3::Z).project(target * 80.0).unwrap(),
            airborne.project(target * 100.0).unwrap(),
        );
    }

    #[test]
    fn degenerate_heading_uses_track_tangent() {
        close(
            map(Vec3::Y).project(Vec3::Z).unwrap(),
            map(Vec3::Z).project(Vec3::Z).unwrap(),
        );
    }

    #[test]
    fn opposite_side_does_not_collapse_onto_player() {
        let map = map(Vec3::Z);
        assert_eq!(map.project(-Vec3::Y), None);
        assert_eq!(map.project(Vec3::ZERO), None);
        assert_eq!(map.project(Vec3::Y), Some(Vec2::ZERO));
        assert!(map.project(Vec3::new(0.0, -1.0, 0.01)).unwrap().length() > 240.0);
    }

    #[test]
    fn border_clamp_preserves_bearing_and_in_bounds_markers() {
        let half_size = Vec2::new(100.0, 60.0);
        assert_eq!(
            border_marker(Vec2::new(20.0, -40.0), half_size),
            (Vec2::new(20.0, -40.0), false)
        );
        assert_eq!(
            border_marker(Vec2::new(40.0, 240.0), half_size),
            (Vec2::new(10.0, 60.0), true)
        );
        assert_eq!(
            border_marker(Vec2::new(-400.0, -80.0), half_size),
            (Vec2::new(-100.0, -20.0), true)
        );
        assert_eq!(border_marker(Vec2::ZERO, half_size), (Vec2::ZERO, false));
    }
}
