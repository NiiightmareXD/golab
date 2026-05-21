use bevy::app::AppExit;
use bevy::diagnostic::{Diagnostic, DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::{CursorGrabMode, CursorOptions};
use golab_shared::PLAYER_MAX_HEALTH;

use crate::assets::BlobAssets;
use crate::constants::{DASH_RECHARGE_SECONDS, MAX_DASH_CHARGES};
use crate::network::{
    ConnectToServer, DisconnectFromServer, NetworkClientState, PlayerNameInput, RespawnPlayer, ServerAddressInput,
};
use crate::player::{Player, PlayerBody, PlayerHealth, RespawnState};
use crate::settings::{AudioSettings, GraphicsOptionKind, GraphicsSettings, MouseSettings};

#[derive(Resource)]
pub struct MenuState {
    open: bool,
    screen: MenuScreen,
    join_field: JoinField,
    next_join_attempt: u64,
    pending_join_attempt: Option<u64>,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            open: true,
            screen: MenuScreen::Main,
            join_field: JoinField::UserName,
            next_join_attempt: 0,
            pending_join_attempt: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuScreen {
    Main,
    JoinServer,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JoinField {
    UserName,
    ServerAddress,
}

#[derive(Component)]
pub struct FpsText;

#[derive(Component)]
pub struct PingText;

#[derive(Component)]
pub struct SensitivityText;

#[derive(Component)]
pub struct AudioVolumeText;

#[derive(Component)]
pub struct GraphicsOptionText {
    kind: GraphicsOptionKind,
}

#[derive(Component)]
pub struct DashText;

#[derive(Component)]
pub struct NetworkStatusText;

#[derive(Component)]
pub struct PlayerNameText;

#[derive(Component)]
pub struct ServerAddressText;

#[derive(Component)]
pub struct JoinErrorText;

#[derive(Component)]
pub struct MainJoinButtonText;

#[derive(Component)]
pub struct JoinSubmitButtonText;

#[derive(Component)]
pub struct RespawnCountdownText;

#[derive(Component)]
pub struct HealthText;

#[derive(Component)]
pub struct DashBarFill;

#[derive(Component)]
pub struct HealthBarFill;

#[derive(Component)]
pub struct GameplayHud;

#[derive(Component)]
pub struct MenuRoot;

#[derive(Component)]
pub struct MenuPanel {
    screen: MenuScreen,
}

#[derive(Component)]
pub struct MenuButton;

#[derive(Component, Clone, Copy)]
pub enum MenuAction {
    Resume,
    OpenJoinServer,
    SubmitJoinServer,
    FocusUserName,
    FocusServerAddress,
    Respawn,
    Settings,
    Back,
    Exit,
    SensitivityReset,
    CycleGraphicsOption(GraphicsOptionKind),
}

#[derive(Component)]
pub struct JoinInputField {
    field: JoinField,
}

#[derive(Component, Clone, Copy)]
pub enum SliderKind {
    Sensitivity,
    AudioVolume,
}

#[derive(Component)]
pub struct SliderTrack {
    kind: SliderKind,
}

#[derive(Component)]
pub struct SliderFill {
    kind: SliderKind,
}

type MenuActionButtons<'w, 's> = Query<'w, 's, (&'static Interaction, &'static MenuAction), Changed<Interaction>>;
type MenuRootNodes<'w, 's> = Query<'w, 's, (&'static mut Node, &'static mut Visibility), With<MenuRoot>>;
type MenuPanelNodes<'w, 's> = Query<
    'w,
    's,
    (&'static MenuPanel, &'static mut Node, &'static mut Visibility),
    (With<MenuPanel>, Without<MenuRoot>),
>;
type GameplayHudNodes<'w, 's> =
    Query<'w, 's, &'static mut Visibility, (With<GameplayHud>, Without<MenuRoot>, Without<MenuPanel>)>;
type MenuVisualButtons<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static mut BackgroundColor), (Changed<Interaction>, With<MenuButton>)>;
type JoinFieldVisuals<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static JoinInputField, &'static mut BackgroundColor, &'static mut BorderColor),
>;
type SliderInteractions<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static RelativeCursorPosition, &'static SliderTrack), With<SliderTrack>>;
type SliderFills<'w, 's> = Query<'w, 's, (&'static SliderFill, &'static mut Node)>;
type GraphicsOptionTexts<'w, 's> = Query<'w, 's, (&'static GraphicsOptionText, &'static mut Text)>;

#[derive(SystemParam)]
pub struct MenuDisplayNodes<'w, 's> {
    roots: MenuRootNodes<'w, 's>,
    panels: MenuPanelNodes<'w, 's>,
    hud: GameplayHudNodes<'w, 's>,
}

pub fn menu_closed(menu: Res<MenuState>) -> bool {
    !menu.open
}

pub fn setup_ui(
    mut commands: Commands,
    settings: Res<MouseSettings>,
    audio_settings: Res<AudioSettings>,
    graphics_settings: Res<GraphicsSettings>,
    assets: Res<BlobAssets>,
) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(50.0),
            width: px(2),
            height: px(18),
            margin: UiRect::left(px(-1)).with_top(px(-9)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
        Visibility::Inherited,
        GameplayHud,
    ));

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(50.0),
            width: px(18),
            height: px(2),
            margin: UiRect::left(px(-9)).with_top(px(-1)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
        Visibility::Inherited,
        GameplayHud,
    ));

    commands
        .spawn((
            Node {
                display: Display::Flex,
                position_type: PositionType::Absolute,
                left: px(12),
                top: px(10),
                column_gap: px(10),
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Inherited,
            GameplayHud,
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("FPS: --"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.86)),
                FpsText,
            ));
            row.spawn((
                Text::new("   Ping: -- ms"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.76)),
                PingText,
            ));
        });

    commands
        .spawn((
            Node {
                display: Display::Flex,
                position_type: PositionType::Absolute,
                right: px(22),
                top: px(18),
                width: px(300),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: px(10),
                ..default()
            },
            Visibility::Inherited,
            GameplayHud,
        ))
        .with_children(|hud| {
            spawn_hud_meter(
                hud,
                assets.heart_icon.clone(),
                "HEALTH",
                format!("{PLAYER_MAX_HEALTH}/{PLAYER_MAX_HEALTH}"),
                Color::srgba(1.0, 0.08, 0.24, 0.95),
                HealthText,
                HealthBarFill,
            );
            spawn_hud_meter(
                hud,
                assets.dash_icon.clone(),
                "DASH",
                format!("{MAX_DASH_CHARGES}/{MAX_DASH_CHARGES}"),
                Color::srgba(0.08, 0.68, 1.0, 0.95),
                DashText,
                DashBarFill,
            );
        });

    commands.spawn((
        Text::new("WASD move | Space jump | Ctrl crouch | Right Click dash | Left Click shoot | Esc menu"),
        TextFont { font_size: 18.0, ..default() },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.82)),
        Node { position_type: PositionType::Absolute, left: px(12), bottom: px(10), ..default() },
        Visibility::Inherited,
        GameplayHud,
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: 38.0, ..default() },
        TextColor(Color::WHITE),
        TextShadow { offset: Vec2::new(2.0, 2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.86) },
        Node {
            display: Display::None,
            position_type: PositionType::Absolute,
            left: percent(0.0),
            right: percent(0.0),
            top: percent(40.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Visibility::Inherited,
        GameplayHud,
        RespawnCountdownText,
    ));

    commands
        .spawn((
            Node {
                display: Display::Flex,
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                bottom: px(0),
                width: percent(100.0),
                height: percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.025, 0.035, 0.78)),
            Visibility::Visible,
            MenuRoot,
        ))
        .with_children(|root| {
            spawn_main_menu(root);
            spawn_join_server_menu(root);
            spawn_settings_menu(root, &settings, &audio_settings, &graphics_settings);
        });
}

