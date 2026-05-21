use std::f32::consts::FRAC_PI_2;

use avian3d::math::{AdjustPrecision, AsF32, Dir, Scalar};
use avian3d::prelude::*;
use bevy::audio::Volume;
use bevy::camera::visibility::RenderLayers;
use bevy::color::Alpha;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::alpha::AlphaMode;
use bevy::window::{CursorGrabMode, CursorOptions};
use golab_shared::{
    PLAYER_HITBOX_BOTTOM_ENDPOINT_Y, PLAYER_HITBOX_RADIUS, PLAYER_HITBOX_TOP_ENDPOINT_Y, PLAYER_MAX_HEALTH,
    RESPAWN_SECONDS, SPAWN_MAX_X, SPAWN_MAX_Z, SPAWN_MIN_X, SPAWN_MIN_Z,
};
use rand::RngExt;

use crate::assets::BlobAssets;
use crate::constants::{
    CROUCH_EYE_HEIGHT, CROUCH_SPEED_MULTIPLIER, CROUCH_TRANSITION_SPEED, DASH_AMOUNT, DASH_RECHARGE_SECONDS,
    DASH_STOP_SPEED, DASH_VELOCITY_DAMPING, DEFAULT_RENDER_LAYER, GHOST_MODEL_CENTER_OFFSET_Z,
    GHOST_MODEL_EYE_OFFSET_Y, GHOST_MODEL_SCALE, JUMP_SPEED, MAX_DASH_CHARGES, MOVE_SPEED, PLAYER_GRAVITY,
    PLAYER_HITBOX_LAYER, PLAYER_MOVEMENT_COLLISION_LAYERS, SOLID_WORLD_COLLISION_LAYERS, STAND_EYE_HEIGHT,
    TERRAIN_HEIGHT, VIEW_MODEL_RENDER_LAYER, WORLD_COLLISION_LAYER, WORLD_HALF,
};
use crate::environment::EnvironmentCamera;
use crate::network::{NetworkClientState, RespawnPlayer};
use crate::settings::{AudioSettings, MouseSettings};
use crate::world::sun_direction;

const MIN_CAPSULE_LENGTH: f32 = 0.05;
const PLAYER_HEAD_CLEARANCE: f32 = 0.21;
const GROUND_NORMAL_DOT: Scalar = 0.65;
const GROUNDING_VELOCITY: f32 = -0.5;
const GROUND_PROBE_DISTANCE: Scalar = 0.06;
const STAIR_JUMP_PROBE_DISTANCE: Scalar = 0.28;
const THIRD_PERSON_CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 0.35, 2.6);
const THIRD_PERSON_CAMERA_TARGET: Vec3 = Vec3::new(0.0, 0.05, 0.0);
const THIRD_PERSON_CAMERA_COLLISION_RADIUS: f32 = 0.18;
const THIRD_PERSON_CAMERA_WALL_MARGIN: f32 = 0.03;
const THIRD_PERSON_CAMERA_MIN_DISTANCE: f32 = 0.22;
const THIRD_PERSON_MODEL_FADE_START_DISTANCE: f32 = 0.85;
const THIRD_PERSON_MODEL_FADE_END_DISTANCE: f32 = 0.35;
const THIRD_PERSON_MODEL_FADE_SPEED: f32 = 18.0;
const FIRST_PERSON_GUN_OFFSET: Vec3 = Vec3::new(0.25, -0.50, -0.25);
const FIRST_PERSON_GUN_SCALE: f32 = 0.75;
const THIRD_PERSON_GUN_OFFSET: Vec3 = Vec3::new(0.12, -0.24, -0.12);
const THIRD_PERSON_GUN_SCALE: f32 = 0.45;
const VIEW_MODEL_LIGHT_DIRECTION: Vec3 = Vec3::new(-0.25, -0.35, -1.0);
const VIEW_MODEL_LIGHT_LIT_ILLUMINANCE: f32 = 9_000.0;
const VIEW_MODEL_LIGHT_SHADOWED_ILLUMINANCE: f32 = 2_000.0;
const VIEW_MODEL_LIGHT_BLEND_SPEED: f32 = 10.0;
const VIEW_MODEL_SUN_OCCLUSION_DISTANCE: f32 = 80.0;
const VIEW_MODEL_SUN_PROBE_OFFSET: Vec3 = Vec3::new(0.0, 0.05, 0.0);

