use std::collections::HashMap;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use golab_shared::{
    BULLET_AIM_DISTANCE, BULLET_DAMAGE, BULLET_DROP_GRAVITY, BULLET_LIFETIME, BULLET_RADIUS, BULLET_SPEED,
    ClientPlayerState, ControlChannel, FIXED_TIMESTEP_HZ, JoinInfo, NETCODE_CLIENT_TIMEOUT_SECS,
    PLAYER_HITBOX_BOTTOM_ENDPOINT_Y, PLAYER_HITBOX_RADIUS, PLAYER_HITBOX_TOP_ENDPOINT_Y, PLAYER_MAX_HEALTH,
    PRIVATE_KEY, PROTOCOL_ID, PingMessage, PlayerPose, PlayerStateChannel, PongMessage, RESPAWN_SECONDS,
    RemotePlayerState, RespawnRequest, SERVER_ADDR, SERVER_SNAPSHOT_INTERVAL, SPAWN_MAX_X, SPAWN_MAX_Z, SPAWN_MIN_X,
    SPAWN_MIN_Z, SPAWN_PLAYER_CLEARANCE, ServerSnapshot, TERRAIN_HEIGHT, WORLD_HALF, sanitize_player_name,
};
use lightyear::connection::client::{Connected, Connecting, Disconnected};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use rand::RngExt;

const MUZZLE_FORWARD_DISTANCE: f32 = 0.5;
const MUZZLE_RIGHT_OFFSET: f32 = 0.2;
const MUZZLE_DOWN_OFFSET: f32 = 0.15;
const SERVER_EYE_HEIGHT: f32 = 0.4;
const SPAWN_ATTEMPTS: usize = 32;

#[derive(Clone)]
struct ServerPlayer {
    name: String,
    pose: PlayerPose,
    health: i32,
    alive: bool,
    crouching: bool,
    last_shot_sequence: u64,
    respawn_timer: Option<Timer>,
}

impl ServerPlayer {
    fn new(client_id: u64, pose: PlayerPose) -> Self {
        Self {
            name: default_player_name(client_id),
            pose,
            health: PLAYER_MAX_HEALTH,
            alive: true,
            crouching: false,
            last_shot_sequence: 0,
            respawn_timer: None,
        }
    }

    const fn respawn(&mut self, pose: PlayerPose) {
        self.pose = pose;
        self.health = PLAYER_MAX_HEALTH;
        self.alive = true;
        self.crouching = false;
        self.respawn_timer = None;
    }

    fn damage(&mut self, amount: i32) {
        if !self.alive {
            return;
        }

        self.health = (self.health - amount).max(0);
        if self.health == 0 {
            self.alive = false;
            self.respawn_timer = Some(Timer::from_seconds(RESPAWN_SECONDS, TimerMode::Once));
        }
    }
}

struct ServerBullet {
    owner: u64,
    position: Vec3,
    velocity: Vec3,
    lifetime: Timer,
}

#[derive(Resource)]
struct ServerPlayers {
    players: HashMap<u64, ServerPlayer>,
    bullets: Vec<ServerBullet>,
    snapshot_timer: Timer,
}

impl Default for ServerPlayers {
    fn default() -> Self {
        Self {
            players: HashMap::default(),
            bullets: Vec::default(),
            snapshot_timer: Timer::new(SERVER_SNAPSHOT_INTERVAL, TimerMode::Repeating),
        }
    }
}

fn main() {
    App::new()
        .add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ))),
        )
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(ServerPlugins { tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ) })
        .add_plugins(golab_shared::ProtocolPlugin)
        .init_resource::<ServerPlayers>()
        .add_systems(Startup, start_server)
        .add_observer(add_message_components_to_client)
        .add_observer(log_netcode_connecting)
        .add_observer(register_connected_client)
        .add_observer(remove_disconnected_client)
        .add_systems(
            Update,
            (
                receive_join_info,
                receive_player_states,
                receive_respawn_requests,
                echo_ping_messages,
                simulate_bullets,
                broadcast_snapshot,
            )
                .chain(),
        )
        .run();
}