fn spawn_main_menu(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::Flex,
                width: px(420),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(px(28)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.96)),
            Visibility::Visible,
            MenuPanel { screen: MenuScreen::Main },
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Golab"),
                TextFont { font_size: 48.0, ..default() },
                TextColor(Color::WHITE),
                Node { align_self: AlignSelf::Center, margin: UiRect::bottom(px(20)), ..default() },
            ));

            panel.spawn((
                Text::new("Play offline immediately, or join a server with a name and address."),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgba(0.86, 0.92, 1.0, 0.88)),
                Node { margin: UiRect::bottom(px(14)), ..default() },
            ));
            panel.spawn((
                Text::new("Network: offline"),
                TextFont { font_size: 20.0, ..default() },
                TextColor(Color::srgb(0.52, 0.84, 1.0)),
                Node { margin: UiRect::bottom(px(12)), ..default() },
                NetworkStatusText,
            ));

            spawn_menu_button(panel, "Play Offline / Resume", MenuAction::Resume, percent(100.0));
            spawn_menu_button_with_marker(
                panel,
                "Join Server",
                MenuAction::OpenJoinServer,
                percent(100.0),
                MainJoinButtonText,
            );
            spawn_menu_button(panel, "Respawn", MenuAction::Respawn, percent(100.0));
            spawn_menu_button(panel, "Settings", MenuAction::Settings, percent(100.0));
            spawn_menu_button(panel, "Exit", MenuAction::Exit, percent(100.0));
        });
}

