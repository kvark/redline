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

        let steering_limit = 0.52 * (1.0 / (1.0 + speed / 48.0)).clamp(0.42, 1.0);
        Command {
            target_speed: self.throttle * 30.0,
            steering_angle: self.steering * steering_limit,
        }
    }

    pub fn steering(&self) -> f32 {
        self.steering
    }
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
}
