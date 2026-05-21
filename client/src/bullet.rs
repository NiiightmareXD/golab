use avian3d::math::{AdjustPrecision, Dir};
use avian3d::prelude::*;
use bevy::audio::Volume;
use bevy::prelude::*;
use golab_shared::{BULLET_AIM_DISTANCE, GUN_FIRE_RATE_PER_SECOND};

use crate::assets::BlobAssets;
use crate::constants::{
    BULLET_DROP_GRAVITY, BULLET_LIFETIME, BULLET_QUERY_LAYERS, BULLET_RADIUS, BULLET_SPEED, TERRAIN_HEIGHT, WORLD_HALF,
};
use crate::network::NetworkClientState;
use crate::player::{Player, PlayerHealth};
use crate::settings::AudioSettings;

const MUZZLE_FORWARD_DISTANCE: f32 = 0.5;
const MUZZLE_RIGHT_OFFSET: f32 = 0.2;
const MUZZLE_DOWN_OFFSET: f32 = 0.15;

#[derive(Message)]
pub struct LocalShotFired {
    pub target: Vec3,
}

#[derive(Resource)]
pub struct GunFireLimiter {
    last_fire_at: f64,
}

impl Default for GunFireLimiter {
    fn default() -> Self {
        Self { last_fire_at: f64::NEG_INFINITY }
    }
}

impl GunFireLimiter {
    fn try_fire(&mut self, now: f64) -> bool {
        let seconds_per_shot = 1.0 / GUN_FIRE_RATE_PER_SECOND;
        if now - self.last_fire_at < seconds_per_shot {
            return false;
        }

        self.last_fire_at = now;
        true
    }
}

#[derive(Component)]
pub struct Bullet {
    velocity: Vec3,
    lifetime: Timer,
    owner: Option<u64>,
}

#[derive(Component)]
pub struct BulletHitbox {
    pub owner: u64,
}

impl BulletHitbox {
    pub const fn player(owner: u64) -> Self {
        Self { owner }
    }
}

pub fn shoot(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut fire_limiter: ResMut<GunFireLimiter>,
    mut shot_events: MessageWriter<LocalShotFired>,
    players: Query<(Entity, &Transform, &PlayerHealth), With<Player>>,
    camera_query: Query<&GlobalTransform, With<crate::player::MainPlayerCamera>>,
    spatial_query: SpatialQuery,
    assets: Res<BlobAssets>,
    audio_settings: Res<AudioSettings>,
    network_state: Res<NetworkClientState>,
) {
    if !mouse.pressed(MouseButton::Left) {
        return;
    }

    let Some((player_entity, player, health)) = players.iter().next() else {
        return;
    };
    if !health.is_alive() {
        return;
    }
    if !fire_limiter.try_fire(time.elapsed_secs_f64()) {
        return;
    }

    let Some(camera_transform) = camera_query.iter().next() else { return };

    let cam_pos = camera_transform.translation();
    let cam_forward = camera_transform.forward();
    let cam_forward_vec = *cam_forward;

    let target =
        Dir::new(cam_forward_vec.adjust_precision()).map_or(cam_pos + cam_forward_vec * BULLET_AIM_DISTANCE, |dir| {
            spatial_query
                .cast_ray(
                    cam_pos.adjust_precision(),
                    dir,
                    BULLET_AIM_DISTANCE.adjust_precision(),
                    true,
                    &SpatialQueryFilter::from_mask(BULLET_QUERY_LAYERS).with_excluded_entities([player_entity]),
                )
                .map_or(cam_pos + cam_forward_vec * BULLET_AIM_DISTANCE, |hit| cam_pos + cam_forward_vec * hit.distance)
        });

    spawn_bullet_from_transform(&mut commands, &assets, &audio_settings, player, target, network_state.client_id());
    shot_events.write(LocalShotFired { target });
}

