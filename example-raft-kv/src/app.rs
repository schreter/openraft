use std::sync::Arc;

use openraft::Config;

use crate::ExampleRaft;
use crate::ExampleStore;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct ExampleApp {
    pub id: C::NodeId,
    pub addr: String,
    pub raft: ExampleRaft,
    pub store: Arc<ExampleStore>,
    pub config: Arc<Config>,
}
