use avian3d::prelude::*;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use crate::assets::BlobAssets;
use crate::constants::{DEFAULT_RENDER_LAYER, PLAYER_HITBOX_LAYER, WORLD_BOUNDARY_COLLISION_LAYER};

const SKYBOX_STRIP_WIDTH: f32 = 12_288.0;
const SKYBOX_FACE_SIZE: f32 = SKYBOX_STRIP_WIDTH / 6.0;
const SKYBOX_SUN_PIXEL: Vec2 = Vec2::new(4_912.0, 1_691.0);
const SUN_LIGHT_DISTANCE: f32 = 24.0;

pub fn setup_world(mut commands: Commands, assets: Res<BlobAssets>) {
    let light_position = sun_direction();

    commands.spawn((
        DirectionalLight { shadows_enabled: true, illuminance: 14_000.0, ..default() },
        Transform::from_translation(light_position * SUN_LIGHT_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
        RenderLayers::layer(DEFAULT_RENDER_LAYER),
    ));

    commands.spawn((
        SceneRoot(assets.world_scene.clone()),
        Transform::default(),
        ColliderConstructorHierarchy::new(ColliderConstructor::TrimeshFromMesh),
        RigidBody::Static,
    ));

    let half_x = 8.5 / 2.0;
    let half_z = 16.4 / 2.0;
    let boundary_collision_layers = CollisionLayers::from_bits(WORLD_BOUNDARY_COLLISION_LAYER, PLAYER_HITBOX_LAYER);

    commands.spawn((
        Collider::cuboid(1.0, 100.0, 18.0),
        RigidBody::Static,
        boundary_collision_layers,
        Transform::from_xyz(half_x + 0.5, 50.0, 0.0),
    ));
    commands.spawn((
        Collider::cuboid(1.0, 100.0, 18.0),
        RigidBody::Static,
        boundary_collision_layers,
        Transform::from_xyz(-half_x - 0.5, 50.0, 0.0),
    ));
    commands.spawn((
        Collider::cuboid(10.0, 100.0, 1.0),
        RigidBody::Static,
        boundary_collision_layers,
        Transform::from_xyz(0.0, 50.0, half_z + 0.5),
    ));
    commands.spawn((
        Collider::cuboid(10.0, 100.0, 1.0),
        RigidBody::Static,
        boundary_collision_layers,
        Transform::from_xyz(0.0, 50.0, -half_z - 0.5),
    ));
    commands.spawn((
        Collider::cuboid(10.0, 1.0, 18.0),
        RigidBody::Static,
        boundary_collision_layers,
        Transform::from_xyz(0.0, 20.0, 0.0),
    ));
}

pub fn sun_direction() -> Vec3 {
    let sun_direction = skybox_pixel_to_direction(SKYBOX_SUN_PIXEL);
    Vec3::new(sun_direction.x, sun_direction.y, -sun_direction.z)
}

fn skybox_pixel_to_direction(pixel: Vec2) -> Vec3 {
    let face = (pixel.x / SKYBOX_FACE_SIZE).floor().clamp(0.0, 5.0);
    let face_x = (-SKYBOX_FACE_SIZE).mul_add(face, pixel.x);

    let inv_face_size = 2.0 / SKYBOX_FACE_SIZE;
    let u = (face_x + 0.5).mul_add(inv_face_size, -1.0);
    let v = (pixel.y + 0.5).mul_add(inv_face_size, -1.0);

    let direction = if face < 1.0 {
        Vec3::new(1.0, -v, -u)
    } else if face < 2.0 {
        Vec3::new(-1.0, -v, u)
    } else if face < 3.0 {
        Vec3::new(u, 1.0, v)
    } else if face < 4.0 {
        Vec3::new(u, -1.0, -v)
    } else if face < 5.0 {
        Vec3::new(u, -v, 1.0)
    } else {
        Vec3::new(-u, -v, -1.0)
    };

    direction.normalize()
}
