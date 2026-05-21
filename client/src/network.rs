use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use avian3d::prelude::*;
use bevy::camera::visibility::RenderLayers;
use bevy::color::Alpha;
use bevy::prelude::*;
use bevy::render::alpha::AlphaMode;
use bevy::ui::Val::Px;
use golab_shared::{
    CLIENT_SEND_INTERVAL, ClientPlayerState, ControlChannel, JoinInfo, NETCODE_CLIENT_TIMEOUT_SECS,
    NETCODE_TOKEN_EXPIRE_SECS, PING_INTERVAL, PLAYER_NAME_MAX_CHARS, PRIVATE_KEY, PROTOCOL_ID, PingMessage, PlayerPose,
    PlayerStateChannel, PongMessage, RespawnRequest, SERVER_ADDR, ServerSnapshot, sanitize_player_name,
};
use lightyear::connection::client::{Connected, Connecting, Disconnected};
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use crate::assets::BlobAssets;
use crate::bullet::{BulletHitbox, LocalShotFired, spawn_bullet_from_transform};
use crate::constants::{DEFAULT_RENDER_LAYER, PLAYER_HITBOX_LAYER};
use crate::player::{
    MainPlayerCamera, Player, PlayerBody, PlayerHealth, build_player_hitbox_collider, player_model_transform,
    random_player_spawn_translation, respawn_player_at,
};
use crate::settings::AudioSettings;
use crate::ui::GameplayHud;

const NAME_TAG_HEIGHT: f32 = 24.0;
const NAME_TAG_HORIZONTAL_PADDING: f32 = 16.0;
const NAME_TAG_MIN_WIDTH: f32 = 36.0;
const NAME_TAG_MAX_WIDTH: f32 = 170.0;
const NAME_TAG_MAX_CHARS_FOR_WIDTH: u16 = 32;
const NAME_TAG_AVERAGE_GLYPH_WIDTH: f32 = 7.0;
const NAME_TAG_SCREEN_OFFSET: f32 = 6.0;
const NAME_TAG_WORLD_OFFSET: Vec3 = Vec3::new(0.0, 0.48, 0.0);
const DISCONNECTED_CLIENT_DESPAWN_SECONDS: f32 = 0.25;
const NAME_TAG_BACKGROUND_ALPHA: f32 = 0.38;
const NAME_TAG_TEXT_ALPHA: f32 = 0.95;
const NAME_TAG_SHADOW_ALPHA: f32 = 0.9;
const REMOTE_PLAYER_FADE_SPEED: f32 = 7.0;
const REMOTE_PLAYER_FADE_EPSILON: f32 = 0.02;
const REMOTE_PLAYER_INTERPOLATION_SECONDS: f32 = 0.075;
const REMOTE_PLAYER_SNAP_DISTANCE: f32 = 2.5;

#[derive(Message)]
pub struct ConnectToServer {
    pub address: SocketAddr,
    pub player_name: String,
    pub attempt_id: u64,
}

#[derive(Message, Default)]
pub struct DisconnectFromServer;

#[derive(Message, Default)]
pub struct RespawnPlayer;

#[derive(Resource)]
pub struct ServerAddressInput {
    text: String,
    error: Option<String>,
}

impl Default for ServerAddressInput {
    fn default() -> Self {
        Self { text: SERVER_ADDR.to_string(), error: None }
    }
}

impl ServerAddressInput {
    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn push_text(&mut self, text: &str) {
        let remaining = 64_usize.saturating_sub(self.text.len());
        self.text.extend(text.chars().take(remaining));
    }

    pub fn pop(&mut self) {
        self.text.pop();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn parse(&mut self) -> Option<SocketAddr> {
        match self.text.trim().parse::<SocketAddr>() {
            Ok(address) => {
                self.error = None;
                Some(address)
            }
            Err(error) => {
                self.error = Some(format!("Invalid socket address: {error}"));
                None
            }
        }
    }
}

#[derive(Resource)]
pub struct PlayerNameInput {
    text: String,
    error: Option<String>,
}

impl Default for PlayerNameInput {
    fn default() -> Self {
        Self { text: "Player".to_string(), error: None }
    }
}

impl PlayerNameInput {
    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn push_text(&mut self, text: &str) {
        let remaining = PLAYER_NAME_MAX_CHARS.saturating_sub(self.text.chars().count());
        self.text.extend(text.chars().take(remaining));
    }