fn spawn_join_server_menu(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::None,
                width: px(500),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(px(28)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.96)),
            Visibility::Hidden,
            MenuPanel { screen: MenuScreen::JoinServer },
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Join Server"),
                TextFont { font_size: 42.0, ..default() },
                TextColor(Color::WHITE),
                Node { align_self: AlignSelf::Center, margin: UiRect::bottom(px(18)), ..default() },
            ));

            panel.spawn((
                Text::new("Network: offline"),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.52, 0.84, 1.0)),
                Node { margin: UiRect::bottom(px(14)), ..default() },
                NetworkStatusText,
            ));

            spawn_join_field(
                panel,
                "User name",
                "Player",
                MenuAction::FocusUserName,
                JoinField::UserName,
                PlayerNameText,
            );
            spawn_join_field(
                panel,
                "Server address",
                "127.0.0.1:5000",
                MenuAction::FocusServerAddress,
                JoinField::ServerAddress,
                ServerAddressText,
            );

            panel.spawn((
                Text::new(""),
                TextFont { font_size: 15.0, ..default() },
                TextColor(Color::srgba(1.0, 0.55, 0.55, 0.9)),
                Node { min_height: px(24), margin: UiRect::bottom(px(8)), ..default() },
                JoinErrorText,
            ));

            spawn_menu_button_with_marker(
                panel,
                "Join",
                MenuAction::SubmitJoinServer,
                percent(100.0),
                JoinSubmitButtonText,
            );
            spawn_menu_button(panel, "Back", MenuAction::Back, percent(100.0));
        });
}

fn spawn_join_field<Marker: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: &str,
    action: MenuAction,
    field: JoinField,
    marker: Marker,
) {
    parent.spawn((
        Text::new(label),
        TextFont { font_size: 16.0, ..default() },
        TextColor(Color::srgba(0.86, 0.92, 1.0, 0.78)),
        Node { margin: UiRect::bottom(px(5)), ..default() },
    ));

    parent
        .spawn((
            Button,
            Node {
                display: Display::Flex,
                width: percent(100.0),
                height: px(46),
                align_items: AlignItems::Center,
                padding: UiRect::axes(px(12), px(0)),
                border: UiRect::all(px(1)),
                margin: UiRect::bottom(px(12)),
                ..default()
            },
            BackgroundColor(input_normal_color()),
            BorderColor::all(input_border_color(false)),
            JoinInputField { field },
            action,
        ))
        .with_children(|field| {
            field.spawn((Text::new(value), TextFont { font_size: 20.0, ..default() }, TextColor(Color::WHITE), marker));
        });
}

fn spawn_hud_meter<ValueMarker: Component, FillMarker: Component>(
    parent: &mut ChildSpawnerCommands,
    icon: Handle<Image>,
    label: &str,
    value: String,
    fill_color: Color,
    value_marker: ValueMarker,
    fill_marker: FillMarker,
) {
    parent
        .spawn((
            Node {
                display: Display::Flex,
                width: px(270),
                height: px(52),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexEnd,
                column_gap: px(10),
                padding: UiRect::axes(px(10), px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.025, 0.03, 0.04, 0.46)),
        ))
        .with_children(|row| {
            row.spawn((ImageNode::new(icon), Node { width: px(42), height: px(42), flex_shrink: 0.0, ..default() }));

            row.spawn((
                Node {
                    display: Display::Flex,
                    width: px(196),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(4),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|details| {
                details
                    .spawn((
                        Node {
                            display: Display::Flex,
                            width: percent(100.0),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|header| {
                        header.spawn((
                            Text::new(label),
                            TextFont { font_size: 13.0, ..default() },
                            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.72)),
                        ));
                        header.spawn((
                            Text::new(value),
                            TextFont { font_size: 17.0, ..default() },
                            TextColor(Color::WHITE),
                            value_marker,
                        ));
                    });

                details
                    .spawn((
                        Node {
                            width: percent(100.0),
                            height: px(16),
                            border: UiRect::all(px(1)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.02, 0.025, 0.035, 0.72)),
                        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.24)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            Node { width: percent(100.0), height: percent(100.0), ..default() },
                            BackgroundColor(fill_color),
                            fill_marker,
                        ));
                    });
            });
        });
}

