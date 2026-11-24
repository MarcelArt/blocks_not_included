use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use blocks_not_included::overlay::OverlayPlugin;
use blocks_not_included::player::PlayerPlugin;
use blocks_not_included::readout::ReadoutPlugin;
use blocks_not_included::render::{ChunkMeshPlugin, ScenePlugin};
use blocks_not_included::sim_bridge::SimBridgePlugin;
use blocks_not_included::tools::GodToolsPlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            SimBridgePlugin::sandbox(),
            ChunkMeshPlugin,
            ScenePlugin,
            PlayerPlugin,
            EguiPlugin::default(),
            GodToolsPlugin,
            OverlayPlugin,
            ReadoutPlugin,
        ))
        .run();
}