pub fn spawn_bullet_from_transform(
    commands: &mut Commands,
    assets: &BlobAssets,
    audio_settings: &AudioSettings,
    shooter: &Transform,
    target: Vec3,
    owner: Option<u64>,
) {
    let forward = shooter.rotation * Vec3::NEG_Z;
    let right = shooter.rotation * Vec3::X;
    let up = shooter.rotation * Vec3::Y;

    let muzzle =
        shooter.translation + forward * MUZZLE_FORWARD_DISTANCE + right * MUZZLE_RIGHT_OFFSET - up * MUZZLE_DOWN_OFFSET;

    let direction = (target - muzzle).try_normalize().unwrap_or(forward);
    let velocity = direction * BULLET_SPEED;

    commands.spawn((
        Mesh3d(assets.bullet_mesh.clone()),
        MeshMaterial3d(assets.bullet_material.clone()),
        Transform::from_translation(muzzle).looking_at(muzzle + direction, Vec3::Y).with_scale(Vec3::new(
            BULLET_RADIUS,
            BULLET_RADIUS,
            BULLET_RADIUS * 12.0,
        )),
        Bullet { velocity, lifetime: Timer::from_seconds(BULLET_LIFETIME, TimerMode::Once), owner },
        PointLight {
            color: Color::srgb(1.0, 0.0, 0.0),
            intensity: 80.0,
            range: 4.0,
            shadows_enabled: false,
            ..default()
        },
    ));

    commands.spawn((
        AudioPlayer::new(assets.shot_sound.clone()),
        PlaybackSettings { volume: Volume::Linear(audio_settings.volume), ..PlaybackSettings::DESPAWN },
    ));
}

#[derive(Component)]
pub struct HitEffect {
    lifetime: Timer,
}

pub fn update_hit_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut effects: Query<(Entity, &mut HitEffect, &mut Transform, &mut PointLight)>,
) {
    for (entity, mut effect, mut transform, mut light) in &mut effects {
        effect.lifetime.tick(time.delta());
        if effect.lifetime.is_finished() {
            commands.entity(entity).despawn();
        } else {
            let percent = effect.lifetime.fraction_remaining();
            transform.scale = Vec3::splat(BULLET_RADIUS * 6.0 * percent);
            light.intensity = 15000.0 * percent;
        }
    }
}

pub fn move_bullets(
    mut commands: Commands,
    time: Res<Time>,
    spatial_query: SpatialQuery,
    assets: Res<BlobAssets>,
    mut bullets: Query<(Entity, &mut Transform, &mut Bullet)>,
    hitboxes: Query<&BulletHitbox>,
    local_players: Query<&PlayerHealth, With<Player>>,
) {
    let dt = time.delta_secs();

    for (entity, mut transform, mut bullet) in &mut bullets {
        let previous_position = transform.translation;
        bullet.velocity.y = (-BULLET_DROP_GRAVITY).mul_add(dt, bullet.velocity.y);
        transform.translation += bullet.velocity * dt;
        bullet.lifetime.tick(time.delta());
        let hit = segment_hits_physics(
            &spatial_query,
            previous_position,
            transform.translation,
            bullet.owner,
            &hitboxes,
            &local_players,
        );

        if let Some(hit) = hit.as_ref() {
            commands.spawn((
                Mesh3d(assets.bullet_mesh.clone()),
                MeshMaterial3d(assets.bullet_material.clone()),
                Transform::from_translation(hit.point1).with_scale(Vec3::splat(BULLET_RADIUS * 6.0)),
                HitEffect { lifetime: Timer::from_seconds(0.15, TimerMode::Once) },
                PointLight {
                    color: Color::srgb(1.0, 0.2, 0.0),
                    intensity: 15000.0,
                    range: 3.0,
                    shadows_enabled: false,
                    ..default()
                },
            ));
        }

        if hit.is_some()
            || bullet.lifetime.is_finished()
            || transform.translation.x.abs() > WORLD_HALF
            || transform.translation.z.abs() > WORLD_HALF
            || transform.translation.y <= TERRAIN_HEIGHT + BULLET_RADIUS
        {
            commands.entity(entity).despawn();
        }
    }
}

fn segment_hits_physics(
    spatial_query: &SpatialQuery,
    start: Vec3,
    end: Vec3,
    owner: Option<u64>,
    hitboxes: &Query<&BulletHitbox>,
    local_players: &Query<&PlayerHealth, With<Player>>,
) -> Option<ShapeHitData> {
    let segment = end - start;
    let max_distance = segment.length();
    if max_distance <= f32::EPSILON {
        return None;
    }

    let Ok(direction) = Dir::new(segment) else {
        return None;
    };

    spatial_query.cast_shape_predicate(
        &Collider::sphere(BULLET_RADIUS),
        start.adjust_precision(),
        Quat::IDENTITY.adjust_precision(),
        direction,
        &ShapeCastConfig {
            ignore_origin_penetration: true,
            ..ShapeCastConfig::from_max_distance(max_distance.adjust_precision())
        },
        &SpatialQueryFilter::from_mask(BULLET_QUERY_LAYERS),
        &|entity| {
            if let Ok(health) = local_players.get(entity)
                && (owner.is_none() || !health.is_alive())
            {
                return false;
            }

            !hitboxes.get(entity).is_ok_and(|hitbox| owner == Some(hitbox.owner))
        },
    )
}