fn spawn_settings_menu(
    parent: &mut ChildSpawnerCommands,
    settings: &MouseSettings,
    audio_settings: &AudioSettings,
    graphics_settings: &GraphicsSettings,
) {
    parent
        .spawn((
            Node {
                display: Display::None,
                width: px(720),
                max_height: percent(94.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(px(22)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.1, 0.14, 0.96)),
            Visibility::Hidden,
            MenuPanel { screen: MenuScreen::Settings },
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Settings"),
                TextFont { font_size: 36.0, ..default() },
                TextColor(Color::WHITE),
                Node { align_self: AlignSelf::Center, margin: UiRect::bottom(px(12)), ..default() },
            ));

            spawn_section_heading(panel, "Controls");
            panel.spawn((
                Text::new(sensitivity_label(settings)),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.88, 0.93, 1.0)),
                Node { margin: UiRect::bottom(px(6)), ..default() },
                SensitivityText,
            ));
            spawn_slider(panel, SliderKind::Sensitivity, settings.sensitivity_normalized());
            spawn_menu_button(panel, "Reset Sensitivity", MenuAction::SensitivityReset, percent(100.0));

            spawn_section_heading(panel, "Audio");
            panel.spawn((
                Text::new(audio_volume_label(audio_settings)),
                TextFont { font_size: 18.0, ..default() },
                TextColor(Color::srgb(0.88, 0.93, 1.0)),
                Node { margin: UiRect::bottom(px(6)), ..default() },
                AudioVolumeText,
            ));
            spawn_slider(panel, SliderKind::AudioVolume, audio_settings.volume_normalized());

            spawn_section_heading(panel, "Graphics");
            spawn_graphics_options(panel, graphics_settings);

            spawn_menu_button(panel, "Back", MenuAction::Back, percent(100.0));
        });
}

fn spawn_section_heading(parent: &mut ChildSpawnerCommands, label: &str) {
    parent.spawn((
        Text::new(label),
        TextFont { font_size: 22.0, ..default() },
        TextColor(Color::WHITE),
        Node { margin: UiRect::top(px(8)).with_bottom(px(8)), ..default() },
    ));
}

fn spawn_graphics_options(parent: &mut ChildSpawnerCommands, graphics_settings: &GraphicsSettings) {
    parent
        .spawn((
            Node {
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(10),
                row_gap: px(8),
                margin: UiRect::bottom(px(8)),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|grid| {
            for kind in GraphicsOptionKind::ALL {
                spawn_graphics_option(grid, kind, graphics_settings.value_label(kind));
            }
        });
}

fn spawn_graphics_option(parent: &mut ChildSpawnerCommands, kind: GraphicsOptionKind, value: &str) {
    parent
        .spawn((
            Node {
                display: Display::Flex,
                width: px(333),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|option| {
            option.spawn((
                Text::new(kind.label()),
                TextFont { font_size: 13.0, ..default() },
                TextColor(Color::srgba(0.86, 0.92, 1.0, 0.76)),
            ));

            option
                .spawn((
                    Button,
                    Node {
                        width: percent(100.0),
                        height: px(32),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(px(8), px(0)),
                        ..default()
                    },
                    BackgroundColor(button_normal_color()),
                    MenuButton,
                    MenuAction::CycleGraphicsOption(kind),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new(value),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::WHITE),
                        GraphicsOptionText { kind },
                    ));
                });
        });
}

fn spawn_slider(parent: &mut ChildSpawnerCommands, kind: SliderKind, normalized_value: f32) {
    parent
        .spawn((
            Button,
            Node {
                width: percent(100.0),
                height: px(24),
                align_items: AlignItems::Stretch,
                margin: UiRect::bottom(px(10)),
                ..default()
            },
            BackgroundColor(slider_track_color()),
            RelativeCursorPosition::default(),
            SliderTrack { kind },
        ))
        .with_children(|track| {
            track.spawn((
                Node { width: percent(normalized_value.clamp(0.0, 1.0) * 100.0), height: percent(100.0), ..default() },
                BackgroundColor(slider_fill_color()),
                SliderFill { kind },
            ));
        });
}

fn spawn_menu_button(parent: &mut ChildSpawnerCommands, label: &str, action: MenuAction, width: Val) {
    spawn_menu_button_with_marker(parent, label, action, width, ());
}

fn spawn_menu_button_with_marker<Marker: Bundle>(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: MenuAction,
    width: Val,
    marker: Marker,
) {
    parent
        .spawn((
            Button,
            Node {
                width,
                height: px(48),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                margin: UiRect::top(px(6)).with_bottom(px(6)),
                ..default()
            },
            BackgroundColor(button_normal_color()),
            MenuButton,
            action,
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::WHITE),
                marker,
            ));
        });
}

