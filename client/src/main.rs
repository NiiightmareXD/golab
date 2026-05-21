mod assets;
mod bullet;
mod constants;
mod environment;
mod network;
mod player;
mod settings;
mod ui;
mod world;

use std::time::Duration;

use assets::{BlobAssets, EmbeddedGameAssetsPlugin};
use avian3d::prelude::*;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{InstanceFlags, RenderCreation, WgpuSettings};
use bevy::window::{MonitorSelection, WindowMode};
use bevy::winit::WinitSettings;
use bullet::{GunFireLimiter, LocalShotFired, move_bullets, shoot, update_hit_effects};
use environment::{EnvironmentMapState, apply_skybox, prepare_environment_map_cubemap};
use golab_shared::{FIXED_TIMESTEP_HZ, ProtocolPlugin};
use lightyear::prelude::client::ClientPlugins;
use network::MultiplayerClientPlugin;
use player::{
    CameraMode, RespawnState, ThirdPersonCameraState, cursor_grab, look, move_player, setup_player, sync_camera_mode,
    sync_view_model_render_layers, tick_respawn_state, toggle_camera_mode, update_camera,
    update_third_person_model_opacity, update_view_model_lighting,
};
use settings::{AudioSettings, GraphicsSettings, MouseSettings, apply_graphics_settings};
use ui::{
    MenuState, menu_button_actions, menu_button_visuals, menu_closed, menu_input, server_address_text_input, setup_ui,
    slider_interactions, sync_join_connection, sync_menu_display, update_audio_text, update_dash_text, update_fps_text,
    update_graphics_text, update_health_text, update_join_button_text, update_join_error_text,
    update_join_field_visuals, update_network_status_text, update_ping_text, update_player_name_text,
    update_respawn_countdown_text, update_sensitivity_text, update_server_address_text, update_slider_visuals,
};
use world::setup_world;

fn main() {
    let graphics_settings = GraphicsSettings::default();

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.54, 0.73, 0.92)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.86, 0.92, 1.0), brightness: 650.0, ..default() })
        .insert_resource(WinitSettings::continuous())
        .init_resource::<MouseSettings>()
        .init_resource::<AudioSettings>()
        .insert_resource(graphics_settings)
        .init_resource::<MenuState>()
        .init_resource::<GunFireLimiter>()
        .init_resource::<EnvironmentMapState>()
        .init_resource::<CameraMode>()
        .init_resource::<ThirdPersonCameraState>()
        .init_resource::<RespawnState>()
        .add_plugins(DefaultPlugins.set(render_plugin()).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Golab".to_string(),
                mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(ClientPlugins { tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ) })
        .add_plugins(ProtocolPlugin)
        .add_plugins(MultiplayerClientPlugin)
        .add_message::<LocalShotFired>()
        .add_plugins(PhysicsPlugins::default())
        .add_plugins(EmbeddedGameAssetsPlugin)
        .init_resource::<BlobAssets>()
        .add_systems(Startup, (setup_world, setup_player, setup_ui).chain())
        .add_systems(Update, menu_input)
        .add_systems(Update, server_address_text_input.after(menu_input))
        .add_systems(Update, tick_respawn_state)
        .add_systems(Update, sync_view_model_render_layers)
        .add_systems(Update, (prepare_environment_map_cubemap, apply_skybox).chain())
        .add_systems(
            Update,
            (
                cursor_grab,
                toggle_camera_mode,
                sync_camera_mode.after(toggle_camera_mode),
                look,
                move_player.after(look),
                update_view_model_lighting.after(move_player),
                update_camera.after(move_player),
                update_third_person_model_opacity.after(update_camera).after(sync_camera_mode),
                shoot.after(move_player),
                move_bullets,
                update_hit_effects,
            )
                .run_if(menu_closed)
                .after(menu_input),
        )
        .add_systems(Update, menu_button_actions.after(move_bullets))
        .add_systems(Update, apply_graphics_settings.after(menu_button_actions))
        .add_systems(Update, slider_interactions.after(menu_button_actions))
        .add_systems(Update, sync_join_connection.after(slider_interactions))
        .add_systems(Update, sync_menu_display.after(sync_join_connection))
        .add_systems(
            Update,
            (
                update_fps_text,
                update_ping_text,
                update_sensitivity_text,
                update_audio_text,
                update_graphics_text,
                update_network_status_text,
                update_server_address_text,
                update_player_name_text,
                update_join_error_text,
                update_join_button_text,
                update_respawn_countdown_text,
                update_health_text,
                update_dash_text,
                update_slider_visuals,
                menu_button_visuals,
                update_join_field_visuals,
            )
                .after(sync_menu_display),
        )
        .run();
}

fn render_plugin() -> RenderPlugin {
    let wgpu_settings = WgpuSettings { instance_flags: InstanceFlags::VALIDATION_INDIRECT_CALL, ..default() };

    RenderPlugin { render_creation: RenderCreation::Automatic(wgpu_settings), ..default() }
}
