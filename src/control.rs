#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub throttle_forward: bool,
    pub throttle_reverse: bool,
    pub steer_left: bool,
    pub steer_right: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Command {
    pub target_speed: f32,
    pub steering_angle: f32,
}

/// Renderer-independent player input state.
///
/// Keeping response filtering here lets controller behavior run in ordinary unit
/// tests. The Blade adapter only has to apply the resulting command to physics.
#[derive(Debug, Default)]
pub struct PlayerController {
    throttle: f32,
    steering: f32,
}

impl PlayerController {
    pub fn update(&mut self, input: Input, speed: f32, dt: f32) -> Command {
        let throttle_target = match (input.throttle_forward, input.throttle_reverse) {
            (true, false) => 1.0,
            (false, true) => -0.35,
            _ => 0.0,
        };
        let steering_target = match (input.steer_left, input.steer_right) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0,
        };

        let dt = dt.clamp(0.0, 0.1);
        let throttle_response = 1.0 - (-dt * 10.0).exp();
        let steering_speed = if steering_target == 0.0 { 18.0 } else { 14.0 };
        let steering_response = 1.0 - (-dt * steering_speed).exp();
        self.throttle += (throttle_target - self.throttle) * throttle_response;
        self.steering += (steering_target - self.steering) * steering_response;
        self.command(speed)
    }

    pub fn analog_command(&mut self, throttle: f32, steering: f32, speed: f32, dt: f32) -> Command {
        let dt = dt.clamp(0.0, 0.1);
        let throttle_response = 1.0 - (-dt * 8.0).exp();
        let steering_response = 1.0 - (-dt * 12.0).exp();
        self.throttle += (throttle.clamp(-1.0, 1.0) - self.throttle) * throttle_response;
        self.steering += (steering.clamp(-1.0, 1.0) - self.steering) * steering_response;
        self.command(speed)
    }

    fn command(&self, speed: f32) -> Command {
        let steering_limit = 0.46 * (1.0 / (1.0 + speed / 36.0)).clamp(0.38, 1.0);
        Command {
            target_speed: self.throttle * 26.0,
            steering_angle: self.steering * steering_limit,
        }
    }

    pub fn steering(&self) -> f32 {
        self.steering
    }

    pub fn throttle(&self) -> f32 {
        self.throttle
    }
}

pub fn signed_heading_error(forward: glam::Vec3, desired: glam::Vec3, up: glam::Vec3) -> f32 {
    forward
        .cross(desired)
        .dot(up)
        .atan2(desired.dot(forward).clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_self_centers_without_a_renderer() {
        let mut controller = PlayerController::default();
        let left = Input {
            steer_left: true,
            ..Input::default()
        };
        for _ in 0..20 {
            controller.update(left, 12.0, 1.0 / 60.0);
        }
        let turned = controller.steering();
        assert!(turned > 0.9);

        for _ in 0..20 {
            controller.update(Input::default(), 12.0, 1.0 / 60.0);
        }
        assert!(controller.steering().abs() < turned.abs() * 0.01);
    }

    #[test]
    fn steering_authority_reduces_at_speed() {
        let input = Input {
            steer_left: true,
            ..Input::default()
        };
        let mut slow = PlayerController::default();
        let mut fast = PlayerController::default();
        let slow_command = slow.update(input, 0.0, 0.1);
        let fast_command = fast.update(input, 40.0, 0.1);
        assert!(slow_command.steering_angle > fast_command.steering_angle);
    }

    #[test]
    fn heading_error_turns_toward_the_target() {
        let up = glam::Vec3::Y;
        let forward = glam::Vec3::Z;
        assert!(signed_heading_error(forward, glam::Vec3::X, up) > 0.0);
        assert!(signed_heading_error(forward, -glam::Vec3::X, up) < 0.0);
        assert_eq!(signed_heading_error(forward, forward, up), 0.0);
    }
}
