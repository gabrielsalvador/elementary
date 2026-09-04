//! Shared, named buffers: sample data loaded by the host and the tap buffers
//! that implement feedback between `tapOut` and `tapIn` nodes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A named resource nodes can hold a reference to.
pub enum SharedResource {
    /// Immutable audio data, one `Vec` per channel.
    Audio { channels: Vec<Vec<f32>> },
    /// A single-channel block written by `tapOut` and read by `tapIn`.
    Tap(Mutex<Vec<f32>>),
}

pub type SharedResourcePtr = Arc<SharedResource>;

impl SharedResource {
    pub fn mono(data: &[f32]) -> Self {
        SharedResource::Audio {
            channels: vec![data.to_vec()],
        }
    }

    pub fn multi(channels: &[&[f32]]) -> Self {
        SharedResource::Audio {
            channels: channels.iter().map(|c| c.to_vec()).collect(),
        }
    }

    pub fn tap(block_size: usize) -> Self {
        SharedResource::Tap(Mutex::new(vec![0.0; block_size]))
    }

    pub fn num_channels(&self) -> usize {
        match self {
            SharedResource::Audio { channels } => channels.len(),
            SharedResource::Tap(_) => 1,
        }
    }

    pub fn num_samples(&self) -> usize {
        match self {
            SharedResource::Audio { channels } => channels.first().map(|c| c.len()).unwrap_or(0),
            SharedResource::Tap(buf) => buf.lock().map(|b| b.len()).unwrap_or(0),
        }
    }

    /// Audio channel data. Tap resources expose no immutable channel data.
    pub fn channel(&self, i: usize) -> Option<&[f32]> {
        match self {
            SharedResource::Audio { channels } => channels.get(i).map(|c| c.as_slice()),
            SharedResource::Tap(_) => None,
        }
    }
}

#[derive(Default)]
pub struct SharedResourceMap {
    resources: HashMap<String, SharedResourcePtr>,
}

impl SharedResourceMap {
    /// Returns false if a resource with that name already exists.
    pub fn add(&mut self, name: &str, resource: SharedResource) -> bool {
        if self.resources.contains_key(name) {
            return false;
        }
        self.resources.insert(name.to_string(), Arc::new(resource));
        true
    }

    pub fn has(&self, name: &str) -> bool {
        self.resources.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<SharedResourcePtr> {
        self.resources.get(name).cloned()
    }

    /// Fetch the named resource, creating a tap buffer of `block_size` if absent.
    pub fn get_tap(&mut self, name: &str, block_size: usize) -> SharedResourcePtr {
        self.resources
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(SharedResource::tap(block_size)))
            .clone()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.resources.keys().map(|k| k.as_str())
    }

    /// Drop every resource no node currently references.
    pub fn prune(&mut self) {
        self.resources.retain(|_, r| Arc::strong_count(r) > 1);
    }
}
