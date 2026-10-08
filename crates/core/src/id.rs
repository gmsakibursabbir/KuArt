//! Generational node ids and the arena that owns nodes.

use std::num::NonZeroU32;

use crate::scene::Node;

/// Stable handle to a node. 8 bytes; `Option<NodeId>` is also 8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    index: u32,
    generation: NonZeroU32,
}

impl NodeId {
    pub fn index(self) -> u32 {
        self.index
    }
}

struct Slot {
    generation: NonZeroU32,
    node: Option<Node>,
}

/// Slot arena with free-list reuse. Reused slots bump their generation so
/// stale ids never alias a new node.
#[derive(Default)]
pub(crate) struct Arena {
    slots: Vec<Slot>,
    free: Vec<u32>,
    len: usize,
}

impl Arena {
    pub fn alloc(&mut self, node: Node) -> NodeId {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.node = Some(node);
            return NodeId {
                index,
                generation: slot.generation,
            };
        }
        let index = u32::try_from(self.slots.len()).expect("more than u32::MAX nodes");
        let generation = NonZeroU32::MIN;
        self.slots.push(Slot {
            generation,
            node: Some(node),
        });
        NodeId { index, generation }
    }

    pub fn free(&mut self, id: NodeId) -> Option<Node> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let node = slot.node.take()?;
        // Skip 0 on wrap-around; a slot that wraps after 4 billion reuses is acceptable.
        slot.generation = slot.generation.checked_add(1).unwrap_or(NonZeroU32::MIN);
        self.free.push(id.index);
        self.len -= 1;
        Some(node)
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.node.as_ref()
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.node.as_mut()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.node.as_ref().map(|n| {
                (
                    NodeId {
                        index: i as u32,
                        generation: s.generation,
                    },
                    n,
                )
            })
        })
    }

    pub fn shrink_to_fit(&mut self) {
        self.slots.shrink_to_fit();
        self.free.shrink_to_fit();
        for slot in &mut self.slots {
            if let Some(n) = &mut slot.node {
                n.children.shrink_to_fit();
            }
        }
    }

    pub fn heap_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot>() + self.free.capacity() * size_of::<u32>()
    }
}
