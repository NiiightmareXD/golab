use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

pub const FIXED_TIMESTEP_HZ: f64 = 64.0;
pub const PROTOCOL_ID: u64 = 0x676f_6c61_625f_4650;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const SERVER_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5000);
pub const CLIENT_SEND_INTERVAL: Duration = Duration::from_millis(33);
pub const SERVER_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);
pub const PING_INTERVAL: Duration = Duration::from_secs(1);
pub const NETCODE_CLIENT_TIMEOUT_SECS: i32 = 60;
pub const NETCODE_TOKEN_EXPIRE_SECS: i32 = -1;
pub const PLAYER_MAX_HEALTH: i32 = 100;
pub const PLAYER_NAME_MAX_CHARS: usize = 20;
pub const BULLET_DAMAGE: i32 = 20;
pub const RESPAWN_SECONDS: f32 = 3.0;
pub const BULLET_SPEED: f32 = 15.0;
pub const BULLET_LIFETIME: f32 = 1.8;
pub const BULLET_DROP_GRAVITY: f32 = 6.0;
pub const BULLET_RADIUS: f32 = 0.03;
pub const BULLET_AIM_DISTANCE: f32 = 1_000.0;
pub const PLAYER_HITBOX_RADIUS: f32 = 0.18;
pub const PLAYER_HITBOX_BOTTOM_ENDPOINT_Y: f32 = -0.22;
pub const PLAYER_HITBOX_TOP_ENDPOINT_Y: f32 = 0.03;
pub const SPAWN_MIN_X: f32 = -3.6;
pub const SPAWN_MAX_X: f32 = 3.6;
pub const SPAWN_MIN_Z: f32 = -7.4;
pub const SPAWN_MAX_Z: f32 = 7.4;
pub const SPAWN_PLAYER_CLEARANCE: f32 = 1.1;
pub const GUN_FIRE_RATE_PER_SECOND: f64 = 6.0;
pub const WORLD_HALF: f32 = 36.0;
pub const TERRAIN_HEIGHT: f32 = 0.0;

#[must_use]
pub fn sanitize_player_name(input: &str) -> String {
    input
        .chars()
        .filter(|character| character.is_ascii() && !character.is_ascii_control())
        .take(PLAYER_NAME_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayerPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub shot_target: [f32; 3],
    pub shot_sequence: u64,
}

impl Default for PlayerPose {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.4, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            shot_target: [0.0, 0.4, -BULLET_AIM_DISTANCE],
            shot_sequence: 0,
        }
    }
}

impl PlayerPose {
    #[must_use]
    pub fn from_transform(transform: &Transform, shot_sequence: u64) -> Self {
        let shot_target = transform.translation + transform.rotation * Vec3::NEG_Z * BULLET_AIM_DISTANCE;
        Self::from_transform_with_shot_target(transform, shot_sequence, shot_target)
    }

    #[must_use]
    pub fn from_transform_with_shot_target(transform: &Transform, shot_sequence: u64, shot_target: Vec3) -> Self {
        let rotation = transform.rotation;
        Self {
            translation: transform.translation.to_array(),
            rotation: [rotation.x, rotation.y, rotation.z, rotation.w],
            shot_target: shot_target.to_array(),
            shot_sequence,
        }
    }

    #[must_use]
    pub fn to_transform(&self) -> Transform {
        Transform::from_translation(Vec3::from_array(self.translation)).with_rotation(self.rotation_quat())
    }

    #[must_use]
    pub fn rotation_quat(&self) -> Quat {
        let rotation = Quat::from_xyzw(self.rotation[0], self.rotation[1], self.rotation[2], self.rotation[3]);
        if rotation.length_squared() > 0.0001 { rotation.normalize() } else { Quat::IDENTITY }
    }

    #[must_use]
    pub const fn shot_target_vec(&self) -> Vec3 {
        Vec3::from_array(self.shot_target)
    }

    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.translation.iter().all(|value| value.is_finite())
            && self.rotation.iter().all(|value| value.is_finite())
            && self.shot_target.iter().all(|value| value.is_finite())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ClientPlayerState {
    pub pose: PlayerPose,
    pub crouching: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RemotePlayerState {
    pub client_id: u64,
    pub name: String,
    pub pose: PlayerPose,
    pub health: i32,
    pub alive: bool,
    pub crouching: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ServerSnapshot {
    pub players: Vec<RemotePlayerState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RespawnRequest {
    pub translation: [f32; 3],
}

impl RespawnRequest {
    #[must_use]
    pub const fn from_translation(translation: Vec3) -> Self {
        Self { translation: translation.to_array() }
    }

    #[must_use]
    pub const fn translation_vec(&self) -> Vec3 {
        Vec3::from_array(self.translation)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct JoinInfo {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PingMessage {
    pub sequence: u64,
    pub client_time: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PongMessage {
    pub sequence: u64,
    pub client_time: f64,
}

pub struct PlayerStateChannel;
pub struct ControlChannel;

#[derive(Clone)]
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.register_message::<ClientPlayerState>().add_direction(NetworkDirection::ClientToServer);
        app.register_message::<ServerSnapshot>().add_direction(NetworkDirection::ServerToClient);
        app.register_message::<RespawnRequest>().add_direction(NetworkDirection::ClientToServer);
        app.register_message::<JoinInfo>().add_direction(NetworkDirection::ClientToServer);
        app.register_message::<PingMessage>().add_direction(NetworkDirection::ClientToServer);
        app.register_message::<PongMessage>().add_direction(NetworkDirection::ServerToClient);

        app.add_channel::<PlayerStateChannel>(ChannelSettings { mode: ChannelMode::SequencedUnreliable, ..default() })
            .add_direction(NetworkDirection::Bidirectional);
        app.add_channel::<ControlChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}