pub fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<MenuState>,
    mut cursor_options: Query<&mut CursorOptions>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }

    menu.open = true;
    menu.screen = MenuScreen::Main;
    set_cursor_locked(&mut cursor_options, false);
}

pub fn menu_button_actions(
    mut buttons: MenuActionButtons<'_, '_>,
    mut menu: ResMut<MenuState>,
    mut settings: ResMut<MouseSettings>,
    mut graphics_settings: ResMut<GraphicsSettings>,
    mut cursor_options: Query<&mut CursorOptions>,
    mut address_input: ResMut<ServerAddressInput>,
    mut name_input: ResMut<PlayerNameInput>,
    network_state: Res<NetworkClientState>,
    mut connect_writer: MessageWriter<ConnectToServer>,
    mut disconnect_writer: MessageWriter<DisconnectFromServer>,
    mut respawn_writer: MessageWriter<RespawnPlayer>,
    mut app_exit: MessageWriter<AppExit>,
) {
    for (interaction, action) in &mut buttons {
        if !matches!(interaction, Interaction::Pressed) {
            continue;
        }

        match action {
            MenuAction::Resume => {
                menu.open = false;
                menu.screen = MenuScreen::Main;
                set_cursor_locked(&mut cursor_options, true);
            }
            MenuAction::OpenJoinServer => {
                if network_state.is_join_active() {
                    disconnect_writer.write(DisconnectFromServer);
                    menu.pending_join_attempt = None;
                    return;
                }

                menu.open = true;
                menu.screen = MenuScreen::JoinServer;
                menu.join_field = JoinField::UserName;
            }
            MenuAction::SubmitJoinServer => {
                if network_state.is_join_active() {
                    disconnect_writer.write(DisconnectFromServer);
                    menu.pending_join_attempt = None;
                    menu.open = true;
                    menu.screen = MenuScreen::Main;
                    return;
                }
                if menu.pending_join_attempt.is_some() {
                    return;
                }

                let address = address_input.parse();
                let player_name = name_input.parse();
                if let (Some(address), Some(player_name)) = (address, player_name) {
                    menu.next_join_attempt = menu.next_join_attempt.saturating_add(1);
                    let attempt_id = menu.next_join_attempt;
                    menu.pending_join_attempt = Some(attempt_id);
                    connect_writer.write(ConnectToServer { address, player_name, attempt_id });
                }
            }
            MenuAction::FocusUserName => {
                menu.join_field = JoinField::UserName;
            }
            MenuAction::FocusServerAddress => {
                menu.join_field = JoinField::ServerAddress;
            }
            MenuAction::Respawn => {
                respawn_writer.write(RespawnPlayer);
                menu.open = false;
                menu.screen = MenuScreen::Main;
                set_cursor_locked(&mut cursor_options, true);
            }
            MenuAction::Settings => {
                menu.open = true;
                menu.screen = MenuScreen::Settings;
            }
            MenuAction::Back => {
                menu.open = true;
                menu.screen = MenuScreen::Main;
                menu.pending_join_attempt = None;
            }
            MenuAction::Exit => {
                app_exit.write(AppExit::Success);
            }
            MenuAction::SensitivityReset => settings.reset_sensitivity(),
            MenuAction::CycleGraphicsOption(kind) => graphics_settings.cycle(*kind),
        }
    }
}

pub fn sync_join_connection(
    mut menu: ResMut<MenuState>,
    network_state: Res<NetworkClientState>,
    mut cursor_options: Query<&mut CursorOptions>,
) {
    let Some(attempt_id) = menu.pending_join_attempt else {
        return;
    };

    if network_state.ready_attempt_id() == Some(attempt_id) {
        menu.pending_join_attempt = None;
        menu.open = false;
        menu.screen = MenuScreen::Main;
        set_cursor_locked(&mut cursor_options, true);
    } else if network_state.is_failed_attempt(attempt_id) {
        menu.pending_join_attempt = None;
        menu.open = true;
        menu.screen = MenuScreen::JoinServer;
        set_cursor_locked(&mut cursor_options, false);
    }
}