#[derive(Resource, Default)]
pub struct CameraMode {
    third_person: bool,
}

#[derive(Resource)]
pub struct ThirdPersonCameraState {
    model_alpha: f32,
    target_model_alpha: f32,
}

impl Default for ThirdPersonCameraState {
    fn default() -> Self {
        Self { model_alpha: 1.0, target_model_alpha: 1.0 }
    }
}

#[derive(Resource)]
pub struct RespawnState {
    timer: Option<Timer>,
    was_alive: bool,
}

impl Default for RespawnState {
    fn default() -> Self {
        Self { timer: None, was_alive: true }
    }
}

impl RespawnState {
    pub fn remaining_secs(&self) -> Option<f32> {
        self.timer.as_ref().map(|timer| (RESPAWN_SECONDS - timer.elapsed_secs()).max(0.0))
    }
}

#[derive(Component)]
pub struct Player;

#[derive(Component)]
pub struct PlayerHealth {
    health: i32,
    alive: bool,
}

impl Default for PlayerHealth {
    fn default() -> Self {
        Self { health: PLAYER_MAX_HEALTH, alive: true }
    }
}

impl PlayerHealth {
    pub const fn health(&self) -> i32 {
        self.health
    }

    pub const fn is_alive(&self) -> bool {
        self.alive
    }

    pub fn set_from_server(&mut self, health: i32, alive: bool) {
        self.health = health.clamp(0, PLAYER_MAX_HEALTH);
        self.alive = alive;
    }

    pub const fn reset(&mut self) {
        self.health = PLAYER_MAX_HEALTH;
        self.alive = true;
    }
}

#[derive(Component)]
pub struct MainPlayerCamera;

#[derive(Component)]
pub struct ViewModelCamera;

#[derive(Component)]
pub struct ViewModel;

#[derive(Component)]
pub struct ViewModelLight;

#[derive(Component)]
pub struct ThirdPersonModel;

#[derive(Component, Clone, Copy)]
pub struct ThirdPersonFadeMaterial {
    original_alpha: f32,
    original_alpha_mode: AlphaMode,
}

#[derive(Component)]
pub struct PlayerBody {
    vertical_velocity: f32,
    eye_height: f32,
    crouching: bool,
    grounded: bool,
    dash_charges: u8,
    dash_recharge: Timer,
    dash_velocity: Vec3,
}

impl PlayerBody {
    fn reset(&mut self) {
        self.vertical_velocity = 0.0;
        self.eye_height = STAND_EYE_HEIGHT;
        self.crouching = false;
        self.grounded = true;
        self.dash_charges = MAX_DASH_CHARGES;
        self.dash_recharge = Timer::from_seconds(DASH_RECHARGE_SECONDS, TimerMode::Once);
        self.dash_velocity = Vec3::ZERO;
    }

    pub const fn dash_charges(&self) -> u8 {
        self.dash_charges
    }

    pub const fn is_crouching(&self) -> bool {
        self.crouching
    }

    pub fn dash_recharge_remaining_secs(&self) -> f32 {
        if self.dash_charges >= MAX_DASH_CHARGES {
            0.0
        } else {
            (DASH_RECHARGE_SECONDS - self.dash_recharge.elapsed_secs()).max(0.0)
        }
    }
}

