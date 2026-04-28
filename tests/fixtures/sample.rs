//! Fixture for testing Rust extraction, in particular that methods inside
//! `impl Trait for Type` blocks are attached to `Type` (not orphaned).

use std::collections::HashMap;

pub trait ChannelSource {
    fn open(&self) -> bool;
    fn close(&mut self);
}

pub struct LocalChannel {
    name: String,
    cache: HashMap<String, String>,
}

impl LocalChannel {
    pub fn new(name: String) -> Self {
        Self {
            name,
            cache: HashMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl ChannelSource for LocalChannel {
    fn open(&self) -> bool {
        !self.name.is_empty()
    }

    fn close(&mut self) {
        self.cache.clear();
    }
}

impl<T> std::fmt::Debug for LocalChannel
where
    T: Sized,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LocalChannel({})", self.name)
    }
}