fn start_server(mut commands: Commands) {
    let config = NetcodeConfig::default()
        .with_protocol_id(PROTOCOL_ID)
        .with_key(PRIVATE_KEY)
        .with_client_timeout_secs(NETCODE_CLIENT_TIMEOUT_SECS);
    let server = commands
        .spawn((NetcodeServer::new(config), LocalAddr(SERVER_ADDR), ServerUdpIo::default(), Name::new("Golab Server")))
        .id();
    commands.trigger(Start { entity: server });
    info!("Golab server listening on {SERVER_ADDR}");
}

fn add_message_components_to_client(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.entity).insert((
        MessageReceiver::<ClientPlayerState>::default(),
        MessageReceiver::<RespawnRequest>::default(),
        MessageReceiver::<JoinInfo>::default(),
        MessageReceiver::<PingMessage>::default(),
        MessageSender::<ServerSnapshot>::default(),
        MessageSender::<PongMessage>::default(),
        Name::new("Golab Client Link"),
    ));
}

fn log_netcode_connecting(trigger: On<Add, Connecting>, client_ids: Query<&RemoteId, With<ClientOf>>) {
    if let Ok(remote_id) = client_ids.get(trigger.entity) {
        info!("Netcode challenge sent to client {} on link {:?}", remote_id.0.to_bits(), trigger.entity);
    } else {
        info!("Netcode challenge sent on link {:?}", trigger.entity);
    }
}

fn register_connected_client(
    trigger: On<Add, Connected>,
    client_ids: Query<&RemoteId, With<ClientOf>>,
    mut server_players: ResMut<ServerPlayers>,
) {
    let Ok(remote_id) = client_ids.get(trigger.entity) else { return };
    let client_id = remote_id.0.to_bits();
    ensure_player_registered(&mut server_players, client_id);
    info!("Client {client_id} connected");
}

fn remove_disconnected_client(
    trigger: On<Add, Disconnected>,
    client_ids: Query<&RemoteId, With<ClientOf>>,
    mut server_players: ResMut<ServerPlayers>,
) {
    let Ok(remote_id) = client_ids.get(trigger.entity) else { return };
    let client_id = remote_id.0.to_bits();
    server_players.players.remove(&client_id);
    server_players.bullets.retain(|bullet| bullet.owner != client_id);
    info!("Client {client_id} disconnected");
}

fn receive_join_info(
    mut receivers: Query<(&RemoteId, &mut MessageReceiver<JoinInfo>), With<ClientOf>>,
    mut server_players: ResMut<ServerPlayers>,
) {
    for (remote_id, mut receiver) in &mut receivers {
        let client_id = remote_id.0.to_bits();
        for join_info in receiver.receive() {
            ensure_player_registered(&mut server_players, client_id);
            let name = sanitize_player_name(&join_info.name);
            let name = if name.is_empty() { default_player_name(client_id) } else { name };
            let player = server_players.players.get_mut(&client_id).expect("registered player exists");
            player.name = name;
            info!("Client {client_id} joined as {}", player.name);
        }
    }
}

fn receive_player_states(
    mut receivers: Query<(&RemoteId, &mut MessageReceiver<ClientPlayerState>), With<ClientOf>>,
    mut server_players: ResMut<ServerPlayers>,
) {
    for (remote_id, mut receiver) in &mut receivers {
        let client_id = remote_id.0.to_bits();
        for state in receiver.receive() {
            let shots_to_spawn;
            {
                ensure_player_registered(&mut server_players, client_id);
                let player = server_players.players.get_mut(&client_id).expect("registered player exists");
                shots_to_spawn = apply_client_state_and_count_shots(player, &state);
            }

            for _ in 0..shots_to_spawn {
                server_players.bullets.push(spawn_bullet(client_id, &state.pose));
            }
        }
    }
}

fn apply_client_state_and_count_shots(player: &mut ServerPlayer, state: &ClientPlayerState) -> u64 {
    let shots_to_spawn =
        if player.alive { state.pose.shot_sequence.saturating_sub(player.last_shot_sequence) } else { 0 };
    player.last_shot_sequence = state.pose.shot_sequence;
    player.pose.clone_from(&state.pose);
    player.crouching = state.crouching;
    shots_to_spawn
}