pub fn setup_player(mut commands: Commands, assets: Res<BlobAssets>) {
    let start = player_spawn_translation();

    commands.spawn((
        Player,
        PlayerHealth::default(),
        RigidBody::Kinematic,
        build_player_collider(STAND_EYE_HEIGHT),
        CollisionLayers::from_bits(PLAYER_HITBOX_LAYER, PLAYER_MOVEMENT_COLLISION_LAYERS),
        CustomPositionIntegration,
        SpeculativeMargin(0.0),
        PlayerBody {
            vertical_velocity: 0.0,
            eye_height: STAND_EYE_HEIGHT,
            crouching: false,
            grounded: true,
            dash_charges: MAX_DASH_CHARGES,
            dash_recharge: Timer::from_seconds(DASH_RECHARGE_SECONDS, TimerMode::Once),
            dash_velocity: Vec3::ZERO,
        },
        Transform::from_translation(start).looking_at(Vec3::new(0.0, STAND_EYE_HEIGHT, -1.0), Vec3::Y),
        Visibility::default(),
        children![
            (
                Camera3d::default(),
                bevy::render::view::Hdr,
                Camera::default(),
                Tonemapping::None,
                Projection::from(PerspectiveProjection { near: 0.01, fov: 90.0_f32.to_radians(), ..default() }),
                Transform::default(),
                RenderLayers::layer(DEFAULT_RENDER_LAYER),
                EnvironmentCamera,
                MainPlayerCamera,
            ),
            (
                Camera3d::default(),
                bevy::render::view::Hdr,
                Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
                Tonemapping::TonyMcMapface,
                Bloom::default(),
                Projection::from(PerspectiveProjection { near: 0.01, fov: 90.0_f32.to_radians(), ..default() }),
                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                ViewModelCamera,
                Transform::default(),
            ),
            (
                DirectionalLight { shadows_enabled: false, illuminance: VIEW_MODEL_LIGHT_LIT_ILLUMINANCE, ..default() },
                Transform::from_translation(Vec3::ZERO).looking_at(VIEW_MODEL_LIGHT_DIRECTION, Vec3::Y),
                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                ViewModelLight,
            ),
            (
                SceneRoot(assets.gun_scene.clone()),
                Transform::from_translation(FIRST_PERSON_GUN_OFFSET)
                    .with_scale(Vec3::splat(FIRST_PERSON_GUN_SCALE))
                    .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
                RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
                ViewModel,
                Visibility::Visible,
                NotShadowCaster,
                NotShadowReceiver,
            ),
            (
                ThirdPersonModel,
                SceneRoot(assets.player_scene.clone()),
                player_model_transform(),
                RenderLayers::layer(DEFAULT_RENDER_LAYER),
                Visibility::Hidden,
            ),
            (
                ThirdPersonModel,
                SceneRoot(assets.gun_scene.clone()),
                Transform::from_translation(THIRD_PERSON_GUN_OFFSET)
                    .with_scale(Vec3::splat(THIRD_PERSON_GUN_SCALE))
                    .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
                RenderLayers::layer(DEFAULT_RENDER_LAYER),
                Visibility::Hidden,
            )
        ],
    ));
}

pub fn player_spawn_translation() -> Vec3 {
    Vec3::new(0.0, TERRAIN_HEIGHT + STAND_EYE_HEIGHT, 4.5)
}

pub fn random_player_spawn_translation() -> Vec3 {
    let mut rng = rand::rng();
    Vec3::new(
        rng.random_range(SPAWN_MIN_X..=SPAWN_MAX_X),
        TERRAIN_HEIGHT + STAND_EYE_HEIGHT,
        rng.random_range(SPAWN_MIN_Z..=SPAWN_MAX_Z),
    )
}

pub fn player_model_transform() -> Transform {
    Transform::from_translation(Vec3::new(0.0, GHOST_MODEL_EYE_OFFSET_Y, GHOST_MODEL_CENTER_OFFSET_Z))
        .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
        .with_scale(Vec3::splat(GHOST_MODEL_SCALE))
}

pub fn respawn_player_at(
    transform: &mut Transform,
    body: &mut PlayerBody,
    health: &mut PlayerHealth,
    translation: Vec3,
) {
    transform.translation = translation;
    transform.rotation =
        Transform::from_translation(translation).looking_at(Vec3::new(0.0, STAND_EYE_HEIGHT, -1.0), Vec3::Y).rotation;
    body.reset();
    health.reset();
}

