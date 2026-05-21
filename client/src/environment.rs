use bevy::core_pipeline::Skybox;
use bevy::image::TextureFormatPixelInfo;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureViewDescriptor, TextureViewDimension};

use crate::assets::BlobAssets;

const SKYBOX_BRIGHTNESS: f32 = 1_000.0;
const SKYBOX_FACE_SIZE: u32 = 1024;

#[derive(Component)]
pub struct EnvironmentCamera;

#[derive(Component)]
pub struct SkyboxApplied;

#[derive(Resource, Default)]
pub struct EnvironmentMapState {
    prepared: bool,
    warned: bool,
}

pub fn prepare_environment_map_cubemap(
    assets: Res<BlobAssets>,
    mut images: ResMut<Assets<Image>>,
    mut state: ResMut<EnvironmentMapState>,
) {
    if state.prepared {
        return;
    }

    let Some(image) = images.get_mut(&assets.environment_map) else {
        return;
    };

    match reinterpret_horizontal_strip_as_cubemap(image) {
        Ok(()) => state.prepared = true,
        Err(reason) => {
            if !state.warned {
                warn!("Cloudy day EXR could not be prepared as a skybox cubemap: {reason}");
                state.warned = true;
            }
        }
    }
}

pub fn apply_skybox(
    mut commands: Commands,
    assets: Res<BlobAssets>,
    state: Res<EnvironmentMapState>,
    cameras: Query<Entity, (With<EnvironmentCamera>, Without<SkyboxApplied>)>,
) {
    if !state.prepared {
        return;
    }

    for entity in &cameras {
        commands.entity(entity).insert((
            SkyboxApplied,
            Skybox { image: assets.environment_map.clone(), brightness: SKYBOX_BRIGHTNESS, ..default() },
        ));
    }
}

fn reinterpret_horizontal_strip_as_cubemap(image: &mut Image) -> Result<(), String> {
    if image.texture_descriptor.size.depth_or_array_layers == 6 {
        image.texture_view_descriptor =
            Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
        return Ok(());
    }

    if image.texture_descriptor.size.depth_or_array_layers != 1 {
        return Err(format!(
            "expected one 2D layer or an existing 6-layer cubemap, found {} layers",
            image.texture_descriptor.size.depth_or_array_layers
        ));
    }

    let width = image.width();
    let height = image.height();
    if width != height * 6 {
        return Err(format!("expected a 6x1 horizontal cubemap strip, found {width}x{height}"));
    }

    let pixel_size = image
        .texture_descriptor
        .format
        .pixel_size()
        .map_err(|err| format!("unsupported texture format {:?}: {err:?}", image.texture_descriptor.format))?;
    let Some(data) = image.data.as_mut() else {
        return Err("image has no CPU pixel data".to_string());
    };

    let source_face_size = height as usize;
    let target_face_size = SKYBOX_FACE_SIZE.min(height) as usize;
    let source_width = width as usize;
    let source_row_bytes = source_width * pixel_size;
    let source_face_row_bytes = source_face_size * pixel_size;
    let expected_len = source_row_bytes * source_face_size;

    if data.len() != expected_len {
        return Err(format!("unexpected data size: expected {expected_len} bytes, found {} bytes", data.len()));
    }

    let source = std::mem::take(data);
    let mut cubemap_data = vec![0; target_face_size * target_face_size * 6 * pixel_size];

    for face in 0..6 {
        for y in 0..target_face_size {
            let source_y = y * source_face_size / target_face_size;
            for x in 0..target_face_size {
                let source_x = x * source_face_size / target_face_size;
                let source_start = source_y * source_row_bytes + face * source_face_row_bytes + source_x * pixel_size;
                let destination_start =
                    (face * target_face_size * target_face_size + y * target_face_size + x) * pixel_size;

                cubemap_data[destination_start..destination_start + pixel_size]
                    .copy_from_slice(&source[source_start..source_start + pixel_size]);
            }
        }
    }

    *data = cubemap_data;
    image.texture_descriptor.size =
        Extent3d { width: target_face_size as u32, height: target_face_size as u32, depth_or_array_layers: 6 };
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });

    Ok(())
}