fn receive_respawn_requests(
    mut receivers: Query<(&RemoteId, &mut MessageReceiver<RespawnRequest>), With<ClientOf>>,
    mut server_players: ResMut<ServerPlayers>,
) {
    for (remote_id, mut receiver) in &mut receivers {
        let client_id = remote_id.0.to_bits();
        for request in receiver.receive() {
            ensure_player_registered(&mut server_players, client_id);
            let player = server_players.players.get_mut(&client_id).expect("registered player exists");
            if player.alive || player.respawn_timer.as_ref().is_some_and(|timer| !timer.is_finished()) {
                continue;
            }

            let Some(pose) = respawn_pose_from_request(&request) else {
                warn!("Ignoring invalid respawn position from client {client_id}");
                continue;
            };

            player.respawn(pose);
            info!("Client {client_id} respawned");
        }
    }
}

fn echo_ping_messages(
    mut links: Query<(&mut MessageReceiver<PingMessage>, &mut MessageSender<PongMessage>), With<ClientOf>>,
) {
    for (mut receiver, mut sender) in &mut links {
        for ping in receiver.receive() {
            sender.send::<ControlChannel>(PongMessage { sequence: ping.sequence, client_time: ping.client_time });
        }
    }
}

fn simulate_bullets(time: Res<Time>, mut server_players: ResMut<ServerPlayers>) {
    let dt = time.delta_secs();
    tick_respawns(&time, &mut server_players.players);

    let ServerPlayers { players, bullets, .. } = &mut *server_players;
    let mut bullet_index = 0;
    while bullet_index < bullets.len() {
        let previous_position = bullets[bullet_index].position;
        {
            let bullet = &mut bullets[bullet_index];
            bullet.velocity.y = (-BULLET_DROP_GRAVITY).mul_add(dt, bullet.velocity.y);
            bullet.position += bullet.velocity * dt;
            bullet.lifetime.tick(time.delta());
        }

        let hit_target = players.iter().find_map(|(&target_id, target)| {
            if target_id == bullets[bullet_index].owner || !target.alive {
                return None;
            }

            segment_hits_player_hitbox(previous_position, bullets[bullet_index].position, &target.pose)
                .then_some(target_id)
        });

        let expired = bullets[bullet_index].lifetime.is_finished()
            || bullets[bullet_index].position.x.abs() > WORLD_HALF
            || bullets[bullet_index].position.z.abs() > WORLD_HALF
            || bullets[bullet_index].position.y <= TERRAIN_HEIGHT;

        if let Some(target_id) = hit_target {
            if let Some(target) = players.get_mut(&target_id) {
                target.damage(BULLET_DAMAGE);
                info!("Client {target_id} took {BULLET_DAMAGE} damage: {} HP", target.health);
            }
            bullets.swap_remove(bullet_index);
        } else if expired {
            bullets.swap_remove(bullet_index);
        } else {
            bullet_index += 1;
        }
    }
}

fn tick_respawns(time: &Time, players: &mut HashMap<u64, ServerPlayer>) {
    for player in players.values_mut() {
        let Some(timer) = player.respawn_timer.as_mut() else { continue };
        timer.tick(time.delta());
    }
}

fn respawn_pose_from_request(request: &RespawnRequest) -> Option<PlayerPose> {
    let mut translation = request.translation_vec();
    if !translation.is_finite() {
        return None;
    }

    translation.x = translation.x.clamp(SPAWN_MIN_X, SPAWN_MAX_X);
    translation.y = TERRAIN_HEIGHT + SERVER_EYE_HEIGHT;
    translation.z = translation.z.clamp(SPAWN_MIN_Z, SPAWN_MAX_Z);

    let rotation = Transform::from_translation(translation).looking_at(Vec3::ZERO, Vec3::Y).rotation;
    let transform = Transform::from_translation(translation).with_rotation(rotation);
    Some(PlayerPose::from_transform(&transform, 0))
}

