use bevy::prelude::*;
use blocks_not_included::sim_bridge::SimBridgePlugin;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, SimBridgePlugin::sandbox()))
        .run();
}
