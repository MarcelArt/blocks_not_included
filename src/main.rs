use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin};
use blocks_not_included::debug_panel::DebugPanelPlugin;
use blocks_not_included::input_policy::InputPolicyPlugin;
use blocks_not_included::overlay::OverlayPlugin;
use blocks_not_included::player::PlayerPlugin;
use blocks_not_included::readout::ReadoutPlugin;
use blocks_not_included::render::{ChunkMeshPlugin, ScenePlugin};
use blocks_not_included::session::{OpenWorld, SessionPlugin, WorldStart};
use blocks_not_included::sim_bridge::{SimBridgePlugin, SimStart};
use blocks_not_included::sim_files::SimFilesPlugin;
use blocks_not_included::tools::GodToolsPlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            SessionPlugin,
            InputPolicyPlugin,
            SimBridgePlugin,
            SimFilesPlugin,
            ChunkMeshPlugin,
            ScenePlugin,
            PlayerPlugin,
        ))
        .add_plugins((
            EguiPlugin::default(),
            GodToolsPlugin,
            OverlayPlugin,
            ReadoutPlugin,
            DebugPanelPlugin,
        ))
        // Each World's camera holds egui's primary context (`crate::player`): egui would give it
        // only to the first camera ever spawned.
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..default()
        })
        // Until the main menu (step 19), a new sandbox World opens straight away.
        .add_systems(Startup, |mut commands: Commands| {
            commands.queue(OpenWorld(WorldStart {
                sim: SimStart::sandbox(),
                player: None,
            }));
        })
        .run();
}
