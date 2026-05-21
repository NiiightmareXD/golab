use bevy::asset::{AssetPath, embedded_asset, embedded_path, load_embedded_asset};
use bevy::prelude::*;

pub struct EmbeddedGameAssetsPlugin;

impl Plugin for EmbeddedGameAssetsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "../assets/sounds/kew.ogg");
        embedded_asset!(app, "../assets/sounds/weep.ogg");
        embedded_asset!(app, "../assets/models/lowpoly_fps.glb");
        embedded_asset!(app, "../assets/models/ghost.glb");
        embedded_asset!(app, "../assets/models/scifi-gun.glb");
        embedded_asset!(app, "../assets/environment_maps/cloudy-bright-day.exr");
        embedded_asset!(app, "../assets/textures/dash.png");
        embedded_asset!(app, "../assets/textures/heart.png");
    }
}

#[derive(Resource)]
pub struct BlobAssets {
    pub bullet_mesh: Handle<Mesh>,
    pub world_scene: Handle<Scene>,
    pub player_scene: Handle<Scene>,
    pub gun_scene: Handle<Scene>,
    pub environment_map: Handle<Image>,
    pub bullet_material: Handle<StandardMaterial>,
    pub shot_sound: Handle<AudioSource>,
    pub dash_sound: Handle<AudioSource>,
    pub dash_icon: Handle<Image>,
    pub heart_icon: Handle<Image>,
}

impl FromWorld for BlobAssets {
    fn from_world(world: &mut World) -> Self {
        let small_blob = Sphere::new(1.0).mesh().uv(18, 10);
        let shot_sound: Handle<AudioSource> = load_embedded_asset!(world, "../assets/sounds/kew.ogg");
        let dash_sound: Handle<AudioSource> = load_embedded_asset!(world, "../assets/sounds/weep.ogg");
        let dash_icon: Handle<Image> = load_embedded_asset!(world, "../assets/textures/dash.png");
        let heart_icon: Handle<Image> = load_embedded_asset!(world, "../assets/textures/heart.png");
        let environment_map: Handle<Image> =
            load_embedded_asset!(world, "../assets/environment_maps/cloudy-bright-day.exr");
        let world_asset_path =
            AssetPath::from_path_buf(embedded_path!("../assets/models/lowpoly_fps.glb")).with_source("embedded");
        let world_scene = world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(world_asset_path));

        let player_asset_path =
            AssetPath::from_path_buf(embedded_path!("../assets/models/ghost.glb")).with_source("embedded");
        let player_scene = world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(player_asset_path));

        let gun_asset_path =
            AssetPath::from_path_buf(embedded_path!("../assets/models/scifi-gun.glb")).with_source("embedded");
        let gun_scene = world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(gun_asset_path));

        let bullet_mesh = {
            let mut meshes = world.resource_mut::<Assets<Mesh>>();
            meshes.add(small_blob)
        };

        let bullet_material = {
            let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
            materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.0, 0.0),
                emissive: Color::srgb(5.0, 0.0, 0.0).into(),
                perceptual_roughness: 0.35,
                ..default()
            })
        };

        Self {
            bullet_mesh,
            world_scene,
            player_scene,
            gun_scene,
            environment_map,
            bullet_material,
            shot_sound,
            dash_sound,
            dash_icon,
            heart_icon,
        }
    }
}