fn broadcast_snapshot(
    time: Res<Time>,
    mut server_players: ResMut<ServerPlayers>,
    mut senders: Query<&mut MessageSender<ServerSnapshot>, (With<ClientOf>, With<Connected>)>,
) {
    server_players.snapshot_timer.tick(time.delta());
    if !server_players.snapshot_timer.just_finished() {
        return;
    }

    let snapshot = ServerSnapshot {
        players: server_players
            .players
            .iter()
            .map(|(&client_id, player)| RemotePlayerState {
                client_id,
                name: player.name.clone(),
                pose: player.pose.clone(),
                health: player.health,
                alive: player.alive,
                crouching: player.crouching,
            })
            .collect(),
    };

    for mut sender in &mut senders {
        sender.send::<PlayerStateChannel>(snapshot.clone());
    }
}

fn spawn_bullet(owner: u64, pose: &PlayerPose) -> ServerBullet {
    let translation = Vec3::from_array(pose.translation);
    let rotation = pose.rotation_quat();
    let forward = rotation * Vec3::NEG_Z;
    let right = rotation * Vec3::X;
    let up = rotation * Vec3::Y;
    let muzzle =
        translation + forward * MUZZLE_FORWARD_DISTANCE + right * MUZZLE_RIGHT_OFFSET - up * MUZZLE_DOWN_OFFSET;
    let mut target = pose.shot_target_vec();
    if target.distance_squared(muzzle) <= f32::EPSILON {
        target = translation + forward * BULLET_AIM_DISTANCE;
    }
    let direction = (target - muzzle).try_normalize().unwrap_or(forward);

    ServerBullet {
        owner,
        position: muzzle,
        velocity: direction * BULLET_SPEED,
        lifetime: Timer::from_seconds(BULLET_LIFETIME, TimerMode::Once),
    }
}

fn ensure_player_registered(server_players: &mut ServerPlayers, client_id: u64) {
    if server_players.players.contains_key(&client_id) {
        return;
    }

    let pose = random_spawn_pose(client_id, &server_players.players);
    server_players.players.insert(client_id, ServerPlayer::new(client_id, pose));
}

fn default_player_name(client_id: u64) -> String {
    format!("Player {}", client_id % 10_000)
}

fn random_spawn_pose(client_id: u64, players: &HashMap<u64, ServerPlayer>) -> PlayerPose {
    let translation = random_spawn_translation(client_id, players);
    let rotation = Transform::from_translation(translation).looking_at(Vec3::ZERO, Vec3::Y).rotation;
    let transform = Transform::from_translation(translation).with_rotation(rotation);
    PlayerPose::from_transform(&transform, 0)
}

fn random_spawn_translation(client_id: u64, players: &HashMap<u64, ServerPlayer>) -> Vec3 {
    let mut rng = rand::rng();
    for _ in 0..SPAWN_ATTEMPTS {
        let candidate = Vec3::new(
            rng.random_range(SPAWN_MIN_X..=SPAWN_MAX_X),
            TERRAIN_HEIGHT + SERVER_EYE_HEIGHT,
            rng.random_range(SPAWN_MIN_Z..=SPAWN_MAX_Z),
        );
        if spawn_position_is_clear(client_id, candidate, players) {
            return candidate;
        }
    }

    fallback_spawn_translation(client_id, players)
}

fn fallback_spawn_translation(client_id: u64, players: &HashMap<u64, ServerPlayer>) -> Vec3 {
    for ring in 0_u8..4 {
        for lane in 0_u8..8 {
            let angle = (f32::from(lane) / 8.0 + f32::from((client_id % 8) as u8) / 64.0) * std::f32::consts::TAU;
            let radius = f32::from(ring).mul_add(0.85, 1.0);
            let candidate = Vec3::new(
                (angle.cos() * radius).clamp(SPAWN_MIN_X, SPAWN_MAX_X),
                TERRAIN_HEIGHT + SERVER_EYE_HEIGHT,
                (angle.sin() * radius).clamp(SPAWN_MIN_Z, SPAWN_MAX_Z),
            );
            if spawn_position_is_clear(client_id, candidate, players) {
                return candidate;
            }
        }
    }

    Vec3::new(0.0, TERRAIN_HEIGHT + SERVER_EYE_HEIGHT, 0.0)
}