pub fn sync_menu_display(menu: Res<MenuState>, mut ui_nodes: MenuDisplayNodes<'_, '_>) {
    if !menu.is_changed() {
        return;
    }

    for (mut node, mut visibility) in &mut ui_nodes.roots {
        node.display = if menu.open { Display::Flex } else { Display::None };
        *visibility = if menu.open { Visibility::Visible } else { Visibility::Hidden };
    }

    for (panel, mut node, mut visibility) in &mut ui_nodes.panels {
        let active = menu.open && panel.screen == menu.screen;
        node.display = if active { Display::Flex } else { Display::None };
        *visibility = if active { Visibility::Visible } else { Visibility::Hidden };
    }

    for mut visibility in &mut ui_nodes.hud {
        *visibility = if menu.open { Visibility::Hidden } else { Visibility::Inherited };
    }
}

pub fn menu_button_visuals(mut buttons: MenuVisualButtons<'_, '_>) {
    for (interaction, mut background) in &mut buttons {
        background.0 = match interaction {
            Interaction::Pressed => button_pressed_color(),
            Interaction::Hovered => button_hover_color(),
            Interaction::None => button_normal_color(),
        };
    }
}

pub fn update_join_field_visuals(menu: Res<MenuState>, mut fields: JoinFieldVisuals<'_, '_>) {
    for (interaction, field, mut background, mut border) in &mut fields {
        let focused = menu.open && menu.screen == MenuScreen::JoinServer && menu.join_field == field.field;
        background.0 = match (focused, interaction) {
            (true, _) => input_focused_color(),
            (false, Interaction::Hovered | Interaction::Pressed) => input_hover_color(),
            (false, Interaction::None) => input_normal_color(),
        };
        *border = BorderColor::all(input_border_color(focused));
    }
}

pub fn update_fps_text(diagnostics: Res<DiagnosticsStore>, mut fps_text: Query<&mut Text, With<FpsText>>) {
    let Some(fps) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(Diagnostic::smoothed) else {
        return;
    };

    for mut text in &mut fps_text {
        **text = format!("FPS: {fps:.0}");
    }
}

pub fn update_sensitivity_text(settings: Res<MouseSettings>, mut text_query: Query<&mut Text, With<SensitivityText>>) {
    if !settings.is_changed() {
        return;
    }

    for mut text in &mut text_query {
        **text = sensitivity_label(&settings);
    }
}

pub fn update_audio_text(audio_settings: Res<AudioSettings>, mut volume_text: Query<&mut Text, With<AudioVolumeText>>) {
    if !audio_settings.is_changed() {
        return;
    }

    for mut text in &mut volume_text {
        **text = audio_volume_label(&audio_settings);
    }
}

pub fn update_graphics_text(settings: Res<GraphicsSettings>, mut option_text: GraphicsOptionTexts<'_, '_>) {
    if !settings.is_changed() {
        return;
    }

    for (option, mut text) in &mut option_text {
        **text = settings.value_label(option.kind).to_string();
    }
}

pub fn update_ping_text(network_state: Res<NetworkClientState>, mut ping_text: Query<&mut Text, With<PingText>>) {
    if !network_state.is_changed() {
        return;
    }

    let label = network_state.ping_label();
    for mut text in &mut ping_text {
        text.clear();
        text.push_str(&label);
    }
}

pub fn update_network_status_text(
    network_state: Res<NetworkClientState>,
    mut status_text: Query<&mut Text, With<NetworkStatusText>>,
) {
    if !network_state.is_changed() {
        return;
    }

    let label = network_state.status_label();
    for mut text in &mut status_text {
        text.clear();
        text.push_str(&label);
    }
}

pub fn server_address_text_input(
    mut menu: ResMut<MenuState>,
    mut keyboard_input: MessageReader<KeyboardInput>,
    mut address_input: ResMut<ServerAddressInput>,
    mut name_input: ResMut<PlayerNameInput>,
) {
    if !menu.open || menu.screen != MenuScreen::JoinServer {
        return;
    }

    for input in keyboard_input.read() {
        if input.state != ButtonState::Pressed || input.repeat {
            continue;
        }

        match input.key_code {
            KeyCode::Tab => {
                menu.join_field = match menu.join_field {
                    JoinField::UserName => JoinField::ServerAddress,
                    JoinField::ServerAddress => JoinField::UserName,
                };
            }
            KeyCode::Backspace => {
                match menu.join_field {
                    JoinField::UserName => name_input.pop(),
                    JoinField::ServerAddress => address_input.pop(),
                }
                name_input.clear_error();
                address_input.clear_error();
            }
            KeyCode::Delete => {
                match menu.join_field {
                    JoinField::UserName => name_input.clear(),
                    JoinField::ServerAddress => address_input.clear(),
                }
                name_input.clear_error();
                address_input.clear_error();
            }
            _ => {
                if let Some(text) = &input.text {
                    let accepted = accepted_join_text(menu.join_field, text);
                    if !accepted.is_empty() {
                        match menu.join_field {
                            JoinField::UserName => name_input.push_text(&accepted),
                            JoinField::ServerAddress => address_input.push_text(&accepted),
                        }
                        name_input.clear_error();
                        address_input.clear_error();
                    }
                }
            }
        }
    }
}

