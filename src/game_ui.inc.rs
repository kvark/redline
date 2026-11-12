const GHOST_CYAN: egui::Color32 = egui::Color32::from_rgb(110, 220, 255);
const CHAMP_GOLD: egui::Color32 = egui::Color32::from_rgb(255, 205, 90);

impl Game {
    pub fn on_event(
        &mut self,
        event: &winit::event::WindowEvent,
    ) -> Result<winit::event_loop::ControlFlow, QuitEvent> {
        // Escape must win over egui focus (selectables mark the event consumed).
        if let winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    physical_key: winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape),
                    state: winit::event::ElementState::Pressed,
                    ..
                },
            ..
        } = *event
        {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if self.in_menu {
                    return Err(QuitEvent);
                }
                self.open_menu();
                self.window.request_redraw();
                return Ok(winit::event_loop::ControlFlow::Poll);
            }
        }

        // Pause is global during a race, including when an egui widget has focus.
        if let winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    physical_key: winit::keyboard::PhysicalKey::Code(key),
                    state,
                    repeat,
                    ..
                },
            ..
        } = *event
            && !self.in_menu
            && control::pause_key_intent(key, state, repeat)
        {
            self.toggle_pause();
            self.window.request_redraw();
            return Ok(winit::event_loop::ControlFlow::Poll);
        }

        // Enter rematch from the results board (keep craft + circuit).
        if let winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    physical_key: winit::keyboard::PhysicalKey::Code(key),
                    state: winit::event::ElementState::Pressed,
                    ..
                },
            ..
        } = *event
            && matches!(
                key,
                winit::keyboard::KeyCode::Enter | winit::keyboard::KeyCode::NumpadEnter
            )
            && self.race_results.is_some()
        {
            self.rematch();
            self.window.request_redraw();
            return Ok(winit::event_loop::ControlFlow::Poll);
        }

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
                // Throttle/steer may be held through the lights; impulses wait for GO.
                let on_grid = !self.in_menu;
                let drive_ok = on_grid
                    && !self.countdown_locks_drive()
                    && self.race_results.is_none();
                match key_code {
                    winit::keyboard::KeyCode::ArrowUp | winit::keyboard::KeyCode::KeyW
                        if on_grid =>
                    {
                        self.throttle_forward = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowDown | winit::keyboard::KeyCode::KeyS
                        if on_grid =>
                    {
                        self.throttle_reverse = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowLeft | winit::keyboard::KeyCode::KeyA
                        if on_grid =>
                    {
                        self.steer_left = pressed;
                    }
                    winit::keyboard::KeyCode::ArrowRight | winit::keyboard::KeyCode::KeyD
                        if on_grid =>
                    {
                        self.steer_right = pressed;
                    }
                    winit::keyboard::KeyCode::KeyR if pressed && drive_ok => {
                        self.respawn();
                    }
                    winit::keyboard::KeyCode::Space if pressed && drive_ok => {
                        let pose = self.vehicle.pose(&self.engine);
                        let up = pose.position.normalize_or_zero();
                        self.engine.apply_linear_impulse(
                            self.vehicle.body_handle,
                            (self.vehicle.jump_impulse * up).into(),
                        );
                    }
                    winit::keyboard::KeyCode::Comma if pressed && drive_ok => {
                        let pose = self.vehicle.pose(&self.engine);
                        let forward = pose.orientation * glam::Vec3::Z;
                        self.engine.apply_angular_impulse(
                            self.vehicle.body_handle,
                            (self.vehicle.roll_impulse * forward).into(),
                        );
                    }
                    winit::keyboard::KeyCode::Period if pressed && drive_ok => {
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
                    log::info!(
                        "Drive script finished sim={:.1}s lap={} cp={} race_time={:.1} last_lap={:?} best_lap={:?} pad_hits={}/{} ai_pad_hits={:?}",
                        self.sim_time,
                        self.race.lap,
                        self.race.next_checkpoint,
                        self.race.time,
                        self.race.last_lap_time,
                        self.race.best_lap,
                        self.vehicle.pad_hits(),
                        self.boost_pads.len(),
                        self.ai_drivers
                            .iter()
                            .map(|driver| driver.vehicle.pad_hits())
                            .collect::<Vec<_>>()
                    );
                    log::info!("{}", self.race_stats_summary());
                    log::info!("{}", self.launch_summary());
                    log::info!("{}", self.frame_summary());
                    log::info!("{}", self.dust_ledger.summary());
                    log::info!("{}", self.audio_cues.summary());
                    if let Some(session) = self.ghost.as_ref() {
                        log::info!("{}", session.summary());
                    }
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

    fn toggle_pause(&mut self) {
        self.is_paused = !self.is_paused;
        // Never include paused wall time in the next fixed-step update.
        self.last_physics_update = time::Instant::now();
    }

    fn respawn(&mut self) {
        self.vehicle.teleport(&mut self.engine, &self.spawn);
        self.vehicle.reset_boost_stats();
        self.race.reset();
        if let Some(session) = self.ghost.as_mut() {
            session.lapper.restart();
        }
        self.lap_callout = None;
        self.race_results = None;
        for driver in self.ai_drivers.iter_mut() {
            driver.race.reset();
        }
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
        let frame_started = time::Instant::now();
        self.update_time();
        self.update_engine_audio();
        let sim_ms = frame_started.elapsed().as_secs_f32() * 1e3;

        let ui_started = time::Instant::now();
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let egui_context = self.egui_state.egui_ctx().clone();
        let egui_output = egui_context.run_ui(raw_input, |egui_ctx| {
            if self.in_menu {
                self.populate_menu(egui_ctx);
            } else {
                let mut frame = egui::Frame::side_top_panel(&egui_ctx.global_style());
                let mut fill = frame.fill.to_array();
                for channel in fill.iter_mut() {
                    *channel = (*channel as u32 * 7 / 8) as u8;
                }
                frame.fill =
                    egui::Color32::from_rgba_premultiplied(fill[0], fill[1], fill[2], fill[3]);
                egui::Panel::right("hud")
                    .frame(frame)
                    .show_inside(egui_ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.populate_hud(ui));
                    });
                self.populate_countdown(egui_ctx);
                self.populate_launch_banner(egui_ctx);
                self.populate_lap_callout(egui_ctx);
                self.populate_results(egui_ctx);
                // Explicit layers: lines on Background, BOOST on Foreground.
                self.populate_speed_lines(egui_ctx);
                self.populate_boost_indicator(egui_ctx);
            }
        });

        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);

        self.place_ghost();
        let camera = self.follow_camera();
        self.update_local_lights(glam::Vec3::from(camera.transform.position));
        let primitives = self
            .egui_state
            .egui_ctx()
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let ui_ms = ui_started.elapsed().as_secs_f32() * 1e3;
        let render_started = time::Instant::now();
        self.engine.render(
            &camera,
            &primitives,
            &egui_output.textures_delta,
            self.window.inner_size(),
            self.window.scale_factor() as f32,
        );
        if self.script.is_some() {
            self.frame_ms.push((
                frame_started.elapsed().as_secs_f32() * 1e3,
                render_started.elapsed().as_secs_f32() * 1e3,
            ));
        }
        if cfg!(debug_assertions) && self.frame_index.is_multiple_of(20) {
            log::debug!(
                "cpu frame sim={:.1}ms ui={:.1}ms render={:.1}ms total={:.1}ms",
                sim_ms,
                ui_ms,
                render_started.elapsed().as_secs_f32() * 1e3,
                frame_started.elapsed().as_secs_f32() * 1e3
            );
        }
        egui_output.viewport_output[&self.egui_viewport_id].repaint_delay
    }


    fn populate_speed_lines(&self, ui: &egui::Ui) {
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let speed = glam::Vec3::from(lin).length();
        let boost = self.vehicle.boost_intensity().max(self.launch_thrust());
        let intensity = speed_fx::line_intensity(speed, boost);
        let streaks = speed_fx::streaks(intensity, self.sim_time);
        if streaks.is_empty() {
            return;
        }
        let ctx = ui.ctx();
        // Behind every HUD area; egui still composites over the 3D frame.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("speed_lines"),
        ));
        let rect = ctx.content_rect();
        // Focus a little above center: the chase camera looks over the craft.
        let focus = egui::pos2(rect.center().x, rect.top() + rect.height() * 0.46);
        let reach = (rect.width().powi(2) + rect.height().powi(2)).sqrt() * 0.5;
        for streak in streaks {
            let dir = egui::vec2(streak.angle.cos(), streak.angle.sin());
            let a = focus + dir * (streak.inner * reach);
            let b = focus + dir * (streak.outer * reach);
            let alpha = (streak.alpha * 255.0) as u8;
            painter.line_segment(
                [a, b],
                egui::Stroke::new(
                    1.0 + 1.5 * intensity,
                    egui::Color32::from_rgba_unmultiplied(190, 240, 255, alpha),
                ),
            );
        }
    }

    fn populate_boost_indicator(&self, ui: &egui::Ui) {
        let pad = self.vehicle.pad_boost_intensity().max(self.launch_thrust());
        let rejoin = self.vehicle.ribbon_boost_intensity();
        let level = pad.max(rejoin);
        if level <= 0.01 {
            return;
        }
        let alpha = (170.0 + 85.0 * level) as u8;
        let color = if pad >= rejoin {
            egui::Color32::from_rgba_unmultiplied(90, 230, 255, alpha)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 170, 70, alpha)
        };
        egui::Area::new(egui::Id::new("boost_indicator"))
            // Upper third: over the sky, clear of the dust and lap banners.
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("» BOOST «")
                            .size(30.0 + 10.0 * level)
                            .strong()
                            .color(color),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
    }

    fn countdown_locks_drive(&self) -> bool {
        self.start_countdown
            .as_ref()
            .is_some_and(|c| c.controls_locked())
    }

    fn populate_countdown(&self, ui: &egui::Ui) {
        let Some(label) = self
            .start_countdown
            .as_ref()
            .and_then(|c| c.label())
        else {
            return;
        };
        let go = label == "GO";
        let color = if go {
            egui::Color32::from_rgb(120, 255, 140)
        } else {
            egui::Color32::from_rgb(255, 220, 90)
        };
        egui::Area::new(egui::Id::new("start_countdown"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 140.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.vertical_centered(|ui| {
                    // Hint sits above the numeral, over the sky, clear of the craft.
                    let hint = if go { " " } else { "Throttle on GO to launch · too early bogs" };
                    ui.label(
                        egui::RichText::new(hint)
                            .size(20.0)
                            .strong()
                            .color(egui::Color32::from_rgba_unmultiplied(240, 240, 240, 220)),
                    );
                    ui.label(
                        egui::RichText::new(label)
                            .size(128.0)
                            .strong()
                            .color(color),
                    );
                });
            });
    }

    fn populate_launch_banner(&self, ui: &egui::Ui) {
        let Some(rating) = self.launch.as_ref().and_then(|s| s.banner()) else {
            return;
        };
        let color = match rating {
            launch::Rating::Perfect => egui::Color32::from_rgb(90, 230, 255),
            launch::Rating::Good => egui::Color32::from_rgb(120, 255, 140),
            launch::Rating::Jump => egui::Color32::from_rgb(255, 90, 90),
            launch::Rating::Late => egui::Color32::from_rgb(220, 200, 160),
        };
        egui::Area::new(egui::Id::new("launch_banner"))
            // Under the BOOST tag, clear of the craft and the GO numeral.
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 112.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                ui.label(
                    egui::RichText::new(rating.label())
                        .size(36.0)
                        .strong()
                        .color(color),
                );
            });
    }

    fn populate_lap_callout(&self, ui: &egui::Ui) {
        let Some(banner) = self.lap_callout.as_ref() else {
            return;
        };
        // Don't fight the start lights for screen center.
        if self.start_countdown.is_some() {
            return;
        }
        let (title, subtitle, color, title_size) = match banner.kind() {
            lap_callout::CalloutKind::LapComplete { lap_time } => (
                "LAP",
                Some(format_time(lap_time)),
                egui::Color32::from_rgb(120, 220, 255),
                64.0,
            ),
            lap_callout::CalloutKind::FinalLap { previous_lap } => (
                "FINAL LAP",
                Some(format_time(previous_lap)),
                egui::Color32::from_rgb(255, 90, 90),
                72.0,
            ),
            lap_callout::CalloutKind::Finish => (
                "FINISH",
                self.race.last_lap_time.map(format_time),
                egui::Color32::from_rgb(120, 255, 140),
                64.0,
            ),
            lap_callout::CalloutKind::SectorBest { sector_time } => (
                "BEST SECTOR",
                Some(format_time(sector_time)),
                egui::Color32::from_rgb(200, 120, 255),
                42.0,
            ),
        };
        egui::Area::new(egui::Id::new("lap_callout"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -72.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(title_size)
                            .strong()
                            .color(color),
                    );
                    if let Some(sub) = subtitle {
                        ui.label(
                            egui::RichText::new(sub)
                                .size(28.0)
                                .color(egui::Color32::from_rgba_unmultiplied(240, 240, 240, 220)),
                        );
                    }
                });
            });
    }

    fn populate_results(&mut self, ui: &egui::Ui) {
        let Some(board) = self.race_results.as_ref() else {
            return;
        };
        if self.start_countdown.is_some() || self.lap_callout.is_some() {
            return;
        }
        let place = board.place;
        let field = board.field;
        let race_time = board.race_time;
        let best_lap = board.best_lap;
        let last_lap = board.last_lap;
        let launch = board.launch;
        let margin = board.margin();
        let new_ghost = self.ghost.as_ref().is_some_and(|g| g.saved);
        let rows: Vec<(String, String, bool)> = board
            .standings
            .iter()
            .enumerate()
            .map(|(i, row)| {
                (
                    format!("P{}  {}", i + 1, row.name()),
                    row.gap_text(i + 1),
                    row.racer == 0,
                )
            })
            .collect();
        let champ = self.champ_mode.then(|| {
            let table = &self.championship.table;
            let rows: Vec<(String, String, String, bool)> = table
                .table()
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let won = self.championship.last.iter().find(|l| l.0 == e.racer);
                    (
                        format!("{}  {}", i + 1, championship::name(e.racer)),
                        won.map_or(String::new(), |l| format!("+{}", l.1)),
                        format!("{} pts", e.points),
                        e.racer == 0,
                    )
                })
                .collect();
            (table.races, table.champion(), rows)
        });
        let rematch_label = match champ {
            Some((_, Some(_), _)) => "New Championship".to_string(),
            Some((races, None, _)) => format!("Next Race {}/{}", races + 1, championship::RACES),
            None => "Rematch".to_string(),
        };
        let mut rematch_clicked = false;
        let mut continue_clicked = false;
        egui::Area::new(egui::Id::new("race_results"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -24.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(&ui.ctx().global_style())
                    .inner_margin(egui::Margin::symmetric(28, 20))
                    .show(ui, |ui| {
                        ui.set_min_width(280.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("RESULTS")
                                    .size(28.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(120, 220, 255)),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!("P{place} / {field}"))
                                    .size(64.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(120, 255, 140)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("Race  {}", format_time(race_time)))
                                    .size(22.0)
                                    .color(egui::Color32::from_rgba_unmultiplied(240, 240, 240, 230)),
                            );
                            if let Some(best) = best_lap {
                                ui.label(format!("Best lap  {}", format_time(best)));
                            }
                            if let Some(last) = last_lap {
                                ui.label(format!("Last lap  {}", format_time(last)));
                            }
                            if let Some(m) = margin {
                                ui.label(format!("{} {}", m.text(), m.reference()));
                            }
                            if let Some(rating) = launch {
                                ui.label(format!("Launch  {}", rating.label()));
                            }
                            if new_ghost {
                                ui.colored_label(GHOST_CYAN, "New ghost saved: race it next");
                            }
                            if !rows.is_empty() {
                                ui.add_space(8.0);
                                egui::Grid::new("standings").num_columns(2).spacing([28.0, 2.0]).show(
                                    ui,
                                    |ui| {
                                        for row in &rows {
                                            let color = if row.2 {
                                                egui::Color32::from_rgb(120, 255, 140)
                                            } else {
                                                egui::Color32::from_rgb(220, 220, 220)
                                            };
                                            ui.colored_label(color, &row.0);
                                            ui.colored_label(color, &row.1);
                                            ui.end_row();
                                        }
                                    },
                                );
                            }
                            if let Some(&(races, champion, ref rows)) = champ.as_ref() {
                                ui.add_space(10.0);
                                let title = format!(
                                    "CHAMPIONSHIP  ·  RACE {races} OF {}",
                                    championship::RACES
                                );
                                ui.label(egui::RichText::new(title).size(18.0).strong().color(CHAMP_GOLD));
                                egui::Grid::new("championship").num_columns(3).spacing([24.0, 2.0]).show(
                                    ui,
                                    |ui| {
                                        for row in rows {
                                            let color = if row.3 { CHAMP_GOLD } else { egui::Color32::from_rgb(220, 220, 220) };
                                            ui.colored_label(color, &row.0);
                                            ui.colored_label(color, &row.1);
                                            ui.colored_label(color, &row.2);
                                            ui.end_row();
                                        }
                                    },
                                );
                                if let Some(racer) = champion {
                                    let text = format!("CHAMPION: {}", championship::name(racer));
                                    ui.label(egui::RichText::new(text).size(26.0).strong().color(CHAMP_GOLD));
                                    ui.label("Championship complete");
                                }
                            }
                            ui.add_space(14.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized([150.0, 32.0], egui::Button::new(&rematch_label))
                                    .clicked()
                                {
                                    rematch_clicked = true;
                                }
                                if ui
                                    .add_sized([150.0, 32.0], egui::Button::new("Continue"))
                                    .clicked()
                                {
                                    continue_clicked = true;
                                }
                            });
                            ui.label(
                                egui::RichText::new(format!("Enter {} · Esc menu · auto-return", rematch_label.to_lowercase()))
                                    .size(14.0)
                                    .color(egui::Color32::from_rgba_unmultiplied(200, 200, 200, 160)),
                            );
                        });
                    });
            });
        if rematch_clicked {
            // Same craft + circuit; start_race clears results and restarts lights.
            self.rematch();
        } else if continue_clicked {
            self.race_results = None;
            self.open_menu();
        }
    }

    fn populate_menu(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.heading("REDLINE");
                ui.label("Futuristic planet-circuit racing. Physics-first. Mars is home.");
                ui.add_space(16.0);

                ui.group(|ui| {
                    ui.set_min_width(360.0);
                    ui.label(egui::RichText::new("Craft").strong());
                    for id in menu::VehicleId::ALL {
                        ui.selectable_value(&mut self.menu_vehicle, id, id.label());
                    }
                    ui.add_space(4.0);
                    ui.label(self.menu_vehicle.blurb());
                });

                ui.add_space(12.0);

                ui.group(|ui| {
                    ui.set_min_width(360.0);
                    ui.label(egui::RichText::new("Circuit").strong());
                    for id in menu::MapId::ALL {
                        ui.selectable_value(&mut self.menu_map, id, id.label());
                    }
                    ui.add_space(4.0);
                    ui.label(self.menu_map.blurb());
                    let circuit = self.menu_map.key();
                    if let Some(session) = self.ghost.as_mut()
                        && let Some(lap) = session.stored(circuit)
                    {
                        ui.add_space(6.0);
                        let text = format!("Best-lap ghost  {}", format_time(lap));
                        ui.colored_label(GHOST_CYAN, text);
                        if ui.button("Clear Ghost").clicked() {
                            session.clear(circuit);
                        }
                    }
                });

                ui.add_space(12.0);
                let mut start_champ = false;
                ui.group(|ui| {
                    ui.set_min_width(360.0);
                    ui.label(egui::RichText::new("Championship").strong());
                    let champ = &self.championship;
                    ui.label(format!(
                        "{} races on {} · points 10-6-4-3",
                        championship::RACES,
                        menu::MapId::MarsClassic.label()
                    ));
                    let standings: Vec<String> = champ
                        .table
                        .table()
                        .iter()
                        .map(|e| format!("{} {}", championship::name(e.racer), e.points))
                        .collect();
                    if !standings.is_empty() {
                        ui.colored_label(CHAMP_GOLD, standings.join("  ·  "));
                    }
                    let label = if let Some(racer) = champ.table.champion() {
                        ui.label(format!("Complete: {} won", championship::name(racer)));
                        "New Championship".to_string()
                    } else if champ.in_progress() {
                        format!("Resume Championship: race {} of {}", champ.race_number(), championship::RACES)
                    } else {
                        "Start Championship".to_string()
                    };
                    let has_points = champ.table.races > 0;
                    if ui.add_sized([300.0, 32.0], egui::Button::new(label)).clicked() {
                        start_champ = true;
                    }
                    if has_points && ui.button("Reset Championship").clicked() {
                        self.championship.reset();
                    }
                });
                if start_champ {
                    self.start_championship();
                }

                ui.add_space(20.0);
                if ui
                    .add_sized([220.0, 36.0], egui::Button::new("Enter Circuit"))
                    .clicked()
                {
                    self.champ_mode = false;
                    self.start_race();
                }

                ui.add_space(24.0);
                ui.label("Esc quit   ·   Menu from race pauses without rebuilding the world");
            });
        });
    }

    fn populate_hud(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Redline");
            let label = if self.is_paused { "Resume [P]" } else { "Pause [P]" };
            if ui.button(label).clicked() {
                self.toggle_pause();
            }
        });
        ui.label(format!(
            "{} on {}",
            self.menu_vehicle.label(),
            self.active_map.label()
        ));
        if self.champ_mode {
            let race = match self.race.finished {
                true => self.championship.table.races,
                false => self.championship.race_number(),
            };
            let text = format!("Championship  race {race} / {}", championship::RACES);
            ui.colored_label(CHAMP_GOLD, text);
        }
        ui.separator();
        let pose = self.vehicle.pose(&self.engine);
        let (_cp, progress) = planet::track_progress(pose.position, &self.planet.track);
        let place = self.live_place();
        let field = self.field_size();
        ui.label(
            egui::RichText::new(format!("P{place} / {field}"))
                .size(26.0)
                .strong()
                .color(egui::Color32::from_rgb(120, 255, 140)),
        );
        if let Some(split) = self.gap_timer.split(0) {
            let color = match split {
                split::Split::Leading(_) => egui::Color32::from_rgb(120, 255, 140),
                split::Split::Behind(_) => egui::Color32::from_rgb(255, 200, 90),
            };
            ui.label(
                egui::RichText::new(format!("{}  {}", split.text(), split.reference()))
                    .size(20.0)
                    .strong()
                    .color(color),
            );
        }
        if let Some(gap) = self.ghost_split() {
            let color = if gap < 0.0 {
                egui::Color32::from_rgb(120, 255, 140)
            } else {
                egui::Color32::from_rgb(255, 200, 90)
            };
            ui.label(
                egui::RichText::new(format!("{}  vs ghost", ghost::split_text(gap)))
                    .size(20.0)
                    .strong()
                    .color(color),
            );
        }
        if let Some(lap) = self.ghost.as_ref().and_then(|g| g.playing.as_ref()) {
            ui.colored_label(GHOST_CYAN, format!("Ghost  {}", format_time(lap.lap_time)));
        }
        ui.label(
            egui::RichText::new(format!(
                "LAP {} / {}",
                self.race.lap, self.race.laps_to_win
            ))
            .size(22.0)
            .strong(),
        );
        if self.race.lap >= self.race.laps_to_win && !self.race.finished {
            ui.colored_label(egui::Color32::from_rgb(255, 90, 90), "FINAL LAP");
        }
        ui.label(format!(
            "Sector {} / {}   {:.0}%   r={:.0}m",
            self.race.sector(),
            self.race.sector_count(),
            progress * 100.0,
            self.planet.radius
        ));
        ui.label(format!(
            "Sec   {}",
            format_time(self.race.current_sector_time())
        ));
        if let Some(best_sec) = self.race.best_sector {
            ui.label(
                egui::RichText::new(format!("Best sec  {}", format_time(best_sec)))
                    .color(egui::Color32::from_rgb(200, 120, 255)),
            );
        }
        ui.label(format!("Lap   {}", format_time(self.race.current_lap_time())));
        if let Some(best) = self.race.best_lap {
            ui.label(format!("Best  {}", format_time(best)));
        }
        if let Some(last) = self.race.last_lap_time {
            ui.label(format!("Last  {}", format_time(last)));
        }
        ui.label(format!("Race  {}", format_time(self.race.time)));
        ui.label(format!("Opponents  {}", self.ai_drivers.len()));
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
        crate::route_map::show(ui, &pose, &self.planet, &self.race, &query);
        if self.countdown_locks_drive() {
            ui.colored_label(
                egui::Color32::from_rgb(255, 220, 90),
                "Start lights — drive locked",
            );
        }
        ui.separator();
        ui.label("W/↑ throttle   S/↓ brake   A/D steer");
        ui.label("R respawn   Space jump   ,/. roll");
        ui.label("P pause/resume   Esc menu");

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
            });

        ui.horizontal(|ui| {
            if ui.button("Menu").clicked() {
                self.open_menu();
            }
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
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let kick_target = if self.is_paused {
            self.fov_kick
        } else {
            speed_fx::fov_kick_target(
                glam::Vec3::from(lin).length(),
                self.vehicle.boost_intensity().max(self.launch_thrust()),
            )
        };
        self.fov_kick = speed_fx::ease_kick(self.fov_kick, kick_target, dt.min(0.1));
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
            fov_y: cc.fov + self.fov_kick,
        }
    }
}