pub fn tick_respawn_state(
    time: Res<Time>,
    network_state: Res<NetworkClientState>,
    mut respawn_state: ResMut<RespawnState>,
    mut respawn_writer: MessageWriter<RespawnPlayer>,
    mut players: Query<(&mut Transform, &mut PlayerBody, &mut PlayerHealth), With<Player>>,
) {
    let Some((mut transform, mut body, mut health)) = players.iter_mut().next() else {
        respawn_state.timer = None;
        respawn_state.was_alive = true;
        return;
    };

    if health.is_alive() {
        respawn_state.timer = None;
        respawn_state.was_alive = true;
        return;
    }

    if respawn_state.was_alive || respawn_state.timer.is_none() {
        respawn_state.timer = Some(Timer::from_seconds(RESPAWN_SECONDS, TimerMode::Once));
    }
    respawn_state.was_alive = false;

    let Some(timer) = respawn_state.timer.as_mut() else {
        return;
    };

    timer.tick(time.delta());
    if timer.just_finished() {
        if network_state.is_connected() {
            respawn_writer.write(RespawnPlayer);
        } else {
            respawn_player_at(&mut transform, &mut body, &mut health, random_player_spawn_translation());
            respawn_state.timer = None;
            respawn_state.was_alive = true;
        }
    }
}

pub fn build_player_hitbox_collider() -> Collider {
    Collider::capsule_endpoints(
        PLAYER_HITBOX_RADIUS,
        Vec3::Y * PLAYER_HITBOX_BOTTOM_ENDPOINT_Y,
        Vec3::Y * PLAYER_HITBOX_TOP_ENDPOINT_Y,
    )
}

fn build_player_collider(eye_height: f32) -> Collider {
    let bottom = -eye_height + PLAYER_HITBOX_RADIUS;
    let top = (PLAYER_HEAD_CLEARANCE - PLAYER_HITBOX_RADIUS).max(bottom + MIN_CAPSULE_LENGTH);
    Collider::capsule_endpoints(PLAYER_HITBOX_RADIUS, Vec3::Y * bottom, Vec3::Y * top)
}

pub fn cursor_grab(mut cursor_options: Query<&mut CursorOptions>, mouse: Res<ButtonInput<MouseButton>>) {
    if mouse.just_pressed(MouseButton::Left) {
        lock_cursor(&mut cursor_options);
    }
}

fn lock_cursor(cursor_options: &mut Query<&mut CursorOptions>) {
    for mut cursor_options in cursor_options.iter_mut() {
        cursor_options.visible = false;
        cursor_options.grab_mode = CursorGrabMode::Locked;
    }
}

pub fn toggle_camera_mode(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<CameraMode>) {
    if keys.just_pressed(KeyCode::KeyR) {
        mode.third_person = !mode.third_person;
    }
}

pub fn sync_camera_mode(
    mode: Res<CameraMode>,
    mut main_cameras: Query<&mut Transform, (With<MainPlayerCamera>, Without<ViewModelCamera>)>,
    mut view_model_visibilities: Query<&mut Visibility, (With<ViewModel>, Without<ThirdPersonModel>)>,
    mut third_person_models: Query<&mut Visibility, (With<ThirdPersonModel>, Without<ViewModel>)>,
) {
    if !mode.is_changed() {
        return;
    }

    for mut transform in &mut main_cameras {
        if mode.third_person {
            *transform =
                Transform::from_translation(THIRD_PERSON_CAMERA_OFFSET).looking_at(THIRD_PERSON_CAMERA_TARGET, Vec3::Y);
        } else {
            *transform = Transform::default();
        }
    }

    for mut visibility in &mut view_model_visibilities {
        *visibility = if mode.third_person { Visibility::Hidden } else { Visibility::Visible };
    }

    for mut visibility in &mut third_person_models {
        *visibility = if mode.third_person { Visibility::Visible } else { Visibility::Hidden };
    }
}

pub fn sync_view_model_render_layers(
    mut commands: Commands,
    view_models: Query<Entity, With<ViewModel>>,
    children: Query<&Children>,
    configured: Query<(Option<&RenderLayers>, Has<NotShadowCaster>, Has<NotShadowReceiver>)>,
) {
    let render_layers = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);

    for root in &view_models {
        ensure_view_model_render_settings(&mut commands, &configured, root, &render_layers);

        for descendant in children.iter_descendants::<Children>(root) {
            ensure_view_model_render_settings(&mut commands, &configured, descendant, &render_layers);
        }
    }
}

