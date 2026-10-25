/// Marks a node spawned while the scene was already running (a missile, a
/// flare, a respawned aircraft) rather than while it was being built - see
/// `Scene::spawn_node`. The editor's hierarchy hides these unless asked.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedLive;