    pub fn pop(&mut self) {
        self.text.pop();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn parse(&mut self) -> Option<String> {
        let name = sanitize_player_name(&self.text);
        if name.is_empty() {
            self.error = Some("User name cannot be empty.".to_string());
            return None;
        }

        self.text.clone_from(&name);
        self.error = None;
        Some(name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NetworkStatus {
    #[default]
    Offline,
    Connecting,
    Connected,
    Disconnected(String),
}

#[derive(Resource)]
pub struct NetworkClientState {
    client_entity: Option<Entity>,
    client_id: u64,
    connect_attempt_id: u64,
    server_addr: Option<SocketAddr>,
    player_name: String,
    sent_join_info: bool,
    shot_sequence: u64,
    shot_target: Vec3,
    send_timer: Timer,
    ping_timer: Timer,
    ping_sequence: u64,
    ping_ms: Option<f64>,
    status: NetworkStatus,
    received_local_snapshot: bool,
    awaiting_respawn_snapshot: bool,
}

impl Default for NetworkClientState {
    fn default() -> Self {
        Self {
            client_entity: None,
            client_id: 0,
            connect_attempt_id: 0,
            server_addr: None,
            player_name: "Player".to_string(),
            sent_join_info: false,
            shot_sequence: 0,
            shot_target: Vec3::ZERO,
            send_timer: Timer::new(CLIENT_SEND_INTERVAL, TimerMode::Repeating),
            ping_timer: Timer::new(PING_INTERVAL, TimerMode::Repeating),
            ping_sequence: 0,
            ping_ms: None,
            status: NetworkStatus::Offline,
            received_local_snapshot: false,
            awaiting_respawn_snapshot: false,
        }
    }
}

impl NetworkClientState {
    pub const fn is_connected(&self) -> bool {
        matches!(self.status, NetworkStatus::Connected)
    }

    pub const fn is_join_active(&self) -> bool {
        matches!(self.status, NetworkStatus::Connecting | NetworkStatus::Connected)
    }

    pub const fn client_id(&self) -> Option<u64> {
        if self.is_connected() { Some(self.client_id) } else { None }
    }

    pub const fn ready_attempt_id(&self) -> Option<u64> {
        if self.is_connected() && self.received_local_snapshot { Some(self.connect_attempt_id) } else { None }
    }

    pub const fn is_failed_attempt(&self, attempt_id: u64) -> bool {
        self.connect_attempt_id == attempt_id
            && matches!(self.status, NetworkStatus::Disconnected(_) | NetworkStatus::Offline)
    }

    pub fn ping_label(&self) -> String {
        self.ping_ms.map_or_else(|| "Ping: -- ms".to_string(), |ping| format!("Ping: {ping:.0} ms"))
    }

    pub fn status_label(&self) -> String {
        match &self.status {
            NetworkStatus::Offline => "Network: offline".to_string(),
            NetworkStatus::Connecting => format!(
                "Network: connecting to {}",
                self.server_addr.map_or_else(|| SERVER_ADDR.to_string(), |address| address.to_string())
            ),
            NetworkStatus::Connected if self.received_local_snapshot => format!(
                "Network: connected to {} as {}",
                self.server_addr.map_or_else(|| SERVER_ADDR.to_string(), |address| address.to_string()),
                self.player_name
            ),
            NetworkStatus::Connected => format!(
                "Network: connected to {}, syncing game",
                self.server_addr.map_or_else(|| SERVER_ADDR.to_string(), |address| address.to_string())
            ),
            NetworkStatus::Disconnected(reason) if reason.is_empty() => "Network: disconnected".to_string(),
            NetworkStatus::Disconnected(reason) => format!("Network: disconnected ({reason})"),
        }
    }
}

#[derive(Resource, Default)]
struct RemotePlayers {
    entities: HashMap<u64, RemotePlayerEntities>,
}

struct RemotePlayerEntities {
    body: Entity,
    name_tag: Entity,
}

#[derive(Component)]
struct RemotePlayer {
    name: String,
    alive: bool,
    crouching: bool,
    visual_alpha: f32,
    target_visual_alpha: f32,
    interpolation_start: Transform,
    target_transform: Transform,
    interpolation_elapsed: f32,
    last_shot_sequence: u64,
}

#[derive(Component, Clone, Copy)]
struct RemotePlayerFadeMaterial {
    original_alpha: f32,
    original_alpha_mode: AlphaMode,
}

#[derive(Component)]
struct RemoteNameTag {
    player: Entity,
    text: Entity,
}

#[derive(Component)]
struct RemoteNameTagText;

#[derive(Component)]
struct DisconnectedClientCleanup {
    timer: Timer,
}

type RemotePlayerSnapshotQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Transform, &'static mut Visibility, &'static mut RemotePlayer),
    (With<RemotePlayer>, Without<Player>),
>;
type RemoteNameTagQuery<'w, 's> =
    Query<'w, 's, (&'static RemoteNameTag, &'static mut Node, &'static mut Visibility, &'static mut BackgroundColor)>;
type RemoteNameTagTextQuery<'w, 's> =
    Query<'w, 's, (&'static mut Text, &'static mut TextColor, &'static mut TextShadow), With<RemoteNameTagText>>;
type LocalPlayerSnapshotQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Transform, &'static mut PlayerBody, &'static mut PlayerHealth),
    (With<Player>, Without<RemotePlayer>),
>;

pub struct MultiplayerClientPlugin;

impl Plugin for MultiplayerClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConnectToServer>()
            .add_message::<DisconnectFromServer>()
            .add_message::<RespawnPlayer>()
            .init_resource::<NetworkClientState>()
            .init_resource::<ServerAddressInput>()
            .init_resource::<PlayerNameInput>()
            .init_resource::<RemotePlayers>()
            .add_systems(Update, handle_connect_requests)
            .add_systems(Update, handle_disconnect_requests.after(handle_connect_requests))
            .add_systems(Update, update_connection_status.after(handle_disconnect_requests))
            .add_systems(Update, cleanup_disconnected_clients.after(update_connection_status))
            .add_systems(Update, send_join_info.after(update_connection_status))
            .add_systems(Update, sync_local_hitbox.after(update_connection_status))
            .add_systems(Update, handle_respawn_requests.after(update_connection_status))
            .add_systems(Update, send_ping_messages.after(update_connection_status))
            .add_systems(Update, receive_pong_messages.after(send_ping_messages))
            .add_systems(Update, record_local_shots.after(crate::bullet::shoot).run_if(crate::ui::menu_closed))
            .add_systems(Update, send_player_pose.after(record_local_shots))
            .add_systems(Update, receive_server_snapshots.after(send_player_pose))
            .add_systems(Update, smooth_remote_player_transforms.after(receive_server_snapshots))
            .add_systems(Update, update_remote_player_fades.after(smooth_remote_player_transforms))
            .add_systems(
                Update,
                update_remote_name_tags.after(update_remote_player_fades).run_if(crate::ui::menu_closed),
            );
    }
}

const fn reset_network_session_state(state: &mut NetworkClientState) {
    state.client_id = 0;
    state.server_addr = None;
    state.sent_join_info = false;
    state.shot_sequence = 0;
    state.shot_target = Vec3::ZERO;
    state.ping_sequence = 0;
    state.ping_ms = None;
    state.received_local_snapshot = false;
    state.awaiting_respawn_snapshot = false;
}

fn handle_connect_requests(
    mut commands: Commands,
    mut requests: MessageReader<ConnectToServer>,
    mut state: ResMut<NetworkClientState>,
    existing_clients: Query<(), With<Client>>,
) -> Result {
    let Some(request) = requests.read().last() else {
        return Ok(());
    };

    if state.is_join_active() {
        warn!("Ignoring join request while a server connection is already active");
        return Ok(());
    }

    if let Some(entity) = state.client_entity
        && existing_clients.get(entity).is_ok()
    {
        commands.entity(entity).despawn();
    }

    let client_id = generate_client_id();
    let auth = Authentication::Manual {
        server_addr: request.address,
        client_id,
        private_key: PRIVATE_KEY,
        protocol_id: PROTOCOL_ID,
    };
    let local_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    let client = commands
        .spawn((
            Client::default(),
            LocalAddr(local_addr),
            PeerAddr(request.address),
            Link::new(None),
            ReplicationReceiver::default(),
            MessageSender::<ClientPlayerState>::default(),
            MessageSender::<RespawnRequest>::default(),
            MessageSender::<JoinInfo>::default(),
            MessageSender::<PingMessage>::default(),
            MessageReceiver::<ServerSnapshot>::default(),
            MessageReceiver::<PongMessage>::default(),
            NetcodeClient::new(
                auth,
                NetcodeConfig {
                    client_timeout_secs: NETCODE_CLIENT_TIMEOUT_SECS,
                    token_expire_secs: NETCODE_TOKEN_EXPIRE_SECS,
                    ..default()
                },
            )?,
            UdpIo::default(),
            Name::new("Golab Multiplayer Client"),
        ))
        .id();

    commands.trigger(Connect { entity: client });
    state.client_entity = Some(client);
    state.client_id = client_id;
    state.connect_attempt_id = request.attempt_id;
    state.server_addr = Some(request.address);
    state.player_name.clone_from(&request.player_name);
    state.sent_join_info = false;
    state.shot_sequence = 0;
    state.shot_target = Vec3::ZERO;
    state.ping_sequence = 0;
    state.ping_ms = None;
    state.status = NetworkStatus::Connecting;
    state.received_local_snapshot = false;
    state.awaiting_respawn_snapshot = false;
    info!("Connecting to {} as {} ({client_id})", request.address, request.player_name);
    Ok(())
}

fn handle_disconnect_requests(
    mut commands: Commands,
    mut requests: MessageReader<DisconnectFromServer>,
    mut state: ResMut<NetworkClientState>,
    mut remote_players: ResMut<RemotePlayers>,
) {
    if requests.read().next().is_none() {
        return;
    }

    if let Some(entity) = state.client_entity.take() {
        commands.trigger(Disconnect { entity });
        commands.entity(entity).insert(DisconnectedClientCleanup {
            timer: Timer::from_seconds(DISCONNECTED_CLIENT_DESPAWN_SECONDS, TimerMode::Once),
        });
    }

    for (_client_id, entities) in remote_players.entities.drain() {
        commands.entity(entities.body).despawn();
        commands.entity(entities.name_tag).despawn();
    }

    reset_network_session_state(&mut state);
    state.status = NetworkStatus::Offline;
}

fn cleanup_disconnected_clients(
    mut commands: Commands,
    time: Res<Time>,
    mut clients: Query<(Entity, &mut DisconnectedClientCleanup)>,
) {
    for (entity, mut cleanup) in &mut clients {
        cleanup.timer.tick(time.delta());
        if cleanup.timer.just_finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn update_connection_status(
    mut state: ResMut<NetworkClientState>,
    connection_query: Query<(Option<&Connected>, Option<&Connecting>, Option<&Disconnected>)>,
) {
    let Some(entity) = state.client_entity else { return };
    let Ok((connected, connecting, disconnected)) = connection_query.get(entity) else {
        state.client_entity = None;
        reset_network_session_state(&mut state);
        state.status = NetworkStatus::Offline;
        return;
    };

    let next_status = if connected.is_some() {
        NetworkStatus::Connected
    } else if connecting.is_some() {
        NetworkStatus::Connecting
    } else if let Some(disconnected) = disconnected {
        NetworkStatus::Disconnected(disconnected.reason.clone().unwrap_or_default())
    } else {
        NetworkStatus::Offline
    };

    if state.status != next_status {
        state.status = next_status;
    }
}

fn sync_local_hitbox(
    mut commands: Commands,
    state: Res<NetworkClientState>,
    local_player: Query<(Entity, Option<&BulletHitbox>), With<Player>>,
) {
    let Some((entity, hitbox)) = local_player.iter().next() else { return };
    let Some(client_id) = state.client_id() else {
        if hitbox.is_some() {
            commands.entity(entity).remove::<BulletHitbox>();
        }
        return;
    };

    if hitbox.is_none_or(|hitbox| hitbox.owner != client_id) {
        commands.entity(entity).insert(BulletHitbox::player(client_id));
    }
}

fn send_join_info(
    mut state: ResMut<NetworkClientState>,
    mut senders: Query<&mut MessageSender<JoinInfo>, With<Connected>>,
) {
    if state.sent_join_info {
        return;
    }

    let Some(client_entity) = state.client_entity else { return };
    let Ok(mut sender) = senders.get_mut(client_entity) else { return };
    sender.send::<ControlChannel>(JoinInfo { name: state.player_name.clone() });
    state.sent_join_info = true;
}

fn send_ping_messages(
    time: Res<Time>,
    mut state: ResMut<NetworkClientState>,
    mut senders: Query<&mut MessageSender<PingMessage>, With<Connected>>,
) {
    state.ping_timer.tick(time.delta());
    if !state.ping_timer.just_finished() {
        return;
    }

    let Some(client_entity) = state.client_entity else { return };
    let Ok(mut sender) = senders.get_mut(client_entity) else { return };

    state.ping_sequence = state.ping_sequence.saturating_add(1);
    sender.send::<ControlChannel>(PingMessage { sequence: state.ping_sequence, client_time: time.elapsed_secs_f64() });
}

fn receive_pong_messages(
    time: Res<Time>,
    mut state: ResMut<NetworkClientState>,
    mut receivers: Query<&mut MessageReceiver<PongMessage>, With<Client>>,
) {
    let Some(client_entity) = state.client_entity else { return };
    let Ok(mut receiver) = receivers.get_mut(client_entity) else { return };

    for pong in receiver.receive() {
        if pong.sequence <= state.ping_sequence {
            state.ping_ms = Some(((time.elapsed_secs_f64() - pong.client_time) * 1_000.0).max(0.0));
        }
    }
}

fn handle_respawn_requests(
    mut requests: MessageReader<RespawnPlayer>,
    mut state: ResMut<NetworkClientState>,
    mut senders: Query<&mut MessageSender<RespawnRequest>, With<Connected>>,
    mut local_player: Query<(&mut Transform, &mut PlayerBody, &mut PlayerHealth), With<Player>>,
) {
    if requests.read().next().is_none() {
        return;
    }

    if let Some(client_entity) = state.client_entity
        && let Ok(mut sender) = senders.get_mut(client_entity)
    {
        sender.send::<ControlChannel>(RespawnRequest::from_translation(random_player_spawn_translation()));
        state.awaiting_respawn_snapshot = true;
        return;
    }

    let Some((mut transform, mut body, mut health)) = local_player.iter_mut().next() else { return };
    let spawn_translation = random_player_spawn_translation();
    respawn_player_at(&mut transform, &mut body, &mut health, spawn_translation);
}

fn record_local_shots(
    mut shot_events: MessageReader<LocalShotFired>,
    mut state: ResMut<NetworkClientState>,
    local_player: Query<&PlayerHealth, With<Player>>,
) {
    let mut shot_count = 0_u64;
    for shot in shot_events.read() {
        shot_count = shot_count.saturating_add(1);
        state.shot_target = shot.target;
    }

    let alive = local_player.iter().next().is_some_and(PlayerHealth::is_alive);
    if shot_count > 0
        && state.is_connected()
        && state.received_local_snapshot
        && !state.awaiting_respawn_snapshot
        && alive
    {
        state.shot_sequence = state.shot_sequence.saturating_add(shot_count);
    }
}

fn send_player_pose(
    time: Res<Time>,
    mut state: ResMut<NetworkClientState>,
    players: Query<(&Transform, &PlayerHealth, &PlayerBody), With<Player>>,
    mut senders: Query<&mut MessageSender<ClientPlayerState>, With<Connected>>,
) {
    state.send_timer.tick(time.delta());
    if !state.send_timer.just_finished() {
        return;
    }

    let Some(client_entity) = state.client_entity else { return };
    if !state.received_local_snapshot || state.awaiting_respawn_snapshot {
        return;
    }

    let Some((player_transform, health, body)) = players.iter().next() else { return };
    if !health.is_alive() {
        return;
    }
    let Ok(mut sender) = senders.get_mut(client_entity) else { return };

    sender.send::<PlayerStateChannel>(ClientPlayerState {
        pose: PlayerPose::from_transform_with_shot_target(player_transform, state.shot_sequence, state.shot_target),
        crouching: body.is_crouching(),
    });
}

#[allow(clippy::too_many_arguments)]
fn receive_server_snapshots(
    mut commands: Commands,
    assets: Res<BlobAssets>,
    audio_settings: Res<AudioSettings>,
    mut state: ResMut<NetworkClientState>,
    mut remote_players: ResMut<RemotePlayers>,
    mut receivers: Query<&mut MessageReceiver<ServerSnapshot>, With<Client>>,
    mut remote_query: RemotePlayerSnapshotQuery<'_, '_>,
    mut local_player: LocalPlayerSnapshotQuery<'_, '_>,
) {
    let Some(client_entity) = state.client_entity else { return };
    let Ok(mut receiver) = receivers.get_mut(client_entity) else { return };

    for snapshot in receiver.receive() {
        let mut seen = HashSet::new();
        for player in snapshot.players {
            if player.client_id == state.client_id {
                let force_spawn_sync = !state.received_local_snapshot || state.awaiting_respawn_snapshot;
                apply_local_snapshot(&mut local_player, &player, force_spawn_sync);
                state.received_local_snapshot = true;
                state.awaiting_respawn_snapshot = false;
                continue;
            }

            seen.insert(player.client_id);
            let target_transform = player.pose.to_transform();
            let entities = remote_players.entities.entry(player.client_id).or_insert_with(|| {
                spawn_remote_player(
                    &mut commands,
                    &assets,
                    player.client_id,
                    player.name.clone(),
                    target_transform,
                    player.alive,
                )
            });

            let Ok((mut transform, mut visibility, mut remote_player)) = remote_query.get_mut(entities.body) else {
                continue;
            };
            let was_alive = remote_player.alive;
            queue_remote_player_transform(&mut transform, &mut remote_player, target_transform, player.alive);
            remote_player.name = player.name;
            remote_player.alive = player.alive;
            remote_player.crouching = player.crouching;
            if player.alive {
                remote_player.visual_alpha = 1.0;
                remote_player.target_visual_alpha = 1.0;
                *visibility = Visibility::Inherited;
                commands.entity(entities.body).remove::<ColliderDisabled>();
            } else {
                remote_player.target_visual_alpha = 0.0;
                *visibility = if was_alive || remote_player.visual_alpha > REMOTE_PLAYER_FADE_EPSILON {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                commands.entity(entities.body).insert(ColliderDisabled);
            }

            if player.alive && player.pose.shot_sequence > remote_player.last_shot_sequence {
                spawn_bullet_from_transform(
                    &mut commands,
                    &assets,
                    &audio_settings,
                    &target_transform,
                    player.pose.shot_target_vec(),
                    Some(player.client_id),
                );
                remote_player.last_shot_sequence = player.pose.shot_sequence;
            }
        }

        let stale_players =
            remote_players.entities.keys().copied().filter(|client_id| !seen.contains(client_id)).collect::<Vec<_>>();
        for client_id in stale_players {
            if let Some(entities) = remote_players.entities.remove(&client_id) {
                commands.entity(entities.body).despawn();
                commands.entity(entities.name_tag).despawn();
            }
        }
    }
}

fn apply_local_snapshot(
    local_player: &mut LocalPlayerSnapshotQuery<'_, '_>,
    snapshot: &golab_shared::RemotePlayerState,
    force_spawn_sync: bool,
) {
    let Some((mut transform, mut body, mut health)) = local_player.iter_mut().next() else { return };
    let was_alive = health.is_alive();
    health.set_from_server(snapshot.health, snapshot.alive);
    if snapshot.alive && (force_spawn_sync || !was_alive) {
        respawn_player_at(&mut transform, &mut body, &mut health, Vec3::from_array(snapshot.pose.translation));
        health.set_from_server(snapshot.health, snapshot.alive);
    }
}

fn spawn_remote_player(
    commands: &mut Commands,
    assets: &BlobAssets,
    client_id: u64,
    name: String,
    transform: Transform,
    alive: bool,
) -> RemotePlayerEntities {
    let visual_alpha = if alive { 1.0 } else { 0.0 };
    let body = commands
        .spawn((
            RemotePlayer {
                name: name.clone(),
                alive,
                crouching: false,
                visual_alpha,
                target_visual_alpha: visual_alpha,
                interpolation_start: transform,
                target_transform: transform,
                interpolation_elapsed: REMOTE_PLAYER_INTERPOLATION_SECONDS,
                last_shot_sequence: 0,
            },
            BulletHitbox::player(client_id),
            RigidBody::Kinematic,
            build_player_hitbox_collider(),
            CollisionLayers::from_bits(PLAYER_HITBOX_LAYER, PLAYER_HITBOX_LAYER),
            transform,
            if alive { Visibility::Inherited } else { Visibility::Hidden },
            Name::new(format!("Remote Player {client_id}")),
            children![(
                SceneRoot(assets.player_scene.clone()),
                player_model_transform(),
                RenderLayers::layer(DEFAULT_RENDER_LAYER),
            )],
        ))
        .id();

    if !alive {
        commands.entity(body).insert(ColliderDisabled);
    }

    let name_tag = commands
        .spawn((
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                width: Px(name_tag_width(&name)),
                height: Px(NAME_TAG_HEIGHT),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(Px(8.0), Px(3.0)),
                border_radius: BorderRadius::all(Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, NAME_TAG_BACKGROUND_ALPHA)),
            Visibility::Hidden,
            GameplayHud,
        ))
        .id();
    let text = commands
        .spawn((
            Text::new(name),
            TextFont { font_size: 13.0, ..default() },
            TextColor(Color::srgba(1.0, 1.0, 1.0, NAME_TAG_TEXT_ALPHA)),
            TextShadow { offset: Vec2::new(1.0, 1.0), color: Color::srgba(0.0, 0.0, 0.0, NAME_TAG_SHADOW_ALPHA) },
            RemoteNameTagText,
        ))
        .id();
    commands.entity(name_tag).add_child(text).insert(RemoteNameTag { player: body, text });

    RemotePlayerEntities { body, name_tag }
}

fn queue_remote_player_transform(
    transform: &mut Transform,
    remote_player: &mut RemotePlayer,
    target_transform: Transform,
    target_alive: bool,
) {
    let should_snap = !remote_player.alive
        || !target_alive
        || transform.translation.distance_squared(target_transform.translation)
            >= REMOTE_PLAYER_SNAP_DISTANCE * REMOTE_PLAYER_SNAP_DISTANCE;

    if should_snap {
        *transform = target_transform;
        remote_player.interpolation_start = target_transform;
        remote_player.target_transform = target_transform;
        remote_player.interpolation_elapsed = REMOTE_PLAYER_INTERPOLATION_SECONDS;
        return;
    }

    remote_player.interpolation_start = *transform;
    remote_player.target_transform = target_transform;
    remote_player.interpolation_elapsed = 0.0;
}

fn smooth_remote_player_transforms(time: Res<Time>, mut remote_players: Query<(&mut Transform, &mut RemotePlayer)>) {
    for (mut transform, mut remote_player) in &mut remote_players {
        if remote_player.interpolation_elapsed >= REMOTE_PLAYER_INTERPOLATION_SECONDS {
            continue;
        }

        remote_player.interpolation_elapsed =
            (remote_player.interpolation_elapsed + time.delta_secs()).min(REMOTE_PLAYER_INTERPOLATION_SECONDS);
        let alpha = (remote_player.interpolation_elapsed / REMOTE_PLAYER_INTERPOLATION_SECONDS).clamp(0.0, 1.0);

        transform.translation =
            remote_player.interpolation_start.translation.lerp(remote_player.target_transform.translation, alpha);
        transform.rotation =
            remote_player.interpolation_start.rotation.slerp(remote_player.target_transform.rotation, alpha);
    }
}

#[allow(clippy::type_complexity)]
fn update_remote_player_fades(
    mut commands: Commands,
    time: Res<Time>,
    mut remote_players: Query<(Entity, &mut RemotePlayer, &mut Visibility)>,
    children: Query<&Children>,
    mut mesh_materials: Query<(Entity, &mut MeshMaterial3d<StandardMaterial>, Option<&RemotePlayerFadeMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let fade_step = 1.0 - (-REMOTE_PLAYER_FADE_SPEED * time.delta_secs()).exp();

    for (body, mut remote_player, mut visibility) in &mut remote_players {
        remote_player.visual_alpha = (remote_player.target_visual_alpha - remote_player.visual_alpha)
            .mul_add(fade_step, remote_player.visual_alpha);
        if (remote_player.target_visual_alpha - remote_player.visual_alpha).abs() <= REMOTE_PLAYER_FADE_EPSILON {
            remote_player.visual_alpha = remote_player.target_visual_alpha;
        }

        let visible = remote_player.alive || remote_player.visual_alpha > REMOTE_PLAYER_FADE_EPSILON;
        *visibility = if visible { Visibility::Inherited } else { Visibility::Hidden };

        for descendant in children.iter_descendants::<Children>(body) {
            let Ok((mesh_entity, mut mesh_material, fade_material)) = mesh_materials.get_mut(descendant) else {
                continue;
            };

            if fade_material.is_none() && remote_player.visual_alpha >= 0.99 {
                continue;
            }

            let fade_material = if let Some(fade_material) = fade_material {
                *fade_material
            } else {
                let Some(original_material) = materials.get(&mesh_material.0).cloned() else {
                    continue;
                };
                let fade_material = RemotePlayerFadeMaterial {
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

            material.base_color.set_alpha(fade_material.original_alpha * remote_player.visual_alpha);
            material.alpha_mode =
                if remote_player.visual_alpha >= 0.99 { fade_material.original_alpha_mode } else { AlphaMode::Blend };
        }
    }
}

fn update_remote_name_tags(
    camera_query: Query<(&Camera, &GlobalTransform), With<MainPlayerCamera>>,
    remote_players: Query<(&RemotePlayer, &Transform)>,
    mut name_tags: RemoteNameTagQuery<'_, '_>,
    mut name_texts: RemoteNameTagTextQuery<'_, '_>,
) {
    let Some((camera, camera_transform)) = camera_query.iter().next() else {
        return;
    };

    for (name_tag, mut node, mut visibility, mut background) in &mut name_tags {
        let Ok((remote_player, player_transform)) = remote_players.get(name_tag.player) else {
            node.display = Display::None;
            *visibility = Visibility::Hidden;
            continue;
        };

        if remote_player.visual_alpha <= REMOTE_PLAYER_FADE_EPSILON || (remote_player.alive && remote_player.crouching)
        {
            node.display = Display::None;
            *visibility = Visibility::Hidden;
            continue;
        }

        let world_position = player_transform.translation + NAME_TAG_WORLD_OFFSET;
        let Ok(viewport_position) = camera.world_to_viewport(camera_transform, world_position) else {
            node.display = Display::None;
            *visibility = Visibility::Hidden;
            continue;
        };

        let width = name_tag_width(&remote_player.name);
        node.display = Display::Flex;
        node.width = Px(width);
        node.left = Px(width.mul_add(-0.5, viewport_position.x));
        node.top = Px(viewport_position.y - NAME_TAG_HEIGHT - NAME_TAG_SCREEN_OFFSET);
        *visibility = Visibility::Inherited;
        background.0.set_alpha(NAME_TAG_BACKGROUND_ALPHA * remote_player.visual_alpha);

        if let Ok((mut text, mut text_color, mut text_shadow)) = name_texts.get_mut(name_tag.text) {
            if text.as_str() != remote_player.name {
                text.clear();
                text.push_str(&remote_player.name);
            }

            text_color.0.set_alpha(NAME_TAG_TEXT_ALPHA * remote_player.visual_alpha);
            text_shadow.color.set_alpha(NAME_TAG_SHADOW_ALPHA * remote_player.visual_alpha);
        }
    }
}

fn name_tag_width(name: &str) -> f32 {
    let char_count = u16::try_from(name.chars().count()).unwrap_or(NAME_TAG_MAX_CHARS_FOR_WIDTH);
    f32::from(char_count.min(NAME_TAG_MAX_CHARS_FOR_WIDTH))
        .mul_add(NAME_TAG_AVERAGE_GLYPH_WIDTH, NAME_TAG_HORIZONTAL_PADDING)
        .clamp(NAME_TAG_MIN_WIDTH, NAME_TAG_MAX_WIDTH)
}

fn generate_client_id() -> u64 {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let nanos = duration.as_nanos();
    let folded_time = (nanos as u64) ^ ((nanos >> u64::BITS) as u64);
    folded_time ^ u64::from(std::process::id())
}