pub fn update_view_model_lighting(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    players: Query<(Entity, &Transform), With<Player>>,
    mut view_model_lights: Query<&mut DirectionalLight, With<ViewModelLight>>,
) {
    let Some((player_entity, player_transform)) = players.iter().next() else {
        return;
    };

    let sun_direction = sun_direction();
    let Ok(sun_direction) = Dir::new(sun_direction.adjust_precision()) else {
        return;
    };
    let probe_origin = player_transform.translation + VIEW_MODEL_SUN_PROBE_OFFSET;
    let sun_is_occluded = spatial_query
        .cast_ray(
            probe_origin.adjust_precision(),
            sun_direction,
            VIEW_MODEL_SUN_OCCLUSION_DISTANCE.adjust_precision(),
            true,
            &SpatialQueryFilter::from_mask(WORLD_COLLISION_LAYER).with_excluded_entities([player_entity]),
        )
        .is_some();
    let target_illuminance =
        if sun_is_occluded { VIEW_MODEL_LIGHT_SHADOWED_ILLUMINANCE } else { VIEW_MODEL_LIGHT_LIT_ILLUMINANCE };
    let blend_step = 1.0 - (-VIEW_MODEL_LIGHT_BLEND_SPEED * time.delta_secs()).exp();

    for mut light in &mut view_model_lights {
        light.illuminance = (target_illuminance - light.illuminance).mul_add(blend_step, light.illuminance);
    }
}

fn ensure_view_model_render_settings(
    commands: &mut Commands,
    configured: &Query<(Option<&RenderLayers>, Has<NotShadowCaster>, Has<NotShadowReceiver>)>,
    entity: Entity,
    render_layers: &RenderLayers,
) {
    let Ok((layers, not_shadow_caster, not_shadow_receiver)) = configured.get(entity) else {
        return;
    };

    let has_render_layers = layers.is_some_and(|layers| layers == render_layers);
    if has_render_layers && not_shadow_caster && not_shadow_receiver {
        return;
    }

    let mut entity_commands = commands.entity(entity);
    if !has_render_layers {
        entity_commands.insert(render_layers.clone());
    }
    if !not_shadow_caster {
        entity_commands.insert(NotShadowCaster);
    }
    if !not_shadow_receiver {
        entity_commands.insert(NotShadowReceiver);
    }
}

pub fn look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mouse_settings: Res<MouseSettings>,
    mut players: Query<&mut Transform, With<Player>>,
) {
    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    let Some(mut player) = players.iter_mut().next() else {
        return;
    };

    let (yaw, pitch, roll) = player.rotation.to_euler(EulerRot::YXZ);
    let yaw = (-delta.x).mul_add(mouse_settings.sensitivity, yaw);
    let pitch = (-delta.y).mul_add(mouse_settings.sensitivity, pitch).clamp(-FRAC_PI_2 + 0.03, FRAC_PI_2 - 0.03);

    player.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll);
}

fn has_ground_below(
    spatial_query: &SpatialQuery,
    entity: Entity,
    collider: &Collider,
    position: Vec3,
    rotation: Quat,
    max_distance: Scalar,
) -> bool {
    spatial_query
        .cast_shape(
            collider,
            position.adjust_precision(),
            rotation.adjust_precision(),
            Dir::NEG_Y,
            &ShapeCastConfig { ignore_origin_penetration: true, ..ShapeCastConfig::from_max_distance(max_distance) },
            &SpatialQueryFilter::from_mask(SOLID_WORLD_COLLISION_LAYERS).with_excluded_entities([entity]),
        )
        .is_some_and(|hit| hit.normal1.adjust_precision().dot(Vec3::Y.adjust_precision()) >= GROUND_NORMAL_DOT)
}