pub fn update_server_address_text(
    address_input: Res<ServerAddressInput>,
    mut address_text: Query<&mut Text, With<ServerAddressText>>,
) {
    if !address_input.is_changed() {
        return;
    }

    for mut text in &mut address_text {
        text.clear();
        text.push_str(address_input.text());
    }
}

pub fn update_player_name_text(
    name_input: Res<PlayerNameInput>,
    mut name_text: Query<&mut Text, With<PlayerNameText>>,
) {
    if !name_input.is_changed() {
        return;
    }

    for mut text in &mut name_text {
        text.clear();
        text.push_str(name_input.text());
    }
}

pub fn update_join_error_text(
    address_input: Res<ServerAddressInput>,
    name_input: Res<PlayerNameInput>,
    mut error_text: Query<&mut Text, With<JoinErrorText>>,
) {
    if !address_input.is_changed() && !name_input.is_changed() {
        return;
    }

    let label = name_input.error().or_else(|| address_input.error()).unwrap_or_default();
    for mut text in &mut error_text {
        text.clear();
        text.push_str(label);
    }
}

pub fn update_join_button_text(
    menu: Res<MenuState>,
    network_state: Res<NetworkClientState>,
    mut main_join_text: Query<&mut Text, (With<MainJoinButtonText>, Without<JoinSubmitButtonText>)>,
    mut submit_join_text: Query<&mut Text, (With<JoinSubmitButtonText>, Without<MainJoinButtonText>)>,
) {
    let main_label = if network_state.is_join_active() { "Disconnect" } else { "Join Server" };
    for mut text in &mut main_join_text {
        text.clear();
        text.push_str(main_label);
    }

    let submit_label = if network_state.is_join_active() {
        "Disconnect"
    } else if menu.pending_join_attempt.is_some() {
        "Joining..."
    } else {
        "Join"
    };
    for mut text in &mut submit_join_text {
        text.clear();
        text.push_str(submit_label);
    }
}

pub fn slider_interactions(
    mut sliders: SliderInteractions<'_, '_>,
    mut mouse_settings: ResMut<MouseSettings>,
    mut audio_settings: ResMut<AudioSettings>,
) {
    for (interaction, cursor_position, slider) in &mut sliders {
        if !matches!(interaction, Interaction::Pressed) {
            continue;
        }

        let Some(position) = cursor_position.normalized else {
            continue;
        };

        let normalized = (position.x + 0.5).clamp(0.0, 1.0);
        match slider.kind {
            SliderKind::Sensitivity => mouse_settings.set_sensitivity_normalized(normalized),
            SliderKind::AudioVolume => audio_settings.set_volume_normalized(normalized),
        }
    }
}

pub fn update_slider_visuals(
    mouse_settings: Res<MouseSettings>,
    audio_settings: Res<AudioSettings>,
    mut fills: SliderFills<'_, '_>,
) {
    for (fill, mut node) in &mut fills {
        let normalized = match fill.kind {
            SliderKind::Sensitivity => mouse_settings.sensitivity_normalized(),
            SliderKind::AudioVolume => audio_settings.volume_normalized(),
        };
        node.width = percent(normalized * 100.0);
    }
}

pub fn update_respawn_countdown_text(
    respawn_state: Res<RespawnState>,
    mut countdown_text: Query<(&mut Text, &mut Node), With<RespawnCountdownText>>,
) {
    let Some(remaining) = respawn_state.remaining_secs() else {
        for (mut text, mut node) in &mut countdown_text {
            text.clear();
            node.display = Display::None;
        }
        return;
    };

    let seconds = remaining.ceil().max(1.0);
    let label = if remaining > 0.0 { format!("Respawning in {seconds:.0}") } else { "Respawning".to_string() };
    for (mut text, mut node) in &mut countdown_text {
        node.display = Display::Flex;
        text.clear();
        text.push_str(&label);
    }
}