fn spawn_position_is_clear(client_id: u64, candidate: Vec3, players: &HashMap<u64, ServerPlayer>) -> bool {
    if !(SPAWN_MIN_X..=SPAWN_MAX_X).contains(&candidate.x) || !(SPAWN_MIN_Z..=SPAWN_MAX_Z).contains(&candidate.z) {
        return false;
    }

    let clearance_squared = SPAWN_PLAYER_CLEARANCE * SPAWN_PLAYER_CLEARANCE;
    players.iter().all(|(&other_id, other)| {
        if other_id == client_id || !other.alive {
            return true;
        }

        let other_position = Vec3::from_array(other.pose.translation);
        Vec2::new(candidate.x - other_position.x, candidate.z - other_position.z).length_squared() >= clearance_squared
    })
}

fn segment_hits_player_hitbox(bullet_start: Vec3, bullet_end: Vec3, pose: &PlayerPose) -> bool {
    let translation = Vec3::from_array(pose.translation);
    let hitbox_axis = pose.rotation_quat() * Vec3::Y;
    let capsule_start = translation + hitbox_axis * PLAYER_HITBOX_BOTTOM_ENDPOINT_Y;
    let capsule_end = translation + hitbox_axis * PLAYER_HITBOX_TOP_ENDPOINT_Y;
    let radius = PLAYER_HITBOX_RADIUS + BULLET_RADIUS;

    segment_segment_distance_squared(bullet_start, bullet_end, capsule_start, capsule_end) <= radius * radius
}

fn segment_segment_distance_squared(
    bullet_start: Vec3,
    bullet_end: Vec3,
    capsule_start: Vec3,
    capsule_end: Vec3,
) -> f32 {
    let bullet_delta = bullet_end - bullet_start;
    let capsule_delta = capsule_end - capsule_start;
    let between_starts = bullet_start - capsule_start;
    let bullet_length_squared = bullet_delta.length_squared();
    let capsule_length_squared = capsule_delta.length_squared();

    if bullet_length_squared <= f32::EPSILON && capsule_length_squared <= f32::EPSILON {
        return bullet_start.distance_squared(capsule_start);
    }

    if bullet_length_squared <= f32::EPSILON {
        let capsule_t = (capsule_delta.dot(between_starts) / capsule_length_squared).clamp(0.0, 1.0);
        return bullet_start.distance_squared(capsule_start + capsule_delta * capsule_t);
    }

    if capsule_length_squared <= f32::EPSILON {
        let bullet_t = (-bullet_delta.dot(between_starts) / bullet_length_squared).clamp(0.0, 1.0);
        return (bullet_start + bullet_delta * bullet_t).distance_squared(capsule_start);
    }

    let bullet_capsule_dot = bullet_delta.dot(capsule_delta);
    let bullet_start_dot = bullet_delta.dot(between_starts);
    let capsule_start_dot = capsule_delta.dot(between_starts);
    let denominator = bullet_capsule_dot.mul_add(-bullet_capsule_dot, bullet_length_squared * capsule_length_squared);

    let mut bullet_t = if denominator > f32::EPSILON {
        (bullet_start_dot.mul_add(-capsule_length_squared, bullet_capsule_dot * capsule_start_dot) / denominator)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };

    let capsule_t_numerator = bullet_capsule_dot.mul_add(bullet_t, capsule_start_dot);
    let capsule_t = if capsule_t_numerator < 0.0 {
        bullet_t = (-bullet_start_dot / bullet_length_squared).clamp(0.0, 1.0);
        0.0
    } else if capsule_t_numerator > capsule_length_squared {
        bullet_t = ((bullet_capsule_dot - bullet_start_dot) / bullet_length_squared).clamp(0.0, 1.0);
        1.0
    } else {
        capsule_t_numerator / capsule_length_squared
    };

    let closest_bullet = bullet_start + bullet_delta * bullet_t;
    let closest_capsule = capsule_start + capsule_delta * capsule_t;
    closest_bullet.distance_squared(closest_capsule)
}