#[allow(clippy::type_complexity)]
pub fn update_camera(
    mode: Res<CameraMode>,
    mut third_person_state: ResMut<ThirdPersonCameraState>,
    spatial_query: SpatialQuery,
    player_query: Query<(Entity, &Transform), With<Player>>,
    mut camera_query: Query<&mut Transform, (With<MainPlayerCamera>, Without<Player>)>,
) {
    if !mode.third_person {
        third_person_state.target_model_alpha = 1.0;
        return;
    }

    let Some((player_entity, player_transform)) = player_query.iter().next() else { return };
    let Some(mut camera_transform) = camera_query.iter_mut().next() else { return };

    let camera_target = player_transform.translation + player_transform.rotation * THIRD_PERSON_CAMERA_TARGET;
    let camera_offset_world = player_transform.rotation * THIRD_PERSON_CAMERA_OFFSET;
    let desired_camera_pos = player_transform.translation + camera_offset_world;

    let ray_dir = desired_camera_pos - camera_target;
    let max_distance = ray_dir.length();

    if max_distance > 0.0
        && let Ok(dir) = Dir::new(ray_dir)
    {
        let hit = spatial_query.cast_shape(
            &Collider::sphere(THIRD_PERSON_CAMERA_COLLISION_RADIUS),
            camera_target.adjust_precision(),
            Quat::IDENTITY.adjust_precision(),
            dir,
            &ShapeCastConfig {
                ignore_origin_penetration: true,
                ..ShapeCastConfig::from_max_distance(max_distance.adjust_precision())
            },
            &SpatialQueryFilter::from_mask(SOLID_WORLD_COLLISION_LAYERS).with_excluded_entities([player_entity]),
        );

        let collision_distance = hit.map_or(max_distance, |hit_data| {
            (hit_data.distance - THIRD_PERSON_CAMERA_WALL_MARGIN.adjust_precision()).max(0.0)
        });
        let final_distance = collision_distance.clamp(THIRD_PERSON_CAMERA_MIN_DISTANCE.min(max_distance), max_distance);
        let final_pos = camera_target + ray_dir.normalize() * final_distance;

        third_person_state.target_model_alpha = third_person_model_alpha(final_distance);
        *camera_transform = Transform::from_translation(
            player_transform.rotation.inverse() * (final_pos - player_transform.translation),
        )
        .looking_at(THIRD_PERSON_CAMERA_TARGET, Vec3::Y);
    } else {
        third_person_state.target_model_alpha = 1.0;
        *camera_transform =
            Transform::from_translation(THIRD_PERSON_CAMERA_OFFSET).looking_at(THIRD_PERSON_CAMERA_TARGET, Vec3::Y);
    }
}

fn third_person_model_alpha(camera_distance: f32) -> f32 {
    ((camera_distance - THIRD_PERSON_MODEL_FADE_END_DISTANCE)
        / (THIRD_PERSON_MODEL_FADE_START_DISTANCE - THIRD_PERSON_MODEL_FADE_END_DISTANCE))
        .clamp(0.0, 1.0)
}