pub fn update_health_text(
    players: Query<&PlayerHealth, With<Player>>,
    mut text_query: Query<&mut Text, With<HealthText>>,
    mut fill_query: Query<&mut Node, With<HealthBarFill>>,
) {
    let Some(health) = players.iter().next() else {
        return;
    };

    let current_health = if health.is_alive() { health.health() } else { 0 };
    #[allow(clippy::cast_precision_loss)]
    let health_percent = current_health as f32 / PLAYER_MAX_HEALTH as f32 * 100.0;
    let label = if health.is_alive() {
        format!("{current_health}/{PLAYER_MAX_HEALTH}")
    } else {
        format!("0/{PLAYER_MAX_HEALTH}")
    };

    for mut text in &mut text_query {
        text.clear();
        text.push_str(&label);
    }

    for mut node in &mut fill_query {
        node.width = percent(health_percent.clamp(0.0, 100.0));
    }
}

pub fn update_dash_text(
    players: Query<&PlayerBody, With<Player>>,
    mut text_query: Query<&mut Text, With<DashText>>,
    mut fill_query: Query<&mut Node, With<DashBarFill>>,
) {
    let Some(body) = players.iter().next() else {
        return;
    };

    let fill_percent = if body.dash_charges() >= MAX_DASH_CHARGES {
        100.0
    } else {
        let charge_percent = f32::from(body.dash_charges()) / f32::from(MAX_DASH_CHARGES);
        let recharge_percent = (DASH_RECHARGE_SECONDS - body.dash_recharge_remaining_secs()) / DASH_RECHARGE_SECONDS;
        ((charge_percent + recharge_percent / f32::from(MAX_DASH_CHARGES)) * 100.0).clamp(0.0, 100.0)
    };

    let label = if body.dash_charges() >= MAX_DASH_CHARGES {
        format!("{}/{}", body.dash_charges(), MAX_DASH_CHARGES)
    } else {
        format!("{}/{}  {:.1}s", body.dash_charges(), MAX_DASH_CHARGES, body.dash_recharge_remaining_secs())
    };

    for mut text in &mut text_query {
        text.clear();
        text.push_str(&label);
    }

    for mut node in &mut fill_query {
        node.width = percent(fill_percent);
    }
}

fn set_cursor_locked(cursor_options: &mut Query<&mut CursorOptions>, locked: bool) {
    for mut cursor_options in cursor_options.iter_mut() {
        cursor_options.visible = !locked;
        cursor_options.grab_mode = if locked { CursorGrabMode::Locked } else { CursorGrabMode::None };
    }
}

fn sensitivity_label(settings: &MouseSettings) -> String {
    format!("Mouse sensitivity: {:.2}x", settings.display_multiplier())
}

fn audio_volume_label(settings: &AudioSettings) -> String {
    format!("Master volume: {:.0}%", settings.volume_normalized() * 100.0)
}

fn accepted_join_text(field: JoinField, text: &str) -> String {
    text.chars()
        .filter(|character| match field {
            JoinField::UserName => character.is_ascii() && !character.is_ascii_control(),
            JoinField::ServerAddress => character.is_ascii_graphic(),
        })
        .collect()
}

const fn button_normal_color() -> Color {
    Color::srgba(0.18, 0.22, 0.32, 0.94)
}

const fn button_hover_color() -> Color {
    Color::srgba(0.28, 0.36, 0.52, 0.98)
}

const fn button_pressed_color() -> Color {
    Color::srgba(0.1, 0.56, 0.82, 1.0)
}

const fn input_normal_color() -> Color {
    Color::srgba(0.06, 0.075, 0.105, 0.96)
}

const fn input_hover_color() -> Color {
    Color::srgba(0.1, 0.13, 0.18, 0.98)
}

const fn input_focused_color() -> Color {
    Color::srgba(0.08, 0.12, 0.17, 1.0)
}

const fn input_border_color(focused: bool) -> Color {
    if focused { Color::srgba(0.35, 0.75, 1.0, 0.96) } else { Color::srgba(1.0, 1.0, 1.0, 0.18) }
}

const fn slider_track_color() -> Color {
    Color::srgba(0.12, 0.15, 0.22, 0.96)
}

const fn slider_fill_color() -> Color {
    Color::srgba(0.1, 0.56, 0.82, 1.0)
}
