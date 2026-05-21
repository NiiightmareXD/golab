use std::time::Duration;

use bevy::anti_alias::fxaa::Fxaa;
use bevy::anti_alias::smaa::Smaa;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap, ShadowFilteringMethod};
use bevy::pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::camera::{MipBias, TemporalJitter};
use bevy::render::view::Msaa;
use bevy::window::{
    Monitor, MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, VideoModeSelection, WindowMode,
};
use bevy::winit::{UpdateMode, WinitSettings};

use crate::player::ViewModelLight;

pub const DEFAULT_MOUSE_SENSITIVITY: f32 = 0.0009;
pub const MIN_MOUSE_SENSITIVITY: f32 = 0.0002;
pub const MAX_MOUSE_SENSITIVITY: f32 = 0.003;
pub const DEFAULT_AUDIO_VOLUME: f32 = 0.5;
pub const MIN_AUDIO_VOLUME: f32 = 0.0;
pub const MAX_AUDIO_VOLUME: f32 = 1.0;

#[derive(Resource)]
pub struct MouseSettings {
    pub sensitivity: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self { sensitivity: DEFAULT_MOUSE_SENSITIVITY }
    }
}

impl MouseSettings {
    pub const fn reset_sensitivity(&mut self) {
        self.sensitivity = DEFAULT_MOUSE_SENSITIVITY;
    }

    pub fn set_sensitivity_normalized(&mut self, normalized: f32) {
        self.sensitivity =
            normalized.clamp(0.0, 1.0).mul_add(MAX_MOUSE_SENSITIVITY - MIN_MOUSE_SENSITIVITY, MIN_MOUSE_SENSITIVITY);
    }

    pub fn sensitivity_normalized(&self) -> f32 {
        ((self.sensitivity - MIN_MOUSE_SENSITIVITY) / (MAX_MOUSE_SENSITIVITY - MIN_MOUSE_SENSITIVITY)).clamp(0.0, 1.0)
    }

    pub fn display_multiplier(&self) -> f32 {
        self.sensitivity / DEFAULT_MOUSE_SENSITIVITY
    }
}

#[derive(Resource)]
pub struct AudioSettings {
    pub volume: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { volume: DEFAULT_AUDIO_VOLUME }
    }
}

impl AudioSettings {
    pub const fn set_volume_normalized(&mut self, normalized: f32) {
        self.volume = normalized.clamp(MIN_AUDIO_VOLUME, MAX_AUDIO_VOLUME);
    }

    pub const fn volume_normalized(&self) -> f32 {
        self.volume.clamp(MIN_AUDIO_VOLUME, MAX_AUDIO_VOLUME)
    }
}

#[derive(Resource, Clone, Copy)]
pub struct GraphicsSettings {
    pub window_mode: WindowModeSetting,
    pub present_mode: PresentModeSetting,
    pub background_fps: BackgroundFpsSetting,
    pub anti_aliasing: AntiAliasingSetting,
    pub msaa: MsaaSetting,
    pub shadow_quality: ShadowQualitySetting,
    pub shadow_filtering: ShadowFilteringSetting,
    pub ambient_occlusion: AmbientOcclusionSetting,
    pub motion_blur: MotionBlurSetting,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            window_mode: WindowModeSetting::BorderlessFullscreen,
            present_mode: PresentModeSetting::Fifo,
            background_fps: BackgroundFpsSetting::Fps15,
            anti_aliasing: AntiAliasingSetting::Taa,
            msaa: MsaaSetting::Off,
            shadow_quality: ShadowQualitySetting::High,
            shadow_filtering: ShadowFilteringSetting::Gaussian,
            ambient_occlusion: AmbientOcclusionSetting::Ultra,
            motion_blur: MotionBlurSetting::High,
        }
    }
}

impl GraphicsSettings {
    pub fn cycle(&mut self, option: GraphicsOptionKind) {
        match option {
            GraphicsOptionKind::WindowMode => self.window_mode = self.window_mode.next(),
            GraphicsOptionKind::PresentMode => self.present_mode = self.present_mode.next(),
            GraphicsOptionKind::BackgroundFps => self.background_fps = self.background_fps.next(),
            GraphicsOptionKind::AntiAliasing => {
                self.anti_aliasing = self.anti_aliasing.next();
                if self.anti_aliasing == AntiAliasingSetting::Taa {
                    self.msaa = MsaaSetting::Off;
                }
            }
            GraphicsOptionKind::Msaa => {
                self.msaa = self.msaa.next();
                if self.msaa != MsaaSetting::Off {
                    self.anti_aliasing = self.anti_aliasing.without_taa();
                    self.ambient_occlusion = AmbientOcclusionSetting::Off;
                }
            }
            GraphicsOptionKind::ShadowQuality => self.shadow_quality = self.shadow_quality.next(),
            GraphicsOptionKind::ShadowFiltering => self.shadow_filtering = self.shadow_filtering.next(),
            GraphicsOptionKind::AmbientOcclusion => {
                self.ambient_occlusion = self.ambient_occlusion.next();
                if self.ambient_occlusion != AmbientOcclusionSetting::Off {
                    self.msaa = MsaaSetting::Off;
                }
            }
            GraphicsOptionKind::MotionBlur => self.motion_blur = self.motion_blur.next(),
        }
    }