#[allow(clippy::type_complexity)]
pub fn update_third_person_model_opacity(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<CameraMode>,
    mut state: ResMut<ThirdPersonCameraState>,
    roots: Query<(Entity, &mut Visibility), With<ThirdPersonModel>>,
    children: Query<&Children>,
    mut mesh_materials: Query<(Entity, &mut MeshMaterial3d<StandardMaterial>, Option<&ThirdPersonFadeMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !mode.third_person {
        state.model_alpha = 1.0;
        state.target_model_alpha = 1.0;
        return;
    }

    let fade_step = 1.0 - (-THIRD_PERSON_MODEL_FADE_SPEED * time.delta_secs()).exp();
    state.model_alpha = (state.target_model_alpha - state.model_alpha).mul_add(fade_step, state.model_alpha);
    if (state.target_model_alpha - state.model_alpha).abs() <= 0.01 {
        state.model_alpha = state.target_model_alpha;
    }

    for (root, mut visibility) in roots {
        *visibility = Visibility::Visible;

        for descendant in children.iter_descendants::<Children>(root) {
            let Ok((mesh_entity, mut mesh_material, fade_material)) = mesh_materials.get_mut(descendant) else {
                continue;
            };

            let fade_material = if let Some(fade_material) = fade_material {
                *fade_material
            } else {
                let Some(original_material) = materials.get(&mesh_material.0).cloned() else {
                    continue;
                };
                let fade_material = ThirdPersonFadeMaterial {
                    original_alpha: original_material.base_color.alpha(),
                    original_alpha_mode: original_material.alpha_mode,
                };
                mesh_material.0 = materials.add(original_material);
                commands.entity(mesh_entity).insert(fade_material);
                fade_material
            };

            let Some(material) = materials.get_mut(&mesh_material.0) else {
                continue;
            };

            material.base_color.set_alpha(fade_material.original_alpha * state.model_alpha);
            material.alpha_mode =
                if state.model_alpha >= 0.99 { fade_material.original_alpha_mode } else { AlphaMode::Blend };
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn move_player(
    mut commands: Commands,
    assets: Res<BlobAssets>,
    audio_settings: Res<AudioSettings>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut params: ParamSet<(
        Query<(Entity, &mut Transform, &mut PlayerBody, &mut PlayerHealth, &mut Collider), With<Player>>,
        MoveAndSlide,
        SpatialQuery,
    )>,
) {
    let dt = time.delta_secs();

    let Some((
        entity,
        start_position,
        yaw_rotation,
        mut velocity,
        dash_velocity,
        collider,
        requested_jump,
        was_grounded,
        previous_vertical_velocity,
    )) = (|| {
        let mut players = params.p0();
        let (entity, mut player, mut body, health, mut collider) = players.iter_mut().next()?;
        if !health.is_alive() {
            return None;
        }

        if body.dash_charges < MAX_DASH_CHARGES {
            body.dash_recharge.tick(time.delta());
            if body.dash_recharge.just_finished() {
                body.dash_charges += 1;
                body.dash_recharge.reset();
            }
        } else {
            body.dash_recharge.reset();
        }

        let dash_damping = (-DASH_VELOCITY_DAMPING * dt).exp();
        body.dash_velocity *= dash_damping;
        if body.dash_velocity.length_squared() <= DASH_STOP_SPEED * DASH_STOP_SPEED {
            body.dash_velocity = Vec3::ZERO;
        }

        let crouching = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        body.crouching = crouching;
        let target_eye_height = if crouching { CROUCH_EYE_HEIGHT } else { STAND_EYE_HEIGHT };
        let crouch_step = (CROUCH_TRANSITION_SPEED * dt).min(1.0);
        let previous_eye_height = body.eye_height;
        body.eye_height = (target_eye_height - body.eye_height).mul_add(crouch_step, body.eye_height);
        player.translation.y += body.eye_height - previous_eye_height;
        let updated_collider = build_player_collider(body.eye_height);
        *collider = updated_collider.clone();

        let mut input = Vec3::ZERO;
        if keys.pressed(KeyCode::KeyW) {
            input.z -= 1.0;
        }
        if keys.pressed(KeyCode::KeyS) {
            input.z += 1.0;
        }
        if keys.pressed(KeyCode::KeyA) {
            input.x -= 1.0;
        }
        if keys.pressed(KeyCode::KeyD) {
            input.x += 1.0;
        }

        let yaw = player.rotation.to_euler(EulerRot::YXZ).0;
        let yaw_rotation = Quat::from_rotation_y(yaw);

        if mouse.just_pressed(MouseButton::Right) && body.dash_charges > 0 {
            let dash_was_full = body.dash_charges == MAX_DASH_CHARGES;

            body.dash_charges -= 1;
            body.dash_velocity = dash_velocity_from_look(player.rotation);
            body.grounded = false;
            if dash_was_full {
                body.dash_recharge.reset();
            }

            commands.spawn((
                AudioPlayer::new(assets.dash_sound.clone()),
                PlaybackSettings { volume: Volume::Linear(audio_settings.volume), ..PlaybackSettings::DESPAWN },
            ));
        }

        let movement = if input == Vec3::ZERO { Vec3::ZERO } else { yaw_rotation * input.normalize() };

        let mut velocity = Vec3::ZERO;
        if movement != Vec3::ZERO {
            let crouch_multiplier = if crouching { CROUCH_SPEED_MULTIPLIER } else { 1.0 };
            velocity += movement.normalize() * MOVE_SPEED * crouch_multiplier;
        }

        Some((
            entity,
            player.translation,
            yaw_rotation,
            velocity,
            body.dash_velocity,
            updated_collider,
            keys.just_pressed(KeyCode::Space),
            body.grounded,
            body.vertical_velocity,
        ))
    })()
    else {
        return;
    };

    let (grounded_for_movement, can_jump) = {
        let spatial_query = params.p2();
        let short_ground_probe = previous_vertical_velocity <= 0.0
            && has_ground_below(&spatial_query, entity, &collider, start_position, yaw_rotation, GROUND_PROBE_DISTANCE);
        let grounded_for_movement = was_grounded || short_ground_probe;
        let can_jump = grounded_for_movement
            || (requested_jump
                && previous_vertical_velocity <= 0.0
                && has_ground_below(
                    &spatial_query,
                    entity,
                    &collider,
                    start_position,
                    yaw_rotation,
                    STAIR_JUMP_PROBE_DISTANCE,
                ));

        (grounded_for_movement, can_jump)
    };

    let horizontal_motion = Vec2::new(velocity.x + dash_velocity.x, velocity.z + dash_velocity.z);
    let vertical_velocity = {
        let mut players = params.p0();
        let Ok((_entity, _player, mut body, _health, _collider)) = players.get_mut(entity) else {
            return;
        };

        if can_jump && requested_jump {
            body.vertical_velocity = JUMP_SPEED;
            body.grounded = false;
        } else if grounded_for_movement {
            body.vertical_velocity =
                if horizontal_motion.length_squared() <= f32::EPSILON { 0.0 } else { GROUNDING_VELOCITY };
            body.grounded = true;
        } else {
            body.vertical_velocity = (-PLAYER_GRAVITY).mul_add(dt, body.vertical_velocity);
            body.grounded = false;
        }

        body.vertical_velocity
    };

    velocity.y = vertical_velocity;
    let base_vertical_velocity = vertical_velocity;
    velocity += dash_velocity;

    let mut hit_ground = false;
    let mut hit_ceiling = false;
    let output = params.p1().move_and_slide(
        &collider,
        start_position.adjust_precision(),
        yaw_rotation.adjust_precision(),
        velocity.adjust_precision(),
        time.delta(),
        &MoveAndSlideConfig::default(),
        &SpatialQueryFilter::from_mask(PLAYER_MOVEMENT_COLLISION_LAYERS).with_excluded_entities([entity]),
        |hit| {
            let normal_dot_up = hit.normal.adjust_precision().dot(Vec3::Y.adjust_precision());
            if normal_dot_up >= GROUND_NORMAL_DOT {
                hit_ground = true;
            } else if normal_dot_up <= -GROUND_NORMAL_DOT {
                hit_ceiling = true;
            }
            MoveAndSlideHitResponse::Accept
        },
    );

    let mut players = params.p0();
    let Ok((_entity, mut player, mut body, _health, _collider)) = players.get_mut(entity) else {
        return;
    };

    player.translation = output.position.f32();
    player.translation.x = player.translation.x.clamp(-WORLD_HALF + 2.0, WORLD_HALF - 2.0);
    player.translation.z = player.translation.z.clamp(-WORLD_HALF + 2.0, WORLD_HALF - 2.0);

    if hit_ground && velocity.y <= 0.0 {
        body.vertical_velocity = 0.0;
        if body.dash_velocity.y < 0.0 {
            body.dash_velocity.y = 0.0;
        }
        body.grounded = true;
    } else {
        body.vertical_velocity = base_vertical_velocity;
        body.grounded = false;
        if hit_ceiling {
            if body.vertical_velocity > 0.0 {
                body.vertical_velocity = 0.0;
            }
            if body.dash_velocity.y > 0.0 {
                body.dash_velocity.y = 0.0;
            }
        }
    }
}

fn dash_velocity_from_look(rotation: Quat) -> Vec3 {
    let look = (rotation * Vec3::NEG_Z).try_normalize().unwrap_or(Vec3::NEG_Z);
    let vertical_share = look.y.clamp(-1.0, 1.0);
    let horizontal_share = (1.0 - vertical_share * vertical_share).sqrt();
    let horizontal_direction = Vec3::new(look.x, 0.0, look.z).try_normalize().unwrap_or(Vec3::ZERO);

    horizontal_direction * (DASH_AMOUNT * horizontal_share) + Vec3::Y * (DASH_AMOUNT * vertical_share)
}
