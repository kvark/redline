impl Game {
    pub fn on_event(
        &mut self,
        event: &winit::event::WindowEvent,
    ) -> Result<winit::event_loop::ControlFlow, QuitEvent> {
        let response = self.egui_state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
        if response.consumed {
            return Ok(winit::event_loop::ControlFlow::Poll);
        }

        match *event {
            winit::event::WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state,
                        ..
                    },
                ..
            } => {
                let pressed = state == winit::event::ElementState::Pressed;
                match key_code {
                    #[cfg(not(target_arch = "wasm32"))]
                    winit::keyboard::KeyCode::Escape => return Err(QuitEvent),
                    winit::keyboard::KeyCode::ArrowUp | winit::keyboard::KeyCode::KeyW => {
                        self.throttle_forward = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowDown | winit::keyboard::KeyCode::KeyS => {
                        self.throttle_reverse = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowLeft | winit::keyboard::KeyCode::KeyA => {
                        self.steer_left = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowRight | winit::keyboard::KeyCode::KeyD => {
                        self.steer_right = pressed;
                    }
                    winit::keyboard::KeyCode::KeyR if pressed => {
                        self.respawn();
                    }
                    winit::keyboard::KeyCode::Space if pressed => {
                        let pose = self.vehicle.pose(&self.engine);
                        let up = pose.position.normalize_or_zero();
                        self.engine.apply_linear_impulse(
                            self.vehicle.body_handle,
                            (self.vehicle.jump_impulse * up).into(),
                        );
                    }
                    winit::keyboard::KeyCode::Comma if pressed => {
                        let pose = self.vehicle.pose(&self.engine);
                        let forward = pose.orientation * glam::Vec3::Z;
                        self.engine.apply_angular_impulse(
                            self.vehicle.body_handle,
                            (self.vehicle.roll_impulse * forward).into(),
                        );
                    }
                    winit::keyboard::KeyCode::Period if pressed => {
                        let pose = self.vehicle.pose(&self.engine);
                        let forward = pose.orientation * glam::Vec3::Z;
                        self.engine.apply_angular_impulse(
                            self.vehicle.body_handle,
                            (-self.vehicle.roll_impulse * forward).into(),
                        );
                    }
                    _ => {}
                }
            }
            winit::event::WindowEvent::Focused(false) => {
                // Key-up events are not guaranteed when the player tabs away.
                self.throttle_forward = false;
                self.throttle_reverse = false;
                self.steer_left = false;
                self.steer_right = false;
            }
            winit::event::WindowEvent::CloseRequested => return Err(QuitEvent),
            winit::event::WindowEvent::Resized(_)
            | winit::event::WindowEvent::ScaleFactorChanged { .. } => {
                #[cfg(target_arch = "wasm32")]
                sync_web_canvas(&self.window);
            }
            winit::event::WindowEvent::RedrawRequested => {
                let wait = self.on_draw();
                if let Some(ref mut left) = self.smoke_frames_left {
                    *left = left.saturating_sub(1);
                    if *left == 0 {
                        log::info!("Smoke test finished");
                        return Err(QuitEvent);
                    }
                }
                if self.script_finished() {
                    log::info!("Drive script finished");
                    return Err(QuitEvent);
                }
                return Ok(if let Some(when) = time::Instant::now().checked_add(wait) {
                    winit::event_loop::ControlFlow::WaitUntil(when)
                } else {
                    winit::event_loop::ControlFlow::Wait
                });
            }
            _ => {}
        }

        Ok(winit::event_loop::ControlFlow::Poll)
    }

    fn respawn(&mut self) {
        self.vehicle.teleport(&mut self.engine, &self.spawn);
        self.race.reset();
    }

    fn recover(&mut self) {
        let pose = self.vehicle.pose(&self.engine);
        let up = pose.position.normalize_or_zero();
        let fwd = project_tangent(pose.orientation * glam::Vec3::Z, up);
        let next = Isometry {
            position: pose.position + up * 2.0,
            orientation: planet::surface_quat(up, fwd),
        };
        self.vehicle.teleport(&mut self.engine, &next);
    }

    fn on_draw(&mut self) -> time::Duration {
        #[cfg(target_arch = "wasm32")]
        sync_web_canvas(&self.window);
        self.update_time();

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let egui_context = self.egui_state.egui_ctx().clone();
        let egui_output = egui_context.run_ui(raw_input, |egui_ctx| {
            let mut frame = egui::Frame::side_top_panel(&egui_ctx.global_style());
            let mut fill = frame.fill.to_array();
            for channel in fill.iter_mut() {
                *channel = (*channel as u32 * 7 / 8) as u8;
            }
            frame.fill = egui::Color32::from_rgba_premultiplied(fill[0], fill[1], fill[2], fill[3]);
            egui::Panel::right("hud")
                .frame(frame)
                .show_inside(egui_ctx, |ui| self.populate_hud(ui));
        });

        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);

        let camera = self.follow_camera();
        self.update_local_lights(glam::Vec3::from(camera.transform.position));
        let primitives = self
            .egui_state
            .egui_ctx()
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        self.engine.render(
            &camera,
            &primitives,
            &egui_output.textures_delta,
            self.window.inner_size(),
            self.window.scale_factor() as f32,
        );
        egui_output.viewport_output[&self.egui_viewport_id].repaint_delay
    }

    fn populate_hud(&mut self, ui: &mut egui::Ui) {
        ui.heading("Redline");
        ui.label("A lap around Mars. Keep the rusty side down.");
        ui.separator();
        let pose = self.vehicle.pose(&self.engine);
        let (_cp, progress) = planet::track_progress(pose.position, &self.planet.track);
        ui.label(format!("Lap {} / {}", self.race.lap, self.race.laps_to_win));
        ui.label(format!(
            "Sector {:.0}%   r={:.0}m",
            progress * 100.0,
            self.planet.radius
        ));
        ui.label(format!("Time  {}", format_time(self.race.time)));
        ui.label(format!("Opponents  {}", self.ai_drivers.len()));
        if let Some(best) = self.race.best_lap {
            ui.label(format!("Best  {}", format_time(best)));
        }
        if self.race.finished {
            ui.colored_label(egui::Color32::LIGHT_GREEN, "Circuit complete");
        }
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        ui.label(format!("Speed {:.0} m/s", glam::Vec3::from(lin).length()));
        let query = planet::query_track(pose.position, &self.planet.track);
        let off = planet::off_track_distance(&query, self.planet.track_width);
        if off > 0.0 {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 90),
                format!("Off course  {off:.1}m"),
            );
        }
        ui.separator();
        ui.label("W/↑ throttle   S/↓ brake   A/D steer");
        ui.label("R respawn   Space jump   ,/. roll");

        egui::CollapsingHeader::new("Camera")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Slider::new(&mut self.cam_config.distance, 3.0..=40.0).text("Distance"),
                );
                ui.add(egui::Slider::new(&mut self.cam_config.height, 0.4..=8.0).text("Height"));
                ui.add(
                    egui::Slider::new(&mut self.cam_config.azimuth, -consts::PI..=consts::PI)
                        .text("Azimuth"),
                );
                ui.add(
                    egui::Slider::new(&mut self.cam_config.altitude, -0.15..=1.1).text("Altitude"),
                );
                ui.add(egui::Slider::new(&mut self.cam_config.fov, 0.6..=1.6).text("FOV"));
                ui.toggle_value(&mut self.is_paused, "Pause");
            });

        ui.horizontal(|ui| {
            if ui.button("Recover").clicked() {
                self.recover();
            }
            if ui.button("Respawn").clicked() {
                self.respawn();
            }
        });
    }

    fn follow_camera(&mut self) -> blade_engine::FrameCamera {
        let pose = self.vehicle.pose(&self.engine);
        // Radial outward from planet center — the only reliable "up" on a sphere.
        let up = pose.position.normalize_or_zero();
        let fwd = project_tangent(pose.orientation * glam::Vec3::Z, up);
        let desired = planet::surface_quat(up, fwd);

        let dt = self.last_camera_update.elapsed().as_secs_f32();
        self.last_camera_update = time::Instant::now();
        let t = 1.0 - (-dt * self.cam_config.speed).exp();
        let smooth = self.last_camera_orient.slerp(desired, t.clamp(0.0, 1.0));
        self.last_camera_orient = smooth;

        let cc = &self.cam_config;
        let back = smooth * -glam::Vec3::Z;
        let cam_up = smooth * glam::Vec3::Y;
        let cam_right = smooth * glam::Vec3::X;
        let yaw = glam::Quat::from_axis_angle(cam_up, cc.azimuth);
        // Positive altitude raises the camera (we look slightly down at the car).
        // The previous sign was inverted, so the default altitude=0.35 placed the
        // eye *under* the vehicle — that is what produced the "from the bottom"
        // view. Radial clamps below still guard against extreme negative values
        // and against the camera falling underground.
        let pitch = glam::Quat::from_axis_angle(cam_right, cc.altitude);
        let orbit = yaw * pitch;
        let mut offset = orbit * (back * cc.distance + cam_up * cc.height);

        // Keep the eye strictly above the car (positive radial component).
        let min_above = (cc.height * 0.55).max(1.0);
        let radial_before = offset.dot(up);
        if radial_before < min_above {
            offset += up * (min_above - radial_before);
        }

        let mut eye = pose.position + offset;

        // Never let the camera fall underground relative to the vehicle radius.
        let vehicle_r = pose.position.length();
        let eye_r = eye.length();
        let min_r = vehicle_r + 0.6;
        if eye_r < min_r {
            eye = eye.normalize_or_zero() * min_r;
        }

        let target = pose.position + cam_up * 0.6;
        let view = glam::Mat4::look_at_rh(eye, target, cam_up);
        let world = view.inverse();
        let (_, rot, trans) = world.to_scale_rotation_translation();

        // Lightweight state-trace journal (RUST_LOG=debug).
        if self.frame_index.is_multiple_of(8) {
            let rel_h = (eye - pose.position).dot(up);
            let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
            let speed = glam::Vec3::from(lin).length();
            log::debug!(
                "cam_trace frame={} veh_r={:.2} eye_r={:.2} rel_h={:.2} radial_before={:.2} speed={:.1} cam_up·up={:.3} alt={:.2}",
                self.frame_index,
                vehicle_r,
                eye.length(),
                rel_h,
                radial_before,
                speed,
                cam_up.dot(up),
                cc.altitude,
            );
        }
        self.frame_index = self.frame_index.wrapping_add(1);

        blade_engine::FrameCamera {
            transform: blade_engine::Transform {
                position: trans.into(),
                orientation: rot.into(),
            },
            fov_y: cc.fov,
        }
    }
}