    pub const fn value_label(&self, option: GraphicsOptionKind) -> &'static str {
        match option {
            GraphicsOptionKind::WindowMode => self.window_mode.label(),
            GraphicsOptionKind::PresentMode => self.present_mode.label(),
            GraphicsOptionKind::BackgroundFps => self.background_fps.label(),
            GraphicsOptionKind::AntiAliasing => self.anti_aliasing.label(),
            GraphicsOptionKind::Msaa => self.msaa.label(),
            GraphicsOptionKind::ShadowQuality => self.shadow_quality.label(),
            GraphicsOptionKind::ShadowFiltering => self.shadow_filtering.label(),
            GraphicsOptionKind::AmbientOcclusion => self.ambient_occlusion.label(),
            GraphicsOptionKind::MotionBlur => self.motion_blur.label(),
        }
    }

    fn bevy_window_mode(self, primary_monitor: Option<(Entity, &Monitor)>) -> WindowMode {
        match self.window_mode {
            WindowModeSetting::Windowed => WindowMode::Windowed,
            WindowModeSetting::BorderlessFullscreen => {
                WindowMode::BorderlessFullscreen(Self::primary_monitor_selection(primary_monitor))
            }
            WindowModeSetting::Fullscreen => WindowMode::Fullscreen(
                Self::primary_monitor_selection(primary_monitor),
                Self::fullscreen_video_mode(primary_monitor.map(|(_, monitor)| monitor)),
            ),
        }
    }

    fn apply_window(self, window: &mut Window, primary_monitor: Option<(Entity, &Monitor)>) {
        window.mode = self.bevy_window_mode(primary_monitor);
        window.present_mode = self.present_mode.bevy_present_mode();
    }

    fn primary_monitor_selection(primary_monitor: Option<(Entity, &Monitor)>) -> MonitorSelection {
        primary_monitor.map_or(MonitorSelection::Primary, |(entity, _)| MonitorSelection::Entity(entity))
    }

    fn fullscreen_video_mode(monitor: Option<&Monitor>) -> VideoModeSelection {
        let Some(monitor) = monitor else { return VideoModeSelection::Current };
        let native_size = monitor.physical_size();

        monitor
            .video_modes
            .iter()
            .copied()
            .filter(|mode| mode.physical_size == native_size)
            .max_by_key(|mode| (mode.refresh_rate_millihertz, mode.bit_depth))
            .or_else(|| {
                monitor.video_modes.iter().copied().max_by_key(|mode| {
                    let area = u64::from(mode.physical_size.x) * u64::from(mode.physical_size.y);
                    (area, mode.physical_size.x, mode.physical_size.y, mode.refresh_rate_millihertz, mode.bit_depth)
                })
            })
            .map_or(VideoModeSelection::Current, VideoModeSelection::Specific)
    }

    fn apply_winit(self, winit_settings: &mut WinitSettings) {
        winit_settings.focused_mode = UpdateMode::Continuous;
        winit_settings.unfocused_mode = self.background_fps.update_mode();
    }

    fn needs_depth_prepass(self) -> bool {
        self.anti_aliasing == AntiAliasingSetting::Taa
            || self.ambient_occlusion != AmbientOcclusionSetting::Off
            || self.motion_blur != MotionBlurSetting::Off
    }

    fn needs_normal_prepass(self) -> bool {
        self.ambient_occlusion != AmbientOcclusionSetting::Off
    }

    fn needs_motion_vector_prepass(self) -> bool {
        self.anti_aliasing == AntiAliasingSetting::Taa || self.motion_blur != MotionBlurSetting::Off
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphicsOptionKind {
    WindowMode,
    PresentMode,
    BackgroundFps,
    AntiAliasing,
    Msaa,
    ShadowQuality,
    ShadowFiltering,
    AmbientOcclusion,
    MotionBlur,
}

impl GraphicsOptionKind {
    pub const ALL: [Self; 9] = [
        Self::WindowMode,
        Self::PresentMode,
        Self::BackgroundFps,
        Self::AntiAliasing,
        Self::Msaa,
        Self::ShadowQuality,
        Self::ShadowFiltering,
        Self::AmbientOcclusion,
        Self::MotionBlur,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::WindowMode => "Window mode",
            Self::PresentMode => "Present mode",
            Self::BackgroundFps => "Background FPS",
            Self::AntiAliasing => "Anti-aliasing",
            Self::Msaa => "MSAA",
            Self::ShadowQuality => "Shadow quality",
            Self::ShadowFiltering => "Shadow filtering",
            Self::AmbientOcclusion => "Ambient occlusion",
            Self::MotionBlur => "Motion blur",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WindowModeSetting {
    Windowed,
    BorderlessFullscreen,
    Fullscreen,
}

impl WindowModeSetting {
    const fn next(self) -> Self {
        match self {
            Self::Windowed => Self::BorderlessFullscreen,
            Self::BorderlessFullscreen => Self::Fullscreen,
            Self::Fullscreen => Self::Windowed,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Windowed => "Windowed",
            Self::BorderlessFullscreen => "Borderless",
            Self::Fullscreen => "Fullscreen",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresentModeSetting {
    AutoVsync,
    AutoNoVsync,
    Fifo,
}

impl PresentModeSetting {
    const fn next(self) -> Self {
        match self {
            Self::AutoVsync => Self::AutoNoVsync,
            Self::AutoNoVsync => Self::Fifo,
            Self::Fifo => Self::AutoVsync,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::AutoVsync => "Auto VSync",
            Self::AutoNoVsync => "Auto No VSync",
            Self::Fifo => "VSync",
        }
    }

    const fn bevy_present_mode(self) -> PresentMode {
        match self {
            Self::AutoVsync => PresentMode::AutoVsync,
            Self::AutoNoVsync => PresentMode::AutoNoVsync,
            Self::Fifo => PresentMode::Fifo,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackgroundFpsSetting {
    Fps15,
    Fps30,
    Fps60,
    Unlimited,
}

impl BackgroundFpsSetting {
    const fn next(self) -> Self {
        match self {
            Self::Fps15 => Self::Fps30,
            Self::Fps30 => Self::Fps60,
            Self::Fps60 => Self::Unlimited,
            Self::Unlimited => Self::Fps15,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Fps15 => "15",
            Self::Fps30 => "30",
            Self::Fps60 => "60",
            Self::Unlimited => "Unlimited",
        }
    }

    fn update_mode(self) -> UpdateMode {
        match self {
            Self::Fps15 => UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 15.0)),
            Self::Fps30 => UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 30.0)),
            Self::Fps60 => UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 60.0)),
            Self::Unlimited => UpdateMode::Continuous,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AntiAliasingSetting {
    Off,
    Fxaa,
    Smaa,
    Taa,
}

impl AntiAliasingSetting {
    const fn next(self) -> Self {
        match self {
            Self::Off => Self::Fxaa,
            Self::Fxaa => Self::Smaa,
            Self::Smaa => Self::Taa,
            Self::Taa => Self::Off,
        }
    }

    const fn without_taa(self) -> Self {
        match self {
            Self::Taa => Self::Off,
            setting => setting,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
            Self::Taa => "TAA",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MsaaSetting {
    Off,
    Sample2,
    Sample4,
    Sample8,
}

impl MsaaSetting {
    const fn next(self) -> Self {
        match self {
            Self::Off => Self::Sample2,
            Self::Sample2 => Self::Sample4,
            Self::Sample4 => Self::Sample8,
            Self::Sample8 => Self::Off,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Sample2 => "2x",
            Self::Sample4 => "4x",
            Self::Sample8 => "8x",
        }
    }

    const fn bevy_msaa(self) -> Msaa {
        match self {
            Self::Off => Msaa::Off,
            Self::Sample2 => Msaa::Sample2,
            Self::Sample4 => Msaa::Sample4,
            Self::Sample8 => Msaa::Sample8,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShadowQualitySetting {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl ShadowQualitySetting {
    const fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Ultra,
            Self::Ultra => Self::Off,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    const fn shadow_map_size(self) -> usize {
        match self {
            Self::Off => 512,
            Self::Low => 1024,
            Self::Medium | Self::High => 2048,
            Self::Ultra => 4096,
        }
    }

    fn cascade_config(self) -> CascadeShadowConfig {
        let (num_cascades, maximum_distance, first_cascade_far_bound) = match self {
            Self::Off | Self::Low => (1, 45.0, 45.0),
            Self::Medium => (2, 90.0, 18.0),
            Self::High => (4, 150.0, 10.0),
            Self::Ultra => (4, 220.0, 12.0),
        };

        CascadeShadowConfigBuilder {
            num_cascades,
            minimum_distance: 0.1,
            maximum_distance,
            first_cascade_far_bound,
            overlap_proportion: 0.2,
        }
        .build()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShadowFilteringSetting {
    Hardware2x2,
    Gaussian,
    Temporal,
}

impl ShadowFilteringSetting {
    const fn next(self) -> Self {
        match self {
            Self::Hardware2x2 => Self::Gaussian,
            Self::Gaussian => Self::Temporal,
            Self::Temporal => Self::Hardware2x2,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Hardware2x2 => "Hardware 2x2",
            Self::Gaussian => "Gaussian",
            Self::Temporal => "Temporal",
        }
    }

    const fn bevy_shadow_filtering(self) -> ShadowFilteringMethod {
        match self {
            Self::Hardware2x2 => ShadowFilteringMethod::Hardware2x2,
            Self::Gaussian => ShadowFilteringMethod::Gaussian,
            Self::Temporal => ShadowFilteringMethod::Temporal,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AmbientOcclusionSetting {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl AmbientOcclusionSetting {
    const fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Ultra,
            Self::Ultra => Self::Off,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    const fn quality(self) -> Option<ScreenSpaceAmbientOcclusionQualityLevel> {
        match self {
            Self::Off => None,
            Self::Low => Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
            Self::Medium => Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
            Self::High => Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
            Self::Ultra => Some(ScreenSpaceAmbientOcclusionQualityLevel::Ultra),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MotionBlurSetting {
    Off,
    Low,
    Medium,
    High,
}

impl MotionBlurSetting {
    const fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    const fn motion_blur(self) -> Option<MotionBlur> {
        match self {
            Self::Off => None,
            Self::Low => Some(MotionBlur { shutter_angle: 0.25, samples: 1 }),
            Self::Medium => Some(MotionBlur { shutter_angle: 0.5, samples: 2 }),
            Self::High => Some(MotionBlur { shutter_angle: 0.75, samples: 4 }),
        }
    }
}

pub fn apply_graphics_settings(
    mut commands: Commands,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut winit_settings: ResMut<WinitSettings>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut lights: Query<(&mut DirectionalLight, &mut CascadeShadowConfig), Without<ViewModelLight>>,
    cameras: Query<Entity, With<Camera3d>>,
    primary_monitors: Query<(Entity, &Monitor), With<PrimaryMonitor>>,
) {
    if !settings.is_changed() {
        return;
    }

    let primary_monitor = primary_monitors.iter().next();
    for mut window in &mut windows {
        settings.apply_window(&mut window, primary_monitor);
    }

    settings.apply_winit(&mut winit_settings);
    shadow_map.size = settings.shadow_quality.shadow_map_size();

    let shadows_enabled = settings.shadow_quality != ShadowQualitySetting::Off;
    let cascade_config = settings.shadow_quality.cascade_config();
    for (mut light, mut cascade) in &mut lights {
        light.shadows_enabled = shadows_enabled;
        *cascade = cascade_config.clone();
    }

    for entity in &cameras {
        let mut camera = commands.entity(entity);
        camera.remove::<(
            Fxaa,
            Smaa,
            TemporalAntiAliasing,
            ScreenSpaceAmbientOcclusion,
            MotionBlur,
            ShadowFilteringMethod,
        )>();

        if settings.anti_aliasing != AntiAliasingSetting::Taa {
            camera.remove::<(TemporalJitter, MipBias)>();
        }
        if !settings.needs_depth_prepass() {
            camera.remove::<DepthPrepass>();
        }
        if !settings.needs_normal_prepass() {
            camera.remove::<NormalPrepass>();
        }
        if !settings.needs_motion_vector_prepass() {
            camera.remove::<MotionVectorPrepass>();
        }

        camera.insert((settings.msaa.bevy_msaa(), settings.shadow_filtering.bevy_shadow_filtering()));

        match settings.anti_aliasing {
            AntiAliasingSetting::Off => {}
            AntiAliasingSetting::Fxaa => {
                camera.insert(Fxaa::default());
            }
            AntiAliasingSetting::Smaa => {
                camera.insert(Smaa::default());
            }
            AntiAliasingSetting::Taa => {
                camera.insert(TemporalAntiAliasing::default());
            }
        }

        if let Some(quality_level) = settings.ambient_occlusion.quality() {
            camera.insert(ScreenSpaceAmbientOcclusion { quality_level, ..default() });
        }

        if let Some(motion_blur) = settings.motion_blur.motion_blur() {
            camera.insert(motion_blur);
        }
    }
}
